//! The account's web data, in helpers so the app itself never loads WebKit.
//!
//! `--login-helper`: a window with Google's own sign-in page in a system web
//! view. Kilo never sees the password. The window has no address bar, so it
//! only shows Google's and YouTube's pages, over https, and names the page's
//! host in its title bar; any other link opens in the browser. Once YouTube
//! Music shows the user as signed in, the helper copies YouTube's cookies,
//! and only those, into WebKit's store for the player helper, prints them
//! (one per line) for the app, and exits. The window keeps its cookies in
//! memory, so Google's account session ends with it and never touches the
//! disk. The app reads the output with `parse_output`.
//!
//! `--prune-helper`: deletes everything WebKit stores for Kilo except
//! YouTube's (older versions kept the whole Google session).
//!
//! `--sign-out-helper`: deletes everything WebKit stores for Kilo.

use std::cell::Cell;
use std::io::Write;
use std::ptr::NonNull;
use std::rc::Rc;

use crate::strings::{self, S, t};
use block2::RcBlock;
use dispatch2::{DispatchQueue, DispatchTime};
use kilo_core::auth::{Cookie, is_sign_in_host, is_youtube_domain, parse_binary_cookies};
use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSWindow, NSWindowDelegate, NSWindowStyleMask, NSWorkspace,
};
use objc2_foundation::{NSArray, NSDate, NSHTTPCookie, NSNotification, NSPoint, NSRect, NSSize, NSString, NSTimer, NSURL, NSURLRequest};
use objc2_web_kit::{
    WKNavigation, WKNavigationAction, WKNavigationActionPolicy, WKNavigationDelegate, WKWebView, WKWebViewConfiguration,
    WKWebsiteDataRecord, WKWebsiteDataStore,
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
    strings::init();
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Regular);

    // SAFETY: main-thread AppKit/WebKit calls with valid arguments.
    let (window, web_view, delegate) = unsafe {
        let config = WKWebViewConfiguration::new(mtm);
        // Signing in creates Google's whole account session: it stays in
        // memory, and only YouTube's cookies are kept (`keep_youtube_cookies`).
        config.setWebsiteDataStore(&WKWebsiteDataStore::nonPersistentDataStore(mtm));
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
        window.setTitle(&NSString::from_str(t(S::SignInWindow)));
        window.setContentView(Some(&web_view));
        let delegate: Retained<Delegate> = msg_send![Delegate::alloc(mtm), init];
        window.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        // Held weakly by WebKit; `_keep` below holds it.
        web_view.setNavigationDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        window.center();
        window.makeKeyAndOrderFront(None);
        // In front of Kilo, which started it. (`activate()` is only a
        // request since macOS 14, and was turned down: the window stayed
        // behind.)
        #[allow(deprecated)]
        app.activateIgnoringOtherApps(true);
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
                keep_youtube_cookies(&wv);
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

/// Signed in: copies YouTube's cookies (with all their attributes) from the
/// window's memory into WebKit's own store, where the player helper finds
/// them, then reports them and exits.
fn keep_youtube_cookies(web_view: &WKWebView) {
    let mtm = MainThreadMarker::new().expect("main thread");
    let done = RcBlock::new(move |cookies: NonNull<NSArray<NSHTTPCookie>>| {
        // SAFETY: as above.
        let cookies = unsafe { cookies.as_ref() };
        let youtube: Vec<Retained<NSHTTPCookie>> = cookies.iter().filter(|c| is_youtube_domain(&c.domain().to_string())).collect();
        let report: Rc<Vec<Cookie>> = Rc::new(
            youtube
                .iter()
                .map(|c| Cookie {
                    domain: c.domain().to_string(),
                    name: c.name().to_string(),
                    value: c.value().to_string(),
                    expires: c.expiresDate().map_or(0.0, |d| d.timeIntervalSince1970()),
                })
                .collect(),
        );
        // SAFETY: main-thread WebKit call.
        let store = unsafe { WKWebsiteDataStore::defaultDataStore(mtm).httpCookieStore() };
        let left = Rc::new(Cell::new(youtube.len()));
        for cookie in &youtube {
            let (left, report) = (left.clone(), report.clone());
            let saved = RcBlock::new(move || {
                left.set(left.get() - 1);
                if left.get() == 0 {
                    report_and_exit(&report);
                }
            });
            // SAFETY: main-thread WebKit call with a live cookie.
            unsafe { store.setCookie_completionHandler(cookie, Some(&saved)) };
        }
    });
    // SAFETY: main-thread WebKit calls.
    unsafe { web_view.configuration().websiteDataStore().httpCookieStore().getAllCookies(&done) };
}

/// Prints `cookies` for the app and exits. WebKit writes them to its cookie
/// file as this process exits (and not before: measured); the app waits for
/// that with `wait_until_saved`.
fn report_and_exit(cookies: &[Cookie]) -> ! {
    let mut out = std::io::stdout().lock();
    for cookie in cookies {
        let _ = writeln!(out, "{}", format_cookie(cookie));
    }
    let _ = writeln!(out, "done");
    let _ = out.flush();
    std::process::exit(0)
}

/// Waits, up to 3 s, until WebKit's cookie file holds `cookies` (all but
/// session cookies, which it never saves), so the player helper starts
/// signed in. Called by the app once the login helper has exited.
pub fn wait_until_saved(cookies: &[Cookie]) {
    for _ in 0..30 {
        let saved = std::fs::read(crate::paths::cookies()).ok().and_then(|data| parse_binary_cookies(&data));
        if saved.is_some_and(|saved| {
            cookies
                .iter()
                .filter(|c| c.expires > 0.0)
                .all(|c| saved.iter().any(|s| (&s.domain, &s.name, &s.value) == (&c.domain, &c.name, &c.value)))
        }) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

/// `--prune-helper`: deletes everything WebKit stores for Kilo except
/// YouTube's data, then exits: 0 once it's done, 1 if WebKit didn't finish
/// in time.
pub fn prune() -> ! {
    let mtm = MainThreadMarker::new().expect("main thread");
    let app = windowless(mtm);
    // SAFETY: main-thread WebKit calls; WebKit retains the blocks.
    unsafe {
        let store = WKWebsiteDataStore::defaultDataStore(mtm);
        let types = WKWebsiteDataStore::allWebsiteDataTypes(mtm);
        let (store2, types2) = (store.clone(), types.clone());
        let fetched = RcBlock::new(move |records: NonNull<NSArray<WKWebsiteDataRecord>>| {
            // A record is a site (registrable domain): youtube.com's covers
            // music.youtube.com too.
            let others: Vec<Retained<WKWebsiteDataRecord>> =
                records.as_ref().iter().filter(|r| r.displayName().to_string() != "youtube.com").collect();
            let removed = RcBlock::new(|| std::process::exit(0));
            store2.removeDataOfTypes_forDataRecords_completionHandler(&types2, &NSArray::from_retained_slice(&others), &removed);
        });
        store.fetchDataRecordsOfTypes_completionHandler(&types, &fetched);
    }
    run_or_give_up(&app)
}

/// `--sign-out-helper`: deletes everything WebKit stores for Kilo (cookies,
/// so the session, and every site's caches and storage), then exits: 0 once
/// it's done, 1 if WebKit didn't finish in time.
pub fn sign_out() -> ! {
    let mtm = MainThreadMarker::new().expect("main thread");
    let app = windowless(mtm);
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
    run_or_give_up(&app)
}

/// An app with no Dock icon and no windows, for the data helpers.
fn windowless(mtm: MainThreadMarker) -> Retained<NSApplication> {
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Prohibited);
    app
}

/// Runs until the helper's work exits the process, or exits 1 after 15 s.
fn run_or_give_up(app: &NSApplication) -> ! {
    let when = DispatchTime::NOW.time(15_000_000_000);
    let _ = DispatchQueue::main().after(when, || std::process::exit(1));
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
