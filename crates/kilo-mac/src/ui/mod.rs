//! Native AppKit building blocks: fonts and labels, image layers, round and
//! pill buttons, and layout helpers. The views Kilo defines are in `views`
//! (and the search field in `search`); `actions` is the target buttons and
//! menus call into.
//!
//! Colors come from `theme`. Images are drawn by handing a decoded IOSurface
//! straight to a layer (`layer.contents`, see `images`): no `NSImage`, no copy.

mod actions;
pub mod bar;
pub mod menus;
pub mod page;
mod search;
pub mod shell;
pub mod theme;
mod views;

pub use actions::actions;
pub use search::{SEARCH_HEIGHT, SearchField, search_field};
pub use views::{FlippedView, FocusSink, HOVER_PLAY, ItemView, RootView, ShelfScrollView, reset_hover_play};

use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Sel};
use objc2_app_kit::{
    NSButton, NSColor, NSFont, NSFontWeightBold, NSFontWeightMedium, NSFontWeightRegular, NSFontWeightSemibold, NSImage,
    NSImageSymbolConfiguration, NSLayoutConstraintOrientation, NSLineBreakMode, NSStackView, NSTextField, NSUserInterfaceLayoutOrientation,
    NSView,
};
use objc2_foundation::{NSArray, NSString};
use objc2_quartz_core::{CALayer, kCAGravityResizeAspectFill};

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
