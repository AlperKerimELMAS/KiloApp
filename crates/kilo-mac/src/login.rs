//! The account's web data, in helpers so the app itself never loads WebKit.
//!
//! `--login-helper`: a window with Google's own sign-in page in a system web
//! view. Kilo never sees the password. The window has no address bar, so it
//! only shows Google's and YouTube's pages, over https, and names the page's
//! host in its title bar; any other link opens in the browser. Once YouTube
//! Music shows the user as signed in, the helper prints the YouTube cookies
//! (one per line) for the app and exits; WebKit keeps them in its store for
//! the player helper too. The app reads that output with `parse_output`.
//!
//! `--sign-out-helper`: deletes everything WebKit stores for Kilo.

use std::cell::Cell;
use std::io::Write;
use std::ptr::NonNull;
use std::rc::Rc;

use block2::RcBlock;
use kilo_core::auth::{Cookie, is_sign_in_host, is_youtube_domain};
use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSWindow, NSWindowDelegate, NSWindowStyleMask, NSWorkspace,
};
use objc2_foundation::{NSArray, NSDate, NSHTTPCookie, NSNotification, NSPoint, NSRect, NSSize, NSString, NSTimer, NSURL, NSURLRequest};
use objc2_web_kit::{
    WKNavigation, WKNavigationAction, WKNavigationActionPolicy, WKNavigationDelegate, WKWebView, WKWebViewConfiguration, WKWebsiteDataStore,
};

const SIGN_IN_URL: &str = "https://accounts.google.com/ServiceLogin?ltmpl=music&service=youtube&passive=true&continue=https%3A%2F%2Fwww.youtube.com%2Fsignin%3Faction_handle_signin%3Dtrue%26next%3Dhttps%253A%252F%252Fmusic.youtube.com%252F";

/// WKWebView is Safari's engine: identify as Safari, exactly like the app's
/// own requests (`kilo_core::http::USER_AGENT`).
const SAFARI_APP_NAME: &str = "Version/27.0.1 Safari/605.1.15";

define_class!(
    // SAFETY: NSObject has no subclassing requirements; no ivars, no Drop;
    // methods match the delegate protocols' signatures.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "KiloLoginDelegate"]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl NSWindowDelegate for Delegate {
        #[unsafe(method(windowWillClose:))]
        fn window_will_close(&self, _notification: &NSNotification) {
            // Closed without signing in.
            std::process::exit(1);
        }
    }

    unsafe impl WKNavigationDelegate for Delegate {
        #[unsafe(method(webView:decidePolicyForNavigationAction:decisionHandler:))]
        fn decide(
            &self,
            _web_view: &WKWebView,
            action: &WKNavigationAction,
            decision: &block2::DynBlock<dyn Fn(WKNavigationActionPolicy)>,
        ) {
            // SAFETY: WebKit passes live objects on the main thread.
            let (frame, url) = unsafe { (action.targetFrame(), action.request().URL()) };
            let (scheme, host) = url.as_deref().map(scheme_and_host).unwrap_or_default();
            let allowed = match frame {
                // The sign-in page's own frames (reCAPTCHA, say).
                // SAFETY: a live frame, on the main thread.
                Some(f) if !unsafe { f.isMainFrame() } => true,
                Some(_) => scheme == "https" && is_sign_in_host(&host),
                // A new window: there are none.
                None => false,
            };
            if !allowed
                && matches!(scheme.as_str(), "https" | "http")
                && let Some(url) = &url
            {
                // Somewhere else (a help link, say): the browser shows it,
                // with its address bar.
                NSWorkspace::sharedWorkspace().openURL(url);
            }
            crate::debug::trace(|| format!("sign-in: {} {scheme}://{host}", if allowed { "show" } else { "refuse" }));
            decision.call((if allowed { WKNavigationActionPolicy::Allow } else { WKNavigationActionPolicy::Cancel },));
        }

        #[unsafe(method(webView:didCommitNavigation:))]
        fn did_commit(&self, web_view: &WKWebView, _navigation: Option<&WKNavigation>) {
            // No address bar: the title bar says whose page this is.
            // SAFETY: WebKit passes a live web view on the main thread.
            let host = unsafe { web_view.URL() }.as_deref().map(scheme_and_host).unwrap_or_default().1;
            if let Some(window) = web_view.window() {
                window.setSubtitle(&NSString::from_str(&host));
            }
        }
    }
);

/// A URL's scheme and host, lowercased ("" where it has none).
fn scheme_and_host(url: &NSURL) -> (String, String) {
    let lower = |s: Option<Retained<NSString>>| s.map(|s| s.to_string().to_ascii_lowercase()).unwrap_or_default();
    (lower(url.scheme()), lower(url.host()))
}

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
        let delegate: Retained<Delegate> = msg_send![Delegate::alloc(mtm), init];
        window.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        // Held weakly by WebKit; `_keep` below holds it.
        web_view.setNavigationDelegate(Some(ProtocolObject::from_ref(&*delegate)));
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
    let reported = Cell::new(false);
    let wv = web_view.clone();
    let tick = RcBlock::new(move |_t: NonNull<NSTimer>| {
        ticks.set(ticks.get() + 1);
        if let Some(at) = signed_in_at.get() {
            // Let the redirect chain finish setting cookies, then report
            // (once: the helper exits when the cookies arrive).
            if ticks.get() >= at + 2 && !reported.replace(true) {
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
        then(cookies.iter().any(|c| c.name().to_string() == "__Secure-3PAPISID" && is_youtube_domain(&c.domain().to_string())));
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
            if !is_youtube_domain(&domain) {
                continue;
            }
            let cookie = Cookie {
                domain,
                name: c.name().to_string(),
                value: c.value().to_string(),
                expires: c.expiresDate().map_or(0.0, |d| d.timeIntervalSince1970()),
            };
            let _ = writeln!(out, "{}", format_cookie(&cookie));
        }
        let _ = writeln!(out, "done");
        let _ = out.flush();
        std::process::exit(0);
    });
    // SAFETY: main-thread WebKit calls.
    unsafe { web_view.configuration().websiteDataStore().httpCookieStore().getAllCookies(&done) };
}

/// `--sign-out-helper`: deletes everything WebKit stores for Kilo (cookies,
/// so the session, and every site's caches and storage), then exits: 0 once
/// it's done, 1 if WebKit didn't finish within 15 s.
pub fn sign_out() -> ! {
    let mtm = MainThreadMarker::new().expect("main thread");
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Prohibited);
    let done = RcBlock::new(|| std::process::exit(0));
    // SAFETY: main-thread WebKit calls; the block outlives the call.
    unsafe {
        let store = WKWebsiteDataStore::defaultDataStore(mtm);
        store.removeDataOfTypes_modifiedSince_completionHandler(
            &WKWebsiteDataStore::allWebsiteDataTypes(mtm),
            &NSDate::distantPast(),
            &done,
        );
    }
    let when = dispatch2::DispatchTime::NOW.time(15_000_000_000);
    let _ = dispatch2::DispatchQueue::main().after(when, || std::process::exit(1));
    app.run();
    std::process::exit(1)
}

/// One line of the helper's output: `cookie⇥domain⇥name⇥value⇥expires`.
fn format_cookie(c: &Cookie) -> String {
    format!("cookie\t{}\t{}\t{}\t{}", c.domain, c.name, c.value, c.expires)
}

/// The cookies in the helper's output (other lines are skipped).
pub fn parse_output(stdout: &str) -> Vec<Cookie> {
    stdout
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cookies_round_trip_through_the_helper_output() {
        let c = Cookie { domain: ".youtube.com".into(), name: "SAPISID".into(), value: "a/b=c".into(), expires: 1_790_000_000.5 };
        let out = format!("{}\nsomething else\ndone\n", format_cookie(&c));
        assert_eq!(parse_output(&out), vec![c]);
    }
}
