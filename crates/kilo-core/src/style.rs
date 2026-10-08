//! Kilo's colors, for every front end: `0xRRGGBBAA`, sRGB.
//!
//! One frame color behind the sidebar, the player bar and the gaps, with
//! pages on a slightly lighter rounded panel, as in Spotify: the parts read
//! as one window rather than as separate surfaces. Translucent colors
//! (hover, pressed, placeholder, track) work over the frame and the panel
//! alike.

pub struct Palette {
    pub dark: bool,
    /// The window's frame: behind the sidebar, the player bar and the gaps.
    pub frame: u32,
    /// The page panel.
    pub panel: u32,
    /// Buttons and fields on the panel: back, search, secondary buttons.
    pub raised: u32,
    /// The selected sidebar section, on the frame.
    pub selected: u32,
    pub hover: u32,
    pub pressed: u32,
    /// Image slots before their image arrives.
    pub placeholder: u32,
    pub text: u32,
    pub text_dim: u32,
    /// The play button: white on dark, near-black on light.
    pub primary: u32,
    pub on_primary: u32,
    /// Slider tracks.
    pub track: u32,
}

pub const DARK: Palette = Palette {
    dark: true,
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

pub const LIGHT: Palette = Palette {
    dark: false,
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
pub fn rgba(color: u32) -> [f64; 4] {
    let c = |shift: u32| f64::from((color >> shift) & 0xff) / 255.0;
    [c(24), c(16), c(8), c(0)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_colors() {
        assert_eq!(rgba(0xff000080), [1.0, 0.0, 0.0, 128.0 / 255.0]);
    }
}
