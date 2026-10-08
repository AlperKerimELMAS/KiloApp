//! What a thumbnail must be before any image decoder sees it: from Google's
//! image servers, over https, and a JPEG, PNG or WebP. Image decoders are a
//! classic way in for attackers; this keeps them to the formats and sources
//! thumbnails actually use.

/// Hosts thumbnails come from (and their subdomains).
const HOSTS: &[&str] = &["googleusercontent.com", "ggpht.com", "ytimg.com"];

/// Whether `url` may be downloaded as a thumbnail.
pub fn trusted_url(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else { return false };
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    // No user names or ports: the host is exactly what's checked.
    !host.contains(['@', ':']) && HOSTS.iter().any(|d| host.strip_suffix(d).is_some_and(|rest| rest.is_empty() || rest.ends_with('.')))
}

/// Whether `bytes` start like a JPEG, PNG or WebP file.
pub fn known_format(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0xFF, 0xD8, 0xFF])
        || bytes.starts_with(b"\x89PNG\r\n\x1a\n")
        || (bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trusts_only_google_image_servers_over_https() {
        for u in [
            "https://lh3.googleusercontent.com/abc=w60-h60",
            "https://yt3.ggpht.com/x=s88",
            "https://i.ytimg.com/vi_webp/abc/mqdefault.webp",
            "https://i9.ytimg.com/vi/abc/hq.jpg?sqp=1",
        ] {
            assert!(trusted_url(u), "{u}");
        }
        for u in [
            "http://i.ytimg.com/vi/abc/hq.jpg",
            "https://i.ytimg.com.evil.io/x.jpg",
            "https://evil.io@i.ytimg.com/x.jpg",
            "https://i.ytimg.com:8443/x.jpg",
            "https://notytimg.com/x.jpg",
            "https://example.com/i.ytimg.com/x.jpg",
            "file:///etc/passwd",
        ] {
            assert!(!trusted_url(u), "{u}");
        }
    }

    #[test]
    fn knows_thumbnail_formats_by_their_bytes() {
        assert!(known_format(b"\xFF\xD8\xFF\xE0rest"));
        assert!(known_format(b"\x89PNG\r\n\x1a\nrest"));
        assert!(known_format(b"RIFF\x10\0\0\0WEBPVP8 "));
        assert!(!known_format(b"%PDF-1.7"));
        assert!(!known_format(b"GIF89a"));
        assert!(!known_format(b"RIFF\x10\0\0\0WAVE"));
        assert!(!known_format(b""));
    }
}
