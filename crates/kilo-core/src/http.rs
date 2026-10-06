//! Minimal blocking HTTPS on the OS's TLS stack. Callers run it on worker
//! threads; the UI thread never blocks on the network.

use std::time::Duration;

use ureq::Agent;
use ureq::tls::{RootCerts, TlsConfig, TlsProvider};

use crate::{Error, Result};

/// Desktop Safari: the same browser identity as the sign-in web view.
pub const USER_AGENT: &str =
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/27.0.1 Safari/605.1.15";

/// Largest response body we accept (browse pages are 0.2–1 MB of JSON).
const MAX_BODY: u64 = 8 * 1024 * 1024;

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
            .user_agent(USER_AGENT)
            .http_status_as_error(false)
            .build()
            .into();
        Http { agent }
    }

    pub fn get(&self, url: &str, headers: &[(&str, &str)]) -> Result<Vec<u8>> {
        let mut req = self.agent.get(url);
        for (k, v) in headers {
            req = req.header(*k, *v);
        }
        read(req.call())
    }

    pub fn post_json(&self, url: &str, headers: &[(&str, &str)], body: &str) -> Result<Vec<u8>> {
        let mut req = self.agent.post(url).header("Content-Type", "application/json");
        for (k, v) in headers {
            req = req.header(*k, *v);
        }
        read(req.send(body))
    }
}

fn read(resp: std::result::Result<ureq::http::Response<ureq::Body>, ureq::Error>) -> Result<Vec<u8>> {
    let mut resp = resp.map_err(|e| Error::Http(e.to_string()))?;
    let status = resp.status().as_u16();
    if status == 401 || status == 403 {
        return Err(Error::SignedOut);
    }
    if status >= 400 {
        return Err(Error::Http(format!("HTTP {status}")));
    }
    resp.body_mut().with_config().limit(MAX_BODY).read_to_vec().map_err(|e| Error::Http(e.to_string()))
}
