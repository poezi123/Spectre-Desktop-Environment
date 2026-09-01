//! Sizes, radii and spacing, in logical pixels.
//!
//! Every value is pre-scale: renderers multiply by the output's fractional
//! scale. Keeping one table here stops the panel and the compositor from
//! disagreeing about how tall a title bar is.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct Metrics {
    /// Height of a server-side title bar.
    pub titlebar_height: u32,
    /// Window border thickness. `0` disables borders.
    pub border_width: u32,
    /// Corner radius of windows, menus and dialogs.
    pub corner_radius: u32,
    /// Gap between tiled windows and between a window and the screen edge.
    pub gap: u32,

    /// Panel thickness along its anchored edge.
    pub panel_height: u32,
    /// Inner padding at the panel's short ends.
    pub panel_padding: u32,
    /// Gap between adjacent panel widgets.
    pub panel_spacing: u32,
    /// Edge margin when the panel is in floating mode.
    pub panel_margin: u32,

    /// Square size of a title bar button hit box.
    pub button_size: u32,
    /// Icon edge length inside a panel button or tray slot.
    pub icon_size: u32,
}

impl Default for Metrics {
    fn default() -> Self {
        // Matches the proportions in Fensterconcept.png / Taskleiste Concept.png.
        Self {
            titlebar_height: 32,
            border_width: 1,
            corner_radius: 8,
            gap: 8,
            panel_height: 32,
            panel_padding: 6,
            panel_spacing: 4,
            panel_margin: 6,
            button_size: 28,
            icon_size: 18,
        }
    }
}

impl Metrics {
    /// Scale every metric for an output, rounding to whole device pixels.
    ///
    /// A border that rounds to zero is clamped back to one pixel: losing the
    /// focus outline entirely on a fractional scale would be a usability bug,
    /// and the project rules say the desktop stays usable without effects.
    pub fn scaled(self, scale: f64) -> Self {
        let s = |v: u32| ((v as f64 * scale).round() as u32).max(0);
        Self {
            titlebar_height: s(self.titlebar_height),
            border_width: if self.border_width == 0 { 0 } else { s(self.border_width).max(1) },
            corner_radius: s(self.corner_radius),
            gap: s(self.gap),
            panel_height: s(self.panel_height),
            panel_padding: s(self.panel_padding),
            panel_spacing: s(self.panel_spacing),
            panel_margin: s(self.panel_margin),
            button_size: s(self.button_size),
            icon_size: s(self.icon_size),
        }
    }

    /// Total decoration inset around a window's content: `(top, right, bottom, left)`.
    pub fn decoration_insets(self, decorated: bool) -> (u32, u32, u32, u32) {
        if !decorated {
            return (0, 0, 0, 0);
        }
        let b = self.border_width;
        (self.titlebar_height + b, b, b, b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scaling_keeps_a_visible_border() {
        let m = Metrics::default().scaled(1.25);
        assert_eq!(m.titlebar_height, 40);
        assert!(m.border_width >= 1);
    }

    #[test]
    fn a_disabled_border_stays_disabled() {
        let m = Metrics { border_width: 0, ..Default::default() }.scaled(2.0);
        assert_eq!(m.border_width, 0);
    }

    #[test]
    fn undecorated_windows_have_no_insets() {
        assert_eq!(Metrics::default().decoration_insets(false), (0, 0, 0, 0));
    }

    #[test]
    fn decoration_insets_include_the_titlebar() {
        let m = Metrics::default();
        let (top, r, b, l) = m.decoration_insets(true);
        assert_eq!(top, m.titlebar_height + m.border_width);
        assert_eq!((r, b, l), (m.border_width, m.border_width, m.border_width));
    }
}
