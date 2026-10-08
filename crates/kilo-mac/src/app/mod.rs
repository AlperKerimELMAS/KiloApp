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
use kilo_core::model::{Account, Entry, Page, Section};
use kilo_core::queue::{Queue, Rng};
use kilo_player::host::PlayerProcess;
use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject, Sel};
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate, NSAutoresizingMaskOptions, NSImageView, NSTextAlignment, NSView,
    NSViewBoundsDidChangeNotification, NSWindow, NSWindowDelegate, NSWindowDidChangeOcclusionStateNotification,
    NSWindowDidResizeNotification, NSWorkspace,
};
use objc2_foundation::{NSNotification, NSNotificationCenter, NSPoint, NSRunLoop, NSRunLoopCommonModes, NSTimer};
use objc2_quartz_core::CADisplayLink;

pub use browse::{Route, activate, back, go, play_item, reload, scroll_by, scroll_to};
pub use player::{change_volume, is_muted, next, play_pause, previous, seek, seek_by, set_volume, toggle_mute};
pub use queue::{play_all, play_music_video, play_video};
pub use session::{cancel_sign_in, sign_in, sign_out};

use crate::settings::{self, Appearance, Language};
use crate::strings::{self, S, t};
use crate::ui::page::{Items, PageView};
use crate::ui::shell::{self, Shell};
use crate::ui::{self, Weight, theme};
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
    history: Vec<Route>,
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
    shell::install_menu(false, mtm);

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
        shell::select_nav(&shell, a.history.last().and_then(Route::nav_index));
        a.shell = Some(shell);
    });
    player::restore_bar();
    player::sync_timer();
    session::show_account();
    crate::debug::trace(|| "ui: window shown".into());
}

/// Builds the window's views again, in the same window and showing the same
/// thing: for a new look or language.
fn rebuild_window() {
    let Some(Some((window, screen))) = with(|a| {
        a.view = None;
        a.shell.take().map(|s| (s.window.clone(), a.screen.clone()))
    }) else {
        return;
    };
    ui::reset_hover_play();
    open_window(Some(window));
    let page = with(|a| a.page.clone()).flatten();
    match (screen, page) {
        (Screen::Page, Some(page)) => browse::show_page(page),
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
    shell::install_menu(signed_in, mtm());
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
    let menu = shell::account_menu(signed_in, names, mtm());
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

/// Developer switch: renders the window to `KILO_SNAPSHOT`, soon or `now`.
pub fn snapshot(now: bool) {
    with_shell(|s| if now { crate::debug::snapshot_now(&s.window) } else { crate::debug::schedule_snapshot(&s.window) });
}

/// Developer switch: picks the appearance or language as their menus do.
pub fn choose(setting: &str, value: &str) {
    let tag = match value {
        "light" | "en" => 1,
        "dark" | "tr" => 2,
        _ => 0,
    };
    match setting {
        "theme" => set_appearance(tag),
        "lang" => set_language(tag),
        _ => {}
    }
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
    open_window(None);
    match with(|a| a.page.clone()).flatten() {
        Some(page) => browse::show_page(page),
        None => reload(),
    }
}

/// How long the sidebar takes to collapse or expand.
const SIDEBAR_ANIMATION: Duration = Duration::from_millis(220);

/// The sidebar collapsing or expanding, frame by frame.
struct SidebarAnimation {
    link: Retained<CADisplayLink>,
    from: f64,
    to: f64,
    start: Instant,
    /// For the scenario log: frames shown, and the slowest one's work.
    frames: u32,
    slowest: Duration,
}

thread_local! {
    static SIDEBAR: RefCell<Option<SidebarAnimation>> = const { RefCell::new(None) };
}

/// The ☰ button: collapses the sidebar to its icons, or expands it (and
/// remembers which). It slides, briefly, unless the Mac is set to reduce
/// motion; toggling again midway turns it around.
pub fn toggle_sidebar() {
    let collapsed = !settings::sidebar_collapsed();
    settings::set_sidebar_collapsed(collapsed);
    let to = if collapsed { 1.0 } else { 0.0 };
    let reduce = NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion();
    let Some((from, view)) = with_shell(|s| (shell::sidebar_progress(s), s.window.contentView())) else { return };
    let running = SIDEBAR.with_borrow_mut(|a| {
        a.as_mut().map(|a| {
            (a.from, a.to, a.start, a.frames) = (from, to, Instant::now(), 0);
        })
    });
    if running.is_some() {
        return;
    }
    let Some(view) = view.filter(|_| !reduce) else {
        with_shell(|s| {
            shell::set_collapsed(s, collapsed);
            s.window.layoutIfNeeded();
        });
        return browse::schedule_refresh();
    };
    let target: &objc2::runtime::AnyObject = ui::actions(mtm());
    // SAFETY: the target (the app-wide action object) has `sidebarFrame:`.
    let link = unsafe { view.displayLinkWithTarget_selector(target, sel!(sidebarFrame:)) };
    // SAFETY: the main run loop, in its common modes (so it runs during
    // tracking too).
    unsafe { link.addToRunLoop_forMode(&NSRunLoop::mainRunLoop(), NSRunLoopCommonModes) };
    SIDEBAR.set(Some(SidebarAnimation { link, from, to, start: Instant::now(), frames: 0, slowest: Duration::ZERO }));
}

/// One frame of the sidebar animation (from its display link): the
/// sidebar's width eases out, and the page follows (only re-framed, so a
/// frame costs well under a millisecond).
pub fn sidebar_frame() {
    let Some((from, to, start)) = SIDEBAR.with_borrow(|a| a.as_ref().map(|a| (a.from, a.to, a.start))) else { return };
    let work = Instant::now();
    let t = (start.elapsed().as_secs_f64() / SIDEBAR_ANIMATION.as_secs_f64()).min(1.0);
    let eased = 1.0 - (1.0 - t).powi(3);
    let open = with_shell(|s| {
        if t < 1.0 {
            shell::set_sidebar_progress(s, from + (to - from) * eased);
        } else {
            shell::set_collapsed(s, to == 1.0);
        }
        s.window.layoutIfNeeded();
    })
    .is_some();
    with(|a| {
        if let Some(v) = &a.view {
            v.refresh();
        }
    });
    let done = SIDEBAR.with_borrow_mut(|a| {
        let a = a.as_mut()?;
        a.frames += 1;
        a.slowest = a.slowest.max(work.elapsed());
        (t >= 1.0 || !open).then_some((a.frames, a.slowest))
    });
    if let Some((frames, slowest)) = done {
        if let Some(a) = SIDEBAR.take() {
            a.link.invalidate();
        }
        crate::debug::trace(|| format!("sidebar: {frames} frames, slowest {:.2} ms", slowest.as_secs_f64() * 1000.0));
        browse::schedule_refresh();
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

/// The page area while a page loads: its shape in placeholder gray.
fn show_loading() {
    with(|a| {
        a.view = None;
        a.screen = Screen::Loading;
    });
    if with(|a| a.shell.is_some()).unwrap_or(false) {
        set_content(&ui::page::skeleton(mtm()));
    }
}

/// A button on a message screen: its title, action, and whether it's the
/// filled call to action.
type Button<'a> = (&'a str, Sel, bool);

/// Fills the page area with a centered message: the app's icon (`icon`),
/// a title, text, buttons and a small note under them.
fn message(screen: Screen, icon: bool, title: &str, body: &str, buttons: &[Button], note: &str) {
    let mtm = mtm();
    with(|a| {
        a.view = None;
        a.screen = screen;
    });
    if with(|a| a.shell.is_none()).unwrap_or(true) {
        return;
    }
    let mut views: Vec<Retained<NSView>> = Vec::new();
    if icon && let Some(image) = NSApplication::sharedApplication(mtm).applicationIconImage() {
        let image = NSImageView::imageViewWithImage(&image, mtm);
        ui::size(&image, Some(96.0), Some(96.0));
        views.push(Retained::into_super(Retained::into_super(image)));
    }
    views.push(Retained::into_super(Retained::into_super(ui::label(title, 28.0, Weight::Bold, false, mtm))));
    if !body.is_empty() {
        let b = ui::paragraph(body, 15.0, true, 4, 420.0, mtm);
        b.setAlignment(NSTextAlignment::Center);
        views.push(Retained::into_super(Retained::into_super(b)));
    }
    let buttons_at = views.len();
    if !buttons.is_empty() {
        let made: Vec<Retained<objc2_app_kit::NSButton>> =
            buttons.iter().map(|&(label, action, primary)| ui::pill_button(label, action, 44.0, primary, mtm)).collect();
        let refs: Vec<&NSView> = made.iter().map(|b| &**b as &NSView).collect();
        views.push(Retained::into_super(ui::stack(&refs, false, 12.0, mtm)));
    }
    if !note.is_empty() {
        let n = ui::paragraph(note, 12.0, true, 2, 360.0, mtm);
        n.setAlignment(NSTextAlignment::Center);
        views.push(Retained::into_super(Retained::into_super(n)));
    }
    let refs: Vec<&NSView> = views.iter().map(|v| &**v).collect();
    let col = ui::stack(&refs, true, 14.0, mtm);
    if !buttons.is_empty() && buttons_at > 0 {
        // More room above the buttons.
        col.setCustomSpacing_afterView(24.0, refs[buttons_at - 1]);
    }
    let holder = NSView::new(mtm);
    holder.addSubview(&col);
    col.setTranslatesAutoresizingMaskIntoConstraints(false);
    col.centerXAnchor().constraintEqualToAnchor(&holder.centerXAnchor()).setActive(true);
    col.centerYAnchor().constraintEqualToAnchor_constant(&holder.centerYAnchor(), -20.0).setActive(true);
    set_content(&holder);
    with_shell(|s| crate::debug::schedule_snapshot(&s.window));
}

fn show_error(text: &str) {
    message(Screen::Error(text.into()), false, t(S::SomethingWrong), text, &[(t(S::TryAgain), sel!(retry:), true)], "");
}

fn show_sign_in() {
    with_shell(|s| shell::select_nav(s, None));
    message(Screen::SignIn, true, t(S::Welcome), t(S::WelcomeBody), &[(t(S::SignInWithGoogle), sel!(signIn:), true)], t(S::SignInNote));
}

fn show_empty() {
    message(Screen::Empty, false, t(S::NothingHere), t(S::NothingHereBody), &[], "");
}

fn show_signing_out() {
    message(Screen::SigningOut, false, t(S::SigningOut), "", &[], "");
}

fn show_sign_out_failed() {
    message(Screen::SignOutFailed, false, t(S::SignOutFailed), t(S::SignOutFailedBody), &[], "");
}

/// Developer switch: the volume, what has the keyboard focus, and the search
/// field's text.
pub fn describe_keys() -> String {
    with(|a| {
        let Some(s) = &a.shell else { return format!("volume {} (no window)", a.volume) };
        let focus = s.window.firstResponder().map(|r| r.class().name().to_string_lossy().into_owned()).unwrap_or_default();
        format!(
            "volume {} muted {} playing {} focus {focus} search {:?}",
            a.volume,
            a.unmute_to.is_some(),
            a.playing,
            s.search.stringValue()
        )
    })
    .unwrap_or_default()
}

/// Developer switch: changes the page's width `n` times (by resizing the
/// window) and says how long the page took to follow, on average.
pub fn bench_relayout(n: usize) -> String {
    let Some(window) = with_shell(|s| s.window.clone()) else { return "no window".into() };
    let frame = window.frame();
    let mut spent = Duration::ZERO;
    for i in 0..n {
        let width = frame.size.width - if i % 2 == 0 { 40.0 } else { 0.0 };
        window.setFrame_display(objc2_foundation::NSRect::new(frame.origin, objc2_foundation::NSSize::new(width, frame.size.height)), true);
        window.layoutIfNeeded();
        let start = Instant::now();
        with(|a| {
            if let Some(v) = &a.view {
                v.refresh();
            }
        });
        spent += start.elapsed();
    }
    window.setFrame_display(frame, true);
    format!("page relayout: {:.2} ms per width change ({n} changes)", spent.as_secs_f64() * 1000.0 / n.max(1) as f64)
}

/// Developer switch: hands `event` to the page's first shelf, and says
/// where the page and that shelf are scrolled to then.
pub fn scroll_shelf(event: &objc2_app_kit::NSEvent) -> String {
    let Some(Some((page, shelf))) = with(|a| a.view.as_ref().map(|v| (v.root.clone(), v.first_shelf()))) else { return "no page".into() };
    let Some(shelf) = shelf else { return "no shelf on screen".into() };
    shelf.scrollWheel(event);
    format!(
        "page at {:.0}, shelf at {:.0} (event dx {} dy {})",
        page.contentView().bounds().origin.y,
        shelf.contentView().bounds().origin.x,
        event.scrollingDeltaX(),
        event.scrollingDeltaY()
    )
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
                Section::Grid { entries, .. } => format!("grid {}", entries.len()),
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
