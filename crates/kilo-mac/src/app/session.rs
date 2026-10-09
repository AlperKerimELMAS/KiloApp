//! Signing in and out, who's signed in, and connecting to YouTube Music with
//! the saved session.
//!
//! An account's data has clear boundaries: signing out, or a session YouTube
//! stops accepting, forgets everything of it in memory (`forget`), and
//! starts a new account epoch (`paths::new_epoch`), so work that started
//! before writes nothing back to disk and changes nothing on screen.

use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use kilo_core::auth::{Session, is_youtube_domain, parse_binary_cookies};
use kilo_core::client::{Client, Config};
use kilo_core::http::Http;
use kilo_core::parse;
use kilo_core::queue::Queue;
use kilo_player::host::PlayerProcess;
use objc2::sel;

use super::browse::{Route, go};
use super::{
    App, Screen, bring_to_front, message, player, rebuild_window, refresh_menu, reload, show_error, show_loading, show_session_ended,
    show_sign_in, show_sign_out_failed, show_signing_out, with,
};
use crate::strings::{S, t};
use crate::{images, login, net, paths, settings, ui};

/// The login helper while it's open (so Cancel can close it).
static LOGIN: Mutex<Option<Child>> = Mutex::new(None);
/// Bumped by every sign-in and every cancel: a sign-in whose number is no
/// longer current closes its window as soon as it opens.
static LOGIN_ATTEMPT: AtomicU64 = AtomicU64::new(0);

/// At launch: connects with the saved session. If an older Kilo kept
/// Google's account cookies next to YouTube's, the prune helper deletes them
/// first, whether or not YouTube's session still works.
pub(super) fn resume() {
    let cookies = std::fs::read(paths::cookies()).ok().and_then(|data| parse_binary_cookies(&data)).unwrap_or_default();
    let session = Session::from_cookies(&cookies);
    // YouTube's cookies, but no session in them: it expired.
    let ended = session.is_none() && cookies.iter().any(|c| is_youtube_domain(&c.domain) && c.name == "LOGIN_INFO");
    let legacy = cookies.iter().any(|c| !is_youtube_domain(&c.domain));
    let Some(id) = with(|a| a.session) else { return };
    if legacy {
        show_loading();
    }
    let exe = std::env::current_exe().ok();
    net::spawn(
        move || {
            let pruned = !legacy || exe.is_some_and(|exe| Command::new(exe).arg("--prune-helper").status().is_ok_and(|s| s.success()));
            // Old copies of the cookie file may hold them too.
            paths::remove_cookie_copies() && pruned
        },
        move |pruned| {
            if !pruned {
                // Tried again at the next launch.
                eprintln!("kilo: couldn't delete all of Google's account cookies");
            }
            crate::debug::trace(|| format!("session: legacy cookies {}", if pruned { "gone" } else { "NOT all deleted" }));
            if with(|a| a.session) != Some(id) {
                return;
            }
            match session {
                Some(session) => start(session),
                // Its data goes, as when a session ends while Kilo runs.
                None if ended => session_ended(),
                None => {
                    show_sign_in();
                    crate::debug::run_scenario();
                }
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
    let epoch = paths::epoch();
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
                    save_config(&fresh, epoch);
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

/// Saves the page config for the next launch, unless the account it was
/// read for has been let go of (`paths::epoch`).
fn save_config(config: &Config, epoch: u64) {
    paths::if_current(epoch, || config.save(&paths::config()).is_ok());
}

/// Reads music.youtube.com's page config again, for the next launch.
fn refresh_config(client: &Client) {
    let client = client.clone();
    let epoch = paths::epoch();
    net::run(
        net::Pool::Api,
        move || {
            if let Ok(fresh) = Config::fetch(&Http::new(), Some(client.session())) {
                save_config(&fresh, epoch);
            }
        },
        |()| {},
    );
}

/// Learns who's signed in (name, handle, photo) for the account button.
fn fetch_account() {
    let Some(Some((client, id))) = with(|a| a.client.clone().map(|c| (c, a.session))) else { return };
    net::run(
        net::Pool::Api,
        move || client.account().and_then(|j| parse::account(&j)),
        move |result| {
            if with(|a| a.session) != Some(id) {
                return; // signed out meanwhile
            }
            match result {
                Ok(account) => {
                    with(|a| a.account = Some(account));
                    show_account();
                }
                // Asked at every connect, so a session YouTube no longer
                // takes is noticed even when Home comes from the cache.
                Err(kilo_core::Error::SignedOut) => session_ended(),
                Err(e) => eprintln!("kilo: account: {e}"),
            }
        },
    );
}

/// Shows the signed-in account's photo on the account button (a 36-point
/// circle).
pub(super) fn show_account() {
    if crate::debug::hide_account() {
        return;
    }
    let Some(Some((avatar, button, photo, scale, id))) = with(|a| {
        let s = a.shell.as_ref()?;
        Some((s.avatar.clone(), s.avatar_button.clone(), a.account.as_ref()?.photo.clone()?, s.window.backingScaleFactor(), a.session))
    }) else {
        return;
    };
    let px = (36.0 * scale) as u32;
    images::load(photo.sized(px), px, move |image| {
        if with(|a| a.session) != Some(id) {
            return;
        }
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
    let attempt = LOGIN_ATTEMPT.fetch_add(1, Ordering::SeqCst) + 1;
    let exe = std::env::current_exe().ok();
    net::spawn(
        move || {
            // Its stdin is a pipe that only Kilo holds: if Kilo goes away,
            // the helper sees it close and quits too.
            let mut child = Command::new(exe?).arg("--login-helper").stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().ok()?;
            let mut stdout = child.stdout.take()?;
            *LOGIN.lock().ok()? = Some(child);
            if LOGIN_ATTEMPT.load(Ordering::SeqCst) != attempt {
                cancel_sign_in(); // cancelled before the window was up
            }
            // Until the helper exits: signed in, closed, or cancelled.
            let mut out = String::new();
            let _ = stdout.read_to_string(&mut out);
            if let Some(mut child) = LOGIN.lock().ok()?.take() {
                let _ = child.wait();
            }
            let cookies = login::parse_output(&out);
            if !login::wait_until_saved(&cookies) {
                // The player will say if it isn't signed in.
                eprintln!("kilo: the sign-in wasn't saved in time");
            }
            Some(cookies)
        },
        move |cookies| {
            with(|a| a.account_busy = false);
            // Back from Google's window (signed in, closed or cancelled).
            bring_to_front();
            match cookies.and_then(|c| Session::from_cookies(&c)).filter(|_| LOGIN_ATTEMPT.load(Ordering::SeqCst) == attempt) {
                Some(session) => start(session),
                None => show_sign_in(),
            }
        },
    );
}

/// Cancel, while signing in (and when Kilo quits): closes the login
/// helper's window, or keeps it from opening.
pub fn cancel_sign_in() {
    LOGIN_ATTEMPT.fetch_add(1, Ordering::SeqCst);
    if let Ok(mut login) = LOGIN.lock()
        && let Some(child) = login.as_mut()
    {
        let _ = child.kill();
    }
}

/// Lets go of everything the signed-in account left in memory: its client
/// and name, pages and history, queue and player. Returns the player, for
/// the caller to stop off the main thread. Writes still on their way to disk
/// are dropped (`paths::new_epoch`).
fn forget(a: &mut App) -> Option<PlayerProcess> {
    paths::new_epoch();
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
    a.play_intent += 1;
    a.play_token += 1;
    a.follow = None;
    a.endless = None;
    a.ran_out = false;
    a.radio_for = None;
    a.playing = false;
    a.stalled = false;
    a.loading = None;
    a.position = 0.0;
    a.duration = 0.0;
    a.idle_token += 1;
    a.idle_armed = false;
    // Whatever the old helper still says (it's being stopped) is ignored.
    a.player_serial += 1;
    a.player.take()
}

/// YouTube stopped accepting the session (it expired, or was signed out
/// elsewhere): the account is forgotten here too, and erased from this Mac
/// as by Sign Out (a dead session's cookies and pages are no use to the
/// next sign-in, which gets its own). The sign-in screen says why.
pub(super) fn session_ended() {
    erase_account(true);
}

/// Sign Out: forgets the account and erases it from this Mac.
pub fn sign_out() {
    erase_account(false);
}

/// Forgets the account and erases it from this Mac: stops playback, has a
/// helper delete everything WebKit stores for Kilo (the session), deletes
/// what Kilo wrote itself (page config, caches), and shows the sign-in
/// screen (saying the session `ended`, if it did). If anything couldn't be
/// deleted, the screen says so and offers to try again.
///
/// Not every 401 or 403 means the session is over, so one that `ended`
/// while connected is erased only once YouTube's account check says so
/// too; otherwise its files stay, and the next launch tries it again. (One
/// that expired before launch was never connected, and needs no asking.)
fn erase_account(ended: bool) {
    let Some((player, client)) = with(|a| {
        if std::mem::replace(&mut a.account_busy, true) {
            return Err(()); // a sign-in or sign-out is under way
        }
        let client = a.client.clone();
        Ok((forget(a), client))
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
    net::spawn(
        move || {
            // The helper's WebKit must be gone first, or it could write
            // the session back.
            if let Some(player) = player {
                player.quit(Duration::from_secs(2));
            }
            if ended
                && let Some(client) = client
                && !matches!(client.account().and_then(|j| parse::account(&j)), Err(kilo_core::Error::SignedOut))
            {
                crate::debug::trace(|| "session: not confirmed as ended; its files stay".into());
                return None;
            }
            let cleared = exe.is_some_and(|exe| Command::new(exe).arg("--sign-out-helper").status().is_ok_and(|s| s.success()));
            Some(paths::remove_own_data() && cleared)
        },
        move |erased| {
            with(|a| a.account_busy = false);
            match (erased, ended) {
                (Some(false), _) => show_sign_out_failed(),
                (None, _) | (Some(true), true) => show_session_ended(),
                (Some(true), false) => show_sign_in(),
            }
            crate::debug::run_scenario();
        },
    );
}
