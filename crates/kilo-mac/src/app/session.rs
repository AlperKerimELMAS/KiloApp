//! Signing in, and connecting to YouTube Music with the saved session.

use std::sync::Arc;

use kilo_core::auth::{Session, parse_binary_cookies};
use kilo_core::client::{Client, Config};
use kilo_core::http::Http;

use super::browse::{Route, go};
use super::{message, show_error, show_loading, show_sign_in, with};
use crate::{login, net, paths};

/// The session WebKit stored the last time the user signed in.
pub(super) fn saved_session() -> Option<Session> {
    let data = std::fs::read(paths::cookies()).ok()?;
    Session::from_cookies(&parse_binary_cookies(&data)?)
}

/// Connects with `session` (reading music.youtube.com's page config, cached
/// for a day), then opens Home.
pub(super) fn start(session: Session) {
    show_loading();
    net::run(
        net::Pool::Api,
        move || {
            let http = Http::new();
            let path = paths::config();
            let config = Config::load(&path).or_else(|| {
                let fresh = Config::fetch(&http, Some(&session)).ok()?;
                let _ = fresh.save(&path);
                Some(fresh)
            })?;
            Some(Client::new(http, session, config))
        },
        |client| match client {
            Some(client) => {
                with(|a| a.client = Some(Arc::new(client)));
                go(Route::Home);
                if let Some(route) = crate::debug::open_route() {
                    go(route);
                }
                crate::debug::run_scenario();
            }
            None => show_error("Couldn't reach YouTube Music. Check your connection."),
        },
    );
}

/// Opens Google's sign-in page in the login helper, and connects once the
/// user has signed in there.
pub fn sign_in() {
    if with(|a| std::mem::replace(&mut a.signing_in, true)).unwrap_or(true) {
        return;
    }
    message("Signing in…", "Finish signing in in the window that just opened.", None);
    let exe = std::env::current_exe().ok();
    net::run(
        net::Pool::Api,
        move || {
            let out = std::process::Command::new(exe?).arg("--login-helper").output().ok()?;
            Some(login::parse_output(&String::from_utf8_lossy(&out.stdout)))
        },
        |cookies| {
            with(|a| a.signing_in = false);
            match cookies.and_then(|c| Session::from_cookies(&c)) {
                Some(session) => start(session),
                None => show_sign_in(),
            }
        },
    );
}
