//! Sign-in state: the YouTube cookies from the sign-in web view, and the
//! `SAPISIDHASH` request signature YouTube's own web client sends.
//!
//! Kilo never sees the password. The user signs in to Google inside a system
//! web view; the web engine stores the cookies, and the app reads them.

use crate::unix_now;

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

    /// Tells sign-ins apart (each has its own `SAPISID`) without saying
    /// anything about the cookie: what's cached for one is never shown to
    /// another.
    pub fn fingerprint(&self) -> u64 {
        let sapisid = self.get("SAPISID").or_else(|| self.get("__Secure-3PAPISID")).unwrap_or("");
        crate::fnv1a(format!("kilo-session\n{sapisid}").as_bytes())
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

/// Whether a cookie's domain is youtube.com or one of its subdomains (an
/// exact match: `notyoutube.com` isn't).
pub fn is_youtube_domain(domain: &str) -> bool {
    let domain = domain.strip_prefix('.').unwrap_or(domain);
    under(domain, "youtube.com")
}

/// Whether the sign-in window may show `host`: Google's sign-in and the
/// YouTube pages it leads back to (any google.com or youtube.com host), and
/// Google's country domains, which sign-in may pass through to set their
/// cookies (`accounts.google.com.br`, say). Anything else opens in the
/// browser: the window has no address bar, so it never shows a page the
/// user can't tell apart from Google's.
pub fn is_sign_in_host(host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    if under(&host, "google.com") || under(&host, "youtube.com") {
        return true;
    }
    let Some(tld) = host.strip_prefix("accounts.google.") else { return false };
    let country = tld.strip_prefix("co.").or_else(|| tld.strip_prefix("com.")).unwrap_or(tld);
    country.len() == 2 && country.bytes().all(|b| b.is_ascii_lowercase())
}

/// `host` is `domain` or a subdomain of it.
fn under(host: &str, domain: &str) -> bool {
    host.strip_suffix(domain).is_some_and(|rest| rest.is_empty() || rest.ends_with('.'))
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
    fn matches_youtube_domains_exactly() {
        for d in [".youtube.com", "youtube.com", "music.youtube.com", ".music.youtube.com"] {
            assert!(is_youtube_domain(d), "{d}");
        }
        for d in ["notyoutube.com", ".youtube.com.evil.io", "youtube.co", ""] {
            assert!(!is_youtube_domain(d), "{d}");
        }
    }

    #[test]
    fn sign_in_stays_on_google_and_youtube() {
        for h in [
            "accounts.google.com",
            "Accounts.Google.com",
            "google.com",
            "accounts.youtube.com",
            "music.youtube.com",
            "accounts.google.com.br",
            "accounts.google.co.uk",
            "accounts.google.de",
        ] {
            assert!(is_sign_in_host(h), "{h}");
        }
        for h in [
            "accounts.google.com.evil.io",
            "evilgoogle.com",
            "google.com.evil.io",
            "accounts.google.evil",
            "accounts.google.co.evil",
            "login.example.com",
            "",
        ] {
            assert!(!is_sign_in_host(h), "{h}");
        }
    }

    #[test]
    fn rejects_garbage_cookie_files() {
        assert!(parse_binary_cookies(b"nope").is_none());
        assert!(parse_binary_cookies(b"cook\x00\x00\x00\x01\x00\x00\x10\x00").is_none());
    }
}
