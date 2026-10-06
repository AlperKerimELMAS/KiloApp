//! What Kilo shows, distilled from YouTube Music's responses. Strings are
//! `Box<str>`: exact-size, no spare capacity.

/// A page: an optional header, then shelves.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Page {
    pub header: Option<Header>,
    pub sections: Vec<Section>,
    /// Token for more sections (the home feed is paged).
    pub continuation: Option<Box<str>>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Header {
    pub title: Box<str>,
    /// e.g. "Album • 2020".
    pub subtitle: Box<str>,
    /// e.g. the artist, or "10 songs • 40 minutes".
    pub detail: Box<str>,
    pub thumb: Option<Thumb>,
    pub description: Box<str>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Section {
    /// A horizontal shelf or grid of cards.
    Cards { title: Box<str>, entries: Vec<Entry> },
    /// A vertical list of rows (songs, search results).
    List {
        title: Box<str>,
        entries: Vec<Entry>,
        /// Token for more rows (long playlists).
        continuation: Option<Box<str>>,
    },
    /// Plain text, e.g. an artist's biography.
    Text { title: Box<str>, body: Box<str> },
}

impl Section {
    pub fn title(&self) -> &str {
        match self {
            Section::Cards { title, .. } | Section::List { title, .. } | Section::Text { title, .. } => title,
        }
    }

    pub fn entries(&self) -> &[Entry] {
        match self {
            Section::Cards { entries, .. } | Section::List { entries, .. } => entries,
            Section::Text { .. } => &[],
        }
    }
}

/// One card or row: an album, playlist, artist, song or video.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub title: Box<str>,
    pub subtitle: Box<str>,
    pub thumb: Option<Thumb>,
    pub target: Target,
    /// e.g. "3:45" for songs in lists.
    pub duration: Box<str>,
}

impl Entry {
    pub fn video_id(&self) -> Option<&str> {
        match &self.target {
            Target::Play { video_id, .. } => Some(video_id),
            _ => None,
        }
    }
}

/// What activating an entry does.
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    /// Play a song or video, optionally as part of a playlist.
    Play { video_id: Box<str>, playlist_id: Option<Box<str>> },
    /// Start playing a whole playlist or mix.
    PlayPlaylist { playlist_id: Box<str> },
    /// Open a page: album, playlist, artist, mood, chart…
    Browse { id: Box<str>, params: Option<Box<str>>, kind: PageKind },
    /// Not something Kilo can open (yet).
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageKind {
    Album,
    Playlist,
    Artist,
    Other,
}

/// A thumbnail URL that can be asked for at any pixel size, so we only ever
/// download and decode exactly what's drawn.
#[derive(Clone, Debug, PartialEq)]
pub struct Thumb {
    url: Box<str>,
    /// 16:9 video thumbnail rather than square art.
    pub wide: bool,
}

impl Thumb {
    pub fn new(url: &str, wide: bool) -> Self {
        let url = if url.starts_with("//") { format!("https:{url}") } else { url.to_owned() };
        Thumb { url: url.into(), wide }
    }

    /// URL for an image at least `px` pixels on its longer side.
    pub fn sized(&self, px: u32) -> String {
        let url = &*self.url;
        if url.contains("googleusercontent.com/") || url.contains("ggpht.com/") {
            // Google's image CDN resizes on request: `…=w{W}-h{H}-…`.
            let base = url.rsplit_once('=').map_or(url, |(base, _)| base);
            let h = if self.wide { px * 9 / 16 } else { px };
            return format!("{base}=w{px}-h{h}-l90-rj");
        }
        if let Some(rest) = url.split("/vi/").nth(1) {
            // Video thumbnails come in fixed sizes; pick the smallest that fits.
            if let Some(id) = rest.split('/').next() {
                let file = match px {
                    0..=120 => "default.jpg",
                    121..=320 => "mqdefault.jpg",
                    321..=480 => "hqdefault.jpg",
                    _ => "sddefault.jpg",
                };
                return format!("https://i.ytimg.com/vi/{id}/{file}");
            }
        }
        url.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_cdn_images_exactly() {
        let t = Thumb::new("https://lh3.googleusercontent.com/abc=w60-h60-l90-rj", false);
        assert_eq!(t.sized(320), "https://lh3.googleusercontent.com/abc=w320-h320-l90-rj");
        let w = Thumb::new("https://yt3.googleusercontent.com/x=w544-h544-l90-rj", true);
        assert_eq!(w.sized(320), "https://yt3.googleusercontent.com/x=w320-h180-l90-rj");
    }

    #[test]
    fn picks_smallest_fitting_video_thumbnail() {
        let t = Thumb::new("https://i.ytimg.com/vi/wtXqClgroyY/hqdefault.jpg?sqp=abc", true);
        assert_eq!(t.sized(300), "https://i.ytimg.com/vi/wtXqClgroyY/mqdefault.jpg");
        assert_eq!(t.sized(96), "https://i.ytimg.com/vi/wtXqClgroyY/default.jpg");
    }
}
