//! Light and dark: `kilo_core::style`'s colors as `NSColor`s. Colors are
//! resolved for the app's current appearance when views are made; when the
//! appearance changes (the user's choice, or the system's at sunset), the
//! window's views are rebuilt (`app::appearance_changed`), so nothing has to
//! track it.

use std::cell::RefCell;
use std::rc::Rc;

use kilo_core::style::{self, DARK, LIGHT};
use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{NSAppearance, NSAppearanceNameAqua, NSAppearanceNameDarkAqua, NSApplication, NSColor};
use objc2_foundation::NSArray;

use crate::settings::Appearance;

/// The colors of `kilo_core::style::Palette`, for AppKit.
pub struct Palette {
    pub dark: bool,
    pub frame: Retained<NSColor>,
    pub panel: Retained<NSColor>,
    pub raised: Retained<NSColor>,
    pub selected: Retained<NSColor>,
    pub hover: Retained<NSColor>,
    pub pressed: Retained<NSColor>,
    pub placeholder: Retained<NSColor>,
    pub text: Retained<NSColor>,
    pub text_dim: Retained<NSColor>,
    pub primary: Retained<NSColor>,
    pub on_primary: Retained<NSColor>,
    pub track: Retained<NSColor>,
}

fn color(rgba: u32) -> Retained<NSColor> {
    let [r, g, b, a] = style::rgba(rgba);
    NSColor::colorWithSRGBRed_green_blue_alpha(r, g, b, a)
}

impl Palette {
    fn new(dark: bool) -> Self {
        let s = if dark { &DARK } else { &LIGHT };
        Palette {
            dark: s.dark,
            frame: color(s.frame),
            panel: color(s.panel),
            raised: color(s.raised),
            selected: color(s.selected),
            hover: color(s.hover),
            pressed: color(s.pressed),
            placeholder: color(s.placeholder),
            text: color(s.text),
            text_dim: color(s.text_dim),
            primary: color(s.primary),
            on_primary: color(s.on_primary),
            track: color(s.track),
        }
    }
}

thread_local! {
    static PALETTE: RefCell<Option<Rc<Palette>>> = const { RefCell::new(None) };
}

/// The colors for the current appearance.
pub fn palette() -> Rc<Palette> {
    PALETTE.with_borrow_mut(|p| p.get_or_insert_with(|| Rc::new(Palette::new(is_dark()))).clone())
}

/// Whether the app currently looks dark.
pub fn is_dark() -> bool {
    let mtm = MainThreadMarker::new().expect("main thread");
    let app = NSApplication::sharedApplication(mtm);
    // SAFETY: the appearance names are constant strings.
    let (aqua, dark) = unsafe { (NSAppearanceNameAqua, NSAppearanceNameDarkAqua) };
    let best = app.effectiveAppearance().bestMatchFromAppearancesWithNames(&NSArray::from_slice(&[aqua, dark]));
    best.is_some_and(|b| b.isEqualToString(dark))
}

/// Applies the user's appearance choice to the app. Returns whether the
/// app's look changed (then the window needs rebuilding).
pub fn apply(appearance: Appearance, mtm: MainThreadMarker) -> bool {
    let app = NSApplication::sharedApplication(mtm);
    // SAFETY: the appearance names are constant strings.
    let (aqua, dark) = unsafe { (NSAppearanceNameAqua, NSAppearanceNameDarkAqua) };
    match appearance {
        Appearance::System => app.setAppearance(None),
        Appearance::Light => app.setAppearance(NSAppearance::appearanceNamed(aqua).as_deref()),
        Appearance::Dark => app.setAppearance(NSAppearance::appearanceNamed(dark).as_deref()),
    }
    refresh()
}

/// Re-reads the app's look. Returns whether it changed since the palette
/// was last made.
pub fn refresh() -> bool {
    let dark = is_dark();
    PALETTE.with_borrow_mut(|p| {
        let changed = p.as_ref().is_none_or(|p| p.dark != dark);
        if changed {
            *p = Some(Rc::new(Palette::new(dark)));
        }
        changed
    })
}
