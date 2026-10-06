//! `--login-helper`: a window with Google's own sign-in page in a system web
//! view. Kilo never sees the password. Once YouTube Music shows the user as
//! signed in, the helper prints the YouTube cookies (one per line) for the
//! app and exits; WebKit keeps them in its store for the player helper too.

use std::cell::Cell;
use std::io::Write;
use std::ptr::NonNull;
use std::rc::Rc;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSWindow, NSWindowDelegate, NSWindowStyleMask,
};
use objc2_foundation::{NSArray, NSHTTPCookie, NSNotification, NSPoint, NSRect, NSSize, NSString, NSTimer, NSURL, NSURLRequest};
use objc2_web_kit::{WKWebView, WKWebViewConfiguration};

const SIGN_IN_URL: &str = "https://accounts.google.com/ServiceLogin?ltmpl=music&service=youtube&passive=true&continue=https%3A%2F%2Fwww.youtube.com%2Fsignin%3Faction_handle_signin%3Dtrue%26next%3Dhttps%253A%252F%252Fmusic.youtube.com%252F";

/// WKWebView is Safari's engine: identify as Safari, exactly like the app's
/// own requests (`kilo_core::http::USER_AGENT`).
const SAFARI_APP_NAME: &str = "Version/27.0.1 Safari/605.1.15";

define_class!(
    // SAFETY: NSObject has no subclassing requirements; no ivars, no Drop.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "KiloLoginWindowDelegate"]
    struct WindowDelegate;

    unsafe impl NSObjectProtocol for WindowDelegate {}

    unsafe impl NSWindowDelegate for WindowDelegate {
        #[unsafe(method(windowWillClose:))]
        fn window_will_close(&self, _notification: &NSNotification) {
            // Closed without signing in.
            std::process::exit(1);
        }
    }
);

pub fn run() -> ! {
    let mtm = MainThreadMarker::new().expect("main thread");
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Regular);

    // SAFETY: main-thread AppKit/WebKit calls with valid arguments.
    let (window, web_view, delegate) = unsafe {
        let config = WKWebViewConfiguration::new(mtm);
        config.setApplicationNameForUserAgent(Some(&NSString::from_str(SAFARI_APP_NAME)));
        let size = NSSize::new(480.0, 720.0);
        let web_view = WKWebView::initWithFrame_configuration(WKWebView::alloc(mtm), NSRect::new(NSPoint::ZERO, size), &config);
        let window = NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            NSRect::new(NSPoint::ZERO, size),
            NSWindowStyleMask::Titled | NSWindowStyleMask::Closable | NSWindowStyleMask::Resizable,
            NSBackingStoreType::Buffered,
            false,
        );
        window.setReleasedWhenClosed(false);
        window.setTitle(&NSString::from_str("Sign in to YouTube Music"));
        window.setContentView(Some(&web_view));
        let delegate: Retained<WindowDelegate> = msg_send![WindowDelegate::alloc(mtm), init];
        window.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        window.center();
        window.makeKeyAndOrderFront(None);
        app.activate();
        let url = NSURL::URLWithString(&NSString::from_str(SIGN_IN_URL)).expect("valid url");
        web_view.loadRequest(&NSURLRequest::requestWithURL(&url));
        (window, web_view, delegate)
    };

    // Shared between the timer and the asynchronous cookie check.
    let signed_in_at: Rc<Cell<Option<u32>>> = Rc::new(Cell::new(None));
    let ticks = Cell::new(0u32);
    let wv = web_view.clone();
    let tick = RcBlock::new(move |_t: NonNull<NSTimer>| {
        ticks.set(ticks.get() + 1);
        if let Some(at) = signed_in_at.get() {
            // Let the redirect chain finish setting cookies, then report.
            if ticks.get() >= at + 2 {
                report_cookies_and_exit(&wv);
            }
            return;
        }
        // SAFETY: main-thread WebKit call.
        let on_ytm = unsafe { wv.URL() }.and_then(|u| u.host()).is_some_and(|h| h.to_string() == "music.youtube.com");
        if on_ytm {
            let now = ticks.get();
            let flag = signed_in_at.clone();
            check_signed_in(&wv, move |yes| {
                if yes && flag.get().is_none() {
                    flag.set(Some(now));
                }
            });
        }
    });
    // SAFETY: the timer retains the block; it runs on the main run loop.
    unsafe { NSTimer::scheduledTimerWithTimeInterval_repeats_block(1.0, true, &tick) };
    let _keep = (window, delegate);
    app.run();
    std::process::exit(1)
}

fn check_signed_in(web_view: &WKWebView, then: impl Fn(bool) + 'static) {
    let done = RcBlock::new(move |cookies: NonNull<NSArray<NSHTTPCookie>>| {
        // SAFETY: WebKit passes a valid array for the duration of the call.
        let cookies = unsafe { cookies.as_ref() };
        then(cookies.iter().any(|c| c.name().to_string() == "__Secure-3PAPISID" && c.domain().to_string().ends_with("youtube.com")));
    });
    // SAFETY: main-thread WebKit calls.
    unsafe { web_view.configuration().websiteDataStore().httpCookieStore().getAllCookies(&done) };
}

fn report_cookies_and_exit(web_view: &WKWebView) {
    let done = RcBlock::new(|cookies: NonNull<NSArray<NSHTTPCookie>>| {
        // SAFETY: as above.
        let cookies = unsafe { cookies.as_ref() };
        let mut out = std::io::stdout().lock();
        for c in cookies.iter() {
            let domain = c.domain().to_string();
            if !domain.ends_with("youtube.com") {
                continue;
            }
            let expires = c.expiresDate().map_or(0.0, |d| d.timeIntervalSince1970());
            let _ = writeln!(out, "cookie\t{domain}\t{}\t{}\t{expires}", c.name(), c.value());
        }
        let _ = writeln!(out, "done");
        let _ = out.flush();
        std::process::exit(0);
    });
    // SAFETY: main-thread WebKit calls.
    unsafe { web_view.configuration().websiteDataStore().httpCookieStore().getAllCookies(&done) };
}
