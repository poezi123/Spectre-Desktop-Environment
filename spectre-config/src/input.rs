//! Keyboard, pointer and touchpad settings.
//!
//! These map onto libinput device options and the xkb rule set. Defaults follow
//! the freedesktop defaults so an empty config behaves like every other desktop.

use serde::{Deserialize, Serialize};
use spectre_theme::Color;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct Keyboard {
    /// xkb layout, e.g. `"de"` or `"us,de"`.
    pub layout: String,
    pub variant: String,
    pub model: String,
    /// xkb options, e.g. `"grp:alt_shift_toggle,caps:escape"`.
    pub options: String,
    /// Milliseconds held before a key starts repeating.
    pub repeat_delay: u32,
    /// Repeats per second once repeating has started.
    pub repeat_rate: u32,
}

impl Default for Keyboard {
    fn default() -> Self {
        Self {
            layout: String::new(),
            variant: String::new(),
            model: String::new(),
            options: String::new(),
            repeat_delay: 400,
            repeat_rate: 30,
        }
    }
}

impl Keyboard {
    /// libinput/xkb wants an unset field as `None`, not as an empty string.
    pub fn xkb_field(value: &str) -> Option<&str> {
        (!value.trim().is_empty()).then_some(value)
    }

    /// Repeat rate clamped into the range Wayland clients can represent.
    pub fn sane_repeat(&self) -> (u32, u32) {
        (self.repeat_delay.clamp(100, 2000), self.repeat_rate.clamp(1, 100))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum AccelProfile {
    Flat,
    #[default]
    Adaptive,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct Pointer {
    /// libinput acceleration, -1.0..=1.0.
    pub accel_speed: f64,
    pub accel_profile: AccelProfile,
    pub natural_scroll: bool,
    pub left_handed: bool,
    /// Touchpad tap-to-click.
    pub tap_to_click: bool,
    /// Touchpad two-finger tap emits a right click.
    pub tap_and_drag: bool,
    pub disable_while_typing: bool,
    /// Pointer focus follows the mouse without a click.
    pub focus_follows_mouse: bool,
}

impl Default for Pointer {
    fn default() -> Self {
        Self {
            accel_speed: 0.0,
            accel_profile: AccelProfile::Adaptive,
            natural_scroll: false,
            left_handed: false,
            tap_to_click: true,
            tap_and_drag: true,
            disable_while_typing: true,
            focus_follows_mouse: false,
        }
    }
}

impl Pointer {
    /// Acceleration clamped to what libinput accepts.
    pub fn sane_accel(&self) -> f64 {
        self.accel_speed.clamp(-1.0, 1.0)
    }
}

/// The pointer Spectre draws when no client has asked for one of its own.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct Cursor {
    /// Height of the arrow in logical pixels.
    pub size: u32,
    /// Fill colour. Unset takes the theme's text colour, which is what makes
    /// the pointer legible over Spectre's own dark surfaces.
    pub fill: Option<Color>,
    /// Outline colour. Unset takes the theme's background.
    pub outline: Option<Color>,
}

impl Default for Cursor {
    fn default() -> Self {
        Self { size: 24, fill: None, outline: None }
    }
}

impl Cursor {
    /// Below the minimum the arrow is not a shape any more; above the maximum
    /// it is a graphic rather than a pointer.
    pub const MIN_SIZE: u32 = 8;
    pub const MAX_SIZE: u32 = 96;

    /// Height in device pixels on an output of this scale.
    pub fn height(&self, scale: f64) -> i32 {
        let size = self.size.clamp(Self::MIN_SIZE, Self::MAX_SIZE) as f64;
        let scale = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
        ((size * scale).round() as i32).max(Self::MIN_SIZE as i32)
    }

    /// The two colours to draw with, falling back to `fill` and `outline` when
    /// the config leaves them out.
    pub fn colors(&self, fill: Color, outline: Color) -> (Color, Color) {
        (self.fill.unwrap_or(fill), self.outline.unwrap_or(outline))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct Input {
    pub keyboard: Keyboard,
    pub pointer: Pointer,
    pub cursor: Cursor,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cursor_is_smaller_than_the_arrow_spectre_used_to_draw() {
        assert_eq!(Cursor::default().height(1.0), 24);
    }

    #[test]
    fn the_cursor_grows_with_the_output_scale() {
        assert_eq!(Cursor::default().height(2.0), 48);
        assert_eq!(Cursor { size: 30, ..Default::default() }.height(1.5), 45);
    }

    #[test]
    fn an_unusable_cursor_size_is_clamped_rather_than_obeyed() {
        assert_eq!(Cursor { size: 0, ..Default::default() }.height(1.0), Cursor::MIN_SIZE as i32);
        assert_eq!(
            Cursor { size: 4000, ..Default::default() }.height(1.0),
            Cursor::MAX_SIZE as i32
        );
    }

    #[test]
    fn a_nonsense_scale_leaves_the_size_alone() {
        for scale in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert_eq!(Cursor::default().height(scale), 24, "scale {scale}");
        }
    }

    #[test]
    fn colours_fall_back_to_the_theme_and_the_config_wins() {
        let theme = (Color::hex(0xffffff), Color::hex(0x000000));
        assert_eq!(Cursor::default().colors(theme.0, theme.1), theme);
        let cursor = Cursor { fill: Some(Color::hex(0xff0000)), ..Default::default() };
        assert_eq!(cursor.colors(theme.0, theme.1), (Color::hex(0xff0000), theme.1));
    }

    #[test]
    fn a_config_without_a_cursor_section_still_parses() {
        let input: Input = toml::from_str("[pointer]\naccel-speed = 0.5\n").unwrap();
        assert_eq!(input.cursor, Cursor::default());
    }

    #[test]
    fn the_cursor_reads_its_colours_as_hex() {
        let input: Input =
            toml::from_str("[cursor]\nsize = 16\nfill = \"#101010\"\n").unwrap();
        assert_eq!(input.cursor.size, 16);
        assert_eq!(input.cursor.fill, Some(Color::hex(0x101010)));
        assert_eq!(input.cursor.outline, None);
    }

    #[test]
    fn empty_xkb_fields_become_none() {
        assert_eq!(Keyboard::xkb_field(""), None);
        assert_eq!(Keyboard::xkb_field("   "), None);
        assert_eq!(Keyboard::xkb_field("de"), Some("de"));
    }

    #[test]
    fn absurd_repeat_values_are_clamped() {
        let k = Keyboard { repeat_delay: 0, repeat_rate: 100_000, ..Default::default() };
        assert_eq!(k.sane_repeat(), (100, 100));
    }

    #[test]
    fn accel_is_clamped_to_the_libinput_range() {
        assert_eq!(Pointer { accel_speed: 9.0, ..Default::default() }.sane_accel(), 1.0);
        assert_eq!(Pointer { accel_speed: -9.0, ..Default::default() }.sane_accel(), -1.0);
    }
}
