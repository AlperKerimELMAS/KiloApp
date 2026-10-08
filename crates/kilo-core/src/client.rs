//! YouTube Music's web API ("InnerTube"), spoken the way music.youtube.com
//! speaks it: same client name and version, same headers, the user's own
//! signed-in session. Responses are returned raw; `parse` turns them into
//! models and drops them.

use std::path::Path;
use std::sync::Mutex;

use serde_json::json;

use crate::auth::Session;
use crate::http::Http;
use crate::{Error, Result, unix_now};

const ORIGIN: &str = "https://music.youtube.com";
const API: &str = "https://music.youtube.com/youtubei/v1/";
/// Used only if the live page can't be read; refreshed from the page daily.
const FALLBACK_VERSION: &str = "1.20261004.17.00";
const CONFIG_MAX_AGE_SECS: u64 = 24 * 60 * 60;
/// How much of music.youtube.com's page to read for its config.
const CONFIG_PREFIX: u64 = 64 * 1024;

/// What music.youtube.com's page tells its own client to use.
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub version: String,
    pub visitor: String,
    pub hl: String,
    pub gl: String,
    /// Unix seconds when this was read from the page.
    pub fetched: u64,
}

impl Config {
    /// Reads music.youtube.com's page config (client version, visitor id,
    /// language and region).
    pub fn fetch(http: &Http, session: Option<&Session>) -> Result<Self> {
        let cookie = session.map(Session::cookie_header).unwrap_or_default();
        let mut headers = vec![("Accept-Language", "en-US,en;q=0.9")];
        if !cookie.is_empty() {
            headers.push(("Cookie", &cookie));
        }
        // The page is ~550 KB, but its config sits in the first ~25 KB.
        let head = http.get_prefix(ORIGIN, &headers, CONFIG_PREFIX)?;
        if let [Some(version), Some(visitor), Some(hl), Some(gl)] = Self::fields(&head) {
            return Ok(Config { version, visitor, hl, gl, fetched: unix_now() });
        }
        let [version, visitor, hl, gl] = Self::fields(&http.get(ORIGIN, &headers)?);
        Ok(Config {
            version: version.unwrap_or_else(|| FALLBACK_VERSION.into()),
            visitor: visitor.unwrap_or_default(),
            hl: hl.unwrap_or_else(|| "en".into()),
            gl: gl.unwrap_or_else(|| "US".into()),
            fetched: unix_now(),
        })
    }

    /// Client version, visitor id, language and region, as found in the page.
    fn fields(page: &[u8]) -> [Option<String>; 4] {
        let page = String::from_utf8_lossy(page);
        let field = |key: &str| {
            let pat = format!("\"{key}\":\"");
            let start = page.find(&pat)? + pat.len();
            let len = page[start..].find('"')?;
            Some(page[start..start + len].to_owned())
        };
        ["INNERTUBE_CLIENT_VERSION", "VISITOR_DATA", "HL", "GL"].map(field)
    }

    /// Whether this config is older than a day (time to read the page again).
    pub fn is_stale(&self) -> bool {
        unix_now().saturating_sub(self.fetched) >= CONFIG_MAX_AGE_SECS
    }

    /// Loads a cached config, however old: a day-old client version still
    /// works, so startup never waits for the page (see `is_stale`).
    pub fn load_any(path: &Path) -> Option<Self> {
        let text = std::fs::read_to_string(path).ok()?;
        let mut lines = text.lines();
        Some(Config {
            version: lines.next()?.to_owned(),
            visitor: lines.next()?.to_owned(),
            hl: lines.next()?.to_owned(),
            gl: lines.next()?.to_owned(),
            fetched: lines.next()?.parse().ok()?,
        })
    }

    /// Saves it for the next launch, atomically: a reader sees the old file
    /// or the new one, never half of one.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let text = format!("{}\n{}\n{}\n{}\n{}\n", self.version, self.visitor, self.hl, self.gl, self.fetched);
        crate::write_atomically(path, text.as_bytes())
    }
}

#[derive(Clone)]
pub struct Client {
    http: Http,
    session: Session,
    config: Config,
    masks: bool,
}

/// Field masks the server turned down (it validates every field name, so
/// a renamed field would): requests that would carry one go without it.
static REJECTED_MASKS: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());

impl Client {
    pub fn new(http: Http, session: Session, config: Config) -> Self {
        Client { http, session, config, masks: true }
    }

    /// The same client without field masks (developer tools: comparing
    /// full and masked responses).
    pub fn unmasked(mut self) -> Self {
        self.masks = false;
        self
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn session(&self) -> &Session {
        &self.session
    }

    /// The same client asking for content in language `hl` (`"en"`, `"tr"`…)
    /// instead of the account's.
    pub fn with_language(&self, hl: &str) -> Self {
        let mut client = self.clone();
        client.config.hl = hl.to_owned();
        client
    }

    /// The signed-in account's name, handle and photo.
    pub fn account(&self) -> Result<Vec<u8>> {
        self.call("account/account_menu", json!({}), Some(crate::parse::account_mask()))
    }

    pub fn browse(&self, browse_id: &str, params: Option<&str>) -> Result<Vec<u8>> {
        let mut body = json!({ "browseId": browse_id });
        if let Some(p) = params {
            body["params"] = p.into();
        }
        self.call("browse", body, None)
    }

    /// The next page of a long list (home feed, playlist tracks).
    pub fn continuation(&self, token: &str) -> Result<Vec<u8>> {
        self.call("browse", json!({ "continuation": token }), None)
    }

    pub fn search(&self, query: &str, params: Option<&str>) -> Result<Vec<u8>> {
        let mut body = json!({ "query": query });
        if let Some(p) = params {
            body["params"] = p.into();
        }
        self.call("search", body, None)
    }

    /// "Up next" for a track: its radio, or the rest of the playlist it was
    /// started from.
    pub fn next(&self, video_id: &str, playlist_id: Option<&str>) -> Result<Vec<u8>> {
        let playlist = playlist_id.map_or_else(|| Self::radio_id(video_id), str::to_owned);
        self.call(
            "next",
            json!({
                "videoId": video_id,
                "playlistId": playlist,
                "isAudioOnly": true,
                "enablePersistentPlaylistPanel": true,
                "tunerSettingValue": "AUTOMIX_SETTING_NORMAL",
            }),
            Some(crate::parse::next_mask()),
        )
    }

    /// "Up next" for one track alone: the track as YouTube Music would play
    /// it, with its song version when it's a music video. About 30 KB,
    /// where a radio (`next`) is about 800 KB.
    pub fn next_single(&self, video_id: &str) -> Result<Vec<u8>> {
        self.call("next", json!({ "videoId": video_id, "isAudioOnly": true }), Some(crate::parse::next_mask()))
    }

    /// More of an endless queue (a mix, a radio) that `playlist_id` started:
    /// `token` comes from the previous part (`parse::UpNext`).
    pub fn next_continuation(&self, playlist_id: &str, token: &str) -> Result<Vec<u8>> {
        self.call(
            "next",
            json!({
                "playlistId": playlist_id,
                "continuation": token,
                "isAudioOnly": true,
                "enablePersistentPlaylistPanel": true,
            }),
            Some(crate::parse::next_more_mask()),
        )
    }

    /// The radio playlist of a track, as `next` asks for it.
    pub fn radio_id(video_id: &str) -> String {
        format!("RDAMVM{video_id}")
    }

    /// The queue for playing a whole playlist or mix from its start.
    pub fn next_playlist(&self, playlist_id: &str) -> Result<Vec<u8>> {
        self.call(
            "next",
            json!({
                "playlistId": playlist_id,
                "isAudioOnly": true,
                "enablePersistentPlaylistPanel": true,
            }),
            Some(crate::parse::next_mask()),
        )
    }

    /// Any endpoint with any body (developer tools).
    pub fn raw(&self, endpoint: &str, body: &str) -> Result<Vec<u8>> {
        self.call(endpoint, serde_json::from_str(body).map_err(|e| Error::Parse(e.to_string()))?, None)
    }

    /// Calls `endpoint`, with a field mask if given, so the answer holds only
    /// what Kilo parses. If the server rejects the mask, the request goes
    /// again without it.
    fn call(&self, endpoint: &str, mut body: serde_json::Value, mask: Option<&'static str>) -> Result<Vec<u8>> {
        body["context"] = json!({
            "client": {
                "clientName": "WEB_REMIX",
                "clientVersion": self.config.version,
                "hl": self.config.hl,
                "gl": self.config.gl,
                "visitorData": self.config.visitor,
                "platform": "DESKTOP",
            },
            "user": { "lockedSafetyMode": false },
        });
        let cookie = self.session.cookie_header();
        let auth = self.session.authorization(ORIGIN).ok_or(Error::SignedOut)?;
        let headers = [
            ("Origin", ORIGIN),
            ("X-Origin", ORIGIN),
            ("Referer", "https://music.youtube.com/"),
            ("Cookie", cookie.as_str()),
            ("Authorization", auth.as_str()),
            ("X-Goog-AuthUser", "0"),
            ("X-Goog-Visitor-Id", self.config.visitor.as_str()),
            ("X-Youtube-Client-Name", "67"),
            ("X-Youtube-Client-Version", self.config.version.as_str()),
        ];
        let url = format!("{API}{endpoint}?prettyPrint=false");
        let body = body.to_string();
        let rejected = |mask: &str| REJECTED_MASKS.lock().is_ok_and(|r| r.contains(&mask));
        if let Some(mask) = mask.filter(|m| self.masks && !rejected(m)) {
            let mut masked = headers.to_vec();
            masked.push(("X-Goog-FieldMask", mask));
            match self.http.post_json(&url, &masked, &body) {
                Err(Error::Status(400)) => {
                    if let Ok(mut r) = REJECTED_MASKS.lock() {
                        r.push(mask);
                    }
                    eprintln!("kilo: {endpoint} field mask rejected; sending full requests");
                }
                result => return result,
            }
        }
        self.http.post_json(&url, &headers, &body)
    }
}
