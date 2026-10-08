//! The views Kilo defines: the window's root, the focus sink (which also
//! hears the keyboard shortcuts), flipped containers, clickable cards and
//! rows with the one hover play button, and shelves that scroll sideways.

use std::cell::{Cell, RefCell};

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol};
use objc2::{AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSAccessibility, NSAccessibilityButtonRole, NSButton, NSColor, NSEvent, NSEventPhase, NSResponder, NSScrollElasticity, NSScrollView,
    NSTrackingArea, NSTrackingAreaOptions, NSView,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

use super::{circle_button, fill, theme};
use crate::app;

// A flipped container, so content in scroll views starts at the top.
define_class!(
    // SAFETY: NSView subclass with no ivars; only overrides isFlipped.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "KiloFlippedView"]
    pub struct FlippedView;

    impl FlippedView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }
    }
);

impl FlippedView {
    pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
        // SAFETY: plain NSView init.
        unsafe { msg_send![Self::alloc(mtm), init] }
    }
}

// The window's root view: clicks nothing else took (the page's background,
// the sidebar) take the focus off the search field, so Space plays and
// pauses again, and the mouse's back button goes back.
define_class!(
    // SAFETY: NSView subclass with no ivars; overrides match AppKit's
    // signatures.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "KiloRootView"]
    pub struct RootView;

    impl RootView {
        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, _event: &NSEvent) {
            app::release_search_focus();
        }

        #[unsafe(method(otherMouseUp:))]
        fn other_mouse_up(&self, event: &NSEvent) {
            // Button 3 is the back button (4, forward).
            if event.buttonNumber() == 3 {
                crate::net::later(app::back);
            }
        }
    }
);

impl RootView {
    pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
        // SAFETY: plain NSView init.
        unsafe { msg_send![Self::alloc(mtm), init] }
    }
}

// Takes keyboard focus when nothing else should have it, and hears the
// keyboard shortcuts the menus don't carry (Space, M, the arrows). Without
// it, AppKit focuses the search field whenever the window becomes key, and
// a focused text field starts macOS's AutoFill service (~11 MB) for no
// reason.
define_class!(
    // SAFETY: NSView subclass with no ivars; overrides match AppKit's
    // signatures.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "KiloFocusSink"]
    pub struct FocusSink;

    impl FocusSink {
        #[unsafe(method(acceptsFirstResponder))]
        fn accepts_first_responder(&self) -> bool {
            true
        }

        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &NSEvent) {
            if !crate::keys::handle(event) {
                // SAFETY: NSView's implementation (passes it on, or beeps).
                let _: () = unsafe { msg_send![super(self), keyDown: event] };
            }
        }

        // Every window has one, so it also hears when the app's look
        // changes (the user's choice, or the system's at sunset).
        #[unsafe(method(viewDidChangeEffectiveAppearance))]
        fn appearance_changed(&self) {
            crate::net::later(app::appearance_changed);
        }
    }
);

impl FocusSink {
    pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
        // SAFETY: plain NSView init.
        unsafe { msg_send![Self::alloc(mtm), init] }
    }
}

pub struct ItemIvars {
    item: Cell<u32>,
    /// Where the hover play button goes, for cards that have one.
    play_at: Cell<Option<NSPoint>>,
}

// A clickable card or row. Clicking it activates item `item` of the current
// page; hovering highlights it (and shows the play button on cards), and
// pressing darkens it at once, before anything loads.
define_class!(
    // SAFETY: NSView subclass; ivars are plain Cells; overridden methods
    // match AppKit's signatures.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "KiloItemView"]
    #[ivars = ItemIvars]
    pub struct ItemView;

    impl ItemView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, _event: &NSEvent) {
            self.set_fill(Some(&theme::palette().pressed));
            app::release_search_focus();
        }

        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, event: &NSEvent) {
            let point = self.convertPoint_fromView(event.locationInWindow(), None);
            let inside = self.mouse_inRect(point, self.bounds());
            self.set_fill(inside.then(|| theme::palette().hover.clone()).as_deref());
            if inside {
                let item = self.ivars().item.get();
                // Step out of AppKit's event handling before changing pages.
                crate::net::later(move || app::activate(item));
            }
        }

        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: Option<&NSEvent>) -> bool {
            true
        }

        #[unsafe(method(mouseEntered:))]
        fn mouse_entered(&self, _event: &NSEvent) {
            self.set_fill(Some(&theme::palette().hover));
            if let Some(at) = self.ivars().play_at.get() {
                show_hover_play(self, at, self.ivars().item.get());
            }
        }

        #[unsafe(method(mouseExited:))]
        fn mouse_exited(&self, _event: &NSEvent) {
            self.set_fill(None);
            hide_hover_play(self);
        }

        // VoiceOver's "press": the same as a click.
        #[unsafe(method(accessibilityPerformPress))]
        fn press(&self) -> bool {
            let item = self.ivars().item.get();
            crate::net::later(move || app::activate(item));
            true
        }
    }

    unsafe impl NSObjectProtocol for ItemView {}
);

impl ItemView {
    /// A card or row for item `item`, with corners of `radius`.
    pub fn new(item: u32, radius: f64, mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ItemIvars { item: Cell::new(item), play_at: Cell::new(None) });
        // SAFETY: NSView's designated initializer.
        let view: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: NSRect::ZERO] };
        fill(&view, None, radius);
        let owner: &AnyObject = &view;
        // SAFETY: the tracking area's owner is the view itself; InVisibleRect
        // keeps it sized to the view automatically.
        let area = unsafe {
            NSTrackingArea::initWithRect_options_owner_userInfo(
                NSTrackingArea::alloc(),
                NSRect::ZERO,
                NSTrackingAreaOptions::MouseEnteredAndExited
                    | NSTrackingAreaOptions::ActiveInKeyWindow
                    | NSTrackingAreaOptions::InVisibleRect,
                Some(owner),
                None,
            )
        };
        view.addTrackingArea(&area);
        view.setAccessibilityElement(true);
        // SAFETY: the role is a constant string.
        view.setAccessibilityRole(Some(unsafe { NSAccessibilityButtonRole }));
        view
    }

    /// What VoiceOver says for it: its title, and its subtitle if any.
    pub fn set_label(&self, title: &str, subtitle: &str) {
        let label = if subtitle.is_empty() { title.to_owned() } else { format!("{title}, {subtitle}") };
        self.setAccessibilityLabel(Some(&NSString::from_str(&label)));
    }

    /// Shows a play button at `at` (in the card's coordinates) on hover.
    pub fn set_play_at(&self, at: NSPoint) {
        self.ivars().play_at.set(Some(at));
    }

    fn set_fill(&self, color: Option<&NSColor>) {
        if let Some(layer) = self.layer() {
            layer.setBackgroundColor(color.map(|c| c.CGColor()).as_deref());
        }
    }
}

/// Diameter of the play button cards show on hover.
pub const HOVER_PLAY: f64 = 44.0;

thread_local! {
    /// The one play button cards show on hover: moved to whichever card the
    /// mouse is over, instead of one per card.
    static HOVER_BUTTON: RefCell<Option<Retained<NSButton>>> = const { RefCell::new(None) };
}

fn show_hover_play(card: &NSView, at: NSPoint, item: u32) {
    let mtm = MainThreadMarker::new().expect("main thread");
    let button = HOVER_BUTTON.with_borrow_mut(|b| {
        b.get_or_insert_with(|| {
            let button = circle_button("play.fill", sel!(playItem:), HOVER_PLAY, 17.0, true, mtm);
            button.setFrameSize(NSSize::new(HOVER_PLAY, HOVER_PLAY));
            button
        })
        .clone()
    });
    button.setTag(item as isize);
    button.setFrameOrigin(at);
    card.addSubview(&button);
}

fn hide_hover_play(card: &NSView) {
    HOVER_BUTTON.with_borrow(|b| {
        let Some(b) = b else { return };
        // SAFETY: main-thread AppKit call on a live view.
        let parent = unsafe { b.superview() };
        if parent.is_some_and(|s| std::ptr::eq(&*s, card)) {
            b.removeFromSuperview();
        }
    });
}

/// Forgets the hover play button (its colors are the old theme's).
pub fn reset_hover_play() {
    HOVER_BUTTON.with_borrow_mut(|b| {
        if let Some(b) = b.take() {
            b.removeFromSuperview();
        }
    });
}

pub struct ShelfIvars {
    /// Where the current scroll gesture goes: 0 not decided yet, 1 this
    /// shelf (sideways), 2 the page.
    axis: Cell<u8>,
}

// A shelf that scrolls sideways inside the page. Each scroll gesture goes to
// the shelf only if it's mostly sideways and the shelf has more to show;
// otherwise the page scrolls, as if the pointer weren't over a shelf.
define_class!(
    // SAFETY: NSScrollView subclass; the ivar is a plain Cell; overrides
    // match AppKit's signatures and call the superclass.
    #[unsafe(super(NSScrollView, NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "KiloShelfScrollView"]
    #[ivars = ShelfIvars]
    pub struct ShelfScrollView;

    impl ShelfScrollView {
        #[unsafe(method(scrollWheel:))]
        fn scroll_wheel(&self, event: &NSEvent) {
            // SAFETY: main-thread AppKit call on a live view.
            let page = unsafe { self.superview() }.and_then(|v| v.enclosingScrollView());
            let mine = |e: &NSEvent| {
                // SAFETY: NSScrollView's own scrolling.
                let _: () = unsafe { msg_send![super(self), scrollWheel: e] };
            };
            let Some(page) = page else { return mine(event) };
            let (phase, momentum) = (event.phase(), event.momentumPhase());
            if phase.contains(NSEventPhase::MayBegin) {
                // Fingers down: whichever is coasting stops.
                mine(event);
                return page.scrollWheel(event);
            }
            // A new gesture, or a mouse wheel's click (no phases at all).
            if phase.contains(NSEventPhase::Began) || (phase.is_empty() && momentum.is_empty()) {
                self.ivars().axis.set(0);
            }
            if self.ivars().axis.get() == 0 {
                let (dx, dy) = (event.scrollingDeltaX().abs(), event.scrollingDeltaY().abs());
                if dx == 0.0 && dy == 0.0 {
                    mine(event);
                    return page.scrollWheel(event);
                }
                let more = self.documentView().is_some_and(|d| d.frame().size.width > self.contentView().bounds().size.width + 0.5);
                self.ivars().axis.set(if dx > dy && more { 1 } else { 2 });
            }
            if self.ivars().axis.get() == 1 { mine(event) } else { page.scrollWheel(event) }
        }
    }
);

impl ShelfScrollView {
    pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ShelfIvars { axis: Cell::new(0) });
        // SAFETY: NSScrollView's designated initializer.
        let scroll: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: NSRect::ZERO] };
        scroll.setHasHorizontalScroller(false);
        scroll.setHasVerticalScroller(false);
        scroll.setDrawsBackground(false);
        scroll.setVerticalScrollElasticity(NSScrollElasticity::None);
        scroll
    }
}
