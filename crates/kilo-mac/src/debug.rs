//! Developer switches (environment variables), all off by default:
//! - `KILO_SNAPSHOT=/path/shot`: after each page appears, render the window
//!   to `/path/shot-N.png` (no Screen Recording permission needed).
//! - `KILO_OPEN=search:QUERY` or `browse:ID`: open that page after Home.
//! - `KILO_NO_ACTIVATE=1`: don't bring the app to the front on launch.
//! - `KILO_IDLE=SECS`: shut the player helper down after SECS paused.
//! - `KILO_SCENARIO=play:ID,wait:90,close,wait:60,pause,wait:330`: a scripted
//!   session for measurements; each step is logged to stderr with the time
//!   since launch. Steps: `play:ID`, `pause`, `next`, `close`, `open`,
//!   `playvideo:ID` (as a click on a music video's card), `browse:ID`,
//!   `scroll:Y` (or `scroll:end`), `playall`, `shuffleall`, `volume:0-100`
//!   (`volume:0` first to test playback silently), `theme:system|light|dark`,
//!   `lang:system|en|tr`, `playcard:N` (a card's play button, item N of the
//!   page), `snap` (a snapshot in 4 s, with `KILO_SNAPSHOT`; `snap:now` at
//!   once), `state` (logs the
//!   page and queue), `focus` (the search field), `key:SPEC` (a key press as
//!   if typed: `space`, `m`, `cmd+right`, `shift+cmd+left`, `esc`, …),
//!   `keys` (logs the volume, what has the focus and the search text),
//!   `wheel:DX/DY/PHASE` (a trackpad scroll over the page's first shelf;
//!   PHASE is `maybegin`, `began`, `changed`, `ended` or `none` for a mouse
//!   wheel; logs where the page and the shelf are scrolled), `relayout:N`
//!   (resizes the window N times; logs the page's average relayout time),
//!   `app` (logs whether Kilo is active and owns the menu bar), `front`
//!   (logs the frontmost app), `signin` and `cancelsignin` (in a copy with
//!   another bundle id: the real app's helpers use the owner's session),
//!   `wait:SECS`.

use std::cell::Cell;

use objc2::Message;
use objc2_app_kit::NSWindow;
use objc2_core_foundation::{CFString, CFURL, CGPoint, CGRect, CGSize};
use objc2_core_graphics::{CGBitmapContextCreateImage, CGColorSpace, CGContext};
use objc2_image_io::CGImageDestination;

thread_local! {
    static SHOTS: Cell<u32> = const { Cell::new(0) };
}

/// `KILO_IDLE=SECS`: shut the player helper down after this long paused
/// instead of 5 minutes (for measuring what's left afterwards).
pub fn idle_shutdown() -> Option<std::time::Duration> {
    let secs: f64 = std::env::var("KILO_IDLE").ok()?.parse().ok()?;
    Some(std::time::Duration::from_secs_f64(secs))
}

pub fn activate_on_launch() -> bool {
    std::env::var_os("KILO_NO_ACTIVATE").is_none()
}

/// The page to open after Home, if `KILO_OPEN` asks for one.
pub fn open_route() -> Option<crate::app::Route> {
    let v = std::env::var("KILO_OPEN").ok()?;
    let (kind, arg) = v.split_once(':')?;
    match kind {
        "search" => Some(crate::app::Route::Search(arg.to_owned())),
        "browse" => Some(crate::app::Route::Browse { id: arg.into(), params: None }),
        _ => None,
    }
}

/// If `KILO_SNAPSHOT` is set, renders `window` to a PNG a few seconds from
/// now (once thumbnails have had time to arrive).
pub fn schedule_snapshot(window: &NSWindow) {
    let Some(path) = next_shot() else { return };
    /// Carries a main-thread object through GCD's main queue.
    struct MainOnly<T>(T);
    // SAFETY: the value is created on the main thread and only used again
    // on the main queue, which runs on the main thread.
    unsafe impl<T> Send for MainOnly<T> {}
    let window = MainOnly(window.retain());
    let when = dispatch2::DispatchTime::NOW.time(4_000_000_000);
    let _ = dispatch2::DispatchQueue::main().after(when, move || {
        // Captures the whole wrapper, not just its (non-`Send`) field.
        let window = window;
        render(&window.0, &path);
    });
}

/// If `KILO_SNAPSHOT` is set, renders `window` to a PNG right now (a
/// moment in an animation).
pub fn snapshot_now(window: &NSWindow) {
    if let Some(path) = next_shot() {
        render(window, &path);
    }
}

fn next_shot() -> Option<String> {
    let base = std::env::var_os("KILO_SNAPSHOT")?;
    let n = SHOTS.with(|s| {
        s.set(s.get() + 1);
        s.get()
    });
    Some(format!("{}-{n}.png", base.to_string_lossy()))
}

fn render(window: &NSWindow, path: &str) {
    let Some(view) = window.contentView() else { return };
    let Some(layer) = view.layer() else { return };
    let scale = window.backingScaleFactor();
    let size = view.bounds().size;
    let (w, h) = ((size.width * scale) as usize, (size.height * scale) as usize);
    let Some(space) = CGColorSpace::new_device_rgb() else { return };
    // SAFETY: no data pointer: CoreGraphics allocates the pixels.
    let Some(ctx) = (unsafe { crate::images::bitmap_context(None, w, h, 0, &space, crate::images::PREMULTIPLIED_BGRA) }) else { return };
    // SAFETY: CoreGraphics and ImageIO calls with live objects created here.
    unsafe {
        // The window's background (Kilo's frame) isn't in the layer tree.
        let background = window.backgroundColor().CGColor();
        CGContext::set_fill_color_with_color(Some(&ctx), Some(&background));
        CGContext::fill_rect(Some(&ctx), CGRect::new(CGPoint::ZERO, CGSize::new(w as f64, h as f64)));
        CGContext::scale_ctm(Some(&ctx), scale, scale);
        layer.renderInContext(&ctx);
        let Some(image) = CGBitmapContextCreateImage(Some(&ctx)) else { return };
        let Some(url) = CFURL::from_file_path(path) else { return };
        let Some(dest) = CGImageDestination::with_url(&url, &CFString::from_static_str("public.png"), 1, None) else { return };
        dest.add_image(&image, None);
        if dest.finalize() {
            eprintln!("kilo: snapshot written to {path}");
        }
    }
}

fn launched() -> &'static std::time::Instant {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    START.get_or_init(std::time::Instant::now)
}

/// Marks process start (call first thing in `main`).
pub fn mark_launch() {
    launched();
}

/// During a `KILO_SCENARIO`, logs what happens (player events, …).
pub fn trace(what: impl FnOnce() -> String) {
    if std::env::var_os("KILO_SCENARIO").is_some() {
        log(&what());
    }
}

/// Like `trace`, but only the first time for each `what` (startup steps).
pub fn trace_once(what: &'static str) {
    thread_local! {
        static SEEN: std::cell::RefCell<Vec<&'static str>> = const { std::cell::RefCell::new(Vec::new()) };
    }
    if std::env::var_os("KILO_SCENARIO").is_some() && SEEN.with_borrow_mut(|seen| !seen.contains(&what) && (seen.push(what), true).1) {
        log(what);
    }
}

fn log(step: &str) {
    eprintln!("[{:8.3}s] {step}", launched().elapsed().as_secs_f64());
}

/// Runs `KILO_SCENARIO`, if set, once the app is ready: signed in, or on
/// the sign-in screen. Only once per launch.
pub fn run_scenario() {
    static STARTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    let Ok(script) = std::env::var("KILO_SCENARIO") else { return };
    if STARTED.swap(true, std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    let steps: Vec<String> = script.split(',').map(str::to_owned).collect();
    step(steps, 0);
}

/// Queues a key press as if typed (AppKit handles it as it would a real
/// one): `SPEC` is modifiers and a key joined by `+` (`cmd+right`, `space`,
/// `m`).
fn press(spec: &str) {
    use objc2_app_kit::{NSApplication, NSEvent, NSEventModifierFlags, NSEventType};
    use objc2_foundation::{NSPoint, NSString};
    let mtm = objc2::MainThreadMarker::new().expect("main thread");
    let app = NSApplication::sharedApplication(mtm);
    let Some(window) = app.keyWindow().or_else(|| app.mainWindow()).or_else(|| app.windows().firstObject()) else { return };
    let mut flags = NSEventModifierFlags::empty();
    let mut key = "";
    for part in spec.split('+') {
        match part {
            "cmd" => flags |= NSEventModifierFlags::Command,
            "shift" => flags |= NSEventModifierFlags::Shift,
            "alt" => flags |= NSEventModifierFlags::Option,
            "ctrl" => flags |= NSEventModifierFlags::Control,
            other => key = other,
        }
    }
    let (chars, code): (String, u16) = match key {
        "space" => (" ".into(), 49),
        "left" => ("\u{F702}".into(), 123),
        "right" => ("\u{F703}".into(), 124),
        "down" => ("\u{F701}".into(), 125),
        "up" => ("\u{F700}".into(), 126),
        "esc" => ("\u{1b}".into(), 53),
        "m" => ("m".into(), 46),
        "f" => ("f".into(), 3),
        "l" => ("l".into(), 37),
        "k" => ("k".into(), 40),
        "1" => ("1".into(), 18),
        "2" => ("2".into(), 19),
        "[" => ("[".into(), 33),
        "/" => ("/".into(), 44),
        "end" => ("\u{F72B}".into(), 119),
        "pagedown" => ("\u{F72D}".into(), 121),
        "a" => ("a".into(), 0),
        "s" => ("s".into(), 1),
        _ => return,
    };
    let chars = NSString::from_str(&chars);
    for kind in [NSEventType::KeyDown, NSEventType::KeyUp] {
        let event = NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
            kind,
            NSPoint::ZERO,
            flags,
            0.0,
            window.windowNumber(),
            None,
            &chars,
            &chars,
            false,
            code,
        );
        if let Some(event) = event {
            app.postEvent_atStart(&event, false);
        }
    }
}

/// A scroll event over the page's first shelf, as a trackpad (or a mouse
/// wheel, for phase `none`) would send it.
fn wheel(spec: &str) {
    use objc2_core_graphics::{CGEvent, CGEventField, CGScrollEventUnit};
    let mut parts = spec.split('/');
    let mut number = || parts.next().and_then(|n| n.parse::<i32>().ok()).unwrap_or(0);
    let (dx, dy) = (number(), number());
    let phase = match spec.rsplit('/').next() {
        Some("began") => 1,
        Some("changed") => 2,
        Some("ended") => 4,
        Some("maybegin") => 128,
        _ => 0,
    };
    let Some(cg) = CGEvent::new_scroll_wheel_event2(None, CGScrollEventUnit::Pixel, 2, dy, dx, 0) else { return };
    CGEvent::set_integer_value_field(Some(&cg), CGEventField::ScrollWheelEventIsContinuous, i64::from(phase != 0));
    CGEvent::set_integer_value_field(Some(&cg), CGEventField::ScrollWheelEventScrollPhase, phase);
    if let Some(event) = objc2_app_kit::NSEvent::eventWithCGEvent(&cg) {
        log(&crate::app::scroll_shelf(&event));
    }
}

fn step(steps: Vec<String>, i: usize) {
    let Some(cmd) = steps.get(i).cloned() else {
        log("scenario done");
        return;
    };
    log(&cmd);
    let (name, arg) = cmd.split_once(':').unwrap_or((&cmd, ""));
    let delay = match name {
        "play" => {
            crate::app::play_video(arg.to_owned());
            0.0
        }
        "pause" => {
            crate::app::play_pause();
            0.0
        }
        "close" => {
            crate::app::close_window();
            0.0
        }
        "open" => {
            crate::app::reopen_window();
            0.0
        }
        "browse" => {
            crate::app::go(crate::app::Route::Browse { id: arg.into(), params: None });
            0.0
        }
        "scroll" => {
            crate::app::scroll_to(arg.parse().ok());
            0.0
        }
        "playall" | "shuffleall" => {
            crate::app::play_all(name == "shuffleall");
            0.0
        }
        "next" => {
            crate::app::next();
            0.0
        }
        "volume" => {
            crate::app::set_volume(arg.parse().unwrap_or(100));
            0.0
        }
        "snap" => {
            crate::app::snapshot(arg == "now");
            0.0
        }
        "playcard" => {
            crate::app::play_item(arg.parse().unwrap_or(0));
            0.0
        }
        "theme" | "lang" => {
            crate::app::choose(name, arg);
            0.0
        }
        "playvideo" => {
            crate::app::play_music_video(arg);
            0.0
        }
        "state" => {
            log(&crate::app::describe());
            0.0
        }
        "focus" => {
            crate::app::focus_search();
            0.0
        }
        "key" => {
            press(arg);
            0.0
        }
        "keys" => {
            log(&crate::app::describe_keys());
            0.0
        }
        "wheel" => {
            wheel(arg);
            0.0
        }
        "signin" => {
            crate::app::sign_in();
            0.0
        }
        "cancelsignin" => {
            crate::app::cancel_sign_in();
            0.0
        }
        "front" => {
            let front = objc2_app_kit::NSWorkspace::sharedWorkspace().frontmostApplication();
            let name = front.as_ref().and_then(|a| a.localizedName()).map(|n| n.to_string()).unwrap_or_default();
            let pid = front.map_or(0, |a| a.processIdentifier());
            log(&format!("front: {name} (pid {pid}; Kilo is {})", std::process::id()));
            0.0
        }
        "app" => {
            let me = objc2_app_kit::NSRunningApplication::currentApplication();
            log(&format!("app: active {}, owns the menu bar {}, policy {:?}", me.isActive(), me.ownsMenuBar(), me.activationPolicy()));
            0.0
        }
        "relayout" => {
            log(&crate::app::bench_relayout(arg.parse().unwrap_or(20)));
            0.0
        }
        "wait" => arg.parse::<f64>().unwrap_or(0.0),
        _ => 0.0,
    };
    let when = dispatch2::DispatchTime::NOW.time((delay * 1e9) as i64);
    let _ = dispatch2::DispatchQueue::main().after(when, move || step(steps, i + 1));
}
