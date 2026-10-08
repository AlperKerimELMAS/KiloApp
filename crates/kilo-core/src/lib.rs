//! Kilo's cross-platform core: talks to YouTube Music's web API, parses the
//! answers into small models, and manages the play queue. No UI code here;
//! each OS gets its own native front end on top of this crate.

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

/// Seconds since the Unix epoch (0 if the clock is before it).
fn unix_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}
