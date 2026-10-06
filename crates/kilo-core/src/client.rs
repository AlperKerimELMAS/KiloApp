//! YouTube Music's web API ("InnerTube"), spoken the way music.youtube.com
//! speaks it: same client name and version, same headers, the user's own
//! signed-in session. Responses are returned raw; `parse` turns them into
//! models and drops them.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::json;

use crate::auth::Session;
use crate::http::Http;
use crate::{Error, Result};

const ORIGIN: &str = "https://music.youtube.com";
const API: &str = "https://music.youtube.com/youtubei/v1/";
/// Used only if the live page can't be read; refreshed from the page daily.
const FALLBACK_VERSION: &str = "1.20261004.17.00";
const CONFIG_MAX_AGE_SECS: u64 = 24 * 60 * 60;

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
        let page = http.get(ORIGIN, &headers)?;
        let page = String::from_utf8_lossy(&page);
        let field = |key: &str| {
            let pat = format!("\"{key}\":\"");
            let start = page.find(&pat)? + pat.len();
            let len = page[start..].find('"')?;
            Some(page[start..start + len].to_owned())
        };
        Ok(Config {
            version: field("INNERTUBE_CLIENT_VERSION").unwrap_or_else(|| FALLBACK_VERSION.into()),
            visitor: field("VISITOR_DATA").unwrap_or_default(),
            hl: field("HL").unwrap_or_else(|| "en".into()),
            gl: field("GL").unwrap_or_else(|| "US".into()),
            fetched: unix_now(),
        })
    }

    /// Loads a cached config if it's fresh enough.
    pub fn load(path: &Path) -> Option<Self> {
        let text = std::fs::read_to_string(path).ok()?;
        let mut lines = text.lines();
        let config = Config {
            version: lines.next()?.to_owned(),
            visitor: lines.next()?.to_owned(),
            hl: lines.next()?.to_owned(),
            gl: lines.next()?.to_owned(),
            fetched: lines.next()?.parse().ok()?,
        };
        (unix_now().saturating_sub(config.fetched) < CONFIG_MAX_AGE_SECS).then_some(config)
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, format!("{}\n{}\n{}\n{}\n{}\n", self.version, self.visitor, self.hl, self.gl, self.fetched))
    }
}

#[derive(Clone)]
pub struct Client {
    http: Http,
    session: Session,
    config: Config,
}

impl Client {
    pub fn new(http: Http, session: Session, config: Config) -> Self {
        Client { http, session, config }
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn browse(&self, browse_id: &str, params: Option<&str>) -> Result<Vec<u8>> {
        let mut body = json!({ "browseId": browse_id });
        if let Some(p) = params {
            body["params"] = p.into();
        }
        self.call("browse", body)
    }

    /// The next page of a long list (home feed, playlist tracks).
    pub fn continuation(&self, token: &str) -> Result<Vec<u8>> {
        self.call("browse", json!({ "continuation": token }))
    }

    pub fn search(&self, query: &str, params: Option<&str>) -> Result<Vec<u8>> {
        let mut body = json!({ "query": query });
        if let Some(p) = params {
            body["params"] = p.into();
        }
        self.call("search", body)
    }

    pub fn search_suggestions(&self, input: &str) -> Result<Vec<u8>> {
        self.call("music/get_search_suggestions", json!({ "input": input }))
    }

    /// "Up next" for a track: its radio, or the rest of the playlist it was
    /// started from.
    pub fn next(&self, video_id: &str, playlist_id: Option<&str>) -> Result<Vec<u8>> {
        let playlist = playlist_id.map_or_else(|| format!("RDAMVM{video_id}"), str::to_owned);
        self.call(
            "next",
            json!({
                "videoId": video_id,
                "playlistId": playlist,
                "isAudioOnly": true,
                "enablePersistentPlaylistPanel": true,
                "tunerSettingValue": "AUTOMIX_SETTING_NORMAL",
            }),
        )
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
        )
    }

    fn call(&self, endpoint: &str, mut body: serde_json::Value) -> Result<Vec<u8>> {
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
        self.http.post_json(&format!("{API}{endpoint}?prettyPrint=false"), &headers, &body.to_string())
    }
}

fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}
