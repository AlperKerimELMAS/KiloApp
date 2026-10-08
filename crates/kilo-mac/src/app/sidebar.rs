//! Collapsing and expanding the sidebar: a short slide on a display link,
//! with the page re-framed (not rebuilt) on every frame.

use std::cell::RefCell;
use std::time::{Duration, Instant};

use objc2::rc::Retained;
use objc2::sel;
use objc2_app_kit::NSWorkspace;
use objc2_foundation::{NSRunLoop, NSRunLoopCommonModes};
use objc2_quartz_core::CADisplayLink;

use super::{browse, mtm, with, with_shell};
use crate::settings;
use crate::ui;
use crate::ui::shell;

/// How long the sidebar takes to collapse or expand.
const SIDEBAR_ANIMATION: Duration = Duration::from_millis(220);

/// The sidebar collapsing or expanding, frame by frame.
struct SidebarAnimation {
    link: Retained<CADisplayLink>,
    from: f64,
    to: f64,
    start: Instant,
    /// For the scenario log: frames shown, and the slowest one's work.
    frames: u32,
    slowest: Duration,
}

thread_local! {
    static SIDEBAR: RefCell<Option<SidebarAnimation>> = const { RefCell::new(None) };
}

/// The ☰ button: collapses the sidebar to its icons, or expands it (and
/// remembers which). It slides, briefly, unless the Mac is set to reduce
/// motion; toggling again midway turns it around.
pub fn toggle_sidebar() {
    let collapsed = !settings::sidebar_collapsed();
    settings::set_sidebar_collapsed(collapsed);
    let to = if collapsed { 1.0 } else { 0.0 };
    let reduce = NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion();
    let Some((from, view)) = with_shell(|s| (shell::sidebar_progress(s), s.window.contentView())) else { return };
    let running = SIDEBAR.with_borrow_mut(|a| {
        a.as_mut().map(|a| {
            (a.from, a.to, a.start, a.frames) = (from, to, Instant::now(), 0);
        })
    });
    if running.is_some() {
        return;
    }
    let Some(view) = view.filter(|_| !reduce) else {
        with_shell(|s| {
            shell::set_collapsed(s, collapsed);
            s.window.layoutIfNeeded();
        });
        return browse::schedule_refresh();
    };
    let target: &objc2::runtime::AnyObject = ui::actions(mtm());
    // SAFETY: the target (the app-wide action object) has `sidebarFrame:`.
    let link = unsafe { view.displayLinkWithTarget_selector(target, sel!(sidebarFrame:)) };
    // SAFETY: the main run loop, in its common modes (so it runs during
    // tracking too).
    unsafe { link.addToRunLoop_forMode(&NSRunLoop::mainRunLoop(), NSRunLoopCommonModes) };
    SIDEBAR.set(Some(SidebarAnimation { link, from, to, start: Instant::now(), frames: 0, slowest: Duration::ZERO }));
}

/// One frame of the sidebar animation (from its display link): the
/// sidebar's width eases out, and the page follows (only re-framed, so a
/// frame costs well under a millisecond).
pub fn sidebar_frame() {
    let Some((from, to, start)) = SIDEBAR.with_borrow(|a| a.as_ref().map(|a| (a.from, a.to, a.start))) else { return };
    let work = Instant::now();
    let t = (start.elapsed().as_secs_f64() / SIDEBAR_ANIMATION.as_secs_f64()).min(1.0);
    let eased = 1.0 - (1.0 - t).powi(3);
    let open = with_shell(|s| {
        if t < 1.0 {
            shell::set_sidebar_progress(s, from + (to - from) * eased);
        } else {
            shell::set_collapsed(s, to == 1.0);
        }
        s.window.layoutIfNeeded();
    })
    .is_some();
    with(|a| {
        if let Some(v) = &a.view {
            v.refresh();
        }
    });
    let done = SIDEBAR.with_borrow_mut(|a| {
        let a = a.as_mut()?;
        a.frames += 1;
        a.slowest = a.slowest.max(work.elapsed());
        (t >= 1.0 || !open).then_some((a.frames, a.slowest))
    });
    if let Some((frames, slowest)) = done {
        if let Some(a) = SIDEBAR.take() {
            a.link.invalidate();
        }
        crate::debug::trace(|| format!("sidebar: {frames} frames, slowest {:.2} ms", slowest.as_secs_f64() * 1000.0));
        browse::schedule_refresh();
    }
}
