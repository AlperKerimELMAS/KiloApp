//! Line protocol between the app and the player helper.
//!
//! One message per line, words separated by single spaces. Hand-rolled
//! instead of JSON: it's six commands and seven events.

/// App → helper.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    /// Start playing a track at a position in seconds (replaces the current
    /// track).
    Load(VideoId, f64),
    Play,
    Pause,
    /// Seek to a position in seconds.
    Seek(f64),
    /// Volume, 0–100.
    Volume(u8),
    Quit,
}

/// Helper → app.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// The helper started and the account is signed in.
    Ready,
    Playing(Position),
    Paused(Position),
    Buffering(Position),
    Ended(Position),
    /// Not signed in: playback is Premium-only, so the helper exits.
    SignedOut,
    /// An ad showed up, which means the account isn't Premium. The helper
    /// pauses, and never plays ads unseen.
    AdShowing,
    /// The "next" media key or Control Center button.
    Next,
    /// The "previous" media key or Control Center button.
    Previous,
    Error(String),
    /// The helper is gone (quit, crashed or killed). Delivered last, by the
    /// host itself once the helper's output ends: never sent by the helper,
    /// so nothing the helper's page says can fake it.
    Exited,
}

/// Where playback is. The app extrapolates from this while playing instead
/// of polling, so a playing track costs no periodic wakeups.
#[derive(Clone, Debug, PartialEq)]
pub struct Position {
    pub seconds: f64,
    pub duration: f64,
    pub video: VideoId,
}

/// An 11-character YouTube video id. Validated, so it's always safe to put
/// into a URL or a script string.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VideoId(String);

impl VideoId {
    pub fn parse(s: &str) -> Option<Self> {
        (s.len() == 11 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')).then(|| VideoId(s.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for VideoId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl Command {
    pub fn encode(&self) -> String {
        match self {
            Command::Load(id, start) => format!("load {id} {start:.2}"),
            Command::Play => "play".into(),
            Command::Pause => "pause".into(),
            Command::Seek(s) => format!("seek {s:.2}"),
            Command::Volume(v) => format!("volume {v}"),
            Command::Quit => "quit".into(),
        }
    }

    pub fn parse(line: &str) -> Option<Self> {
        let mut words = line.split(' ');
        let cmd = match words.next()? {
            "load" => Command::Load(VideoId::parse(words.next()?)?, seconds(words.next()?)?),
            "play" => Command::Play,
            "pause" => Command::Pause,
            "seek" => Command::Seek(seconds(words.next()?)?),
            "volume" => Command::Volume(words.next()?.parse().ok().filter(|v| *v <= 100)?),
            "quit" => Command::Quit,
            _ => return None,
        };
        words.next().is_none().then_some(cmd)
    }
}

/// A finite, non-negative number of seconds.
fn seconds(s: &str) -> Option<f64> {
    s.parse().ok().filter(|v: &f64| v.is_finite() && *v >= 0.0)
}

impl Event {
    pub fn encode(&self) -> String {
        let pos = |name: &str, p: &Position| format!("{name} {:.2} {:.2} {}", p.seconds, p.duration, p.video);
        match self {
            Event::Ready => "ready".into(),
            Event::Playing(p) => pos("playing", p),
            Event::Paused(p) => pos("paused", p),
            Event::Buffering(p) => pos("buffering", p),
            Event::Ended(p) => pos("ended", p),
            Event::SignedOut => "signed-out".into(),
            Event::AdShowing => "ad".into(),
            Event::Next => "next".into(),
            Event::Previous => "previous".into(),
            Event::Error(msg) => format!("error {}", msg.replace('\n', " ")),
            Event::Exited => "exited".into(),
        }
    }

    pub fn parse(line: &str) -> Option<Self> {
        let (name, rest) = line.split_once(' ').unwrap_or((line, ""));
        let pos = || {
            let mut w = rest.split(' ');
            let (at, duration) = (seconds(w.next()?)?, seconds(w.next()?)?);
            let video = VideoId::parse(w.next()?)?;
            Some(Position { seconds: at, duration, video })
        };
        Some(match name {
            "ready" => Event::Ready,
            "playing" => Event::Playing(pos()?),
            "paused" => Event::Paused(pos()?),
            "buffering" => Event::Buffering(pos()?),
            "ended" => Event::Ended(pos()?),
            "signed-out" => Event::SignedOut,
            "ad" => Event::AdShowing,
            "next" => Event::Next,
            "previous" => Event::Previous,
            "error" => Event::Error(rest.to_owned()),
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_round_trip() {
        let id = VideoId::parse("lYBUbBu4W08").unwrap();
        for c in [Command::Load(id, 42.5), Command::Play, Command::Pause, Command::Seek(12.5), Command::Volume(80), Command::Quit] {
            assert_eq!(Command::parse(&c.encode()), Some(c));
        }
    }

    #[test]
    fn events_round_trip() {
        let p = Position { seconds: 1.25, duration: 213.0, video: VideoId::parse("lYBUbBu4W08").unwrap() };
        for e in [
            Event::Ready,
            Event::Playing(p.clone()),
            Event::Paused(p.clone()),
            Event::Buffering(p.clone()),
            Event::Ended(p),
            Event::SignedOut,
            Event::AdShowing,
            Event::Next,
            Event::Previous,
            Event::Error("media failed".into()),
        ] {
            assert_eq!(Event::parse(&e.encode()), Some(e));
        }
        // Only the host knows when the helper is gone.
        assert_eq!(Event::parse(&Event::Exited.encode()), None);
    }

    #[test]
    fn rejects_unsafe_input() {
        assert_eq!(VideoId::parse("abc'); x();"), None);
        assert_eq!(Command::parse("load lYBUbBu4W0' 0"), None);
        assert_eq!(Command::parse("load lYBUbBu4W08"), None);
        assert_eq!(Command::parse("seek -1"), None);
        assert_eq!(Command::parse("seek NaN"), None);
        assert_eq!(Command::parse("volume 101"), None);
        assert_eq!(Command::parse("play now"), None);
        assert_eq!(Event::parse("playing NaN 200.00 lYBUbBu4W08"), None);
        assert_eq!(Event::parse("paused 1.00 inf lYBUbBu4W08"), None);
    }
}
