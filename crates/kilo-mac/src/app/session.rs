//! Signing in and out, and connecting to YouTube Music with the saved
//! session.

use std::sync::Arc;
use std::time::Duration;

use kilo_core::auth::{Session, is_youtube_domain, parse_binary_cookies};
use kilo_core::client::{Client, Config};
use kilo_core::http::Http;
use kilo_core::queue::Queue;

use super::browse::{Route, go};
use super::{message, player, show_error, show_loading, show_sign_in, with, with_shell};
use crate::{images, login, net, paths};

/// At launch: connects with the saved session. If an older Kilo kept
/// Google's account cookies next to YouTube's, the prune helper deletes
/// them first.
pub(super) fn resume() {
    let saved = std::fs::read(paths::cookies()).ok().and_then(|data| parse_binary_cookies(&data));
    let Some((cookies, session)) = saved.and_then(|c| Session::from_cookies(&c).map(|s| (c, s))) else { return show_sign_in() };
    if cookies.iter().all(|c| is_youtube_domain(&c.domain)) {
        return start(session);
    }
    let Some(id) = with(|a| a.session) else { return };
    show_loading();
    let exe = std::env::current_exe().ok();
    net::run(
        net::Pool::Api,
        move || {
            let pruned = exe.is_some_and(|exe| std::process::Command::new(exe).arg("--prune-helper").status().is_ok_and(|s| s.success()));
            paths::remove_cookie_copies();
            pruned
        },
        move |pruned| {
            crate::debug::trace(|| format!("session: Google's account cookies {}", if pruned { "deleted" } else { "NOT deleted" }));
            if with(|a| a.session) == Some(id) {
                start(session);
            }
        },
    );
}

/// The session WebKit stored the last time the user signed in.
pub(super) fn saved_session() -> Option<Session> {
    let data = std::fs::read(paths::cookies()).ok()?;
    Session::from_cookies(&parse_binary_cookies(&data)?)
}

/// Connects with `session` (reading music.youtube.com's page config, cached
/// for a day), then opens Home.
pub(super) fn start(session: Session) {
    let Some(id) = with(|a| a.session) else { return };
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
        move |client| {
            if with(|a| a.session) != Some(id) {
                return; // signed out meanwhile
            }
            match client {
                Some(client) => {
                    with(|a| a.client = Some(Arc::new(client)));
                    go(Route::Home);
                    if let Some(route) = crate::debug::open_route() {
                        go(route);
                    }
                    crate::debug::run_scenario();
                }
                None => show_error("Couldn't reach YouTube Music. Check your connection."),
            }
        },
    );
}

/// Opens Google's sign-in page in the login helper, and connects once the
/// user has signed in there.
pub fn sign_in() {
    if with(|a| std::mem::replace(&mut a.account_busy, true)).unwrap_or(true) {
        return;
    }
    message("Signing in…", "Finish signing in in the window that just opened.", None);
    let exe = std::env::current_exe().ok();
    net::run(
        net::Pool::Api,
        move || {
            let out = std::process::Command::new(exe?).arg("--login-helper").output().ok()?;
            let cookies = login::parse_output(&String::from_utf8_lossy(&out.stdout));
            login::wait_until_saved(&cookies);
            Some(cookies)
        },
        |cookies| {
            with(|a| a.account_busy = false);
            match cookies.and_then(|c| Session::from_cookies(&c)) {
                Some(session) => start(session),
                None => show_sign_in(),
            }
        },
    );
}

/// Forgets the account on this Mac: stops playback, has a helper delete
/// everything WebKit stores for Kilo (the session), deletes what Kilo wrote
/// itself (page config, caches), and shows the sign-in screen.
pub fn sign_out() {
    let Some(player) = with(|a| {
        if std::mem::replace(&mut a.account_busy, true) {
            return Err(()); // a sign-in or sign-out is under way
        }
        a.session += 1;
        a.client = None;
        a.history.clear();
        a.generation += 1;
        a.page = None;
        a.view = None;
        a.items.clear();
        a.fetching.clear();
        a.queue = Queue::default();
        a.queue_id += 1;
        a.play_token += 1;
        a.follow = None;
        a.ran_out = false;
        a.radio_for = None;
        a.playing = false;
        a.position = 0.0;
        a.duration = 0.0;
        a.idle_token += 1;
        a.idle_armed = false;
        Ok(a.player.take())
    })
    .and_then(Result::ok) else {
        return;
    };
    player::clear_bar();
    player::sync_timer();
    with_shell(|s| s.back.setEnabled(false));
    images::purge_memory();
    message("Signing out…", "", None);
    let exe = std::env::current_exe().ok();
    net::run(
        net::Pool::Api,
        move || {
            // The helper's WebKit must be gone first, or it could write
            // the session back.
            if let Some(player) = player {
                player.quit(Duration::from_secs(2));
            }
            let cleared =
                exe.is_some_and(|exe| std::process::Command::new(exe).arg("--sign-out-helper").status().is_ok_and(|s| s.success()));
            paths::remove_own_data();
            cleared
        },
        |cleared| {
            with(|a| a.account_busy = false);
            if cleared {
                show_sign_in();
            } else {
                message("Couldn't sign out completely", "Kilo couldn't delete all of its web data. Choose Sign Out again to retry.", None);
            }
        },
    );
}
