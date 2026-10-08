//! Kilo's cross-platform core: talks to YouTube Music's web API, parses the
//! answers into small models, and manages the play queue. No UI code here,
//! but what every front end shares is: the words (`strings`), the colors
//! (`style`) and the keyboard shortcuts (`shortcuts`). Each OS gets its own
//! native front end on top of this crate.

pub mod auth;
pub mod client;
pub mod http;
pub mod image;
pub mod model;
pub mod parse;
pub mod queue;
pub mod shortcuts;
pub mod strings;
pub mod style;

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
/// renamed into place: a reader, or a crash, never sees half a file.
pub fn write_atomically(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let (Some(dir), Some(name)) = (path.parent(), path.file_name()) else {
        return Err(std::io::Error::other("not a file path"));
    };
    std::fs::create_dir_all(dir)?;
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp = dir.join(format!(".{}.{}-{n}.tmp", name.to_string_lossy(), std::process::id()));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
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
