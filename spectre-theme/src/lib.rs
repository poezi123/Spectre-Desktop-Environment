//! Visual definitions shared by every Spectre component.
//!
//! The compositor, the panel and the settings app all resolve their look from a
//! single [`Theme`], so a colour only ever has to be changed in one place.
//!
//! ```
//! use spectre_theme::Theme;
//!
//! let theme = Theme::default();
//! assert_eq!(theme.metrics.titlebar_height, 32);
//! ```

pub mod color;
pub mod metrics;
pub mod palette;
pub mod pattern;

pub use color::{Color, Gradient};
pub use metrics::Metrics;
pub use palette::Palette;
pub use pattern::{Pattern, PatternKind};

use serde::{Deserialize, Serialize};

/// A complete Spectre look: colours, sizes and pattern settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct Theme {
    pub palette: Palette,
    pub metrics: Metrics,
    /// Pattern drawn on window decorations.
    pub window_pattern: Pattern,
    /// Pattern drawn behind the panel.
    pub panel_pattern: Pattern,
    /// Pattern drawn on the desktop backdrop. Off by default: it is the largest
    /// surface on screen and therefore the most expensive one to animate.
    pub desktop_pattern: DesktopPattern,
}

/// Wrapper so the desktop pattern can default to off while reusing [`Pattern`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DesktopPattern(pub Pattern);

impl Default for DesktopPattern {
    fn default() -> Self {
        Self(Pattern::OFF)
    }
}

impl std::ops::Deref for DesktopPattern {
    type Target = Pattern;

    fn deref(&self) -> &Pattern {
        &self.0
    }
}

impl Theme {
    /// Strip every animation while keeping the desktop fully usable.
    ///
    /// This is the animation kill switch from the project principles: patterns
    /// become static instead of vanishing, and no colour or size changes.
    pub fn without_animation(mut self) -> Self {
        self.window_pattern = self.window_pattern.without_animation();
        self.panel_pattern = self.panel_pattern.without_animation();
        self.desktop_pattern = DesktopPattern(self.desktop_pattern.0.without_animation());
        self
    }

    /// True when any surface needs a new frame every vblank.
    pub fn needs_continuous_redraw(&self) -> bool {
        self.window_pattern.needs_continuous_redraw()
            || self.panel_pattern.needs_continuous_redraw()
            || self.desktop_pattern.needs_continuous_redraw()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_pattern_is_off_by_default() {
        assert!(Theme::default().desktop_pattern.is_noop());
    }

    #[test]
    fn kill_switch_stops_all_redraws_without_hiding_anything() {
        let t = Theme::default();
        assert!(t.needs_continuous_redraw());

        let still = t.clone().without_animation();
        assert!(!still.needs_continuous_redraw());
        assert!(!still.window_pattern.is_noop(), "patterns must stay visible");
        assert_eq!(still.palette, t.palette);
        assert_eq!(still.metrics, t.metrics);
    }

    #[test]
    fn theme_round_trips_through_toml() {
        let t = Theme::default();
        let back: Theme = toml::from_str(&toml::to_string(&t).unwrap()).unwrap();
        assert_eq!(t, back);
    }

    #[test]
    fn unknown_theme_keys_are_rejected() {
        assert!(toml::from_str::<Theme>("nonsense = 1").is_err());
    }
}
