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
//! - `session`: signing in, and connecting with the saved session.

mod browse;
mod player;
mod queue;
mod session;

use std::cell::RefCell;
use std::ptr::NonNull;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use block2::RcBlock;
use kilo_core::client::Client;
use kilo_core::model::{Entry, Page, Section};
use kilo_core::queue::{Queue, Rng};
use kilo_player::host::PlayerProcess;
use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject, Sel};
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate, NSAutoresizingMaskOptions, NSProgressIndicator,
    NSProgressIndicatorStyle, NSTextAlignment, NSView, NSViewBoundsDidChangeNotification, NSWindowDelegate,
    NSWindowDidChangeOcclusionStateNotification, NSWindowDidResizeNotification,
};
use objc2_foundation::{NSNotification, NSNotificationCenter, NSTimer};

pub use browse::{Route, activate, back, go, reload, scroll_to};
pub use player::{next, play_pause, previous, seek, set_volume};
pub use queue::{play_all, play_music_video, play_video};
pub use session::sign_in;

use crate::ui::page::{Items, PageView};
use crate::ui::shell::{self, Shell};
use crate::ui::{self, Weight};
use crate::{images, net};

struct App {
    /// The open window; `None` while it's closed.
    shell: Option<Shell>,
    client: Option<Arc<Client>>,
    history: Vec<Route>,
    /// Bumped on every page load, so a late answer for an old page is
    /// ignored.
    generation: u64,
    page: Option<Rc<Page>>,
    view: Option<PageView>,
    items: Items,
    /// Continuation tokens being fetched right now.
    fetching: Vec<Box<str>>,
    queue: Queue,
    /// Bumped for every new queue, so a late answer for an old one is
    /// ignored.
    queue_id: u64,
    /// Bumped on every track change, so a late answer for an old track is
    /// ignored.
    play_token: u64,
    follow: Option<queue::Follow>,
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
    position: f64,
    duration: f64,
    position_at: Instant,
    volume: u8,
    /// Bumped when playback starts, cancelling a pending idle shutdown.
    idle_token: u64,
    /// An idle shutdown is counting down.
    idle_armed: bool,
    timer: Option<Retained<NSTimer>>,
    signing_in: bool,
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
    app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
    // SAFETY: plain NSObject init.
    let delegate: Retained<Delegate> = unsafe { msg_send![Delegate::alloc(mtm), init] };
    app.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    DELEGATE.set(Some(delegate));
    app.run();
    std::process::exit(0)
}

fn launch() {
    let mtm = mtm();
    shell::install_menu(mtm);

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

    APP.set(Some(App {
        shell: None,
        client: None,
        history: Vec::new(),
        generation: 0,
        page: None,
        view: None,
        items: Vec::new(),
        fetching: Vec::new(),
        queue: Queue::default(),
        queue_id: 0,
        play_token: 0,
        follow: None,
        ran_out: false,
        radio_for: None,
        rng: Rng::new(),
        player: None,
        player_serial: 0,
        playing: false,
        position: 0.0,
        duration: 0.0,
        position_at: Instant::now(),
        volume: 100,
        idle_token: 0,
        idle_armed: false,
        timer: None,
        signing_in: false,
        _observers: observers,
    }));
    open_window();
    if crate::debug::activate_on_launch() {
        NSApplication::sharedApplication(mtm).activate();
    }

    match session::saved_session() {
        Some(session) => session::start(session),
        None => show_sign_in(),
    }
}

/// Builds the window and brings it up to date with the app's state (but
/// not the page area; callers fill that).
fn open_window() {
    let shell = shell::build(mtm());
    DELEGATE.with_borrow(|d| {
        if let Some(d) = d {
            shell.window.setDelegate(Some(ProtocolObject::from_ref(&**d)));
        }
    });
    shell.window.makeKeyAndOrderFront(None);
    shell.window.makeFirstResponder(Some(&shell.focus_sink));
    with(|a| {
        shell.back.setEnabled(a.history.len() > 1);
        shell::select_nav(&shell, a.history.last().and_then(Route::nav_index));
        a.shell = Some(shell);
    });
    player::restore_bar();
    player::sync_timer();
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
    player::sync_timer();
    images::purge_memory();
}

fn show_window() {
    if let Some(window) = with_shell(|s| s.window.clone()) {
        window.makeKeyAndOrderFront(None);
        return;
    }
    open_window();
    match with(|a| a.page.clone()).flatten() {
        Some(page) => browse::show_page(page),
        None => reload(),
    }
}

/// Moves keyboard focus off the search field once a search has been sent.
pub fn release_search_focus() {
    with_shell(|s| s.window.makeFirstResponder(Some(&s.focus_sink)));
}

pub fn focus_search() {
    with_shell(|s| s.window.makeFirstResponder(Some(&s.search)));
}

/// Fills the page area with `view`, sized by autoresizing rather than
/// constraints so the page stays out of Auto Layout.
fn set_content(view: &NSView) {
    with_shell(|s| {
        for old in s.content.subviews().iter() {
            old.removeFromSuperview();
        }
        view.setFrame(s.content.bounds());
        view.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable);
        s.content.addSubview(view);
    });
}

fn show_loading() {
    let mtm = mtm();
    with(|a| a.view = None);
    if with(|a| a.shell.is_none()).unwrap_or(true) {
        return;
    }
    let spinner = NSProgressIndicator::new(mtm);
    spinner.setStyle(NSProgressIndicatorStyle::Spinning);
    // SAFETY: plain AppKit call on the main thread.
    unsafe { spinner.startAnimation(None) };
    let holder = NSView::new(mtm);
    holder.addSubview(&spinner);
    spinner.setTranslatesAutoresizingMaskIntoConstraints(false);
    spinner.centerXAnchor().constraintEqualToAnchor(&holder.centerXAnchor()).setActive(true);
    spinner.centerYAnchor().constraintEqualToAnchor(&holder.centerYAnchor()).setActive(true);
    set_content(&holder);
}

/// Fills the page area with a centered title, text, and optional button.
fn message(title: &str, body: &str, button: Option<(&str, Sel)>) {
    let mtm = mtm();
    with(|a| a.view = None);
    if with(|a| a.shell.is_none()).unwrap_or(true) {
        return;
    }
    let t = ui::label(title, 24.0, Weight::Bold, false, mtm);
    let b = ui::paragraph(body, 14.0, true, 4, mtm);
    b.setAlignment(NSTextAlignment::Center);
    b.widthAnchor().constraintLessThanOrEqualToConstant(440.0).setActive(true);
    let mut views: Vec<Retained<NSView>> =
        vec![Retained::into_super(Retained::into_super(t)), Retained::into_super(Retained::into_super(b))];
    if let Some((label, action)) = button {
        views.push(Retained::into_super(Retained::into_super(ui::text_button(label, action, mtm))));
    }
    let refs: Vec<&NSView> = views.iter().map(|v| &**v).collect();
    let col = ui::stack(&refs, true, 14.0, mtm);
    let holder = NSView::new(mtm);
    holder.addSubview(&col);
    col.setTranslatesAutoresizingMaskIntoConstraints(false);
    col.centerXAnchor().constraintEqualToAnchor(&holder.centerXAnchor()).setActive(true);
    col.centerYAnchor().constraintEqualToAnchor(&holder.centerYAnchor()).setActive(true);
    set_content(&holder);
}

fn show_error(text: &str) {
    message("Something went wrong", text, Some(("Try again", sel!(retry:))));
}

fn show_sign_in() {
    with_shell(|s| shell::select_nav(s, None));
    message(
        "Sign in to YouTube Music",
        "Kilo plays music from your own YouTube Music Premium account. You sign in on Google's own page; Kilo never sees your password.",
        Some(("Sign in", sel!(signIn:))),
    );
}

/// Developer switch: a one-line summary of the page and the queue.
pub fn describe() -> String {
    with(|a| {
        let lists: Vec<String> = a
            .page
            .iter()
            .flat_map(|p| p.sections.iter())
            .map(|s| match s {
                Section::List { entries, continuation, .. } => {
                    format!("list {}{}", entries.len(), if continuation.is_some() { "+" } else { "" })
                }
                Section::Cards { entries, .. } => format!("cards {}", entries.len()),
                Section::Text { .. } => "text".into(),
            })
            .collect();
        let more = a.page.as_ref().is_some_and(|p| p.continuation.is_some());
        format!(
            "page [{}]{} queue {}/{} ({}) following {}",
            lists.join(", "),
            if more { " +more" } else { "" },
            a.queue.index() + 1,
            a.queue.entries().len(),
            a.queue.current().and_then(Entry::video_id).unwrap_or("-"),
            a.follow.is_some()
        )
    })
    .unwrap_or_default()
}
