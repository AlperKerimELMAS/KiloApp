//! Developer switches (environment variables), all off by default:
//! - `KILO_SNAPSHOT=/path/shot`: after each page appears, render the window
//!   to `/path/shot-N.png` (no Screen Recording permission needed).
//! - `KILO_OPEN=search:QUERY` or `browse:ID`: open that page after Home.
//! - `KILO_NO_ACTIVATE=1`: don't bring the app to the front on launch.

use std::cell::Cell;
use std::ffi::c_void;
use std::ptr::NonNull;

use objc2::Message;
use objc2_app_kit::NSWindow;
use objc2_core_foundation::{CFRetained, CFString, CFURL};
use objc2_core_graphics::{CGBitmapContextCreateImage, CGColorSpace, CGContext};
use objc2_image_io::CGImageDestination;

unsafe extern "C-unwind" {
    fn CGBitmapContextCreate(
        data: *mut c_void,
        width: usize,
        height: usize,
        bits_per_component: usize,
        bytes_per_row: usize,
        space: Option<&CGColorSpace>,
        bitmap_info: u32,
    ) -> Option<NonNull<CGContext>>;
}

thread_local! {
    static SHOTS: Cell<u32> = const { Cell::new(0) };
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
    let window = window.retain();
    let when = dispatch2::DispatchTime::NOW.time(4_000_000_000);
    let window = std::sync::Mutex::new(Some(window));
    // The window is only touched back on the main thread.
    struct MainOnly<T>(T);
    // SAFETY: the value is created and used on the main thread only; it's
    // merely carried through GCD's main queue.
    unsafe impl<T> Send for MainOnly<T> {}
    let carried = MainOnly(window);
    let _ = dispatch2::DispatchQueue::main().after(when, move || {
        let carried = carried;
        if let Some(w) = carried.0.lock().ok().and_then(|mut w| w.take()) {
            render(&w, &path);
        }
    });
}

fn render(window: &NSWindow, path: &str) {
    let Some(view) = window.contentView() else { return };
    let Some(layer) = view.layer() else { return };
    let scale = window.backingScaleFactor();
    let size = view.bounds().size;
    let (w, h) = ((size.width * scale) as usize, (size.height * scale) as usize);
    // SAFETY: CoreGraphics calls with a freshly created color space and a
    // context that CoreGraphics allocates itself (data = null).
    unsafe {
        let Some(space) = CGColorSpace::new_device_rgb() else { return };
        // kCGImageAlphaPremultipliedFirst | kCGBitmapByteOrder32Little
        let Some(ctx) = CGBitmapContextCreate(std::ptr::null_mut(), w, h, 8, 0, Some(&space), 2 | (2 << 12)) else { return };
        let ctx: CFRetained<CGContext> = CFRetained::from_raw(ctx);
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
