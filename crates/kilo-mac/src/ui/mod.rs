//! Native AppKit building blocks: colors, labels, image layers, clickable
//! items, and the action target that buttons call into.
//!
//! Images are drawn by handing a decoded IOSurface straight to a layer
//! (`layer.contents`, see `images`): no `NSImage`, no copy.

pub mod bar;
pub mod page;
pub mod shell;

use std::cell::Cell;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol};
use objc2::{AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSButton, NSColor, NSEvent, NSFont, NSFontWeightBold, NSFontWeightMedium, NSFontWeightRegular, NSImage, NSImageSymbolConfiguration,
    NSLayoutConstraintOrientation, NSLineBreakMode, NSSearchField, NSStackView, NSTextField, NSTrackingArea, NSTrackingAreaOptions,
    NSUserInterfaceLayoutOrientation, NSView,
};
use objc2_foundation::{NSArray, NSRect, NSString};
use objc2_quartz_core::{CALayer, kCAGravityResizeAspectFill};

use crate::app::{self, Route};

// YouTube Music's dark palette.
pub fn bg() -> Retained<NSColor> {
    rgb(0x03, 0x03, 0x03)
}
pub fn sidebar_bg() -> Retained<NSColor> {
    rgb(0x00, 0x00, 0x00)
}
pub fn bar_bg() -> Retained<NSColor> {
    rgb(0x21, 0x21, 0x21)
}
pub fn text() -> Retained<NSColor> {
    rgb(0xff, 0xff, 0xff)
}
pub fn text_dim() -> Retained<NSColor> {
    NSColor::colorWithSRGBRed_green_blue_alpha(1.0, 1.0, 1.0, 0.64)
}
pub fn hover() -> Retained<NSColor> {
    NSColor::colorWithSRGBRed_green_blue_alpha(1.0, 1.0, 1.0, 0.10)
}
pub fn placeholder() -> Retained<NSColor> {
    rgb(0x1f, 0x1f, 0x1f)
}
pub fn accent() -> Retained<NSColor> {
    rgb(0xff, 0x00, 0x33)
}

fn rgb(r: u8, g: u8, b: u8) -> Retained<NSColor> {
    NSColor::colorWithSRGBRed_green_blue_alpha(f64::from(r) / 255.0, f64::from(g) / 255.0, f64::from(b) / 255.0, 1.0)
}

#[derive(Clone, Copy)]
pub enum Weight {
    Regular,
    Medium,
    Bold,
}

pub fn font(size: f64, weight: Weight) -> Retained<NSFont> {
    // SAFETY: the NSFontWeight* statics are plain doubles.
    let w = unsafe {
        match weight {
            Weight::Regular => NSFontWeightRegular,
            Weight::Medium => NSFontWeightMedium,
            Weight::Bold => NSFontWeightBold,
        }
    };
    NSFont::systemFontOfSize_weight(size, w)
}

/// A one-line label that truncates instead of growing its container.
pub fn label(s: &str, size: f64, weight: Weight, dim: bool, mtm: MainThreadMarker) -> Retained<NSTextField> {
    let l = NSTextField::labelWithString(&NSString::from_str(s), mtm);
    l.setFont(Some(&font(size, weight)));
    let color = if dim { text_dim() } else { text() };
    l.setTextColor(Some(&color));
    l.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
    l.setContentCompressionResistancePriority_forOrientation(250.0, NSLayoutConstraintOrientation::Horizontal);
    l
}

/// A one-line label for frame-based layout: same look as `label`, but never
/// touches Auto Layout (so AppKit doesn't build constraints for it).
pub fn frame_label(s: &str, size: f64, weight: Weight, dim: bool, mtm: MainThreadMarker) -> Retained<NSTextField> {
    let l = NSTextField::labelWithString(&NSString::from_str(s), mtm);
    l.setFont(Some(&font(size, weight)));
    let color = if dim { text_dim() } else { text() };
    l.setTextColor(Some(&color));
    l.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
    l
}

/// A multi-line label, capped at `lines`.
pub fn paragraph(s: &str, size: f64, dim: bool, lines: isize, mtm: MainThreadMarker) -> Retained<NSTextField> {
    let l = NSTextField::wrappingLabelWithString(&NSString::from_str(s), mtm);
    l.setFont(Some(&font(size, Weight::Regular)));
    let color = if dim { text_dim() } else { text() };
    l.setTextColor(Some(&color));
    l.setMaximumNumberOfLines(lines);
    l.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
    l.setContentCompressionResistancePriority_forOrientation(250.0, NSLayoutConstraintOrientation::Horizontal);
    l
}

/// A layer-hosting view that shows a `CGImage` scaled to fill, with rounded
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
    layer.setBackgroundColor(Some(&placeholder().CGColor()));
    view.setLayer(Some(&layer));
    view.setWantsLayer(true);
    view
}

pub fn set_image(view: &NSView, image: Option<&crate::images::Image>) {
    if let Some(layer) = view.layer() {
        // SAFETY: an IOSurface is valid layer contents.
        unsafe { layer.setContents(image.map(crate::images::Image::contents)) };
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

pub fn symbol_button(symbol: &str, action: objc2::runtime::Sel, point_size: f64, mtm: MainThreadMarker) -> Retained<NSButton> {
    let image = NSImage::imageWithSystemSymbolName_accessibilityDescription(&NSString::from_str(symbol), None).unwrap_or_default();
    let target: &AnyObject = actions(mtm);
    // SAFETY: the target outlives the button (it lives for the whole app).
    let button = unsafe { NSButton::buttonWithImage_target_action(&image, Some(target), Some(action), mtm) };
    button.setBordered(false);
    button.setContentTintColor(Some(&text()));
    let config = NSImageSymbolConfiguration::configurationWithPointSize_weight(point_size, 0.0);
    button.setSymbolConfiguration(Some(&config));
    button
}

pub fn set_symbol(button: &NSButton, symbol: &str) {
    if let Some(image) = NSImage::imageWithSystemSymbolName_accessibilityDescription(&NSString::from_str(symbol), None) {
        button.setImage(Some(&image));
    }
}

pub fn text_button(title: &str, action: objc2::runtime::Sel, mtm: MainThreadMarker) -> Retained<NSButton> {
    let target: &AnyObject = actions(mtm);
    // SAFETY: as in `symbol_button`.
    unsafe { NSButton::buttonWithTitle_target_action(&NSString::from_str(title), Some(target), Some(action), mtm) }
}

pub fn search_field(mtm: MainThreadMarker) -> Retained<NSSearchField> {
    let field = NSSearchField::new(mtm);
    field.setPlaceholderString(Some(&NSString::from_str("Search songs, albums, artists, podcasts")));
    field.setSendsWholeSearchString(true);
    field.setSendsSearchStringImmediately(false);
    let target: &AnyObject = actions(mtm);
    // SAFETY: as in `symbol_button`.
    unsafe {
        field.setTarget(Some(target));
        field.setAction(Some(sel!(search:)));
    }
    field
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

// Takes keyboard focus when nothing else should have it. Without it, AppKit
// focuses the search field whenever the window becomes key, and a focused
// text field starts macOS's AutoFill service (~11 MB) for no reason.
define_class!(
    // SAFETY: NSView subclass with no ivars; only accepts first responder.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "KiloFocusSink"]
    pub struct FocusSink;

    impl FocusSink {
        #[unsafe(method(acceptsFirstResponder))]
        fn accepts_first_responder(&self) -> bool {
            true
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
}

// A clickable card or row. Clicking it activates item `item` of the current
// page; hovering highlights it.
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
        fn mouse_down(&self, _event: &NSEvent) {}

        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, event: &NSEvent) {
            let point = self.convertPoint_fromView(event.locationInWindow(), None);
            if self.mouse_inRect(point, self.bounds()) {
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
            if let Some(layer) = self.layer() {
                layer.setBackgroundColor(Some(&hover().CGColor()));
            }
        }

        #[unsafe(method(mouseExited:))]
        fn mouse_exited(&self, _event: &NSEvent) {
            if let Some(layer) = self.layer() {
                layer.setBackgroundColor(None);
            }
        }
    }

    unsafe impl NSObjectProtocol for ItemView {}
);

impl ItemView {
    pub fn new(item: u32, mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ItemIvars { item: Cell::new(item) });
        // SAFETY: NSView's designated initializer.
        let view: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: NSRect::ZERO] };
        view.setWantsLayer(true);
        if let Some(layer) = view.layer() {
            layer.setCornerRadius(6.0);
        }
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
}

// The target every button and menu item sends its action to.
define_class!(
    // SAFETY: NSObject subclass with no ivars; action methods take a sender.
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
            if let Some(field) = sender.and_then(|s| s.downcast_ref::<NSSearchField>()) {
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
    }

    unsafe impl NSObjectProtocol for Actions {}
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
