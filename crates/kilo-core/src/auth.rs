//! Sign-in state: the YouTube cookies from the sign-in web view, and the
//! `SAPISIDHASH` request signature YouTube's own web client sends.
//!
//! Kilo never sees the password. The user signs in to Google inside a system
//! web view; the web engine stores the cookies, and the app reads them.

use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug, PartialEq)]
pub struct Cookie {
    pub domain: String,
    pub name: String,
    pub value: String,
    /// Unix seconds; 0 for session cookies.
    pub expires: f64,
}

/// The cookies that apply to music.youtube.com.
#[derive(Clone, Debug, Default)]
pub struct Session {
    cookies: Vec<(String, String)>,
}

impl Session {
    /// Keeps unexpired cookies whose domain matches music.youtube.com.
    pub fn from_cookies(all: &[Cookie]) -> Option<Self> {
        let now = unix_now() as f64;
        let cookies: Vec<(String, String)> = all
            .iter()
            .filter(|c| matches!(c.domain.as_str(), ".youtube.com" | "youtube.com" | "music.youtube.com" | ".music.youtube.com"))
            .filter(|c| c.expires == 0.0 || c.expires > now)
            .map(|c| (c.name.clone(), c.value.clone()))
            .collect();
        let session = Session { cookies };
        session.signed_in().then_some(session)
    }

    pub fn signed_in(&self) -> bool {
        self.get("SAPISID").or_else(|| self.get("__Secure-3PAPISID")).is_some()
    }

    fn get(&self, name: &str) -> Option<&str> {
        self.cookies.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
    }

    pub fn cookie_header(&self) -> String {
        let mut out = String::with_capacity(self.cookies.iter().map(|(n, v)| n.len() + v.len() + 2).sum());
        for (i, (n, v)) in self.cookies.iter().enumerate() {
            if i > 0 {
                out.push_str("; ");
            }
            out.push_str(n);
            out.push('=');
            out.push_str(v);
        }
        out
    }

    /// The `Authorization` header YouTube's web client sends:
    /// `SAPISIDHASH <ts>_<sha1("<ts> <SAPISID> <origin>")>`, plus the 1P/3P
    /// variants when those cookies exist.
    pub fn authorization(&self, origin: &str) -> Option<String> {
        let ts = unix_now();
        let hash = |secret: &str| {
            let digest = sha1_smol::Sha1::from(format!("{ts} {secret} {origin}")).digest().to_string();
            format!("{ts}_{digest}")
        };
        let sapisid = self.get("SAPISID").or_else(|| self.get("__Secure-3PAPISID"))?;
        let mut header = format!("SAPISIDHASH {}", hash(sapisid));
        if let Some(p1) = self.get("__Secure-1PAPISID") {
            header.push_str(&format!(" SAPISID1PHASH {}", hash(p1)));
        }
        if let Some(p3) = self.get("__Secure-3PAPISID") {
            header.push_str(&format!(" SAPISID3PHASH {}", hash(p3)));
        }
        Some(header)
    }
}

fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Parses WebKit's `.binarycookies` file (how WKWebView stores cookies on
/// macOS). Returns `None` if the file is malformed.
pub fn parse_binary_cookies(data: &[u8]) -> Option<Vec<Cookie>> {
    const MAC_EPOCH_OFFSET: f64 = 978_307_200.0; // 2001-01-01 in Unix time
    let be32 = |at: usize| Some(u32::from_be_bytes(data.get(at..at + 4)?.try_into().ok()?) as usize);
    if data.get(..4)? != b"cook" {
        return None;
    }
    let pages = be32(4)?;
    let mut page_at = 8 + pages * 4;
    let mut cookies = Vec::new();
    for p in 0..pages {
        let size = be32(8 + p * 4)?;
        let page = data.get(page_at..page_at + size)?;
        page_at += size;
        let le32 = |at: usize| Some(u32::from_le_bytes(page.get(at..at + 4)?.try_into().ok()?) as usize);
        let count = le32(4)?;
        for c in 0..count {
            let start = le32(8 + c * 4)?;
            let rec = page.get(start..start + le32(start)?)?;
            let r32 = |at: usize| Some(u32::from_le_bytes(rec.get(at..at + 4)?.try_into().ok()?) as usize);
            let f64_at = |at: usize| Some(f64::from_le_bytes(rec.get(at..at + 8)?.try_into().ok()?));
            let string = |at: usize| {
                let bytes = rec.get(at..)?;
                let end = bytes.iter().position(|&b| b == 0)?;
                std::str::from_utf8(&bytes[..end]).ok().map(str::to_owned)
            };
            let expiry = f64_at(40)?;
            cookies.push(Cookie {
                domain: string(r32(16)?)?,
                name: string(r32(20)?)?,
                value: string(r32(28)?)?,
                expires: if expiry > 0.0 { expiry + MAC_EPOCH_OFFSET } else { 0.0 },
            });
        }
    }
    Some(cookies)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signs_like_the_web_client() {
        let s = Session { cookies: vec![("SAPISID".into(), "abc".into())] };
        let h = s.authorization("https://music.youtube.com").unwrap();
        let (ts, digest) = h.strip_prefix("SAPISIDHASH ").unwrap().split_once('_').unwrap();
        let expect = sha1_smol::Sha1::from(format!("{ts} abc https://music.youtube.com")).digest().to_string();
        assert_eq!(digest, expect);
    }

    #[test]
    fn rejects_garbage_cookie_files() {
        assert!(parse_binary_cookies(b"nope").is_none());
        assert!(parse_binary_cookies(b"cook\x00\x00\x00\x01\x00\x00\x10\x00").is_none());
    }
}
