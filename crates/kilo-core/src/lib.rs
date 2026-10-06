//! Kilo's cross-platform core: talks to YouTube Music's web API, parses the
//! answers into small models, and manages the play queue. No UI code here;
//! each OS gets its own native front end on top of this crate.

pub mod auth;
pub mod client;
pub mod http;
pub mod model;
pub mod parse;
pub mod queue;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub enum Error {
    /// No usable sign-in cookies.
    SignedOut,
    Http(String),
    /// The response didn't have the shape we expect.
    Parse(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::SignedOut => f.write_str("not signed in"),
            Error::Http(e) => write!(f, "network error: {e}"),
            Error::Parse(e) => write!(f, "unexpected response: {e}"),
        }
    }
}

impl std::error::Error for Error {}
