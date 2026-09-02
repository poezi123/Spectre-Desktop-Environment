//! The Spectre palette.
//!
//! The constants below were sampled straight out of the concept renders
//! (`Fensterconcept.png`, `Taskleiste Concept.png`), so the shipped desktop and
//! the design mockups agree on what "Spectre black" actually is.
//!
//! Rule of thumb from the project principles: black first, RGB second. Only
//! [`Palette::accent`] carries hue; everything structural is a near-black grey.

use serde::{Deserialize, Serialize};

use crate::color::{Color, Gradient};

/// Desktop backdrop. The darkest surface in the system.
pub const BASE: Color = Color::hex(0x020204);
/// Window bodies, panel background, menus.
pub const SURFACE: Color = Color::hex(0x0a0b0d);
/// Title bars, headers, side bars — one step up from [`SURFACE`].
pub const ELEVATED: Color = Color::hex(0x101115);
/// Hovered rows, pressed buttons, tray hover.
pub const OVERLAY: Color = Color::hex(0x16171c);
/// Hairlines between regions.
pub const LINE: Color = Color::hex(0x1c1d23);
/// Border of an unfocused window.
pub const BORDER: Color = Color::hex(0x24252c);
/// Border of the focused window. A lifted neutral rather than an accent: the
/// colour in Spectre lives in the pattern, not in a ring around every window.
pub const BORDER_FOCUS: Color = Color::hex(0x3b3d47);

/// Primary text.
pub const TEXT: Color = Color::hex(0xe6e6ec);
/// Secondary labels, inactive tabs, clock date line.
pub const TEXT_DIM: Color = Color::hex(0x8a8a96);
/// Disabled text and the unfocused title bar caption.
pub const TEXT_MUTED: Color = Color::hex(0x565662);

/// Accent gradient stops, teal to purple, as sampled from the settings mockup.
pub const ACCENT_0: Color = Color::hex(0x16a3c8);
pub const ACCENT_1: Color = Color::hex(0x4376c6);
pub const ACCENT_2: Color = Color::hex(0x7645ab);
pub const ACCENT_3: Color = Color::hex(0xb064dc);

/// Destructive action: the close button and the "Delete" dialog button.
pub const DANGER: Color = Color::hex(0xd04a57);
pub const DANGER_HOVER: Color = Color::hex(0xe46c6c);
pub const WARNING: Color = Color::hex(0xd8a03c);
pub const SUCCESS: Color = Color::hex(0x3fb98a);

/// The full colour set for one Spectre look.
///
/// A theme file overrides any subset of these; everything omitted falls back to
/// the constants above via [`Default`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct Palette {
    pub base: Color,
    pub surface: Color,
    pub elevated: Color,
    pub overlay: Color,
    pub line: Color,
    pub border: Color,
    pub border_focus: Color,

    pub text: Color,
    pub text_dim: Color,
    pub text_muted: Color,

    /// Drives focused borders, the active-workspace pip and the panel underline.
    pub accent: Gradient,

    pub danger: Color,
    pub danger_hover: Color,
    pub warning: Color,
    pub success: Color,
}

impl Default for Palette {
    fn default() -> Self {
        Self {
            base: BASE,
            surface: SURFACE,
            elevated: ELEVATED,
            overlay: OVERLAY,
            line: LINE,
            border: BORDER,
            border_focus: BORDER_FOCUS,
            text: TEXT,
            text_dim: TEXT_DIM,
            text_muted: TEXT_MUTED,
            accent: Gradient::new(vec![ACCENT_0, ACCENT_1, ACCENT_2, ACCENT_3]),
            danger: DANGER,
            danger_hover: DANGER_HOVER,
            warning: WARNING,
            success: SUCCESS,
        }
    }
}

impl Palette {
    /// Border colour for a window in the given focus state.
    ///
    /// Both are neutral greys. An accent-coloured ring around every window
    /// turned the desktop into a light show; the accent now lives in the
    /// pattern inside the title bar, where it reads as material rather than as
    /// an outline.
    pub fn window_border(&self, focused: bool) -> Color {
        if focused {
            self.border_focus
        } else {
            self.border
        }
    }

    /// The glow laid over a focused window edge. Intensity is a 0..1 knob from
    /// the `rgb_glow` setting; `0.0` disables the glow entirely.
    pub fn accent_glow(&self, t: f32, intensity: f32) -> Color {
        self.accent.sample(t).alpha(intensity.clamp(0.0, 1.0) * 0.55)
    }

    /// Title bar background. Focused windows sit one step brighter so the active
    /// window is readable even with every effect switched off.
    pub fn titlebar(&self, focused: bool) -> Color {
        if focused {
            self.elevated
        } else {
            self.surface
        }
    }

    pub fn titlebar_text(&self, focused: bool) -> Color {
        if focused {
            self.text
        } else {
            self.text_muted
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_palette_round_trips_through_toml() {
        let p = Palette::default();
        let s = toml_string(&p);
        let back: Palette = toml::from_str(&s).unwrap();
        assert_eq!(p, back);
    }

    #[test]
    fn partial_theme_file_keeps_defaults() {
        let p: Palette = toml::from_str(r##"accent = { stops = ["#ff0000"] }"##).unwrap();
        assert_eq!(p.base, BASE, "unspecified keys must fall back");
        assert_eq!(p.accent.sample(0.5), Color::hex(0xff0000));
    }

    #[test]
    fn window_borders_are_neutral_and_tell_focus_apart() {
        let p = Palette::default();
        assert_eq!(p.window_border(false), p.border);
        assert_eq!(p.window_border(true), p.border_focus);
        assert_ne!(p.window_border(true), p.window_border(false));
    }

    #[test]
    fn window_borders_stay_grey_rather_than_taking_the_accent() {
        // A cool tint is wanted - the whole palette leans blue - but the border
        // must not become a coloured ring. Anything under a tenth of the range
        // reads as grey next to an accent that spans most of it.
        let p = Palette::default();
        let spread = |c: Color| (c.r - c.g).abs() + (c.g - c.b).abs() + (c.r - c.b).abs();
        for focused in [true, false] {
            let c = p.window_border(focused);
            assert!(spread(c) < 0.12, "the border must read as grey, got {c}");
        }
        assert!(
            spread(p.accent.sample(0.0)) > 0.5,
            "the accent, by contrast, is unmistakably coloured"
        );
    }

    #[test]
    fn the_focused_border_is_the_brighter_one() {
        let p = Palette::default();
        let sum = |c: Color| c.r + c.g + c.b;
        assert!(sum(p.window_border(true)) > sum(p.window_border(false)));
    }

    #[test]
    fn zero_intensity_glow_is_invisible() {
        assert_eq!(Palette::default().accent_glow(0.5, 0.0).a, 0.0);
    }

    fn toml_string(p: &Palette) -> String {
        toml::to_string(p).unwrap()
    }
}
