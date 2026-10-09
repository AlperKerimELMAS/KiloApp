//! Kilo's colors, light and dark. They're defined once (`DARK`, `LIGHT`:
//! `0xRRGGBBAA`, sRGB) and resolved into `NSColor`s for the app's current
//! appearance when views are made; when the appearance changes (the user's
//! choice, or the system's at sunset), the window's views are rebuilt
//! (`app::appearance_changed`), so nothing has to track it.
//!
//! One frame color behind the sidebar, the player bar and the gaps, with
//! pages on a slightly lighter rounded panel, as in Spotify: the parts read
//! as one window rather than as separate surfaces. Translucent colors
//! (hover, pressed, placeholder, track) work over the frame and the panel
//! alike.

use std::cell::RefCell;
use std::rc::Rc;

use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{NSAppearance, NSAppearanceNameAqua, NSAppearanceNameDarkAqua, NSApplication, NSColor};
use objc2_foundation::NSArray;

use crate::settings::Appearance;

/// One appearance's colors.
struct Colors {
    /// The window's frame: behind the sidebar, the player bar and the gaps.
    frame: u32,
    /// The page panel.
    panel: u32,
    /// Buttons and fields on the panel: back, search, secondary buttons.
    raised: u32,
    /// The selected sidebar section, on the frame.
    selected: u32,
    hover: u32,
    pressed: u32,
    /// Image slots before their image arrives.
    placeholder: u32,
    text: u32,
    text_dim: u32,
    /// The play button: white on dark, near-black on light.
    primary: u32,
    on_primary: u32,
    /// Slider tracks.
    track: u32,
}

const DARK: Colors = Colors {
    frame: 0x000000ff,
    panel: 0x121212ff,
    raised: 0x242424ff,
    selected: 0x242424ff,
    hover: 0xffffff12,
    pressed: 0xffffff24,
    placeholder: 0xffffff14,
    text: 0xffffffff,
    text_dim: 0xb3b3b3ff,
    primary: 0xffffffff,
    on_primary: 0x000000ff,
    track: 0xffffff4d,
};

const LIGHT: Colors = Colors {
    frame: 0xefefefff,
    panel: 0xffffffff,
    raised: 0xf0f0f0ff,
    selected: 0xdededeff,
    hover: 0x0000000d,
    pressed: 0x0000001a,
    placeholder: 0x0000000f,
    text: 0x111111ff,
    text_dim: 0x6a6a6aff,
    primary: 0x111111ff,
    on_primary: 0xffffffff,
    track: 0x00000026,
};

/// `color` as red, green, blue and alpha from 0 to 1.
fn rgba(color: u32) -> [f64; 4] {
    let c = |shift: u32| f64::from((color >> shift) & 0xff) / 255.0;
    [c(24), c(16), c(8), c(0)]
}

/// The current appearance's `Colors`, for AppKit.
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

fn color(value: u32) -> Retained<NSColor> {
    let [r, g, b, a] = rgba(value);
    NSColor::colorWithSRGBRed_green_blue_alpha(r, g, b, a)
}

impl Palette {
    fn new(dark: bool) -> Self {
        let s = if dark { &DARK } else { &LIGHT };
        Palette {
            dark,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_colors() {
        assert_eq!(rgba(0xff000080), [1.0, 0.0, 0.0, 128.0 / 255.0]);
    }
}
