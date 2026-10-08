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
//!   (`volume:0` first to test playback silently), `state` (logs the page
//!   and queue), `wait:SECS`.

use std::cell::Cell;

use objc2::Message;
use objc2_app_kit::NSWindow;
use objc2_core_foundation::{CFString, CFURL};
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
    let Some(base) = std::env::var_os("KILO_SNAPSHOT") else { return };
    let n = SHOTS.with(|s| {
        s.set(s.get() + 1);
        s.get()
    });
    let path = format!("{}-{n}.png", base.to_string_lossy());
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

fn log(step: &str) {
    eprintln!("[{:7.1}s] {step}", launched().elapsed().as_secs_f64());
}

/// Runs `KILO_SCENARIO`, if set, once the app is signed in and ready.
pub fn run_scenario() {
    let Ok(script) = std::env::var("KILO_SCENARIO") else { return };
    let steps: Vec<String> = script.split(',').map(str::to_owned).collect();
    step(steps, 0);
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
        "playvideo" => {
            crate::app::play_music_video(arg);
            0.0
        }
        "state" => {
            log(&crate::app::describe());
            0.0
        }
        "wait" => arg.parse::<f64>().unwrap_or(0.0),
        _ => 0.0,
    };
    let when = dispatch2::DispatchTime::NOW.time((delay * 1e9) as i64);
    let _ = dispatch2::DispatchQueue::main().after(when, move || step(steps, i + 1));
}
