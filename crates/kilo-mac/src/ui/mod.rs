//! Native AppKit building blocks: labels, image layers, round buttons, the
//! search field, clickable cards and rows, sideways shelves, and the action
//! target that buttons and menus call into.
//!
//! Colors come from `theme`. Images are drawn by handing a decoded IOSurface
//! straight to a layer (`layer.contents`, see `images`): no `NSImage`, no copy.

pub mod bar;
pub mod page;
pub mod shell;
pub mod theme;

use std::cell::{Cell, RefCell};

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject, Sel};
use objc2::{AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSButton, NSColor, NSControl, NSControlStateValueOff, NSControlStateValueOn, NSControlTextEditingDelegate, NSEvent, NSEventPhase,
    NSFocusRingType, NSFont, NSFontAttributeName, NSFontWeightBold, NSFontWeightMedium, NSFontWeightRegular, NSFontWeightSemibold,
    NSForegroundColorAttributeName, NSImage, NSImageSymbolConfiguration, NSImageView, NSLayoutConstraintOrientation, NSLineBreakMode,
    NSMenuItem, NSMenuItemValidation, NSResponder, NSScrollElasticity, NSScrollView, NSStackView, NSTextField, NSTextFieldDelegate,
    NSTextView, NSTrackingArea, NSTrackingAreaOptions, NSUserInterfaceLayoutOrientation, NSView,
};
use objc2_foundation::{NSArray, NSAttributedString, NSDictionary, NSNotification, NSPoint, NSRect, NSSize, NSString};
use objc2_quartz_core::{CALayer, kCAGravityResizeAspectFill};

use crate::app::{self, Route};
use crate::strings::{S, t};

#[derive(Clone, Copy)]
pub enum Weight {
    Regular,
    Medium,
    Semibold,
    Bold,
}

pub fn font(size: f64, weight: Weight) -> Retained<NSFont> {
    // SAFETY: the NSFontWeight* statics are plain doubles.
    let w = unsafe {
        match weight {
            Weight::Regular => NSFontWeightRegular,
            Weight::Medium => NSFontWeightMedium,
            Weight::Semibold => NSFontWeightSemibold,
            Weight::Bold => NSFontWeightBold,
        }
    };
    NSFont::systemFontOfSize_weight(size, w)
}

fn text_color(dim: bool) -> Retained<NSColor> {
    let p = theme::palette();
    if dim { p.text_dim.clone() } else { p.text.clone() }
}

/// A one-line label that truncates instead of growing its container.
pub fn label(s: &str, size: f64, weight: Weight, dim: bool, mtm: MainThreadMarker) -> Retained<NSTextField> {
    let l = frame_label(s, size, weight, dim, mtm);
    l.setContentCompressionResistancePriority_forOrientation(250.0, NSLayoutConstraintOrientation::Horizontal);
    l
}

/// A one-line label for frame-based layout: same look as `label`, but never
/// touches Auto Layout (so AppKit doesn't build constraints for it).
pub fn frame_label(s: &str, size: f64, weight: Weight, dim: bool, mtm: MainThreadMarker) -> Retained<NSTextField> {
    let l = NSTextField::labelWithString(&NSString::from_str(s), mtm);
    l.setFont(Some(&font(size, weight)));
    l.setTextColor(Some(&text_color(dim)));
    l.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
    l
}

/// A multi-line label, wrapping at `width` points and capped at `lines`
/// (the last one ends in "…" if there's more).
pub fn paragraph(s: &str, size: f64, dim: bool, lines: isize, width: f64, mtm: MainThreadMarker) -> Retained<NSTextField> {
    let l = NSTextField::wrappingLabelWithString(&NSString::from_str(s), mtm);
    l.setFont(Some(&font(size, Weight::Regular)));
    l.setTextColor(Some(&text_color(dim)));
    l.setMaximumNumberOfLines(lines);
    l.setLineBreakMode(NSLineBreakMode::ByWordWrapping);
    if let Some(cell) = l.cell() {
        cell.setTruncatesLastVisibleLine(true);
    }
    l.setPreferredMaxLayoutWidth(width);
    l.setContentCompressionResistancePriority_forOrientation(250.0, NSLayoutConstraintOrientation::Horizontal);
    l
}

/// A layer-hosting view that shows an image scaled to fill, with rounded
/// corners and a placeholder color until the image arrives. Fixed size via
/// Auto Layout (for stacks); see `image_layer_view` for frame layout.
pub fn image_view(width: f64, height: f64, radius: f64, mtm: MainThreadMarker) -> Retained<NSView> {
    let view = image_layer_view(radius, mtm);
    size(&view, Some(width), Some(height));
    view
}

/// Like `image_view`, sized by its frame.
pub fn image_layer_view(radius: f64, mtm: MainThreadMarker) -> Retained<NSView> {
    let view = NSView::new(mtm);
    let layer = CALayer::new();
    // SAFETY: kCAGravityResizeAspectFill is a constant NSString.
    unsafe { layer.setContentsGravity(kCAGravityResizeAspectFill) };
    layer.setCornerRadius(radius);
    layer.setMasksToBounds(true);
    layer.setBackgroundColor(Some(&theme::palette().placeholder.CGColor()));
    view.setLayer(Some(&layer));
    view.setWantsLayer(true);
    // AppKit sets the layer's own clipping from the view's, which is off by
    // default since macOS 14: then the corners stayed square and wide
    // images spilled out of square slots.
    view.setClipsToBounds(true);
    view
}

/// The pixel size to ask for so a thumbnail fills a square `side` points
/// across: a 16:9 one is cropped at the sides, so it must be wider.
pub fn square_px(side: f64, scale: f64, wide: bool) -> u32 {
    let px = side * scale;
    (if wide { px * 16.0 / 9.0 } else { px }).ceil() as u32
}

pub fn set_image(view: &NSView, image: Option<&crate::images::Image>) {
    if image.is_some() {
        crate::debug::trace_once("images: first shown");
    }
    if let Some(layer) = view.layer() {
        // SAFETY: an IOSurface is valid layer contents.
        unsafe { layer.setContents(image.map(crate::images::Image::contents)) };
    }
}

/// Gives `view` a layer filled with `color` (or nothing), with rounded
/// corners. Backgrounds are layer colors: nothing is drawn.
pub fn fill(view: &NSView, color: Option<&NSColor>, radius: f64) {
    view.setWantsLayer(true);
    if let Some(layer) = view.layer() {
        layer.setBackgroundColor(color.map(|c| c.CGColor()).as_deref());
        layer.setCornerRadius(radius);
    }
}

pub fn stack(views: &[&NSView], vertical: bool, spacing: f64, mtm: MainThreadMarker) -> Retained<NSStackView> {
    let s = NSStackView::stackViewWithViews(&NSArray::from_slice(views), mtm);
    s.setOrientation(if vertical { NSUserInterfaceLayoutOrientation::Vertical } else { NSUserInterfaceLayoutOrientation::Horizontal });
    s.setSpacing(spacing);
    s
}

pub fn size(view: &NSView, width: Option<f64>, height: Option<f64>) {
    view.setTranslatesAutoresizingMaskIntoConstraints(false);
    if let Some(w) = width {
        view.widthAnchor().constraintEqualToConstant(w).setActive(true);
    }
    if let Some(h) = height {
        view.heightAnchor().constraintEqualToConstant(h).setActive(true);
    }
}

/// Pins `child` to `parent`'s edges with the given insets.
pub fn pin(child: &NSView, parent: &NSView, top: f64, left: f64, bottom: f64, right: f64) {
    child.setTranslatesAutoresizingMaskIntoConstraints(false);
    child.topAnchor().constraintEqualToAnchor_constant(&parent.topAnchor(), top).setActive(true);
    child.leadingAnchor().constraintEqualToAnchor_constant(&parent.leadingAnchor(), left).setActive(true);
    child.bottomAnchor().constraintEqualToAnchor_constant(&parent.bottomAnchor(), -bottom).setActive(true);
    child.trailingAnchor().constraintEqualToAnchor_constant(&parent.trailingAnchor(), -right).setActive(true);
}

/// A borderless button showing an SF Symbol in the text color.
pub fn symbol_button(symbol: &str, action: Sel, point_size: f64, mtm: MainThreadMarker) -> Retained<NSButton> {
    let image = NSImage::imageWithSystemSymbolName_accessibilityDescription(&NSString::from_str(symbol), None).unwrap_or_default();
    let target: &AnyObject = actions(mtm);
    // SAFETY: the target outlives the button (it lives for the whole app).
    let button = unsafe { NSButton::buttonWithImage_target_action(&image, Some(target), Some(action), mtm) };
    button.setBordered(false);
    button.setContentTintColor(Some(&theme::palette().text));
    let config = NSImageSymbolConfiguration::configurationWithPointSize_weight(point_size, 0.0);
    button.setSymbolConfiguration(Some(&config));
    button
}

/// A round button: an SF Symbol on a filled circle `diameter` across, in
/// the primary color (the play button) or the raised one. Sized by the
/// caller (frame or constraints).
pub fn circle_button(
    symbol: &str,
    action: Sel,
    diameter: f64,
    point_size: f64,
    primary: bool,
    mtm: MainThreadMarker,
) -> Retained<NSButton> {
    let p = theme::palette();
    let button = symbol_button(symbol, action, point_size, mtm);
    button.setContentTintColor(Some(if primary { &p.on_primary } else { &p.text }));
    fill(&button, Some(if primary { &p.primary } else { &p.raised }), diameter / 2.0);
    button
}

/// A pill-shaped button with a title, `height` tall: `primary` is the
/// filled call to action, otherwise it's raised.
pub fn pill_button(title: &str, action: Sel, height: f64, primary: bool, mtm: MainThreadMarker) -> Retained<NSButton> {
    let p = theme::palette();
    let target: &AnyObject = actions(mtm);
    // SAFETY: as in `symbol_button`.
    let button = unsafe { NSButton::buttonWithTitle_target_action(&NSString::from_str(title), Some(target), Some(action), mtm) };
    button.setBordered(false);
    button.setFont(Some(&font(15.0, Weight::Semibold)));
    // Borderless buttons tint their title too.
    button.setContentTintColor(Some(if primary { &p.on_primary } else { &p.text }));
    fill(&button, Some(if primary { &p.primary } else { &p.raised }), height / 2.0);
    let width = button.intrinsicContentSize().width.ceil() + height;
    size(&button, Some(width), Some(height));
    button
}

pub fn set_symbol(button: &NSButton, symbol: &str) {
    if let Some(image) = NSImage::imageWithSystemSymbolName_accessibilityDescription(&NSString::from_str(symbol), None) {
        button.setImage(Some(&image));
    }
}

/// The search field's pill height.
pub const SEARCH_HEIGHT: f64 = 40.0;

/// The search field: a magnifier and a plain text field in a pill of Kilo's
/// own colors (not the system's bezel), ringed while it has the focus.
/// Returns the pill and the field.
pub fn search_field(mtm: MainThreadMarker) -> (Retained<SearchPill>, Retained<SearchField>) {
    let p = theme::palette();
    // SAFETY: plain NSTextField init.
    let field: Retained<SearchField> = unsafe { msg_send![SearchField::alloc(mtm), init] };
    field.setBezeled(false);
    field.setDrawsBackground(false);
    field.setFocusRingType(NSFocusRingType::None);
    field.setFont(Some(&font(14.0, Weight::Regular)));
    field.setTextColor(Some(&p.text));
    field.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
    if let Some(cell) = field.cell() {
        // One line that scrolls, sending its action only on Return.
        cell.setScrollable(true);
        cell.setSendsActionOnEndEditing(false);
    }
    // SAFETY: the attribute names are constant strings, with an NSColor and
    // an NSFont as their values.
    let placeholder = unsafe {
        let attributes = NSDictionary::<NSString, AnyObject>::from_slices(
            &[NSForegroundColorAttributeName, NSFontAttributeName],
            &[&*p.text_dim as &AnyObject, &*font(14.0, Weight::Regular)],
        );
        NSAttributedString::initWithString_attributes(
            NSAttributedString::alloc(),
            &NSString::from_str(t(S::SearchPlaceholder)),
            Some(&attributes),
        )
    };
    field.setPlaceholderAttributedString(Some(&placeholder));
    let target = actions(mtm);
    // SAFETY: the delegate lives for the whole app (the field only keeps a
    // weak reference).
    unsafe { field.setDelegate(Some(ProtocolObject::from_ref(target))) };
    let target: &AnyObject = target;
    // SAFETY: as in `symbol_button`.
    unsafe {
        field.setTarget(Some(target));
        field.setAction(Some(sel!(search:)));
    }
    let icon = NSImageView::new(mtm);
    if let Some(image) = NSImage::imageWithSystemSymbolName_accessibilityDescription(&NSString::from_str("magnifyingglass"), None) {
        icon.setImage(Some(&image));
    }
    icon.setSymbolConfiguration(Some(&NSImageSymbolConfiguration::configurationWithPointSize_weight(15.0, 0.0)));
    icon.setContentTintColor(Some(&p.text_dim));
    // SAFETY: plain NSView init.
    let pill: Retained<SearchPill> = unsafe { msg_send![SearchPill::alloc(mtm), init] };
    fill(&pill, Some(&p.raised), SEARCH_HEIGHT / 2.0);
    pill.addSubview(&icon);
    pill.addSubview(&field);
    for v in [&*icon as &NSView, &field] {
        v.setTranslatesAutoresizingMaskIntoConstraints(false);
        v.centerYAnchor().constraintEqualToAnchor(&pill.centerYAnchor()).setActive(true);
    }
    icon.leadingAnchor().constraintEqualToAnchor_constant(&pill.leadingAnchor(), 14.0).setActive(true);
    field.leadingAnchor().constraintEqualToAnchor_constant(&icon.trailingAnchor(), 8.0).setActive(true);
    field.trailingAnchor().constraintEqualToAnchor_constant(&pill.trailingAnchor(), -16.0).setActive(true);
    (pill, field)
}

// The search field's pill: a click anywhere in it (the magnifier, the
// padding) goes to the field.
define_class!(
    // SAFETY: NSView subclass with no ivars; overrides match AppKit's
    // signatures.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "KiloSearchPill"]
    pub struct SearchPill;

    impl SearchPill {
        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, _event: &NSEvent) {
            app::focus_search();
        }
    }
);

// The search field: rings its pill while it has the focus.
define_class!(
    // SAFETY: NSTextField subclass with no ivars; the override calls the
    // superclass and matches AppKit's signature.
    #[unsafe(super(NSTextField, NSControl, NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "KiloSearchField"]
    pub struct SearchField;

    impl SearchField {
        #[unsafe(method(becomeFirstResponder))]
        fn become_first_responder(&self) -> bool {
            // SAFETY: NSTextField's implementation.
            let focused: bool = unsafe { msg_send![super(self), becomeFirstResponder] };
            if focused {
                set_ring(self, true);
            }
            focused
        }
    }
);

/// Rings (or not) the pill around the search field.
fn set_ring(field: &NSView, on: bool) {
    // SAFETY: main-thread AppKit call on a live view.
    let pill = unsafe { field.superview() };
    if let Some(layer) = pill.and_then(|p| p.layer()) {
        layer.setBorderWidth(if on { 1.5 } else { 0.0 });
        layer.setBorderColor(on.then(|| theme::palette().text.CGColor()).as_deref());
    }
}

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
        view
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

// The target every button and menu item sends its action to, and the
// search field's delegate.
define_class!(
    // SAFETY: NSObject subclass with no ivars; action methods take a sender;
    // protocol methods match AppKit's signatures.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "KiloActions"]
    pub struct Actions;

    impl Actions {
        #[unsafe(method(home:))]
        fn home(&self, _sender: Option<&AnyObject>) {
            app::go(Route::Home);
        }

        #[unsafe(method(explore:))]
        fn explore(&self, _sender: Option<&AnyObject>) {
            app::go(Route::Explore);
        }

        #[unsafe(method(library:))]
        fn library(&self, _sender: Option<&AnyObject>) {
            app::go(Route::Library);
        }

        #[unsafe(method(back:))]
        fn back(&self, _sender: Option<&AnyObject>) {
            app::back();
        }

        #[unsafe(method(search:))]
        fn search(&self, sender: Option<&AnyObject>) {
            if let Some(field) = sender.and_then(|s| s.downcast_ref::<NSTextField>()) {
                let query = field.stringValue().to_string();
                if !query.trim().is_empty() {
                    app::go(Route::Search(query.trim().to_owned()));
                    app::release_search_focus();
                }
            }
        }

        #[unsafe(method(focusSearch:))]
        fn focus_search(&self, _sender: Option<&AnyObject>) {
            app::focus_search();
        }

        #[unsafe(method(playPause:))]
        fn play_pause(&self, _sender: Option<&AnyObject>) {
            app::play_pause();
        }

        #[unsafe(method(next:))]
        fn next(&self, _sender: Option<&AnyObject>) {
            app::next();
        }

        #[unsafe(method(previous:))]
        fn previous(&self, _sender: Option<&AnyObject>) {
            app::previous();
        }

        #[unsafe(method(playAll:))]
        fn play_all(&self, _sender: Option<&AnyObject>) {
            app::play_all(false);
        }

        #[unsafe(method(shuffleAll:))]
        fn shuffle_all(&self, _sender: Option<&AnyObject>) {
            app::play_all(true);
        }

        #[unsafe(method(signIn:))]
        fn sign_in(&self, _sender: Option<&AnyObject>) {
            app::sign_in();
        }

        #[unsafe(method(signOut:))]
        fn sign_out(&self, _sender: Option<&AnyObject>) {
            app::sign_out();
        }

        #[unsafe(method(retry:))]
        fn retry(&self, _sender: Option<&AnyObject>) {
            app::reload();
        }

        #[unsafe(method(cancelSignIn:))]
        fn cancel_sign_in(&self, _sender: Option<&AnyObject>) {
            app::cancel_sign_in();
        }

        #[unsafe(method(playItem:))]
        fn play_item(&self, sender: Option<&AnyObject>) {
            if let Some(button) = sender.and_then(|s| s.downcast_ref::<NSButton>()) {
                let item = button.tag() as u32;
                crate::net::later(move || app::play_item(item));
            }
        }

        #[unsafe(method(account:))]
        fn account(&self, _sender: Option<&AnyObject>) {
            // The menu runs its own event loop: outside this action.
            crate::net::later(app::show_account_menu);
        }

        #[unsafe(method(setAppearance:))]
        fn set_appearance(&self, sender: Option<&AnyObject>) {
            if let Some(item) = sender.and_then(|s| s.downcast_ref::<NSMenuItem>()) {
                let tag = item.tag();
                crate::net::later(move || app::set_appearance(tag));
            }
        }

        #[unsafe(method(setLanguage:))]
        fn set_language(&self, sender: Option<&AnyObject>) {
            if let Some(item) = sender.and_then(|s| s.downcast_ref::<NSMenuItem>()) {
                let tag = item.tag();
                crate::net::later(move || app::set_language(tag));
            }
        }

        #[unsafe(method(mute:))]
        fn mute(&self, _sender: Option<&AnyObject>) {
            app::toggle_mute();
        }

        #[unsafe(method(toggleSidebar:))]
        fn toggle_sidebar(&self, _sender: Option<&AnyObject>) {
            app::toggle_sidebar();
        }

        /// The sidebar animation's display link.
        #[unsafe(method(sidebarFrame:))]
        fn sidebar_frame(&self, _link: Option<&AnyObject>) {
            app::sidebar_frame();
        }

        /// A menu item for a keyboard shortcut; its tag is the action.
        #[unsafe(method(shortcut:))]
        fn shortcut(&self, sender: Option<&AnyObject>) {
            if let Some(action) = sender.and_then(|s| s.downcast_ref::<NSMenuItem>()).and_then(|i| crate::keys::action(i.tag())) {
                crate::keys::perform(action);
            }
        }
    }

    unsafe impl NSObjectProtocol for Actions {}

    unsafe impl NSMenuItemValidation for Actions {
        #[unsafe(method(validateMenuItem:))]
        fn validate_menu_item(&self, item: &NSMenuItem) -> bool {
            // (No early returns: `define_class!` converts only the last
            // expression to Objective-C's BOOL.)
            let action = (item.action() == Some(sel!(shortcut:))).then(|| crate::keys::action(item.tag())).flatten();
            if action == Some(kilo_core::shortcuts::Action::Mute) {
                item.setState(if app::is_muted() { NSControlStateValueOn } else { NSControlStateValueOff });
            }
            if action == Some(kilo_core::shortcuts::Action::ToggleSidebar) {
                let title = if crate::settings::sidebar_collapsed() { S::ExpandSidebar } else { S::CollapseSidebar };
                item.setTitle(&NSString::from_str(t(title)));
            }
            // A key typed into the search field: the arrows and plain keys
            // are for the text, so the item lets them through.
            let mtm = MainThreadMarker::new().expect("main thread");
            action.is_none()
                || !crate::keys::typing_key(mtm)
                || action.and_then(kilo_core::shortcuts::shown).is_some_and(kilo_core::shortcuts::Shortcut::while_typing)
        }
    }

    unsafe impl NSControlTextEditingDelegate for Actions {
        #[unsafe(method(control:textView:doCommandBySelector:))]
        fn do_command(&self, control: &NSControl, _view: &NSTextView, command: Sel) -> bool {
            // Escape clears the search field, then leaves it.
            let escape = command == sel!(cancelOperation:);
            if escape && control.stringValue().length() == 0 {
                crate::net::later(app::release_search_focus);
            } else if escape {
                control.setStringValue(&NSString::from_str(""));
            }
            escape
        }

        #[unsafe(method(controlTextDidEndEditing:))]
        fn did_end_editing(&self, notification: &NSNotification) {
            if let Some(field) = notification.object().and_then(|o| o.downcast::<NSView>().ok()) {
                set_ring(&field, false);
            }
        }
    }

    unsafe impl NSTextFieldDelegate for Actions {}
);

thread_local! {
    static ACTIONS: std::cell::OnceCell<Retained<Actions>> = const { std::cell::OnceCell::new() };
}

/// The app-wide action target (lives as long as the app).
pub fn actions(mtm: MainThreadMarker) -> &'static Actions {
    ACTIONS.with(|a| {
        let actions = a.get_or_init(|| {
            // SAFETY: plain NSObject init.
            unsafe { msg_send![Actions::alloc(mtm), init] }
        });
        // SAFETY: the Retained lives in a thread-local for the rest of the
        // process, on the main thread only.
        unsafe { &*Retained::as_ptr(actions) }
    })
}
