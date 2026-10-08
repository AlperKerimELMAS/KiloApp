//! The app: window, navigation, sign-in, queue and the player helper.
//!
//! All state lives in one `App` on the main thread, reached through `with`.
//! Network work runs on `net` workers and comes back here; the player
//! helper's events do too.
//!
//! The window exists only while it's open: closing it releases every view,
//! layer and decoded image, and reopening it builds a fresh one from the
//! app's state.
//!
//! - `browse`: pages, navigation, and loading more of a page as it scrolls.
//! - `queue`: what plays next, and where it comes from.
//! - `player`: the player helper, the player bar, and the idle shutdown.
//! - `session`: signing in and out, and connecting with the saved session.
//! - `screens`: the page area's message screens (loading, sign-in, errors).
//! - `sidebar`: collapsing and expanding the sidebar.
//! - `dev`: developer switches' entry points (`debug` runs them).

mod browse;
mod dev;
mod player;
mod queue;
mod screens;
mod session;
mod sidebar;

use std::cell::RefCell;
use std::ptr::NonNull;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use block2::RcBlock;
use kilo_core::client::Client;
use kilo_core::model::{Account, Page};
use kilo_core::queue::{Queue, Rng};
use kilo_player::host::PlayerProcess;
use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate, NSViewBoundsDidChangeNotification, NSWindow, NSWindowDelegate,
    NSWindowDidChangeOcclusionStateNotification, NSWindowDidResizeNotification,
};
use objc2_foundation::{NSNotification, NSNotificationCenter, NSPoint, NSTimer};

pub use browse::{Route, activate, back, go, play_item, reload, scroll_by, scroll_to};
pub use dev::{bench_relayout, choose, describe, describe_keys, scroll_shelf, snapshot};
pub use player::{change_volume, is_muted, next, play_pause, previous, seek, seek_by, set_volume, toggle_mute};
pub use queue::{play_all, play_music_video, play_video};
use screens::{message, set_content, show_empty, show_error, show_loading, show_sign_in, show_sign_out_failed, show_signing_out};
pub use session::{cancel_sign_in, sign_in, sign_out};
pub use sidebar::{sidebar_frame, toggle_sidebar};

use crate::settings::{self, Appearance, Language};
use crate::strings;
use crate::ui::menus;
use crate::ui::page::{Items, PageView};
use crate::ui::shell::{self, Shell};
use crate::ui::{self, theme};
use crate::{images, net};

/// What the page area shows, so a rebuilt window can show it again.
#[derive(Clone, Debug, PartialEq)]
enum Screen {
    Page,
    Loading,
    SignIn,
    SigningIn,
    SigningOut,
    SignOutFailed,
    Empty,
    Error(Box<str>),
}

struct App {
    /// The open window; `None` while it's closed.
    shell: Option<Shell>,
    screen: Screen,
    client: Option<Arc<Client>>,
    /// Who's signed in (fetched once connected).
    account: Option<Account>,
    /// Pages visited, with where each was scrolled to when left.
    history: Vec<browse::Visit>,
    /// Bumped on every page load, so a late answer for an old page is
    /// ignored.
    generation: u64,
    page: Option<Rc<Page>>,
    /// The page on screen came from a cache older than half an hour.
    stale_page: bool,
    view: Option<PageView>,
    items: Items,
    /// Continuation tokens being fetched right now.
    fetching: Vec<Box<str>>,
    queue: Queue,
    /// Bumped for every new queue, so a late answer for an old one is
    /// ignored.
    queue_id: u64,
    /// Bumped whenever the user starts something playing: of two queues
    /// still loading, only the one asked for last starts.
    play_intent: u64,
    /// Bumped on every track change, so a late answer for an old track is
    /// ignored.
    play_token: u64,
    follow: Option<queue::Follow>,
    /// More of the radio or mix the queue came from, once it runs low.
    endless: Option<queue::Endless>,
    /// The last track ended with nothing after it yet: when more arrives
    /// (the rest of a list, or the radio), playback carries on.
    ran_out: bool,
    /// The track whose radio was asked for last.
    radio_for: Option<Box<str>>,
    rng: Rng,
    player: Option<PlayerProcess>,
    /// Bumped for every helper started, so events from one that's been
    /// replaced are ignored.
    player_serial: u64,
    playing: bool,
    /// Playing, but waiting for data: the position stands still.
    stalled: bool,
    /// The track (`play_token`) the player was asked to load and hasn't
    /// started yet.
    loading: Option<u64>,
    position: f64,
    duration: f64,
    position_at: Instant,
    volume: u8,
    /// Muted: the volume to go back to.
    unmute_to: Option<u8>,
    /// Bumped when playback starts, cancelling a pending idle shutdown.
    idle_token: u64,
    /// An idle shutdown is counting down.
    idle_armed: bool,
    timer: Option<Retained<NSTimer>>,
    /// Bumped on sign-out, so a connection still being made with the old
    /// session is dropped.
    session: u64,
    /// Signing in or out is under way.
    account_busy: bool,
    _observers: Vec<Retained<ProtocolObject<dyn NSObjectProtocol>>>,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
    static DELEGATE: RefCell<Option<Retained<Delegate>>> = const { RefCell::new(None) };
}

/// Runs `f` with the app's state. `None` if the state is already borrowed
/// (AppKit called back into us from inside `with`), so never call AppKit
/// methods that can run the event loop, like `NSWindow.close`, from `f`.
fn with<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|a| a.try_borrow_mut().ok().and_then(|mut a| a.as_mut().map(f)))
}

/// Runs `f` with the window's views, if the window is open.
fn with_shell<R>(f: impl FnOnce(&Shell) -> R) -> Option<R> {
    with(|a| a.shell.as_ref().map(f)).flatten()
}

fn mtm() -> MainThreadMarker {
    MainThreadMarker::new().expect("main thread")
}

define_class!(
    // SAFETY: NSObject subclass with no ivars; methods match the delegate
    // protocols' signatures.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "KiloAppDelegate"]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl NSApplicationDelegate for Delegate {
        #[unsafe(method(applicationDidFinishLaunching:))]
        fn did_finish_launching(&self, _n: &NSNotification) {
            launch();
        }

        #[unsafe(method(applicationShouldTerminateAfterLastWindowClosed:))]
        fn should_terminate_after_last_window_closed(&self, _app: &NSApplication) -> bool {
            // Music keeps playing with the window closed; the Dock icon
            // brings it back.
            false
        }

        #[unsafe(method(applicationShouldHandleReopen:hasVisibleWindows:))]
        fn should_handle_reopen(&self, _app: &NSApplication, visible: bool) -> bool {
            if !visible {
                show_window();
            }
            true
        }

        #[unsafe(method(applicationWillTerminate:))]
        fn will_terminate(&self, _n: &NSNotification) {
            session::cancel_sign_in();
            if let Some(Some(player)) = with(|a| a.player.take()) {
                player.quit(Duration::from_millis(500));
            }
        }
    }

    unsafe impl NSWindowDelegate for Delegate {
        #[unsafe(method(windowWillClose:))]
        fn window_will_close(&self, _n: &NSNotification) {
            // Nothing on screen: let go of the whole window.
            net::later(window_closed);
        }
    }
);

pub fn run() -> ! {
    let mtm = mtm();
    let app = NSApplication::sharedApplication(mtm);
    // The bundle says background app (`LSUIElement`), so that the helpers
    // never show in the Dock: the app itself becomes a regular one, before
    // it finishes launching.
    app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
    // SAFETY: plain NSObject init.
    let delegate: Retained<Delegate> = unsafe { msg_send![Delegate::alloc(mtm), init] };
    app.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    DELEGATE.set(Some(delegate));
    app.run();
    std::process::exit(0)
}

fn launch() {
    crate::debug::trace(|| "launch: AppKit ready".into());
    let mtm = mtm();
    theme::apply(settings::appearance(), mtm);
    strings::init();
    menus::install_menu(false, mtm);

    // Scrolling or resizing reveals thumbnails and may need more of the
    // page (coalesced); the window being covered or uncovered starts or
    // stops the progress timer.
    let center = NSNotificationCenter::defaultCenter();
    let refresh = RcBlock::new(|_n: NonNull<NSNotification>| browse::schedule_refresh());
    let occlusion = RcBlock::new(|_n: NonNull<NSNotification>| net::later(player::sync_timer));
    // SAFETY: the notification names are constant strings; the blocks are
    // retained by the notification center.
    let observers = unsafe {
        vec![
            center.addObserverForName_object_queue_usingBlock(Some(NSViewBoundsDidChangeNotification), None, None, &refresh),
            center.addObserverForName_object_queue_usingBlock(Some(NSWindowDidResizeNotification), None, None, &refresh),
            center.addObserverForName_object_queue_usingBlock(Some(NSWindowDidChangeOcclusionStateNotification), None, None, &occlusion),
        ]
    };
    images::trim_disk_cache();
    net::run(net::Pool::Images, crate::pagecache::trim, |()| {});

    APP.set(Some(App {
        shell: None,
        screen: Screen::Loading,
        client: None,
        account: None,
        history: Vec::new(),
        generation: 0,
        page: None,
        stale_page: false,
        view: None,
        items: Vec::new(),
        fetching: Vec::new(),
        queue: Queue::default(),
        queue_id: 0,
        play_intent: 0,
        play_token: 0,
        follow: None,
        endless: None,
        ran_out: false,
        radio_for: None,
        rng: Rng::new(),
        player: None,
        player_serial: 0,
        playing: false,
        stalled: false,
        loading: None,
        position: 0.0,
        duration: 0.0,
        position_at: Instant::now(),
        volume: 100,
        unmute_to: None,
        idle_token: 0,
        idle_armed: false,
        timer: None,
        session: 0,
        account_busy: false,
        _observers: observers,
    }));
    open_window(None);
    if crate::debug::activate_on_launch() {
        bring_to_front();
    }

    session::resume();
}

/// Makes Kilo the active app. macOS doesn't bring a bundle marked as a
/// background app (`LSUIElement`) to the front when it's launched, and
/// since macOS 14 `activate()` is only a request, which it turned down at
/// launch and after signing in: Kilo stayed behind, without its menu bar.
/// This older call still does it.
fn bring_to_front() {
    #[allow(deprecated)]
    NSApplication::sharedApplication(mtm()).activateIgnoringOtherApps(true);
}

/// Builds the window (or rebuilds the views of `window`) and brings it up
/// to date with the app's state, but not the page area: callers fill that.
fn open_window(window: Option<Retained<NSWindow>>) {
    let rebuilt = window.is_some();
    let shell = shell::build(window, mtm());
    DELEGATE.with_borrow(|d| {
        if let Some(d) = d {
            shell.window.setDelegate(Some(ProtocolObject::from_ref(&**d)));
        }
    });
    if !rebuilt {
        shell.window.makeKeyAndOrderFront(None);
    }
    shell.window.makeFirstResponder(Some(&shell.focus_sink));
    with(|a| {
        shell.back.setEnabled(a.history.len() > 1);
        shell::select_nav(&shell, a.history.last().and_then(|v| v.route.nav_index()));
        a.shell = Some(shell);
    });
    player::restore_bar();
    player::sync_timer();
    session::show_account();
    crate::debug::trace(|| "ui: window shown".into());
}

/// Builds the window's views again, in the same window and showing the same
/// thing, scrolled as it was: for a new look or language.
fn rebuild_window() {
    let Some(Some((window, scrolled))) = with(|a| {
        let scrolled = a.view.take().map_or(0.0, |v| v.scrolled());
        a.shell.take().map(|s| (s.window.clone(), scrolled))
    }) else {
        return;
    };
    ui::reset_hover_play();
    sidebar::stop_animation();
    open_window(Some(window));
    restore_screen(scrolled);
}

/// Shows what the page area showed when the window's views went away
/// (`Screen` is the truth: an old page stays hidden behind a newer error
/// or sign-out).
fn restore_screen(scrolled: f64) {
    let Some((screen, page)) = with(|a| (a.screen.clone(), a.page.clone())) else { return };
    match (screen, page) {
        (Screen::Page, Some(page)) => browse::show_page_at(page, scrolled),
        (Screen::Loading | Screen::Page, _) => show_loading(),
        (Screen::SignIn, _) => show_sign_in(),
        (Screen::SigningIn, _) => session::show_signing_in(),
        (Screen::SigningOut, _) => show_signing_out(),
        (Screen::SignOutFailed, _) => show_sign_out_failed(),
        (Screen::Empty, _) => show_empty(),
        (Screen::Error(text), _) => show_error(&text),
    }
}

/// The app's look may have changed (the system's appearance, or the
/// user's choice): rebuilds the window if it did.
pub fn appearance_changed() {
    if theme::refresh() {
        rebuild_window();
    }
}

/// The Appearance menu: System, Light or Dark (`tag` 0, 1, 2).
pub fn set_appearance(tag: isize) {
    let appearance = [Appearance::System, Appearance::Light, Appearance::Dark][tag.clamp(0, 2) as usize];
    settings::set_appearance(appearance);
    if theme::apply(appearance, mtm()) {
        rebuild_window();
    }
    refresh_menu();
}

/// The Language menu: System, English or Türkçe (`tag` 0, 1, 2). Kilo's
/// words change at once, and the page reloads in the new language.
pub fn set_language(tag: isize) {
    let language = [Language::System, Language::English, Language::Turkish][tag.clamp(0, 2) as usize];
    settings::set_language(language);
    strings::init();
    refresh_menu();
    rebuild_window();
    session::language_changed();
}

/// Rebuilds the menu bar (its words, ticks, and Sign In or Out).
fn refresh_menu() {
    let signed_in = with(|a| a.client.is_some()).unwrap_or(false);
    menus::install_menu(signed_in, mtm());
}

/// The account button: a menu with who's signed in, the settings, and
/// signing in or out. Called outside any action, as the menu runs its own
/// event loop.
pub fn show_account_menu() {
    let Some(Some((view, signed_in, account))) =
        with(|a| a.shell.as_ref().map(|s| (s.avatar.clone(), a.client.is_some(), a.account.clone())))
    else {
        return;
    };
    let names = account.as_ref().map(|a| (&*a.name, &*a.handle));
    let menu = menus::account_menu(signed_in, names, mtm());
    // Just below the button.
    menu.popUpMenuPositioningItem_atLocation_inView(None, NSPoint::new(0.0, -6.0), Some(&view));
}

pub fn close_window() {
    // Outside the state borrow: closing may run AppKit's event loop, which
    // can run `window_closed`.
    if let Some(window) = with_shell(|s| s.window.clone()) {
        window.close();
    }
}

pub fn reopen_window() {
    show_window();
}

fn window_closed() {
    let Some(shell) = with(|a| {
        a.view = None;
        a.shell.take()
    }) else {
        // The state is busy (AppKit ran this from inside a call that holds
        // it): try again in a moment rather than keep the window.
        return net::later(window_closed);
    };
    if let Some(shell) = shell {
        shell.window.setDelegate(None);
        drop(shell);
    }
    sidebar::stop_animation();
    player::sync_timer();
    images::purge_memory();
}

fn show_window() {
    if let Some(window) = with_shell(|s| s.window.clone()) {
        window.makeKeyAndOrderFront(None);
        return;
    }
    open_window(None);
    restore_screen(0.0);
}

/// Moves keyboard focus off the search field once a search has been sent.
pub fn release_search_focus() {
    with_shell(|s| s.window.makeFirstResponder(Some(&s.focus_sink)));
}

pub fn focus_search() {
    with_shell(|s| s.window.makeFirstResponder(Some(&s.search)));
}
