//! Spike: what does YouTube Music's own player cost when it runs in a hidden
//! system web view?
//!
//! ```text
//! kilo-spike-web login                         sign in once (visible window)
//! kilo-spike-web play <videoId> [--seconds N] [--lean] [--window]
//! ```
//!
//! `play` measures its own process plus WebKit's helper processes with
//! kilo-probe: while playing, while paused, and after the web view is torn
//! down. Playback only starts on a signed-in account (Premium-only rule).
//!
//! A lab, not the app. Like the app, `login` keeps only YouTube's cookies,
//! in this binary's own WebKit store (on disk, keyed by the bundle it runs
//! in; Google's account session stays in the window's memory). Unlike the
//! app, its sign-in window isn't limited to Google's pages, and its browser
//! identity is a fixed Safari version. Delete its WebKit data when you're
//! done measuring.

use std::cell::{Cell, RefCell};
use std::ptr::NonNull;
use std::rc::Rc;
use std::time::{Duration, Instant};

use block2::RcBlock;
use kilo_probe::Sample;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSWindow, NSWindowStyleMask};
use objc2_foundation::{NSArray, NSError, NSHTTPCookie, NSPoint, NSRect, NSSize, NSString, NSTimer, NSURL, NSURLRequest, ns_string};
use objc2_web_kit::{
    WKAudiovisualMediaTypes, WKContentRuleList, WKContentRuleListStore, WKInactiveSchedulingPolicy, WKScriptMessage,
    WKScriptMessageHandler, WKUserContentController, WKUserScript, WKUserScriptInjectionTime, WKWebView, WKWebViewConfiguration,
    WKWebsiteDataStore,
};

const SIGN_IN_URL: &str = "https://accounts.google.com/ServiceLogin?ltmpl=music&service=youtube&passive=true&continue=https%3A%2F%2Fwww.youtube.com%2Fsignin%3Faction_handle_signin%3Dtrue%26next%3Dhttps%253A%252F%252Fmusic.youtube.com%252F";

/// WKWebView is Safari's engine; identify as Safari so Google treats the
/// sign-in page as the browser it is.
const SAFARI_APP_NAME: &str = "Version/27.0.1 Safari/605.1.15";

/// Mobile Safari, for the lighter mobile web apps (`--site ytm-mobile|mweb`).
const IPHONE_UA: &str = "Mozilla/5.0 (iPhone; CPU iPhone OS 18_6 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.0 Mobile/15E148 Safari/604.1";

/// Content blocking for `--lean`: we draw our own UI, so the hidden page never
/// needs images or fonts. Nothing that playback, attestation or play
/// reporting depends on is blocked.
const LEAN_RULES: &str = r#"[{"trigger":{"url-filter":".*","resource-type":["image","font"]},"action":{"type":"block"}}]"#;

const LEAN_CSS_SCRIPT: &str = r#"(() => {
  const s = document.createElement('style');
  s.textContent = '*,*::before,*::after{animation:none!important;transition:none!important}';
  (document.head || document.documentElement).appendChild(s);
})();"#;

/// `--profile`: counts what the page does per second (timers and their run
/// time, network requests by kind, media appended) so we can see where the
/// WebContent CPU goes. Spike-only; it wraps page APIs.
const PROFILE_SCRIPT: &str = r#"(() => {
  const c = { timeout: 0, interval: 0, raf: 0, message: 0, js_ms: 0, append: 0, append_kb: 0, net: {} };
  const timed = (fn, key) => function (...a) {
    const t = performance.now();
    try { return fn.apply(this, a); } finally { c.js_ms += performance.now() - t; c[key]++; }
  };
  const st = setTimeout, si = setInterval, raf = requestAnimationFrame;
  window.setTimeout = function (f, ...r) { return st.call(this, typeof f === 'function' ? timed(f, 'timeout') : f, ...r); };
  window.setInterval = function (f, ...r) { return si.call(this, typeof f === 'function' ? timed(f, 'interval') : f, ...r); };
  window.requestAnimationFrame = function (f) { return raf.call(this, timed(f, 'raf')); };
  addEventListener('message', () => c.message++, true);
  const ab = SourceBuffer.prototype.appendBuffer;
  SourceBuffer.prototype.appendBuffer = function (d) { c.append++; c.append_kb += (d.byteLength || 0) / 1024; return ab.call(this, d); };
  new PerformanceObserver((l) => {
    for (const e of l.getEntries()) {
      const u = new URL(e.name);
      const k = u.hostname.endsWith('googlevideo.com') ? 'media' : u.hostname + u.pathname.split('/').slice(0, 3).join('/');
      c.net[k] = (c.net[k] || 0) + 1;
    }
  }).observe({ type: 'resource', buffered: false });
  let last = performance.now();
  window.__kiloProf = () => {
    const now = performance.now(), secs = (now - last) / 1000; last = now;
    const rate = (v) => Math.round(v / secs * 10) / 10;
    const out = { secs: Math.round(secs), timeout_s: rate(c.timeout), interval_s: rate(c.interval), raf_s: rate(c.raf),
      message_s: rate(c.message), js_ms_s: rate(c.js_ms), append_s: rate(c.append), append_kb_s: rate(c.append_kb), net: c.net };
    for (const k of ['timeout', 'interval', 'raf', 'message', 'js_ms', 'append', 'append_kb']) c[k] = 0;
    c.net = {};
    return JSON.stringify(out);
  };
  window.__kiloTree = () => {
    const walk = (el, d) => d > 3 ? [] : [...el.children].flatMap((ch) => {
      const n = ch.getElementsByTagName('*').length;
      return n < 15 ? [] : [`${'  '.repeat(d)}${ch.tagName.toLowerCase()}${ch.id ? '#' + ch.id : ''} (${n})`, ...walk(ch, d + 1)];
    });
    return walk(document.body, 0).join('\n');
  };
})();"#;

/// `--strip ui`: the page's own interface is never shown, so take it out of
/// style and layout entirely. Cosmetic only: no script is touched.
const STRIP_UI_CSS: &str = "#player-control-container,#header-bar,ytm-mobile-topbar-renderer,ytm-watch,ytm-pivot-bar-renderer,ytm-app>*:not(#player){display:none!important}";
/// `--strip all`: additionally hide the player surface itself.
const STRIP_ALL_CSS: &str = "#player{display:none!important}";

/// Page-world bridge: reports media events and exposes a tiny control API.
const BRIDGE_SCRIPT: &str = r#"(() => {
  const post = (o) => { try { webkit.messageHandlers.kilo.postMessage(JSON.stringify(o)); } catch (e) {} };
  const player = () => document.querySelector('#movie_player');
  const video = () => document.querySelector('video');
  const obs = new MutationObserver(() => {
    const v = video();
    if (!v) return;
    obs.disconnect();
    for (const ev of ['playing', 'pause', 'ended', 'waiting', 'error'])
      v.addEventListener(ev, () => post({ ev, t: Math.round(v.currentTime) }));
  });
  obs.observe(document, { childList: true, subtree: true });
  window.__kilo = {
    status() {
      const v = video(), p = player();
      const md = navigator.mediaSession && navigator.mediaSession.metadata;
      return JSON.stringify({
        has_video: !!v,
        paused: v ? v.paused : null,
        muted: v ? v.muted : null,
        t: v ? Math.round(v.currentTime) : null,
        w: v ? v.videoWidth : null,
        rs: v ? v.readyState : null,
        buf: v && v.buffered.length ? Math.round(v.buffered.end(v.buffered.length - 1)) : 0,
        ps: p && p.getPlayerState ? p.getPlayerState() : null,
        vis: document.visibilityState,
        ad: p ? p.classList.contains('ad-showing') : null,
        logged_in: (window.ytcfg && ytcfg.get) ? ytcfg.get('LOGGED_IN') : null,
        nodes: document.getElementsByTagName('*').length,
        title: md ? md.title + ' - ' + md.artist : null,
      });
    },
    play() { const p = player(); if (p && p.playVideo) p.playVideo(); else if (video()) video().play(); },
    // What tapping the speaker icon and picking the lowest quality would do.
    audible() {
      const p = player(), v = video();
      if (p && p.unMute) { p.unMute(); p.setVolume(100); }
      if (v) v.muted = false;
      if (p && p.setPlaybackQualityRange) p.setPlaybackQualityRange('tiny', 'tiny');
    },
    pause() { const p = player(); if (p && p.pauseVideo) p.pauseVideo(); else if (video()) video().pause(); },
  };
})();"#;

struct Args {
    login: bool,
    video_id: String,
    seconds: u64,
    lean: bool,
    lockdown: bool,
    profile: bool,
    /// 0 = keep, 1 = hide the page UI, 2 = also hide the player surface.
    strip: u8,
    size: (f64, f64),
    sched: WKInactiveSchedulingPolicy,
    site: Site,
    host: Host,
}

/// Which YouTube page hosts the player.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Site {
    /// music.youtube.com, desktop.
    Ytm,
    /// music.youtube.com as served to phones.
    YtmMobile,
    /// m.youtube.com, YouTube's mobile web app.
    Mweb,
}

impl Site {
    fn url(self, id: &str) -> String {
        match self {
            Site::Ytm | Site::YtmMobile => format!("https://music.youtube.com/watch?v={id}"),
            Site::Mweb => format!("https://m.youtube.com/watch?v={id}"),
        }
    }
}

/// Where the web view lives.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Host {
    /// No window at all.
    None,
    /// A 1x1 borderless window far off screen.
    Offscreen,
    /// A small visible window, like Kaset's 160x90 mini player.
    Visible,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Phase {
    SigningIn,
    Loading,
    Playing,
    Paused,
    TornDown,
}

struct State {
    args: Args,
    phase: Phase,
    phase_at: Instant,
    web_view: Option<Retained<WKWebView>>,
    window: Option<Retained<NSWindow>>,
    /// Steady-state playing measurement: (start time, start sample).
    playing_from: Option<(Instant, Sample)>,
    /// Per-process CPU time when playing began.
    playing_cpu: Vec<(u32, u64)>,
    playing_footprints: Vec<u64>,
    report: Vec<String>,
    signed_in_at: Option<Instant>,
    /// Signed in, and copying YouTube's cookies to the store.
    keeping: bool,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> R {
    STATE.with_borrow_mut(|s| f(s.as_mut().expect("state initialized in main")))
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements; Bridge has no ivars
    // and doesn't implement Drop.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "KiloSpikeBridge"]
    struct Bridge;

    unsafe impl NSObjectProtocol for Bridge {}

    unsafe impl WKScriptMessageHandler for Bridge {
        #[unsafe(method(userContentController:didReceiveScriptMessage:))]
        fn did_receive(&self, _controller: &WKUserContentController, message: &WKScriptMessage) {
            // SAFETY: called by WebKit on the main thread with a live message.
            let body = unsafe { message.body() };
            if let Ok(text) = body.downcast::<NSString>() {
                log(&format!("page event {text}"));
            }
        }
    }
);

impl Bridge {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        // SAFETY: plain NSObject init.
        unsafe { msg_send![Self::alloc(mtm), init] }
    }
}

fn parse_args() -> Result<Args, String> {
    let mut it = std::env::args().skip(1);
    let mode = it.next().ok_or("missing mode: login | play <videoId>")?;
    let mut args = Args {
        login: mode == "login",
        video_id: String::new(),
        seconds: 60,
        lean: false,
        lockdown: false,
        profile: false,
        strip: 0,
        size: (400.0, 300.0),
        sched: WKInactiveSchedulingPolicy::None,
        site: Site::Ytm,
        host: Host::None,
    };
    if !args.login {
        if mode != "play" {
            return Err(format!("unknown mode {mode:?}"));
        }
        args.video_id = it.next().ok_or("play needs a videoId")?;
        if args.video_id.len() != 11 || !args.video_id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
            return Err(format!("not a video id: {:?}", args.video_id));
        }
    }
    while let Some(a) = it.next() {
        match a.as_str() {
            "--seconds" => {
                args.seconds = it.next().and_then(|s| s.parse().ok()).filter(|&s| s > 0).ok_or("--seconds needs N")?;
            }
            "--lean" => args.lean = true,
            "--lockdown" => args.lockdown = true,
            "--profile" => args.profile = true,
            "--strip" => {
                args.strip = match it.next().as_deref() {
                    Some("ui") => 1,
                    Some("all") => 2,
                    _ => return Err("--strip needs ui | all".into()),
                };
            }
            "--sched" => {
                args.sched = match it.next().as_deref() {
                    Some("none") => WKInactiveSchedulingPolicy::None,
                    Some("throttle") => WKInactiveSchedulingPolicy::Throttle,
                    Some("suspend") => WKInactiveSchedulingPolicy::Suspend,
                    _ => return Err("--sched needs none | throttle | suspend".into()),
                };
            }
            "--size" => {
                let v = it.next().ok_or("--size needs WxH")?;
                let (w, h) = v.split_once('x').ok_or("--size needs WxH")?;
                args.size = (w.parse().map_err(|_| "bad width")?, h.parse().map_err(|_| "bad height")?);
            }
            "--site" => {
                args.site = match it.next().as_deref() {
                    Some("ytm") => Site::Ytm,
                    Some("ytm-mobile") => Site::YtmMobile,
                    Some("mweb") => Site::Mweb,
                    _ => return Err("--site needs ytm | ytm-mobile | mweb".into()),
                };
            }
            "--host" => {
                args.host = match it.next().as_deref() {
                    Some("none") => Host::None,
                    Some("offscreen") => Host::Offscreen,
                    Some("visible") => Host::Visible,
                    _ => return Err("--host needs none | offscreen | visible".into()),
                };
            }
            _ => return Err(format!("unexpected argument {a:?}")),
        }
    }
    Ok(args)
}

fn main() {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!(
                "error: {e}\nusage: kilo-spike-web login | play <videoId> [--seconds N] [--lean] [--lockdown] [--profile] [--strip ui|all] [--size WxH] [--sched none|throttle|suspend] [--site ytm|ytm-mobile|mweb] [--host none|offscreen|visible]"
            );
            std::process::exit(2);
        }
    };
    let mut report = vec![format!("process start            {}", fmt(&measure()))];

    let mtm = MainThreadMarker::new().expect("main thread");
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(if args.login { NSApplicationActivationPolicy::Regular } else { NSApplicationActivationPolicy::Accessory });
    report.push(format!("AppKit initialized       {}", fmt(&measure())));

    let login = args.login;
    let lean = args.lean;
    let lockdown = args.lockdown;
    let sched = args.sched;
    let profile = args.profile;
    let strip = args.strip;
    STATE.set(Some(State {
        args,
        phase: if login { Phase::SigningIn } else { Phase::Loading },
        phase_at: Instant::now(),
        web_view: None,
        window: None,
        playing_from: None,
        playing_cpu: Vec::new(),
        playing_footprints: Vec::new(),
        report,
        signed_in_at: None,
        keeping: false,
    }));

    let config = make_config(mtm, lean, lockdown, sched, profile, strip);
    if login {
        // Google's whole account session stays in the window's memory; only
        // YouTube's cookies are kept (`keep_youtube_cookies`), as in the app.
        // SAFETY: main-thread WebKit calls.
        unsafe { config.setWebsiteDataStore(&WKWebsiteDataStore::nonPersistentDataStore(mtm)) };
        start_web_view(mtm, &config, SIGN_IN_URL);
    } else {
        // Premium-only: refuse to load a watch page (which autoplays, with
        // ads on free accounts) unless this web view is signed in.
        let config2 = config.clone();
        check_signed_in(&config, move |signed_in| {
            if !signed_in {
                eprintln!("not signed in: run `kilo-spike-web login` first");
                std::process::exit(1);
            }
            let mtm = MainThreadMarker::new().expect("main thread");
            let url = with_state(|s| s.args.site.url(&s.args.video_id));
            if lean {
                let config2 = config2.clone();
                compile_lean_rules(mtm, move |rules| {
                    let mtm = MainThreadMarker::new().expect("main thread");
                    if let Some(rules) = rules {
                        // SAFETY: valid rule list on the main thread.
                        unsafe { config2.userContentController().addContentRuleList(&rules) };
                    }
                    start_web_view(mtm, &config2, &url);
                });
            } else {
                start_web_view(mtm, &config2, &url);
            }
        });
    }

    let tick = RcBlock::new(|_timer: NonNull<NSTimer>| tick());
    // SAFETY: the block is retained by the timer; runs on the main run loop.
    unsafe { NSTimer::scheduledTimerWithTimeInterval_repeats_block(1.0, true, &tick) };
    app.run();
}

fn make_config(
    mtm: MainThreadMarker,
    lean: bool,
    lockdown: bool,
    sched: WKInactiveSchedulingPolicy,
    profile: bool,
    strip: u8,
) -> Retained<WKWebViewConfiguration> {
    // SAFETY: all calls are on the main thread with valid arguments.
    unsafe {
        let config = WKWebViewConfiguration::new(mtm);
        config.setMediaTypesRequiringUserActionForPlayback(WKAudiovisualMediaTypes::None);
        config.setApplicationNameForUserAgent(Some(&NSString::from_str(SAFARI_APP_NAME)));
        let prefs = config.preferences();
        // How WebKit schedules the hidden page: none / throttle / suspend.
        prefs.setInactiveSchedulingPolicy(sched);
        // Only Google/YouTube pages ever load here; skip Safe Browsing's
        // per-load lookups and its database.
        prefs.setFraudulentWebsiteWarningEnabled(false);
        if lockdown {
            // Apple's Lockdown Mode: no JIT, no WebAssembly, fewer features.
            let page_prefs = config.defaultWebpagePreferences();
            page_prefs.setLockdownModeEnabled(true);
            config.setDefaultWebpagePreferences(Some(&page_prefs));
        }

        let controller = config.userContentController();
        let bridge = Bridge::new(mtm);
        controller.addScriptMessageHandler_name(ProtocolObject::from_ref(&*bridge), ns_string!("kilo"));
        let mut scripts = vec![BRIDGE_SCRIPT];
        if lean {
            scripts.push(LEAN_CSS_SCRIPT);
        }
        if profile {
            scripts.push(PROFILE_SCRIPT);
        }
        let strip_script = (strip > 0).then(|| {
            let css = if strip == 2 { format!("{STRIP_UI_CSS}{STRIP_ALL_CSS}") } else { STRIP_UI_CSS.to_owned() };
            format!(
                "(() => {{ const s = document.createElement('style'); s.textContent = {css:?}; (document.head || document.documentElement).appendChild(s); }})();"
            )
        });
        if let Some(src) = &strip_script {
            scripts.push(src);
        }
        for source in scripts {
            let script = WKUserScript::initWithSource_injectionTime_forMainFrameOnly(
                WKUserScript::alloc(mtm),
                &NSString::from_str(source),
                WKUserScriptInjectionTime::AtDocumentStart,
                true,
            );
            controller.addUserScript(&script);
        }
        config
    }
}

fn compile_lean_rules(mtm: MainThreadMarker, then: impl Fn(Option<Retained<WKContentRuleList>>) + 'static) {
    // SAFETY: main-thread WebKit calls; the completion pointer is retained
    // before use.
    unsafe {
        let Some(store) = WKContentRuleListStore::defaultStore(mtm) else {
            log("no content rule store; continuing without rules");
            return then(None);
        };
        let done = RcBlock::new(move |list: *mut WKContentRuleList, err: *mut NSError| {
            if let Some(err) = err.as_ref() {
                log(&format!("rule compile failed: {err:?}"));
            }
            then(Retained::retain(list));
        });
        store.compileContentRuleListForIdentifier_encodedContentRuleList_completionHandler(
            Some(ns_string!("kilo-lean")),
            Some(&NSString::from_str(LEAN_RULES)),
            Some(&done),
        );
    }
}

fn start_web_view(mtm: MainThreadMarker, config: &WKWebViewConfiguration, url: &str) {
    let (login, host, site, (w, h)) = with_state(|s| (s.args.login, s.args.host, s.args.site, s.args.size));
    // SAFETY: main-thread AppKit/WebKit calls with valid arguments.
    unsafe {
        let size = if login { NSSize::new(480.0, 720.0) } else { NSSize::new(w, h) };
        let web_view = WKWebView::initWithFrame_configuration(WKWebView::alloc(mtm), NSRect::new(NSPoint::ZERO, size), config);
        let window = (login || host != Host::None).then(|| {
            let (style, rect) = if login {
                (NSWindowStyleMask::Titled | NSWindowStyleMask::Closable | NSWindowStyleMask::Resizable, NSRect::new(NSPoint::ZERO, size))
            } else if host == Host::Visible {
                (NSWindowStyleMask::Titled, NSRect::new(NSPoint::new(40.0, 40.0), NSSize::new(160.0, 90.0)))
            } else {
                (NSWindowStyleMask::Borderless, NSRect::new(NSPoint::new(-10000.0, -10000.0), size))
            };
            let window = NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(mtm),
                rect,
                style,
                NSBackingStoreType::Buffered,
                false,
            );
            window.setReleasedWhenClosed(false);
            window.setContentView(Some(&web_view));
            if login {
                window.setTitle(ns_string!("Kilo: sign in to YouTube Music"));
                window.center();
                window.makeKeyAndOrderFront(None);
                NSApplication::sharedApplication(mtm).activate();
            } else if host == Host::Visible {
                window.setTitle(ns_string!("Kilo spike"));
                window.orderFrontRegardless();
            } else {
                window.orderBack(None);
            }
            window
        });
        if !login && site != Site::Ytm {
            web_view.setCustomUserAgent(Some(&NSString::from_str(IPHONE_UA)));
        }
        let request = NSURLRequest::requestWithURL(&NSURL::URLWithString(&NSString::from_str(url)).expect("valid url"));
        web_view.loadRequest(&request);
        with_state(|s| {
            s.web_view = Some(web_view);
            s.window = window;
            s.phase_at = Instant::now();
            if !login {
                s.report.push(format!("web view created         {}", fmt(&measure())));
            }
        });
    }
}

fn check_signed_in(config: &WKWebViewConfiguration, then: impl Fn(bool) + 'static) {
    let done = RcBlock::new(move |cookies: NonNull<NSArray<NSHTTPCookie>>| {
        // SAFETY: WebKit passes a valid array for the duration of the call.
        let cookies = unsafe { cookies.as_ref() };
        then(cookies.iter().any(|c| c.name().to_string() == "__Secure-3PAPISID" && is_youtube_domain(&c.domain().to_string())));
    });
    // SAFETY: main-thread WebKit calls.
    unsafe { config.websiteDataStore().httpCookieStore().getAllCookies(&done) };
}

fn eval(web_view: &WKWebView, js: &str, then: impl Fn(Option<String>) + 'static) {
    let done = RcBlock::new(move |result: *mut AnyObject, _err: *mut NSError| {
        // SAFETY: WebKit passes a valid (or null) object for the call.
        let text = unsafe { result.as_ref() }.and_then(|o| o.downcast_ref::<NSString>()).map(|s| s.to_string());
        then(text);
    });
    // SAFETY: main-thread WebKit call.
    unsafe { web_view.evaluateJavaScript_completionHandler(&NSString::from_str(js), Some(&done)) };
}

fn tick() {
    let (phase, elapsed, web_view) = with_state(|s| (s.phase, s.phase_at.elapsed(), s.web_view.clone()));
    match phase {
        Phase::SigningIn => tick_signing_in(web_view),
        Phase::Loading => {
            let Some(web_view) = web_view else { return };
            if elapsed > Duration::from_secs(45) {
                eval(&web_view, "__kilo.status()", |status| {
                    eprintln!("playback did not start within 45 s; last status: {status:?}");
                    std::process::exit(1);
                });
                return;
            }
            let secs = elapsed.as_secs();
            eval(&web_view, "window.__kilo ? __kilo.status() : null", move |status| {
                let Some(status) = status else { return };
                if secs % 5 == 0 {
                    log(&format!("loading: {status}"));
                }
                if field(&status, "ad") == Some("true") {
                    log("an ad is showing: this account is not Premium; stopping (Premium-only rule)");
                    std::process::exit(1);
                }
                if secs == 6 && field(&status, "has_video") == Some("true") {
                    with_state(|s| {
                        if let Some(wv) = &s.web_view {
                            eval(
                                wv,
                                "(() => { const v = document.querySelector('video'); if (v) v.muted = false; __kilo.play(); })()",
                                |_| {},
                            );
                        }
                    });
                }
                let clock = field(&status, "t").and_then(|t| t.parse::<u64>().ok()).unwrap_or(0);
                if field(&status, "paused") == Some("false") && clock >= 1 {
                    with_state(|s| {
                        let load_time = s.phase_at.elapsed();
                        s.report.push(format!("audio started after      {:.1} s", load_time.as_secs_f64()));
                        s.phase = Phase::Playing;
                        s.phase_at = Instant::now();
                        s.playing_from = Some((Instant::now(), measure()));
                        s.playing_cpu =
                            kilo_probe::breakdown(&[std::process::id()]).into_iter().map(|(pid, _, sample)| (pid, sample.cpu_ns)).collect();
                    });
                    log(&format!("playing: {status}"));
                    with_state(|s| {
                        if let Some(wv) = &s.web_view {
                            eval(wv, "__kilo.audible()", |_| {});
                        }
                    });
                }
            });
        }
        Phase::Playing => {
            let now = measure();
            let (done, secs) = with_state(|s| {
                s.playing_footprints.push(now.footprint);
                (elapsed >= Duration::from_secs(s.args.seconds), elapsed.as_secs())
            });
            if secs % 10 == 0 {
                log(&format!("playing {secs:>3}s  {}", fmt(&now)));
                if let Some(wv) = &web_view {
                    eval(wv, "__kilo.status()", |st| log(&format!("  page {}", st.unwrap_or_default())));
                    if with_state(|s| s.args.profile) {
                        eval(wv, "window.__kiloProf ? __kiloProf() : ''", |p| log(&format!("  prof {}", p.unwrap_or_default())));
                        if secs == 20 {
                            eval(wv, "window.__kiloTree ? __kiloTree() : ''", |t| log(&format!("  dom\n{}", t.unwrap_or_default())));
                        }
                    }
                }
            }
            if done {
                finish_playing(now);
                if let Some(wv) = &web_view {
                    eval(wv, "__kilo.pause()", |_| {});
                }
            }
        }
        Phase::Paused => {
            if elapsed >= Duration::from_secs(20) {
                with_state(|s| {
                    s.report.push(format!("paused 20 s              {}", fmt(&measure())));
                    // Tear down the web view: this is what Kilo does after a
                    // few idle minutes, so idle memory returns to the native app.
                    if let Some(wv) = s.web_view.take() {
                        // SAFETY: main-thread WebKit calls.
                        unsafe {
                            wv.stopLoading();
                            wv.configuration().userContentController().removeAllScriptMessageHandlers();
                            wv.removeFromSuperview();
                        }
                    }
                    if let Some(w) = s.window.take() {
                        w.close();
                    }
                    s.phase = Phase::TornDown;
                    s.phase_at = Instant::now();
                });
            }
        }
        Phase::TornDown => {
            if elapsed.as_secs() == 3 || elapsed.as_secs() == 15 {
                with_state(|s| s.report.push(format!("after teardown +{:>2} s     {}", elapsed.as_secs(), fmt(&measure()))));
            }
            if elapsed >= Duration::from_secs(15) {
                let report = with_state(|s| std::mem::take(&mut s.report));
                println!("\n==== report ====");
                for line in report {
                    println!("{line}");
                }
                std::process::exit(0);
            }
        }
    }
}

fn finish_playing(now: Sample) {
    with_state(|s| {
        let (from, start) = s.playing_from.expect("set when playing began");
        let wall = from.elapsed().as_nanos().max(1) as f64;
        let cpu = now.cpu_ns.saturating_sub(start.cpu_ns) as f64 / wall * 100.0;
        let fps = &s.playing_footprints;
        let avg = fps.iter().sum::<u64>() / fps.len().max(1) as u64;
        let peak = fps.iter().copied().max().unwrap_or(0);
        let wake = now.wakeups.saturating_sub(start.wakeups) as f64 / wall * 1e9;
        s.report.push(format!(
            "playing {:>3} s            avg {}  peak {}  cpu {cpu:.1}% of a core  wakeups {wake:.0}/s",
            s.args.seconds,
            mb(avg),
            mb(peak),
        ));
        s.report.push("  breakdown while playing:".into());
        for (pid, name, sample) in kilo_probe::breakdown(&[std::process::id()]) {
            let start_cpu = s.playing_cpu.iter().find(|(p, _)| *p == pid).map_or(0, |(_, c)| *c);
            let cpu = sample.cpu_ns.saturating_sub(start_cpu) as f64 / wall * 100.0;
            s.report.push(format!("    {pid:>6}  {:<34} {}  cpu {cpu:5.1}%", name, mb(sample.footprint)));
        }
        s.phase = Phase::Paused;
        s.phase_at = Instant::now();
    });
}

fn tick_signing_in(web_view: Option<Retained<WKWebView>>) {
    let Some(web_view) = web_view else { return };
    if let Some(at) = with_state(|s| s.signed_in_at) {
        // Let the redirect chain finish setting cookies, then keep them (once).
        if at.elapsed() > Duration::from_secs(3) && !with_state(|s| std::mem::replace(&mut s.keeping, true)) {
            keep_youtube_cookies(&web_view);
        }
        return;
    }
    // SAFETY: main-thread WebKit call.
    let on_ytm = unsafe { web_view.URL() }.and_then(|u| u.host()).is_some_and(|h| h.to_string() == "music.youtube.com");
    if !on_ytm {
        return;
    }
    // SAFETY: as above.
    let config = unsafe { web_view.configuration() };
    check_signed_in(&config, |signed_in| {
        if signed_in {
            with_state(|s| {
                s.signed_in_at.get_or_insert_with(Instant::now);
            });
            log("signed in to YouTube Music");
        }
    });
}

/// Signed in: copies YouTube's cookies, and only those, from the window's
/// memory into this binary's WebKit store, where `play` finds them, then
/// exits (WebKit writes its cookie file as the process exits).
fn keep_youtube_cookies(web_view: &WKWebView) {
    let mtm = MainThreadMarker::new().expect("main thread");
    let done = RcBlock::new(move |cookies: NonNull<NSArray<NSHTTPCookie>>| {
        // SAFETY: WebKit passes a valid array for the duration of the call.
        let cookies = unsafe { cookies.as_ref() };
        let youtube: Vec<Retained<NSHTTPCookie>> = cookies.iter().filter(|c| is_youtube_domain(&c.domain().to_string())).collect();
        if youtube.is_empty() {
            eprintln!("no YouTube cookies to keep");
            std::process::exit(1);
        }
        // SAFETY: main-thread WebKit call.
        let store = unsafe { WKWebsiteDataStore::defaultDataStore(mtm).httpCookieStore() };
        let left = Rc::new(Cell::new(youtube.len()));
        for cookie in &youtube {
            let left = left.clone();
            let saved = RcBlock::new(move || {
                left.set(left.get() - 1);
                if left.get() == 0 {
                    println!("signed in; YouTube's cookies are stored in this app's WebKit data store");
                    std::process::exit(0);
                }
            });
            // SAFETY: main-thread WebKit call with a live cookie.
            unsafe { store.setCookie_completionHandler(cookie, Some(&saved)) };
        }
    });
    // SAFETY: main-thread WebKit calls.
    unsafe { web_view.configuration().websiteDataStore().httpCookieStore().getAllCookies(&done) };
}

/// Extracts a scalar field from our own flat JSON status string.
fn field<'a>(json: &'a str, key: &str) -> Option<&'a str> {
    let start = json.find(&format!("\"{key}\":"))? + key.len() + 3;
    let rest = &json[start..];
    Some(rest[..rest.find([',', '}'])?].trim())
}

fn measure() -> Sample {
    kilo_probe::sample_trees(&[std::process::id()]).map(|(s, _)| s).unwrap_or_default()
}

fn fmt(s: &Sample) -> String {
    let procs = kilo_probe::process_trees(&[std::process::id()]).len();
    format!("footprint {}  ({procs} processes)", mb(s.footprint))
}

fn mb(bytes: u64) -> String {
    format!("{:>6.1} MB", bytes as f64 / (1024.0 * 1024.0))
}

fn log(msg: &str) {
    let t = with_state(|s| s.phase_at.elapsed().as_secs_f64());
    println!("[{t:6.1}s] {msg}");
}

/// youtube.com or a subdomain of it, exactly (`notyoutube.com` isn't), as
/// `kilo_core::auth::is_youtube_domain` checks it.
fn is_youtube_domain(domain: &str) -> bool {
    let domain = domain.strip_prefix('.').unwrap_or(domain);
    domain.strip_suffix("youtube.com").is_some_and(|rest| rest.is_empty() || rest.ends_with('.'))
}
