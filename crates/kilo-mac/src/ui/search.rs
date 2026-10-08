//! The search field: a plain text field in a pill of Kilo's own colors,
//! ringed while it has the focus.

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, ProtocolObject};
use objc2::{AnyThread, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSControl, NSEvent, NSFocusRingType, NSFontAttributeName, NSForegroundColorAttributeName, NSImage, NSImageSymbolConfiguration,
    NSImageView, NSLineBreakMode, NSResponder, NSTextField, NSView,
};
use objc2_foundation::{NSAttributedString, NSDictionary, NSString};

use super::{Weight, actions, fill, font, theme};
use crate::app;
use crate::strings::{S, t};

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
pub(super) fn set_ring(field: &NSView, on: bool) {
    // SAFETY: main-thread AppKit call on a live view.
    let pill = unsafe { field.superview() };
    if let Some(layer) = pill.and_then(|p| p.layer()) {
        layer.setBorderWidth(if on { 1.5 } else { 0.0 });
        layer.setBorderColor(on.then(|| theme::palette().text.CGColor()).as_deref());
    }
}
