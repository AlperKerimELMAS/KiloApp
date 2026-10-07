//! The app: window, navigation, sign-in, queue and the player helper.
//!
//! All state lives on the main thread. Network work runs on `net` workers
//! and comes back here; the player helper's events do too.
//!
//! The window exists only while it's open: closing it releases every view,
//! layer and decoded image, and reopening it builds a fresh one from the
//! app's state.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::ptr::NonNull;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use block2::RcBlock;
use dispatch2::{DispatchQueue, DispatchTime};
use kilo_core::auth::{Cookie, Session, parse_binary_cookies};
use kilo_core::client::{Client, Config};
use kilo_core::http::Http;
use kilo_core::model::{Entry, Header, Page, Section, Target};
use kilo_core::queue::{Queue, Rng};
use kilo_player::host::PlayerProcess;
use kilo_player::protocol::{Command, Event, VideoId};
use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate, NSAutoresizingMaskOptions, NSProgressIndicator,
    NSProgressIndicatorStyle, NSView, NSViewBoundsDidChangeNotification, NSWindowDelegate,
    NSWindowDidChangeOcclusionStateNotification, NSWindowDidResizeNotification, NSWindowOcclusionState,
};
use objc2_foundation::{NSNotification, NSNotificationCenter, NSString, NSTimer};

use crate::ui::page::{Items, More, PageView};
use crate::ui::shell::{self, Shell};
use crate::ui::{self, Weight};
use crate::{images, net, paths};

/// After this long paused, the player helper (and all of WebKit) is shut
/// down; pressing play starts a fresh one where playback left off.
const IDLE_SHUTDOWN: Duration = Duration::from_secs(5 * 60);

#[derive(Clone, Debug, PartialEq)]
pub enum Route {
    Home,
    Explore,
    Library,
    Browse { id: Box<str>, params: Option<Box<str>> },
    Search(String),
}

/// A queue started from a long list that's still loading: the rest of the
/// list joins the queue as it arrives.
struct Follow {
    queue_id: u64,
    shuffle: bool,
    /// For decorating album tracks, as when the queue started.
    header: Option<Header>,
    /// The continuation token of the part that comes next.
    next: Box<str>,
}

struct App {
    /// The open window; `None` while it's closed.
    shell: Option<Shell>,
    client: Option<Arc<Client>>,
    history: Vec<Route>,
    generation: u64,
    page: Option<Rc<Page>>,
    view: Option<PageView>,
    items: Items,
    /// Continuation tokens being fetched right now.
    fetching: Vec<Box<str>>,
    queue: Queue,
    queue_id: u64,
    /// Bumped on every track change, so a late answer for an old track is
    /// ignored.
    play_token: u64,
    follow: Option<Follow>,
    /// The last track ended with nothing after it yet: when more arrives
    /// (the rest of a list, or the radio), playback carries on.
    ran_out: bool,
    rng: Rng,
    player: Option<PlayerProcess>,
    playing: bool,
    position: f64,
    duration: f64,
    position_at: Instant,
    volume: u8,
    /// Bumped when playback starts, cancelling a pending idle shutdown.
    idle_token: u64,
    /// An idle shutdown is counting down.
    idle_armed: bool,
    radio_for: Option<Box<str>>,
    timer: Option<Retained<NSTimer>>,
    signing_in: bool,
    _observers: Vec<Retained<ProtocolObject<dyn NSObjectProtocol>>>,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
    static REFRESH_PENDING: Cell<bool> = const { Cell::new(false) };
    static DELEGATE: RefCell<Option<Retained<Delegate>>> = const { RefCell::new(None) };
}

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
    let refresh = RcBlock::new(|_n: NonNull<NSNotification>| schedule_refresh());
    let occlusion = RcBlock::new(|_n: NonNull<NSNotification>| net::later(sync_timer));
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
        rng: Rng::new(),
        player: None,
        playing: false,
        position: 0.0,
        duration: 0.0,
        position_at: Instant::now(),
        volume: 100,
        idle_token: 0,
        idle_armed: false,
        radio_for: None,
        timer: None,
        signing_in: false,
        _observers: observers,
    }));
    open_window();
    if crate::debug::activate_on_launch() {
        NSApplication::sharedApplication(mtm).activate();
    }

    match saved_session() {
        Some(session) => start(session),
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
        shell::select_nav(&shell, a.history.last().and_then(nav_index));
        a.shell = Some(shell);
    });
    let now = with(|a| (a.queue.current().cloned(), a.playing, a.duration)).and_then(|(e, p, d)| e.map(|e| (e, p, d)));
    if let Some((entry, playing, duration)) = now {
        show_now_playing(&entry);
        with_shell(|s| {
            ui::set_symbol(&s.bar.play, if playing { "pause.fill" } else { "play.fill" });
            s.bar.progress.set_enabled(duration > 0.0);
        });
        update_progress();
    }
    sync_timer();
}

/// The session WebKit stored the last time the user signed in.
fn saved_session() -> Option<Session> {
    let data = std::fs::read(paths::cookies()).ok()?;
    Session::from_cookies(&parse_binary_cookies(&data)?)
}

fn start(session: Session) {
    show_loading();
    let config_path: PathBuf = paths::support().join("innertube.txt");
    net::run(
        net::Pool::Api,
        move || {
            let http = Http::new();
            let config = Config::load(&config_path).or_else(|| {
                let fresh = Config::fetch(&http, Some(&session)).ok()?;
                let _ = fresh.save(&config_path);
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

pub fn go(route: Route) {
    let same = with(|a| a.history.last() == Some(&route)).unwrap_or(false);
    if !same {
        with(|a| a.history.push(route.clone()));
    }
    load(route);
}

pub fn back() {
    if let Some(Some(route)) = with(|a| {
        (a.history.len() > 1).then(|| {
            a.history.pop();
            a.history.last().cloned()
        })?
    }) {
        load(route);
    }
}

pub fn reload() {
    if let Some(Some(route)) = with(|a| a.history.last().cloned()) {
        load(route);
    } else if let Some(s) = saved_session() {
        start(s);
    } else {
        show_sign_in();
    }
}

fn nav_index(route: &Route) -> Option<usize> {
    match route {
        Route::Home => Some(0),
        Route::Explore => Some(1),
        Route::Library => Some(2),
        _ => None,
    }
}

fn load(route: Route) {
    let Some((client, generation)) = with(|a| {
        a.generation += 1;
        if let Some(s) = &a.shell {
            s.back.setEnabled(a.history.len() > 1);
            shell::select_nav(s, nav_index(&route));
        }
        (a.client.clone(), a.generation)
    }) else {
        return;
    };
    let Some(client) = client else { return show_sign_in() };
    show_loading();
    net::run(net::Pool::Api, move || fetch(&client, &route), move |result| {
        if with(|a| a.generation) != Some(generation) {
            return; // the user moved on
        }
        match result {
            Ok(page) => show_page(Rc::new(page)),
            Err(kilo_core::Error::SignedOut) => show_sign_in(),
            Err(e) => show_error(&e.to_string()),
        }
    });
}

/// The first part of a page. More shelves and the rest of long lists load
/// as they scroll into view (`check_more`).
fn fetch(client: &Client, route: &Route) -> kilo_core::Result<Page> {
    use kilo_core::parse;
    match route {
        Route::Home => parse::browse(&client.browse("FEmusic_home", None)?),
        Route::Explore => parse::browse(&client.browse("FEmusic_explore", None)?),
        Route::Library => parse::browse(&client.browse("FEmusic_liked_playlists", None)?),
        Route::Browse { id, params } => parse::browse(&client.browse(id, params.as_deref())?),
        Route::Search(q) => parse::search(&client.search(q, None)?),
    }
}

/// Loads the next part of the page if its end is coming into view.
fn check_more() {
    let Some(Some((kind, token, generation))) = with(|a| {
        let (kind, token) = a.view.as_ref()?.wants_more()?;
        Some((kind, token, a.generation))
    }) else {
        return;
    };
    fetch_more(kind, token, generation);
}

/// Fetches the part of a page that `token` points to, then adds it to the
/// page (if `generation` is still on screen) and to a queue following it.
fn fetch_more(kind: More, token: Box<str>, generation: u64) {
    let Some(Some(client)) = with(|a| {
        if a.fetching.contains(&token) {
            return None;
        }
        a.fetching.push(token.clone());
        a.client.clone()
    }) else {
        return;
    };
    let t = token.clone();
    net::run(net::Pool::Api, move || client.continuation(&t).and_then(|j| kilo_core::parse::browse(&j)), move |result| {
        let Ok(more) = result else {
            // The token stays on the page, so a later scroll retries; not
            // before a few seconds, though, or every scroll would (offline).
            let when = DispatchTime::NOW.time(5_000_000_000);
            let _ = DispatchQueue::main().after(when, move || {
                with(|a| a.fetching.retain(|f| *f != token));
            });
            return;
        };
        with(|a| a.fetching.retain(|f| *f != token));
        match kind {
            More::Page => {
                let next = if more.sections.is_empty() { None } else { more.continuation };
                grow_page(generation, |page| {
                    if page.continuation.as_deref() != Some(&*token) {
                        return false;
                    }
                    page.sections.extend(more.sections);
                    page.continuation = next;
                    true
                });
            }
            More::List => {
                let (entries, next) = more
                    .sections
                    .into_iter()
                    .find_map(|s| match s {
                        Section::List { entries, continuation, .. } => Some((entries, continuation)),
                        _ => None,
                    })
                    .unwrap_or_default();
                let next = if entries.is_empty() { None } else { next };
                grow_page(generation, |page| {
                    let Some(Section::List { entries: list, continuation, .. }) = page
                        .sections
                        .iter_mut()
                        .find(|s| matches!(s, Section::List { continuation: Some(c), .. } if *c == token))
                    else {
                        return false;
                    };
                    list.extend(entries.iter().cloned());
                    *continuation = next.clone();
                    true
                });
                feed_queue(&token, entries, next, generation);
            }
        }
    });
}

/// Replaces the page on screen, if it's still `generation`, with a grown
/// copy of it, keeping the scroll position.
fn grow_page(generation: u64, grow: impl FnOnce(&mut Page) -> bool) {
    let grown = with(|a| {
        if a.generation != generation {
            return false;
        }
        let Some(current) = &a.page else { return false };
        let mut page = (**current).clone();
        if !grow(&mut page) {
            return false;
        }
        let page = Rc::new(page);
        a.page = Some(page.clone());
        if let Some(view) = a.view.as_mut() {
            view.set_page(page, &mut a.items);
            view.refresh();
        }
        true
    });
    if grown == Some(true) {
        // Keep going until the screen is full.
        net::later(check_more);
    }
}

/// Hands the part of a list that `token` pointed to to the queue that's
/// following that list, and asks for the part after it.
fn feed_queue(token: &str, entries: Vec<Entry>, next: Option<Box<str>>, generation: u64) {
    let fetch_next = with(|a| {
        let mut follow = a.follow.take()?;
        if follow.queue_id != a.queue_id || &*follow.next != token {
            a.follow = Some(follow);
            return None;
        }
        let entries = decorate(entries, follow.header.as_ref());
        if follow.shuffle {
            a.queue.extend_shuffled(entries, &mut a.rng);
        } else {
            a.queue.extend(entries);
        }
        let next = next?;
        follow.next = next.clone();
        a.follow = Some(follow);
        Some(next)
    })
    .flatten();
    carry_on();
    if let Some(next) = fetch_next {
        fetch_more(More::List, next, generation);
    }
}

/// If the queue ran out before more tracks arrived, plays the next one now
/// that they have.
fn carry_on() {
    if with(|a| a.ran_out && a.queue.advance().is_some()).unwrap_or(false) {
        play_current(0.0);
    }
}

/// Moves keyboard focus off the search field once a search has been sent.
pub fn release_search_focus() {
    with_shell(|s| s.window.makeFirstResponder(Some(&s.focus_sink)));
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

fn show_page(page: Rc<Page>) {
    let mtm = mtm();
    let Some(scale) = with(|a| {
        a.view = None;
        a.page = Some(page.clone());
        a.shell.as_ref().map(|s| s.window.backingScaleFactor())
    })
    .flatten() else {
        return; // the window is closed: the page shows when it reopens
    };
    let mut items = Items::new();
    let view = PageView::new(page, scale, &mut items, mtm);
    let root = view.root.clone();
    with(|a| a.items = items);
    set_content(&root);
    // Lay out now so the first refresh knows the page's size.
    with(|a| {
        if let Some(s) = &a.shell {
            s.window.layoutIfNeeded();
            view.refresh();
            crate::debug::schedule_snapshot(&s.window);
        }
        a.view = Some(view);
    });
    images::sweep_soon();
    check_more();
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

fn message(title: &str, body: &str, button: Option<(&str, objc2::runtime::Sel)>) {
    let mtm = mtm();
    with(|a| a.view = None);
    if with(|a| a.shell.is_none()).unwrap_or(true) {
        return;
    }
    let t = ui::label(title, 24.0, Weight::Bold, false, mtm);
    let b = ui::paragraph(body, 14.0, true, 4, mtm);
    b.setAlignment(objc2_app_kit::NSTextAlignment::Center);
    b.widthAnchor().constraintLessThanOrEqualToConstant(440.0).setActive(true);
    let mut views: Vec<Retained<NSView>> = vec![Retained::into_super(Retained::into_super(t)), Retained::into_super(Retained::into_super(b))];
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

pub fn sign_in() {
    if with(|a| std::mem::replace(&mut a.signing_in, true)).unwrap_or(true) {
        return;
    }
    message("Signing in…", "Finish signing in in the window that just opened.", None);
    let exe = std::env::current_exe().ok();
    net::run(
        net::Pool::Api,
        move || -> Vec<Cookie> {
            let Some(exe) = exe else { return Vec::new() };
            let Ok(out) = std::process::Command::new(exe).arg("--login-helper").output() else { return Vec::new() };
            String::from_utf8_lossy(&out.stdout)
                .lines()
                .filter_map(|line| {
                    let mut f = line.split('\t');
                    (f.next()? == "cookie").then_some(())?;
                    Some(Cookie {
                        domain: f.next()?.to_owned(),
                        name: f.next()?.to_owned(),
                        value: f.next()?.to_owned(),
                        expires: f.next()?.parse().ok()?,
                    })
                })
                .collect()
        },
        |cookies| {
            with(|a| a.signing_in = false);
            match Session::from_cookies(&cookies) {
                Some(session) => start(session),
                None => show_sign_in(),
            }
        },
    );
}

pub fn focus_search() {
    with_shell(|s| s.window.makeFirstResponder(Some(&s.search)));
}

/// A card or row was clicked.
pub fn activate(item: u32) {
    let Some(Some((entry, siblings, rest, header, generation))) = with(|a| {
        let &(si, ei) = a.items.get(item as usize)?;
        let page = a.page.as_ref()?;
        let section = page.sections.get(si)?;
        let rest = match section {
            Section::List { continuation, .. } => Some(continuation.clone()),
            _ => None,
        };
        Some((section.entries().get(ei)?.clone(), section.entries().to_vec(), rest, page.header.clone(), a.generation))
    }) else {
        return;
    };
    match &entry.target {
        Target::Play { video_id, .. } => {
            // A row in a list plays the list from there, like YouTube Music.
            let (pool, rest) = match rest {
                Some(rest) => (siblings, rest),
                None => (vec![entry.clone()], None),
            };
            let follow = rest.map(|next| Follow { queue_id: 0, shuffle: false, header: header.clone(), next });
            start_queue(Queue::from_entries(&decorate(pool, header.as_ref()), video_id), follow, generation);
        }
        Target::PlayPlaylist { playlist_id } => play_playlist(playlist_id.to_string()),
        Target::Browse { id, params, .. } => go(Route::Browse { id: id.clone(), params: params.clone() }),
        Target::None => {}
    }
}

/// Album tracks come without art or artist; borrow the album's.
fn decorate(mut entries: Vec<Entry>, header: Option<&Header>) -> Vec<Entry> {
    if let Some(h) = header {
        let artist = h.detail.split(" • ").next().unwrap_or("").to_owned();
        for e in &mut entries {
            if e.thumb.is_none() {
                e.thumb = h.thumb.clone();
                if !artist.is_empty() {
                    e.subtitle = artist.clone().into();
                }
            }
        }
    }
    entries
}

/// "Play" / "Shuffle" on a page header. A long list starts right away with
/// what's loaded; the rest joins the queue as it arrives.
pub fn play_all(shuffle: bool) {
    let Some(Some((mut entries, header, rest, generation))) = with(|a| {
        let page = a.page.as_ref()?;
        let list = page.sections.iter().find(|s| matches!(s, Section::List { .. }))?;
        let rest = match list {
            Section::List { continuation, .. } => continuation.clone(),
            _ => None,
        };
        let entries = list.entries().iter().filter(|e| e.video_id().is_some()).cloned().collect::<Vec<_>>();
        Some((entries, page.header.clone(), rest, a.generation))
    }) else {
        return;
    };
    if shuffle {
        with(|a| a.rng.shuffle(&mut entries));
    }
    let Some(first) = entries.first().and_then(|e| e.video_id().map(str::to_owned)) else { return };
    let follow = rest.map(|next| Follow { queue_id: 0, shuffle, header: header.clone(), next });
    start_queue(Queue::from_entries(&decorate(entries, header.as_ref()), &first), follow, generation);
}

fn play_playlist(playlist_id: String) {
    let Some(Some(client)) = with(|a| a.client.clone()) else { return };
    net::run(
        net::Pool::Api,
        move || client.next_playlist(&playlist_id).and_then(|j| kilo_core::parse::up_next(&j)),
        |result| {
            if let Ok(entries) = result
                && let Some(first) = entries.first().and_then(|e| e.video_id().map(str::to_owned))
            {
                start_queue(Queue::from_entries(&entries, &first), None, 0);
            }
        },
    );
}

/// Plays a song by id, queued with its radio (as YouTube Music does when a
/// single song is started).
pub fn play_video(video_id: String) {
    let Some(Some(client)) = with(|a| a.client.clone()) else { return };
    net::run(
        net::Pool::Api,
        move || client.next(&video_id, None).and_then(|j| kilo_core::parse::up_next(&j)).map(|e| (e, video_id)),
        |result| {
            if let Ok((entries, id)) = result {
                start_queue(Queue::from_entries(&entries, &id), None, 0);
            }
        },
    );
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

/// Starts playing `queue`. With `follow`, the rest of the list it came from
/// (page `generation`) is fetched and added as it arrives.
fn start_queue(queue: Queue, follow: Option<Follow>, generation: u64) {
    let next = with(|a| {
        a.queue = queue;
        a.queue_id += 1;
        a.radio_for = None;
        a.follow = follow.map(|f| Follow { queue_id: a.queue_id, ..f });
        a.follow.as_ref().map(|f| f.next.clone())
    })
    .flatten();
    play_current(0.0);
    if let Some(next) = next {
        fetch_more(More::List, next, generation);
    }
}

fn play_current(start: f64) {
    let Some(Some((entry, token, client))) = with(|a| {
        a.play_token += 1;
        a.ran_out = false;
        a.queue.current().cloned().map(|e| (e, a.play_token, a.client.clone()))
    }) else {
        return;
    };
    show_now_playing(&entry);
    // Start the helper now; it takes longer than the lookup below.
    ensure_player();
    if entry.is_music_video()
        && start == 0.0
        && let (Some(client), Some(id)) = (client, entry.video_id().map(str::to_owned))
    {
        // Kilo never shows video: a music video plays as its song version
        // when there is one. Same music; the player streams a still
        // picture instead of video (measured: 41% less data).
        net::run(
            net::Pool::Api,
            move || client.next_single(&id).and_then(|j| kilo_core::parse::up_next(&j)),
            move |result| {
                if with(|a| a.play_token) != Some(token) {
                    return; // the user moved on
                }
                if let Some(song) = result.ok().and_then(|q| q.into_iter().next()).filter(|s| !s.is_music_video() && s.video_id().is_some()) {
                    with(|a| a.queue.replace_current(song.clone()));
                    show_now_playing(&song);
                }
                load_current(start);
            },
        );
        return;
    }
    load_current(start);
}

/// Hands the current track to the player.
fn load_current(start: f64) {
    let Some(Some(entry)) = with(|a| a.queue.current().cloned()) else { return };
    let Some(id) = entry.video_id().and_then(VideoId::parse) else { return };
    ensure_player();
    with(|a| {
        a.playing = false;
        a.position = start;
        a.duration = 0.0;
        a.position_at = Instant::now();
    });
    send(Command::Load(id, start));
    extend_with_radio();
}

/// Like YouTube Music: when the queue runs out, keep going with the radio
/// of what's playing. Not while the rest of a list is still on its way.
fn extend_with_radio() {
    let Some(Some((client, id))) = with(|a| {
        let current = a.queue.current()?.video_id()?.to_owned();
        (a.follow.is_none() && a.queue.needs_more() && a.radio_for.as_deref() != Some(current.as_str())).then(|| {
            a.radio_for = Some(current.clone().into());
            (a.client.clone(), current)
        })
    }) else {
        return;
    };
    let Some(client) = client else { return };
    net::run(
        net::Pool::Api,
        move || client.next(&id, None).and_then(|j| kilo_core::parse::up_next(&j)),
        |result| {
            if let Ok(entries) = result {
                with(|a| a.queue.extend(entries));
                carry_on();
            }
        },
    );
}

fn ensure_player() {
    let missing = with(|a| a.player.is_none()).unwrap_or(false);
    if !missing {
        return;
    }
    let Ok(exe) = std::env::current_exe() else { return };
    let volume = with(|a| a.volume).unwrap_or(100);
    match PlayerProcess::spawn(&exe, |event| net::later(move || on_player_event(event))) {
        Ok(player) => {
            with(|a| a.player = Some(player));
            if volume < 100 {
                send(Command::Volume(volume));
            }
        }
        Err(e) => show_status(&format!("Couldn't start the player: {e}")),
    }
}

fn send(cmd: Command) {
    with(|a| {
        if let Some(p) = &mut a.player
            && p.send(&cmd).is_err()
        {
            crate::debug::trace(|| "player: sending failed; dropping the helper".into());
            a.player = None;
        }
    });
}

fn on_player_event(event: Event) {
    crate::debug::trace(|| format!("player: {}", event.encode()));
    match event {
        Event::Playing(p) => {
            with(|a| {
                a.playing = true;
                a.position = p.seconds;
                a.duration = p.duration;
                a.position_at = Instant::now();
                a.idle_token += 1;
                a.idle_armed = false;
                if let Some(s) = &a.shell {
                    ui::set_symbol(&s.bar.play, "pause.fill");
                    s.bar.progress.set_enabled(true);
                }
            });
            sync_timer();
            update_progress();
        }
        Event::Paused(ref p) | Event::Buffering(ref p) => {
            let paused = matches!(event, Event::Paused(_));
            with(|a| {
                a.position = p.seconds;
                a.duration = p.duration.max(a.duration);
                a.position_at = Instant::now();
                if paused {
                    a.playing = false;
                    if let Some(s) = &a.shell {
                        ui::set_symbol(&s.bar.play, "play.fill");
                    }
                }
            });
            if paused {
                sync_timer();
                schedule_idle_shutdown();
            }
            update_progress();
        }
        Event::Ended(_) => {
            let advanced = with(|a| a.queue.advance().is_some()).unwrap_or(false);
            if advanced {
                play_current(0.0);
            } else {
                with(|a| {
                    a.playing = false;
                    a.ran_out = true;
                    if let Some(s) = &a.shell {
                        ui::set_symbol(&s.bar.play, "play.fill");
                    }
                });
                sync_timer();
                // Nothing more to play unless more tracks arrive.
                schedule_idle_shutdown();
            }
        }
        Event::SignedOut => {
            with(|a| a.player = None);
            show_sign_in();
        }
        Event::AdShowing => {
            if let Some(Some(p)) = with(|a| a.player.take()) {
                p.quit(Duration::from_millis(500));
            }
            show_status("Playback needs a YouTube Music Premium account.");
        }
        Event::Error(msg) => {
            eprintln!("player: {msg}");
            if msg.contains("terminated") || msg.contains("exited") {
                // macOS 27 itself quits an idle helper (SIGTERM, "quiet safe
                // quit") some minutes after playback stops: like our own
                // idle shutdown, the next play starts a fresh one.
                if let Some(Some(p)) = with(|a| a.player.take()) {
                    net::run(net::Pool::Api, move || p.wait(), |status| {
                        crate::debug::trace(|| format!("player: helper ended with {status:?}"));
                    });
                }
            }
        }
        Event::Next => next(),
        Event::Previous => previous(),
        Event::Ready => {}
    }
}

pub fn play_pause() {
    let Some((has_player, playing, position)) = with(|a| (a.player.is_some(), a.playing, a.position)) else { return };
    if !has_player {
        // The helper was shut down while idle: start a new one and pick up
        // where playback stopped.
        play_current(position);
    } else if playing {
        send(Command::Pause);
    } else {
        send(Command::Play);
    }
}

pub fn next() {
    if with(|a| a.queue.advance().is_some()).unwrap_or(false) {
        play_current(0.0);
    }
}

pub fn previous() {
    let pos = with(|a| current_position(a)).unwrap_or(0.0);
    if pos > 3.0 {
        send(Command::Seek(0.0));
    } else if with(|a| a.queue.back().is_some()).unwrap_or(false) {
        play_current(0.0);
    }
}

pub fn seek(fraction: f64) {
    let Some(duration) = with(|a| a.duration) else { return };
    if duration > 0.0 {
        let to = (fraction.clamp(0.0, 1.0) * duration).max(0.0);
        with(|a| {
            a.position = to;
            a.position_at = Instant::now();
        });
        send(Command::Seek(to));
    }
}

pub fn set_volume(volume: u8) {
    with(|a| a.volume = volume.min(100));
    send(Command::Volume(volume.min(100)));
}

fn current_position(a: &App) -> f64 {
    let pos = if a.playing { a.position + a.position_at.elapsed().as_secs_f64() } else { a.position };
    if a.duration > 0.0 { pos.min(a.duration) } else { pos }
}

fn show_now_playing(entry: &Entry) {
    with(|a| {
        let Some(s) = &a.shell else { return };
        let bar = &s.bar;
        bar.title.setStringValue(&NSString::from_str(&entry.title));
        bar.artist.setStringValue(&NSString::from_str(&entry.subtitle));
        bar.time.setStringValue(&NSString::from_str(""));
        bar.progress.set_value(0.0);
        ui::set_image(&bar.art, None);
        if let Some(t) = &entry.thumb {
            let px = (44.0 * s.window.backingScaleFactor()) as u32;
            let art = bar.art.clone();
            images::load(t.sized(px), px, move |img| ui::set_image(&art, img.as_ref()));
        }
    });
}

fn show_status(text: &str) {
    with_shell(|s| {
        s.bar.title.setStringValue(&NSString::from_str(text));
        s.bar.artist.setStringValue(&NSString::from_str(""));
    });
}

/// The progress timer runs only while music plays and the window is on
/// screen; a closed, minimized or covered window costs no wakeups.
fn sync_timer() {
    let want = with(|a| a.playing && a.shell.as_ref().is_some_and(|s| s.window.occlusionState().contains(NSWindowOcclusionState::Visible)))
        .unwrap_or(false);
    if want {
        start_timer();
        update_progress();
    } else {
        stop_timer();
    }
}

/// Moves the progress bar once a second. The position is extrapolated
/// locally; the player helper sends nothing while playing.
fn start_timer() {
    if with(|a| a.timer.is_some()).unwrap_or(true) {
        return;
    }
    let tick = RcBlock::new(|_t: NonNull<NSTimer>| update_progress());
    // SAFETY: the timer retains the block; runs on the main run loop.
    let timer = unsafe { NSTimer::scheduledTimerWithTimeInterval_repeats_block(1.0, true, &tick) };
    // Let macOS coalesce our wakeups with others.
    timer.setTolerance(0.3);
    with(|a| a.timer = Some(timer));
}

fn stop_timer() {
    if let Some(Some(t)) = with(|a| a.timer.take()) {
        t.invalidate();
    }
}

fn update_progress() {
    with(|a| {
        let pos = current_position(a);
        let Some(s) = &a.shell else { return };
        if a.duration > 0.0 {
            s.bar.progress.set_value(pos / a.duration);
            s.bar.time.setStringValue(&NSString::from_str(&format!("{} / {}", clock(pos), clock(a.duration))));
        }
    });
}

fn clock(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    if s >= 3600 { format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60) } else { format!("{}:{:02}", s / 60, s % 60) }
}

/// Starts the countdown to shutting the helper down once playback stops
/// (paused, or the queue ran out). It runs from the first stop: later
/// "paused" reports from the page don't restart it; only playing again
/// cancels it. With the window closed, App Nap may make it fire a little
/// late, and macOS may quit the idle helper on its own first.
fn schedule_idle_shutdown() {
    let Some(Some(token)) = with(|a| (!std::mem::replace(&mut a.idle_armed, true)).then_some(a.idle_token)) else {
        return;
    };
    let idle = crate::debug::idle_shutdown().unwrap_or(IDLE_SHUTDOWN);
    let when = DispatchTime::NOW.time(idle.as_nanos() as i64);
    let _ = DispatchQueue::main().after(when, move || {
        let player = with(|a| {
            if a.idle_token != token {
                return None;
            }
            a.idle_armed = false;
            if a.playing { None } else { a.player.take() }
        })
        .flatten();
        if let Some(player) = player {
            crate::debug::trace(|| "idle: shutting the player helper down".into());
            net::run(net::Pool::Api, move || player.quit(Duration::from_secs(2)), |()| {});
        }
    });
}

fn schedule_refresh() {
    if REFRESH_PENDING.replace(true) {
        return;
    }
    net::later(|| {
        REFRESH_PENDING.set(false);
        with(|a| {
            if let Some(v) = &a.view {
                v.refresh();
            }
        });
        images::sweep_soon();
        check_more();
    });
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
    sync_timer();
    images::purge_memory();
}

fn show_window() {
    if let Some(window) = with_shell(|s| s.window.clone()) {
        window.makeKeyAndOrderFront(None);
        return;
    }
    open_window();
    match with(|a| a.page.clone()).flatten() {
        Some(page) => show_page(page),
        None => reload(),
    }
}

/// Developer switch: scrolls the page to `y` points, or to its end.
pub fn scroll_to(y: Option<f64>) {
    with(|a| {
        if let Some(v) = &a.view {
            v.scroll_to(y);
        }
    });
}

/// Developer switch: a one-line summary of the page and the queue.
pub fn describe() -> String {
    with(|a| {
        let lists: Vec<String> = a
            .page
            .iter()
            .flat_map(|p| p.sections.iter())
            .map(|s| match s {
                Section::List { entries, continuation, .. } => format!("list {}{}", entries.len(), if continuation.is_some() { "+" } else { "" }),
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

/// Developer switch: plays one music video the way a click on its card
/// does (a queue of one, then its radio).
pub fn play_music_video(video_id: &str) {
    let entry = Entry {
        title: video_id.into(),
        subtitle: "".into(),
        thumb: None,
        target: Target::Play { video_id: video_id.into(), playlist_id: None, music_video: true },
        duration: "".into(),
    };
    start_queue(Queue::from_entries(&[entry], video_id), None, 0);
}
