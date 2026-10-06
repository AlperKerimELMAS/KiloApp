//! The app: window, navigation, sign-in, queue and the player helper.
//!
//! All state lives on the main thread. Network work runs on `net` workers
//! and comes back here; the player helper's events do too.

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
use kilo_core::model::{Entry, Page, Section, Target};
use kilo_core::queue::Queue;
use kilo_player::host::PlayerProcess;
use kilo_player::protocol::{Command, Event, VideoId};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate, NSProgressIndicator, NSProgressIndicatorStyle,
    NSView, NSViewBoundsDidChangeNotification, NSWindowDelegate, NSWindowDidResizeNotification,
};
use objc2_foundation::{NSNotification, NSNotificationCenter, NSString, NSTimer};

use crate::ui::page::{Items, PageView};
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

struct App {
    shell: Shell,
    client: Option<Arc<Client>>,
    history: Vec<Route>,
    generation: u64,
    page: Option<Rc<Page>>,
    view: Option<PageView>,
    items: Items,
    queue: Queue,
    player: Option<PlayerProcess>,
    playing: bool,
    position: f64,
    duration: f64,
    position_at: Instant,
    volume: u8,
    idle_token: u64,
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
            // Nothing on screen: let go of every view and decoded image.
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
    let shell = shell::build(mtm);
    DELEGATE.with_borrow(|d| {
        if let Some(d) = d {
            shell.window.setDelegate(Some(ProtocolObject::from_ref(&**d)));
        }
    });
    shell.window.makeKeyAndOrderFront(None);
    shell.window.makeFirstResponder(Some(&shell.focus_sink));
    if crate::debug::activate_on_launch() {
        NSApplication::sharedApplication(mtm).activate();
    }

    // Scrolling or resizing reveals thumbnails; load them (coalesced).
    let center = NSNotificationCenter::defaultCenter();
    let refresh = RcBlock::new(|_n: NonNull<NSNotification>| schedule_refresh());
    // SAFETY: the notification names are constant strings; the block is
    // retained by the notification center.
    let observers = unsafe {
        vec![
            center.addObserverForName_object_queue_usingBlock(Some(NSViewBoundsDidChangeNotification), None, None, &refresh),
            center.addObserverForName_object_queue_usingBlock(Some(NSWindowDidResizeNotification), None, None, &refresh),
        ]
    };
    images::trim_disk_cache();

    APP.set(Some(App {
        shell,
        client: None,
        history: Vec::new(),
        generation: 0,
        page: None,
        view: None,
        items: Vec::new(),
        queue: Queue::default(),
        player: None,
        playing: false,
        position: 0.0,
        duration: 0.0,
        position_at: Instant::now(),
        volume: 100,
        idle_token: 0,
        radio_for: None,
        timer: None,
        signing_in: false,
        _observers: observers,
    }));

    match saved_session() {
        Some(session) => start(session),
        None => show_sign_in(),
    }
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
    }
}

fn load(route: Route) {
    let Some((client, generation)) = with(|a| {
        a.generation += 1;
        a.shell.back.setEnabled(a.history.len() > 1);
        let nav = match route {
            Route::Home => Some(0),
            Route::Explore => Some(1),
            Route::Library => Some(2),
            _ => None,
        };
        shell::select_nav(&a.shell, nav);
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

fn fetch(client: &Client, route: &Route) -> kilo_core::Result<Page> {
    use kilo_core::parse;
    match route {
        Route::Home => {
            let mut page = parse::browse(&client.browse("FEmusic_home", None)?)?;
            // The home feed comes in pages; two more make a full screen.
            for _ in 0..2 {
                let Some(token) = page.continuation.take() else { break };
                let more = parse::browse(&client.continuation(&token)?)?;
                page.sections.extend(more.sections);
                page.continuation = more.continuation;
            }
            Ok(page)
        }
        Route::Explore => parse::browse(&client.browse("FEmusic_explore", None)?),
        Route::Library => parse::browse(&client.browse("FEmusic_liked_playlists", None)?),
        Route::Browse { id, params } => parse::browse(&client.browse(id, params.as_deref())?),
        Route::Search(q) => parse::search(&client.search(q, None)?),
    }
}

/// Moves keyboard focus off the search field once a search has been sent.
pub fn release_search_focus() {
    with(|a| a.shell.window.makeFirstResponder(Some(&a.shell.focus_sink)));
}

fn set_content(view: &NSView) {
    with(|a| {
        for old in a.shell.content.subviews().iter() {
            old.removeFromSuperview();
        }
        a.shell.content.addSubview(view);
        ui::pin(view, &a.shell.content, 0.0, 0.0, 0.0, 0.0);
    });
}

fn show_page(page: Rc<Page>) {
    let mtm = mtm();
    let scale = with(|a| a.shell.window.backingScaleFactor()).unwrap_or(2.0);
    let mut items = Items::new();
    let view = PageView::new(page.clone(), scale, &mut items, mtm);
    let root = view.root.clone();
    with(|a| {
        a.view = None;
        a.page = Some(page);
        a.items = items;
    });
    set_content(&root);
    // Lay out now so the first refresh knows the page's size.
    with(|a| {
        a.shell.window.layoutIfNeeded();
        view.refresh();
        a.view = Some(view);
        crate::debug::schedule_snapshot(&a.shell.window);
    });
}

fn show_loading() {
    let mtm = mtm();
    let spinner = NSProgressIndicator::new(mtm);
    spinner.setStyle(NSProgressIndicatorStyle::Spinning);
    // SAFETY: plain AppKit call on the main thread.
    unsafe { spinner.startAnimation(None) };
    with(|a| a.view = None);
    let holder = NSView::new(mtm);
    holder.addSubview(&spinner);
    spinner.setTranslatesAutoresizingMaskIntoConstraints(false);
    spinner.centerXAnchor().constraintEqualToAnchor(&holder.centerXAnchor()).setActive(true);
    spinner.centerYAnchor().constraintEqualToAnchor(&holder.centerYAnchor()).setActive(true);
    set_content(&holder);
}

fn message(title: &str, body: &str, button: Option<(&str, objc2::runtime::Sel)>) {
    let mtm = mtm();
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
    with(|a| a.view = None);
    set_content(&holder);
}

fn show_error(text: &str) {
    message("Something went wrong", text, Some(("Try again", sel!(retry:))));
}

fn show_sign_in() {
    with(|a| shell::select_nav(&a.shell, None));
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
    with(|a| a.shell.window.makeFirstResponder(Some(&a.shell.search)));
}

/// A card or row was clicked.
pub fn activate(item: u32) {
    let Some(Some((entry, siblings, is_list, header))) = with(|a| {
        let &(si, ei) = a.items.get(item as usize)?;
        let page = a.page.as_ref()?;
        let section = page.sections.get(si)?;
        Some((section.entries().get(ei)?.clone(), section.entries().to_vec(), matches!(section, Section::List { .. }), page.header.clone()))
    }) else {
        return;
    };
    match &entry.target {
        Target::Play { video_id, .. } => {
            let pool = if is_list { siblings } else { vec![entry.clone()] };
            let pool = decorate(pool, header.as_ref());
            start_queue(Queue::from_entries(&pool, video_id));
        }
        Target::PlayPlaylist { playlist_id } => play_playlist(playlist_id.to_string()),
        Target::Browse { id, params, .. } => go(Route::Browse { id: id.clone(), params: params.clone() }),
        Target::None => {}
    }
}

/// Album tracks come without art or artist; borrow the album's.
fn decorate(mut entries: Vec<Entry>, header: Option<&kilo_core::model::Header>) -> Vec<Entry> {
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

/// "Play" / "Shuffle" on a page header.
pub fn play_all(shuffle: bool) {
    let Some(Some((mut entries, header))) = with(|a| {
        let page = a.page.as_ref()?;
        let list = page.sections.iter().find(|s| matches!(s, Section::List { .. }))?;
        Some((list.entries().iter().filter(|e| e.video_id().is_some()).cloned().collect::<Vec<_>>(), page.header.clone()))
    }) else {
        return;
    };
    if shuffle {
        // xorshift seeded by the clock: good enough to shuffle a playlist.
        let mut x = Instant::now().elapsed().as_nanos() as u64 ^ 0x9E37_79B9_7F4A_7C15 ^ std::process::id() as u64;
        for i in (1..entries.len()).rev() {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            entries.swap(i, (x % (i as u64 + 1)) as usize);
        }
    }
    let Some(first) = entries.first().and_then(|e| e.video_id().map(str::to_owned)) else { return };
    start_queue(Queue::from_entries(&decorate(entries, header.as_ref()), &first));
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
                start_queue(Queue::from_entries(&entries, &first));
            }
        },
    );
}

fn start_queue(queue: Queue) {
    with(|a| {
        a.queue = queue;
        a.radio_for = None;
    });
    play_current(0.0);
}

fn play_current(start: f64) {
    let Some(Some(entry)) = with(|a| a.queue.current().cloned()) else { return };
    let Some(id) = entry.video_id().and_then(VideoId::parse) else { return };
    show_now_playing(&entry);
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
/// of what's playing.
fn extend_with_radio() {
    let Some(Some((client, id))) = with(|a| {
        let current = a.queue.current()?.video_id()?.to_owned();
        (a.queue.needs_more() && a.radio_for.as_deref() != Some(current.as_str())).then(|| {
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
            a.player = None;
        }
    });
}

fn on_player_event(event: Event) {
    match event {
        Event::Playing(p) => {
            with(|a| {
                a.playing = true;
                a.position = p.seconds;
                a.duration = p.duration;
                a.position_at = Instant::now();
                a.idle_token += 1;
                ui::set_symbol(&a.shell.bar.play, "pause.fill");
                a.shell.bar.progress.set_enabled(true);
            });
            start_timer();
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
                    ui::set_symbol(&a.shell.bar.play, "play.fill");
                }
            });
            if paused {
                stop_timer();
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
                    ui::set_symbol(&a.shell.bar.play, "play.fill");
                });
                stop_timer();
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
                with(|a| a.player = None);
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
    let mtm = mtm();
    let _ = mtm;
    with(|a| {
        let bar = &a.shell.bar;
        bar.title.setStringValue(&NSString::from_str(&entry.title));
        bar.artist.setStringValue(&NSString::from_str(&entry.subtitle));
        bar.time.setStringValue(&NSString::from_str(""));
        bar.progress.set_value(0.0);
        ui::set_image(&bar.art, None);
        if let Some(t) = &entry.thumb {
            let px = (44.0 * a.shell.window.backingScaleFactor()) as u32;
            let art = bar.art.clone();
            images::load(t.sized(px), px, move |img| ui::set_image(&art, img.as_ref()));
        }
    });
}

fn show_status(text: &str) {
    with(|a| {
        a.shell.bar.title.setStringValue(&NSString::from_str(text));
        a.shell.bar.artist.setStringValue(&NSString::from_str(""));
    });
}

/// Moves the progress bar once a second while playing. The position is
/// extrapolated locally; the player helper sends nothing while playing.
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
        let bar = &a.shell.bar;
        if a.duration > 0.0 {
            bar.progress.set_value(pos / a.duration);
            bar.time.setStringValue(&NSString::from_str(&format!("{} / {}", clock(pos), clock(a.duration))));
        }
    });
}

fn clock(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    if s >= 3600 { format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60) } else { format!("{}:{:02}", s / 60, s % 60) }
}

fn schedule_idle_shutdown() {
    let Some(token) = with(|a| {
        a.idle_token += 1;
        a.idle_token
    }) else {
        return;
    };
    let when = DispatchTime::NOW.time(IDLE_SHUTDOWN.as_nanos() as i64);
    let _ = DispatchQueue::main().after(when, move || {
        let player = with(|a| (a.idle_token == token && !a.playing).then(|| a.player.take()).flatten()).flatten();
        if let Some(player) = player {
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
    });
}

fn window_closed() {
    with(|a| {
        a.view = None;
        for old in a.shell.content.subviews().iter() {
            old.removeFromSuperview();
        }
    });
    images::purge_memory();
}

fn show_window() {
    let page = with(|a| {
        a.shell.window.makeKeyAndOrderFront(None);
        a.view.is_none().then(|| a.page.clone()).flatten()
    })
    .flatten();
    if let Some(page) = page {
        show_page(page);
    }
}

#[allow(dead_code)]
fn _assert_any(_: &AnyObject) {}
