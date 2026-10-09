//! Kilo's core: talks to YouTube Music's web API, parses the answers into
//! small models, and manages the play queue. No UI code here: the app
//! (`kilo-mac`) is built on top of this crate.

pub mod auth;
pub mod client;
pub mod http;
pub mod image;
pub mod model;
pub mod parse;
pub mod queue;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub enum Error {
    /// No usable sign-in cookies, or the server turned them down.
    SignedOut,
    /// The request didn't complete (offline, timeout, TLS…).
    Http(String),
    /// The server answered with this error status.
    Status(u16),
    /// The response didn't have the shape we expect.
    Parse(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::SignedOut => f.write_str("not signed in"),
            Error::Http(e) => write!(f, "network error: {e}"),
            Error::Status(code) => write!(f, "network error: HTTP {code}"),
            Error::Parse(e) => write!(f, "unexpected response: {e}"),
        }
    }
}

impl std::error::Error for Error {}

/// Writes `bytes` to `path` (creating its folder) through a temporary file
/// renamed into place: a reader, or a crash, never sees half a file. Only
/// the user can read what Kilo writes (files 0600, new folders 0700): it's
/// their account's pages and settings.
pub fn write_atomically(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let (Some(dir), Some(name)) = (path.parent(), path.file_name()) else {
        return Err(std::io::Error::other("not a file path"));
    };
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp = dir.join(format!(".{}.{}-{n}.tmp", name.to_string_lossy(), std::process::id()));
    // A leftover from a crash (same pid and number) would keep its mode.
    let _ = std::fs::remove_file(&tmp);
    let written = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&tmp)?.write_all(bytes);
    written.and_then(|()| std::fs::rename(&tmp, path)).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

/// FNV-1a, 64-bit: stable, dependency-free names for cache files.
pub fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3))
}

/// Seconds since the Unix epoch (0 if the clock is before it).
fn unix_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn writes_are_for_the_user_only() {
        let dir = std::env::temp_dir().join(format!("kilo-write-test-{}", std::process::id()));
        let file = dir.join("new/page");
        super::write_atomically(&file, b"one").unwrap();
        super::write_atomically(&file, b"two").unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), b"two");
        let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&file), 0o600);
        assert_eq!(mode(file.parent().unwrap()), 0o700);
        // Nothing left behind but the file.
        assert_eq!(std::fs::read_dir(file.parent().unwrap()).unwrap().count(), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
