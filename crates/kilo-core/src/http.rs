//! Minimal blocking HTTPS on macOS's own TLS (Security.framework, through
//! native-tls). Callers run it on worker threads; the UI thread never
//! blocks on the network.

use std::io::Read;
use std::sync::OnceLock;
use std::time::Duration;

use ureq::config::RedirectAuthHeaders;
use ureq::tls::{RootCerts, TlsConfig, TlsProvider};
use ureq::{Agent, ResponseExt};

use crate::{Error, Result};

/// The Safari version Kilo's browser identity names (`set_safari_version`).
static SAFARI: OnceLock<String> = OnceLock::new();
/// When the Mac's own Safari version can't be read.
const DEFAULT_SAFARI: &str = "27.0.1";

/// Names the Mac's own Safari in Kilo's browser identity, for the app's
/// requests and the sign-in window alike. Call once, before any request: a
/// sign-in window claiming a Safari newer than its engine is the kind of
/// mismatch Google turns away ("This browser or app may not be secure").
pub fn set_safari_version(version: &str) {
    if !version.is_empty() && version.len() <= 16 && version.bytes().all(|b| b.is_ascii_digit() || b == b'.') {
        let _ = SAFARI.set(version.to_owned());
    }
}

/// The Safari version in Kilo's browser identity.
pub fn safari_version() -> &'static str {
    SAFARI.get().map_or(DEFAULT_SAFARI, String::as_str)
}

/// Desktop Safari's user agent: the same browser identity as the sign-in
/// web view.
pub fn user_agent() -> String {
    format!(
        "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/{} Safari/605.1.15",
        safari_version()
    )
}

/// Largest response body we accept (browse pages are 0.2–1 MB of JSON).
const MAX_BODY: u64 = 8 * 1024 * 1024;

/// Per-connection buffer, each way. It must hold a response's headers and a
/// request's headers (the cookie header is the largest, a few KB).
const BUFFER: usize = 16 * 1024;

#[derive(Clone)]
pub struct Http {
    agent: Agent,
}

impl Default for Http {
    fn default() -> Self {
        Self::new()
    }
}

impl Http {
    pub fn new() -> Self {
        let tls = TlsConfig::builder().provider(TlsProvider::NativeTls).root_certs(RootCerts::PlatformVerifier).build();
        let agent: Agent = Agent::config_builder()
            .tls_config(tls)
            .timeout_global(Some(Duration::from_secs(20)))
            .user_agent(user_agent())
            // Every request, and every redirect, is https: nothing Kilo
            // fetches (or decodes) travels in the clear.
            .https_only(true)
            // A redirect never carries the session: ureq always drops the
            // Cookie header, and with this the Authorization one too (its
            // default, stated so it stays that way).
            .redirect_auth_headers(RedirectAuthHeaders::Never)
            .http_status_as_error(false)
            // ureq's default is 128 KB each way per pooled connection: 1.5 MB
            // measured. Requests are a few KB and bodies stream through.
            .input_buffer_size(BUFFER)
            .output_buffer_size(BUFFER)
            .max_idle_connections_per_host(2)
            .build()
            .into();
        Http { agent }
    }

    pub fn get(&self, url: &str, headers: &[(&str, &str)]) -> Result<Vec<u8>> {
        read_all(self.call_get(url, headers)?)
    }

    /// Like `get`, for a URL that must stay `trusted` where any redirects
    /// lead: the body is read only from a trusted final URL.
    pub fn get_trusted(&self, url: &str, headers: &[(&str, &str)], trusted: fn(&str) -> bool) -> Result<Vec<u8>> {
        let resp = self.call_get(url, headers)?;
        if !trusted(&resp.get_uri().to_string()) {
            return Err(Error::Http("redirected away from a trusted host".into()));
        }
        read_all(resp)
    }

    /// Only the first `max` bytes of the body; the rest is never downloaded.
    pub fn get_prefix(&self, url: &str, headers: &[(&str, &str)], max: u64) -> Result<Vec<u8>> {
        let mut resp = self.call_get(url, headers)?;
        let mut out = Vec::new();
        resp.body_mut().as_reader().take(max).read_to_end(&mut out).map_err(|e| Error::Http(e.to_string()))?;
        Ok(out)
    }

    fn call_get(&self, url: &str, headers: &[(&str, &str)]) -> Result<Response> {
        let mut req = self.agent.get(url);
        for (k, v) in headers {
            req = req.header(*k, *v);
        }
        checked(req.call())
    }

    pub fn post_json(&self, url: &str, headers: &[(&str, &str)], body: &str) -> Result<Vec<u8>> {
        let mut req = self.agent.post(url).header("Content-Type", "application/json");
        for (k, v) in headers {
            req = req.header(*k, *v);
        }
        read_all(checked(req.send(body))?)
    }
}

type Response = ureq::http::Response<ureq::Body>;

fn checked(resp: std::result::Result<Response, ureq::Error>) -> Result<Response> {
    let resp = resp.map_err(|e| Error::Http(e.to_string()))?;
    let status = resp.status().as_u16();
    if status == 401 || status == 403 {
        return Err(Error::SignedOut);
    }
    if status >= 400 {
        return Err(Error::Status(status));
    }
    Ok(resp)
}

fn read_all(mut resp: Response) -> Result<Vec<u8>> {
    resp.body_mut().with_config().limit(MAX_BODY).read_to_vec().map_err(|e| Error::Http(e.to_string()))
}
