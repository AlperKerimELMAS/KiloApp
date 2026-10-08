//! Signing in and out, who's signed in, and connecting to YouTube Music with
//! the saved session.

use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use kilo_core::auth::{Session, is_youtube_domain, parse_binary_cookies};
use kilo_core::client::{Client, Config};
use kilo_core::http::Http;
use kilo_core::parse;
use kilo_core::queue::Queue;
use objc2::sel;

use super::browse::{Route, go};
use super::{
    Screen, message, player, rebuild_window, refresh_menu, reload, show_error, show_loading, show_sign_in, show_sign_out_failed,
    show_signing_out, with,
};
use crate::strings::{S, t};
use crate::{images, login, net, paths, settings, ui};

/// The login helper while it's open (so Cancel can close it).
static LOGIN: Mutex<Option<Child>> = Mutex::new(None);

/// At launch: connects with the saved session. If an older Kilo kept
/// Google's account cookies next to YouTube's, the prune helper deletes
/// them first.
pub(super) fn resume() {
    let saved = std::fs::read(paths::cookies()).ok().and_then(|data| parse_binary_cookies(&data));
    let Some((cookies, session)) = saved.and_then(|c| Session::from_cookies(&c).map(|s| (c, s))) else {
        show_sign_in();
        return crate::debug::run_scenario();
    };
    if cookies.iter().all(|c| is_youtube_domain(&c.domain)) {
        return start(session);
    }
    let Some(id) = with(|a| a.session) else { return };
    show_loading();
    let exe = std::env::current_exe().ok();
    net::run(
        net::Pool::Api,
        move || {
            let pruned = exe.is_some_and(|exe| Command::new(exe).arg("--prune-helper").status().is_ok_and(|s| s.success()));
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

/// Connects with `session`, then opens Home. The page config comes from its
/// cache even when it's a day old (then it's refreshed in the background),
/// so startup only waits for the page config the very first time.
pub(super) fn start(session: Session) {
    let Some(id) = with(|a| a.session) else { return };
    let hl = settings::content_language();
    show_loading();
    net::run(
        net::Pool::Api,
        move || {
            let http = Http::new();
            let path = paths::config();
            let (mut config, stale) = match Config::load_any(&path) {
                Some(config) => {
                    let stale = config.is_stale();
                    (config, stale)
                }
                None => {
                    let fresh = Config::fetch(&http, Some(&session)).ok()?;
                    let _ = fresh.save(&path);
                    (fresh, false)
                }
            };
            config.hl = hl;
            Some((Client::new(http, session, config), stale))
        },
        move |connected| {
            if with(|a| a.session) != Some(id) {
                return; // signed out meanwhile
            }
            let Some((client, stale)) = connected else { return show_error(t(S::CantReach)) };
            let client = Arc::new(client);
            with(|a| a.client = Some(client.clone()));
            refresh_menu();
            go(Route::Home);
            fetch_account();
            if stale {
                refresh_config(&client);
            }
            if let Some(route) = crate::debug::open_route() {
                go(route);
            }
            crate::debug::run_scenario();
        },
    );
}

/// Reads music.youtube.com's page config again, for the next launch.
fn refresh_config(client: &Client) {
    let client = client.clone();
    net::run(
        net::Pool::Api,
        move || {
            if let Ok(fresh) = Config::fetch(&Http::new(), Some(client.session())) {
                let _ = fresh.save(&paths::config());
            }
        },
        |()| {},
    );
}

/// Learns who's signed in (name, handle, photo) for the account button.
fn fetch_account() {
    let Some(Some(client)) = with(|a| a.client.clone()) else { return };
    net::run(
        net::Pool::Api,
        move || client.account().and_then(|j| parse::account(&j)),
        |result| match result {
            Ok(account) => {
                with(|a| a.account = Some(account));
                show_account();
            }
            Err(e) => eprintln!("kilo: account: {e}"),
        },
    );
}

/// Shows the signed-in account's photo on the account button (a 36-point
/// circle).
pub(super) fn show_account() {
    let Some(Some((avatar, button, photo, scale))) = with(|a| {
        let s = a.shell.as_ref()?;
        Some((s.avatar.clone(), s.avatar_button.clone(), a.account.as_ref()?.photo.clone()?, s.window.backingScaleFactor()))
    }) else {
        return;
    };
    let px = (36.0 * scale) as u32;
    images::load(photo.sized(px), px, move |image| {
        if image.is_some() {
            // The photo replaces the icon; the button stays, clear, on top.
            button.setImage(None);
        }
        ui::set_image(&avatar, image.as_ref());
    });
}

/// YouTube's content in the newly chosen language: a client asking for it,
/// and the page reloaded.
pub(super) fn language_changed() {
    let hl = settings::content_language();
    let changed = with(|a| {
        let client = a.client.as_ref()?.with_language(&hl);
        a.client = Some(Arc::new(client));
        Some(())
    })
    .flatten();
    if changed.is_some() {
        reload();
    }
}

pub(super) fn show_signing_in() {
    message(Screen::SigningIn, false, t(S::SigningIn), t(S::SigningInBody), &[(t(S::Cancel), sel!(cancelSignIn:), false)], "");
}

/// Opens Google's sign-in page in the login helper, and connects once the
/// user has signed in there.
pub fn sign_in() {
    if with(|a| std::mem::replace(&mut a.account_busy, true)).unwrap_or(true) {
        return;
    }
    show_signing_in();
    let exe = std::env::current_exe().ok();
    net::run(
        net::Pool::Api,
        move || {
            let mut child = Command::new(exe?).arg("--login-helper").stdout(Stdio::piped()).spawn().ok()?;
            let mut stdout = child.stdout.take()?;
            *LOGIN.lock().ok()? = Some(child);
            // Until the helper exits: signed in, closed, or cancelled.
            let mut out = String::new();
            let _ = stdout.read_to_string(&mut out);
            if let Some(mut child) = LOGIN.lock().ok()?.take() {
                let _ = child.wait();
            }
            let cookies = login::parse_output(&out);
            login::wait_until_saved(&cookies);
            Some(cookies)
        },
        |cookies| {
            with(|a| a.account_busy = false);
            // Back from Google's window (signed in, closed or cancelled).
            super::bring_to_front();
            match cookies.and_then(|c| Session::from_cookies(&c)) {
                Some(session) => start(session),
                None => show_sign_in(),
            }
        },
    );
}

/// Cancel, while signing in: closes the login helper's window.
pub fn cancel_sign_in() {
    if let Ok(mut login) = LOGIN.lock()
        && let Some(child) = login.as_mut()
    {
        let _ = child.kill();
    }
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
        a.account = None;
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
    player::sync_timer();
    images::purge_memory();
    refresh_menu();
    show_signing_out();
    // A fresh window: no account photo, nothing playing.
    rebuild_window();
    let exe = std::env::current_exe().ok();
    net::run(
        net::Pool::Api,
        move || {
            // The helper's WebKit must be gone first, or it could write
            // the session back.
            if let Some(player) = player {
                player.quit(Duration::from_secs(2));
            }
            let cleared = exe.is_some_and(|exe| Command::new(exe).arg("--sign-out-helper").status().is_ok_and(|s| s.success()));
            paths::remove_own_data();
            cleared
        },
        |cleared| {
            with(|a| a.account_busy = false);
            if cleared { show_sign_in() } else { show_sign_out_failed() }
        },
    );
}
