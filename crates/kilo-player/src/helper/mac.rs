//! macOS helper: a WKWebView running m.youtube.com's player, tuned for the
//! smallest footprint we measured (see docs/PLAN.md, "Playback").

use std::cell::RefCell;
use std::io::{BufRead, Write};
use std::ptr::NonNull;

use block2::RcBlock;
use dispatch2::DispatchQueue;
use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSWindow, NSWindowStyleMask};
use objc2_foundation::{NSArray, NSError, NSHTTPCookie, NSPoint, NSRect, NSSize, NSString, NSURL, NSURLRequest, ns_string};
use objc2_web_kit::{
    WKAudiovisualMediaTypes, WKContentRuleList, WKContentRuleListStore, WKInactiveSchedulingPolicy, WKNavigation, WKNavigationAction,
    WKNavigationActionPolicy, WKNavigationDelegate, WKScriptMessage, WKScriptMessageHandler, WKUserContentController, WKUserScript,
    WKUserScriptInjectionTime, WKWebView, WKWebViewConfiguration,
};

use crate::protocol::{Command, Event, Position, VideoId};

/// The mobile web player is the lightest page that runs YouTube's full
/// player: 68 MB of WebContent vs. 348 MB for music.youtube.com.
const IPHONE_UA: &str = "Mozilla/5.0 (iPhone; CPU iPhone OS 18_6 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.0 Mobile/15E148 Safari/604.1";

/// The player sizes its video to the view: 128×72 gets the 144p stream.
const VIEW_SIZE: NSSize = NSSize::new(128.0, 72.0);

/// Nothing is ever shown, so images and fonts are never needed. Scripts,
/// attestation and play reporting are never blocked.
const RULES: &str = r#"[{"trigger":{"url-filter":".*","resource-type":["image","font"]},"action":{"type":"block"}}]"#;

/// The page's own interface is never shown: take it out of style and layout
/// entirely (10% less memory, 40% fewer wakeups). Cosmetic only. That
/// includes the video picture: the helper then composites nothing (17 → 10
/// wakeups/s measured), though WebKit still decodes it.
const PAGE_CSS: &str = "*,*::before,*::after{animation:none!important;transition:none!important}\
#player-control-container,#header-bar,ytm-mobile-topbar-renderer,ytm-watch,ytm-pivot-bar-renderer,video{display:none!important}";

/// Page-world bridge. Media events go to the helper as one-line messages;
/// `__kilo` drives YouTube's player through its own API. `player` says that
/// API is there: until then, the helper holds its commands.
const BRIDGE: &str = r#"(() => {
  const post = (m) => { try { webkit.messageHandlers.kilo.postMessage(m); } catch (e) {} };
  let video = null, player = null, ready = false;
  const readyCheck = () => {
    if (!ready && player && typeof player.loadVideoById === 'function') { ready = true; post('player'); }
  };
  const id = () => { try { return player.getVideoData().video_id || '-'; } catch (e) { return '-'; } };
  const report = (kind) => {
    const d = isFinite(video.duration) ? video.duration : 0;
    post(`${kind} ${video.currentTime.toFixed(2)} ${d.toFixed(2)} ${id()}`);
  };
  const adCheck = () => { if (player && player.classList.contains('ad-showing')) post('ad'); };
  const adObserver = new MutationObserver(adCheck);
  const attach = () => {
    const p = document.querySelector('#movie_player');
    if (p && p !== player) {
      player = p;
      adObserver.disconnect();
      adObserver.observe(p, { attributes: true, attributeFilter: ['class'] });
      adCheck();
    }
    readyCheck();
    const v = document.querySelector('video');
    if (v && v !== video) {
      video = v;
      v.addEventListener('loadedmetadata', readyCheck);
      v.addEventListener('playing', () => { readyCheck(); report('playing'); });
      v.addEventListener('pause', () => { if (!v.ended) report('paused'); });
      v.addEventListener('waiting', () => report('buffering'));
      v.addEventListener('ended', () => report('ended'));
      v.addEventListener('seeked', () => report(v.paused ? 'paused' : 'playing'));
      v.addEventListener('error', () => post('error media ' + (v.error ? v.error.code : '?')));
    }
  };
  new MutationObserver(attach).observe(document, { childList: true, subtree: true });
  // Media keys and Control Center: next/previous belong to Kilo's queue, not
  // to the mobile site's own autoplay.
  if (navigator.mediaSession) {
    const ms = navigator.mediaSession, set = ms.setActionHandler.bind(ms);
    const ours = { nexttrack: () => post('next'), previoustrack: () => post('previous') };
    ms.setActionHandler = (action, handler) => set(action, ours[action] || handler);
    for (const [action, handler] of Object.entries(ours)) { try { set(action, handler); } catch (e) {} }
  }
  const p = () => player || document.querySelector('#movie_player');
  window.__kilo = {
    load(id, start) { p().loadVideoById(id, start); },
    play() { p().playVideo(); },
    pause() { p().pauseVideo(); },
    // Checked when it runs, not when the event that asked for it was sent:
    // a stale event about the previous track must not pause the next one.
    pauseUnless(want) { if (id() !== want) p().pauseVideo(); },
    seek(s) { p().seekTo(s, true); },
    // The volume first, then unmuted: never a moment louder than asked.
    volume(v) { const q = p(); q.setVolume(v); if (v > 0) q.unMute(); else q.mute(); if (video) video.muted = v === 0; },
    tiny() { const q = p(); if (q.setPlaybackQualityRange) q.setPlaybackQualityRange('tiny', 'tiny'); },
  };
})();"#;

/// Main-frame navigations may only go to these hosts.
const ALLOWED_HOSTS: &[&str] = &["m.youtube.com", "www.youtube.com", "youtube.com", "consent.youtube.com", "accounts.google.com"];

struct Helper {
    config: Retained<WKWebViewConfiguration>,
    web_view: Option<Retained<WKWebView>>,
    window: Option<Retained<NSWindow>>,
    /// WebKit holds delegates weakly.
    delegate: Retained<Delegate>,
    page_requested: bool,
    /// YouTube's player API is there (`player` from the page): commands run
    /// at once. Before that, they wait in `pending`.
    player_ready: bool,
    /// The track the app asked for. Anything else that starts playing (the
    /// mobile site's autoplay, say) is paused immediately.
    expected: Option<VideoId>,
    volume: u8,
    /// Apply volume and quality on the next `playing` after a load.
    needs_setup: bool,
    /// Commands that arrived before the player could take them.
    pending: Vec<Command>,
}

thread_local! {
    static HELPER: RefCell<Option<Helper>> = const { RefCell::new(None) };
}

fn with<R>(f: impl FnOnce(&mut Helper) -> R) -> R {
    HELPER.with_borrow_mut(|h| f(h.as_mut().expect("helper initialized")))
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements; no ivars, no Drop.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "KiloPlayerDelegate"]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl WKScriptMessageHandler for Delegate {
        #[unsafe(method(userContentController:didReceiveScriptMessage:))]
        fn did_receive(&self, _controller: &WKUserContentController, message: &WKScriptMessage) {
            // Only the page itself speaks for the player: its frames (ads,
            // say) can reach the handler too. And only short messages.
            // SAFETY: called by WebKit on the main thread with a live message.
            let main_frame = unsafe { message.frameInfo().isMainFrame() };
            // SAFETY: as above.
            if let Ok(text) = unsafe { message.body() }.downcast::<NSString>()
                && main_frame
                && text.length() <= 256
            {
                on_page_message(&text.to_string());
            }
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
            let allowed = unsafe {
                match action.targetFrame() {
                    // A new window: there are none.
                    None => false,
                    // The page's own frames (the player needs them).
                    Some(frame) if !frame.isMainFrame() => true,
                    Some(_) => action.request().URL().is_some_and(|u| {
                        u.scheme().is_some_and(|s| s.to_string() == "https")
                            && u.host().is_some_and(|h| ALLOWED_HOSTS.contains(&h.to_string().as_str()))
                    }),
                }
            };
            decision.call((if allowed { WKNavigationActionPolicy::Allow } else { WKNavigationActionPolicy::Cancel },));
        }

        #[unsafe(method(webView:didFailProvisionalNavigation:withError:))]
        fn failed_to_start(&self, _web_view: &WKWebView, _navigation: Option<&WKNavigation>, error: &NSError) {
            page_failed(error);
        }

        #[unsafe(method(webView:didFailNavigation:withError:))]
        fn failed(&self, _web_view: &WKWebView, _navigation: Option<&WKNavigation>, error: &NSError) {
            page_failed(error);
        }

        #[unsafe(method(webViewWebContentProcessDidTerminate:))]
        fn content_terminated(&self, _web_view: &WKWebView) {
            // Let the app respawn a clean helper rather than limp on.
            emit(&Event::Error("web content process terminated".into()));
            std::process::exit(4);
        }
    }
);

/// The player page didn't load (offline, say): this helper can't play, so
/// it says why and exits; the app starts a fresh one on the next play.
fn page_failed(error: &NSError) {
    // Cancelled (by a newer load, or by the navigation policy): not a failure.
    const CANCELLED: isize = -999;
    const INTERRUPTED_BY_POLICY: isize = 102;
    if matches!(error.code(), CANCELLED | INTERRUPTED_BY_POLICY) {
        return;
    }
    emit(&Event::Error(format!("player page failed to load ({} {})", error.domain(), error.code())));
    std::process::exit(4);
}

impl Delegate {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        // SAFETY: plain NSObject init.
        unsafe { msg_send![Self::alloc(mtm), init] }
    }
}

pub fn run() -> ! {
    let mtm = MainThreadMarker::new().expect("the helper runs on the main thread");
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);

    let delegate = Delegate::new(mtm);
    let config = make_config(mtm, &delegate);
    HELPER.set(Some(Helper {
        config: config.clone(),
        web_view: None,
        window: None,
        delegate,
        page_requested: false,
        player_ready: false,
        expected: None,
        volume: 100,
        needs_setup: false,
        pending: Vec::new(),
    }));

    // Premium-only: nothing loads unless this web view is signed in.
    check_signed_in(&config, move |signed_in| {
        if !signed_in {
            emit(&Event::SignedOut);
            std::process::exit(3);
        }
        compile_rules(move |rules| {
            let mtm = MainThreadMarker::new().expect("main thread");
            if let Some(rules) = rules {
                // SAFETY: main-thread WebKit call with a valid rule list.
                unsafe { with(|h| h.config.userContentController().addContentRuleList(&rules)) };
            }
            create_web_view(mtm);
            emit(&Event::Ready);
            for cmd in with(|h| std::mem::take(&mut h.pending)) {
                handle(cmd);
            }
        });
    });

    spawn_command_reader();
    app.run();
    emit(&Event::Error("helper's event loop ended".into()));
    std::process::exit(0)
}

fn make_config(mtm: MainThreadMarker, delegate: &Delegate) -> Retained<WKWebViewConfiguration> {
    // SAFETY: main-thread WebKit calls with valid arguments.
    unsafe {
        let config = WKWebViewConfiguration::new(mtm);
        config.setMediaTypesRequiringUserActionForPlayback(WKAudiovisualMediaTypes::None);
        let prefs = config.preferences();
        // Only YouTube pages load here: skip Safe Browsing's lookups and database.
        prefs.setFraudulentWebsiteWarningEnabled(false);
        // Ask WebKit to suspend the hidden page when it can. Measured: it keeps
        // a page that has played media running (1.4% CPU while paused), so
        // the app also kills the whole helper after a few idle minutes.
        prefs.setInactiveSchedulingPolicy(WKInactiveSchedulingPolicy::Suspend);
        // Lockdown Mode (no JIT, no WebAssembly): 68 MB instead of 140 MB of
        // WebContent, at the same total CPU.
        let page_prefs = config.defaultWebpagePreferences();
        page_prefs.setLockdownModeEnabled(true);
        config.setDefaultWebpagePreferences(Some(&page_prefs));

        let controller = config.userContentController();
        controller.addScriptMessageHandler_name(ProtocolObject::from_ref(delegate), ns_string!("kilo"));
        let css = format!(
            "(() => {{ const s = document.createElement('style'); s.textContent = {PAGE_CSS:?}; (document.head || document.documentElement).appendChild(s); }})();"
        );
        for source in [BRIDGE, css.as_str()] {
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

fn compile_rules(then: impl Fn(Option<Retained<WKContentRuleList>>) + 'static) {
    let mtm = MainThreadMarker::new().expect("main thread");
    // SAFETY: main-thread WebKit calls; the completion pointer is retained
    // before use.
    unsafe {
        let Some(store) = WKContentRuleListStore::defaultStore(mtm) else {
            return then(None);
        };
        let done = RcBlock::new(move |list: *mut WKContentRuleList, err: *mut NSError| {
            if let Some(err) = err.as_ref() {
                emit(&Event::Error(format!("content rules failed to compile: {err:?}")));
            }
            then(Retained::retain(list));
        });
        store.compileContentRuleListForIdentifier_encodedContentRuleList_completionHandler(
            Some(ns_string!("kilo-player")),
            Some(&NSString::from_str(RULES)),
            Some(&done),
        );
    }
}

fn create_web_view(mtm: MainThreadMarker) {
    with(|h| {
        // SAFETY: main-thread AppKit/WebKit calls with valid arguments.
        unsafe {
            let web_view = WKWebView::initWithFrame_configuration(WKWebView::alloc(mtm), NSRect::new(NSPoint::ZERO, VIEW_SIZE), &h.config);
            web_view.setCustomUserAgent(Some(&NSString::from_str(IPHONE_UA)));
            web_view.setNavigationDelegate(Some(ProtocolObject::from_ref(&*h.delegate)));
            // WebKit only runs media for a view that's in a window, so park
            // it in a borderless window far off screen.
            let window = NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(mtm),
                NSRect::new(NSPoint::new(-10000.0, -10000.0), VIEW_SIZE),
                NSWindowStyleMask::Borderless,
                NSBackingStoreType::Buffered,
                false,
            );
            window.setReleasedWhenClosed(false);
            window.setContentView(Some(&web_view));
            window.orderBack(None);
            h.web_view = Some(web_view);
            h.window = Some(window);
        }
    });
}

fn spawn_command_reader() {
    let spawned = std::thread::Builder::new().name("player-commands".into()).stack_size(64 * 1024).spawn(|| {
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            match Command::parse(&line) {
                Some(cmd) => DispatchQueue::main().exec_async(move || handle(cmd)),
                None => emit(&Event::Error(format!("unknown command: {line}"))),
            }
        }
        // The app closed our stdin or died: never outlive it.
        eprintln!("kilo-player: stdin closed, exiting");
        std::process::exit(0);
    });
    if spawned.is_err() {
        std::process::exit(5);
    }
}

fn handle(cmd: Command) {
    if matches!(cmd, Command::Quit) {
        std::process::exit(0);
    }
    if let Command::Volume(v) = cmd {
        with(|h| h.volume = v);
    }
    // The first load opens the player page; until YouTube's player is on
    // it, everything else waits (and then runs in order).
    let (has_view, requested, ready) = with(|h| (h.web_view.is_some(), h.page_requested, h.player_ready));
    if let Command::Load(id, start) = &cmd
        && has_view
        && !requested
    {
        with(|h| {
            h.expected = Some(id.clone());
            h.needs_setup = true;
            h.page_requested = true;
        });
        return load_page(&format!("https://m.youtube.com/watch?v={id}&t={}s", *start as u64));
    }
    if !ready {
        with(|h| h.pending.push(cmd));
        return;
    }
    match cmd {
        Command::Load(id, start) => {
            with(|h| {
                h.expected = Some(id.clone());
                h.needs_setup = true;
            });
            // Same page, same player: no reload, so switching is fast.
            js(&format!("__kilo.load('{id}', {start:.2})"));
        }
        Command::Play => js("__kilo.play()"),
        Command::Pause => js("__kilo.pause()"),
        Command::Seek(s) => js(&format!("__kilo.seek({s:.2})")),
        Command::Volume(v) => js(&format!("__kilo.volume({v})")),
        Command::Quit => {}
    }
}

/// YouTube's player is on the page: the volume first (before any sound),
/// then the commands that waited, in order.
fn player_ready() {
    if with(|h| std::mem::replace(&mut h.player_ready, true)) {
        return;
    }
    let volume = with(|h| h.volume);
    js(&format!("__kilo.volume({volume})"));
    for cmd in with(|h| std::mem::take(&mut h.pending)) {
        handle(cmd);
    }
}

fn on_page_message(msg: &str) {
    let (kind, rest) = msg.split_once(' ').unwrap_or((msg, ""));
    match kind {
        "player" => player_ready(),
        "ad" => {
            js("__kilo.pause()");
            emit(&Event::AdShowing);
        }
        // Page text: marked as such, and short.
        "error" => emit(&Event::Error(format!("page: {}", rest.chars().take(200).collect::<String>()))),
        "next" => emit(&Event::Next),
        "previous" => emit(&Event::Previous),
        "playing" | "paused" | "buffering" | "ended" => {
            // Only well-formed reports: finite, non-negative times.
            let time = |s: Option<&str>| s.and_then(|s| s.parse::<f64>().ok()).filter(|t| t.is_finite() && *t >= 0.0);
            let mut w = rest.split(' ');
            let (Some(seconds), Some(duration)) = (time(w.next()), time(w.next())) else { return };
            let video = w.next().and_then(VideoId::parse);
            // A report means the player is there, even if `player` was
            // missed. First, though: the commands that waited for it may
            // load another track, and then this report is about an old one.
            player_ready();
            let expected = with(|h| h.expected.clone());
            let Some(video) = video.filter(|v| Some(v) == expected.as_ref()) else {
                if kind == "playing" {
                    // Never play anything the app didn't ask for.
                    match &expected {
                        Some(want) => js(&format!("__kilo.pauseUnless('{want}')")),
                        None => js("__kilo.pause()"),
                    }
                }
                return;
            };
            if kind == "playing" && with(|h| std::mem::take(&mut h.needs_setup)) {
                let volume = with(|h| h.volume);
                js(&format!("__kilo.volume({volume}); __kilo.tiny()"));
            }
            let pos = Position { seconds, duration, video };
            emit(&match kind {
                "playing" => Event::Playing(pos),
                "paused" => Event::Paused(pos),
                "buffering" => Event::Buffering(pos),
                _ => Event::Ended(pos),
            });
        }
        _ => {}
    }
}

fn load_page(url: &str) {
    with(|h| {
        let Some(web_view) = &h.web_view else { return };
        let Some(url) = NSURL::URLWithString(&NSString::from_str(url)) else { return };
        // SAFETY: main-thread WebKit call.
        unsafe { web_view.loadRequest(&NSURLRequest::requestWithURL(&url)) };
    });
}

fn js(source: &str) {
    with(|h| {
        if let Some(web_view) = &h.web_view {
            // SAFETY: main-thread WebKit call; no completion handler.
            unsafe { web_view.evaluateJavaScript_completionHandler(&NSString::from_str(source), None) };
        }
    });
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

/// youtube.com or a subdomain of it (`kilo_core::auth::is_youtube_domain`).
fn is_youtube_domain(domain: &str) -> bool {
    let domain = domain.strip_prefix('.').unwrap_or(domain);
    domain.strip_suffix("youtube.com").is_some_and(|rest| rest.is_empty() || rest.ends_with('.'))
}

fn emit(event: &Event) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{}", event.encode());
    let _ = out.flush();
}
