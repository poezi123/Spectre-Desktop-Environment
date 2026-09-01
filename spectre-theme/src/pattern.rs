//! Parameters for the Spectre Pattern — the animated topographic contour lines
//! that run behind title bars, the panel and the lock screen.
//!
//! This module owns no rendering code. It only describes *what* to draw, so the
//! compositor's GLES shader, the panel's software fallback and any future
//! Vulkan path stay in agreement. Keeping the description declarative is also
//! what makes the "Performance" profile cheap: the same struct just reports
//! `animated == false` and the shader stops sampling time.

use serde::{Deserialize, Serialize};

use crate::color::Color;

/// Which pattern family to draw.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum PatternKind {
    /// No pattern at all — flat surfaces.
    None,
    /// Contour lines of a scrolling value-noise field. The Spectre default.
    #[default]
    Topographic,
    /// Straight diagonal hairlines. Cheapest option that still reads as texture.
    Grid,
}

/// A fully resolved pattern description.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct Pattern {
    pub kind: PatternKind,
    /// Whether the field scrolls. When false the pattern is drawn once and
    /// becomes static rather than disappearing.
    pub animated: bool,
    /// Animation rate as a 0..1 knob; the settings UI shows this as a percentage.
    pub speed: f32,
    /// Line opacity as a 0..1 knob. The concept keeps this very low on purpose.
    pub intensity: f32,
    /// Distance between contour lines, in logical pixels.
    pub line_spacing: f32,
    /// Contour line thickness, in logical pixels.
    pub line_width: f32,
}

impl Default for Pattern {
    fn default() -> Self {
        Self {
            kind: PatternKind::Topographic,
            animated: true,
            speed: 0.6,
            intensity: 0.14,
            line_spacing: 26.0,
            line_width: 1.0,
        }
    }
}

impl Pattern {
    /// A pattern that draws nothing.
    pub const OFF: Pattern = Pattern {
        kind: PatternKind::None,
        animated: false,
        speed: 0.0,
        intensity: 0.0,
        line_spacing: 26.0,
        line_width: 1.0,
    };

    /// True when a renderer can skip the pattern pass entirely this frame.
    pub fn is_noop(&self) -> bool {
        self.kind == PatternKind::None || self.intensity <= 0.0 || self.line_width <= 0.0
    }

    /// True when the surface has to be repainted every frame.
    ///
    /// A static pattern still gets drawn, it just does not need a new frame, so
    /// the compositor can leave the surface out of its damage list.
    pub fn needs_continuous_redraw(&self) -> bool {
        self.animated && self.speed > 0.0 && !self.is_noop()
    }

    /// Phase to feed the shader at `elapsed` seconds since compositor start.
    ///
    /// Wrapped into `0.0..1000.0` so an f32 uniform keeps its precision on a
    /// machine that has been up for weeks.
    pub fn phase(&self, elapsed_secs: f64) -> f32 {
        if !self.needs_continuous_redraw() {
            return 0.0;
        }
        // 0.06 cycles/s at speed 1.0 — slow enough to read as ambient movement.
        ((elapsed_secs * self.speed as f64 * 0.06) % 1000.0) as f32
    }

    /// Colour of the contour lines over `background`.
    pub fn line_color(&self, accent: Color, background: Color) -> Color {
        if self.is_noop() {
            return Color::TRANSPARENT;
        }
        // Lift the accent slightly towards the surface so the lines read as
        // texture in the material rather than as drawn-on graphics.
        accent.mix(background, 0.35).alpha(self.intensity.clamp(0.0, 1.0))
    }

    /// Force the pattern static, keeping it visible. Used by the Performance
    /// profile and by the global animation kill switch.
    pub fn without_animation(mut self) -> Self {
        self.animated = false;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::palette;

    #[test]
    fn off_pattern_is_a_noop() {
        assert!(Pattern::OFF.is_noop());
        assert!(!Pattern::OFF.needs_continuous_redraw());
        assert_eq!(Pattern::OFF.phase(1234.0), 0.0);
    }

    #[test]
    fn zero_intensity_is_a_noop_even_when_animated() {
        let p = Pattern { intensity: 0.0, ..Default::default() };
        assert!(p.is_noop());
        assert!(!p.needs_continuous_redraw());
    }

    #[test]
    fn static_pattern_still_draws_but_never_redraws() {
        let p = Pattern::default().without_animation();
        assert!(!p.is_noop(), "static must stay visible, not disappear");
        assert!(!p.needs_continuous_redraw());
    }

    #[test]
    fn phase_advances_and_stays_bounded() {
        let p = Pattern::default();
        assert!(p.phase(10.0) > p.phase(0.0));
        assert!(p.phase(1.0e9).abs() < 1000.0);
    }

    #[test]
    fn line_colour_respects_intensity() {
        let p = Pattern::default();
        let c = p.line_color(palette::ACCENT_2, palette::SURFACE);
        assert!((c.a - p.intensity).abs() < 1e-6);
    }
}
