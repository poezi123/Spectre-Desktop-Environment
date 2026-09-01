//! Keyboard, pointer and touchpad settings.
//!
//! These map onto libinput device options and the xkb rule set. Defaults follow
//! the freedesktop defaults so an empty config behaves like every other desktop.

use serde::{Deserialize, Serialize};

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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct Input {
    pub keyboard: Keyboard,
    pub pointer: Pointer,
}

#[cfg(test)]
mod tests {
    use super::*;

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
