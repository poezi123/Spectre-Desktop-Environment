use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::color::{Color, Gradient};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum PatternKind {
    None,
    #[default]
    Topographic,
    Grid,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct Pattern {
    pub kind: PatternKind,
    pub animated: bool,
    pub speed: f32,
    pub color_cycle: bool,
    pub color_speed: f32,
    pub intensity: f32,
    pub line_spacing: f32,
    pub line_width: f32,
}

impl Default for Pattern {
    fn default() -> Self {
        Self {
            kind: PatternKind::Topographic,
            animated: true,
            speed: 0.6,
            color_cycle: true,
            color_speed: 0.5,
            intensity: 0.55,
            line_spacing: 26.0,
            line_width: 1.0,
        }
    }
}

impl Pattern {
    pub const OFF: Pattern = Pattern {
        kind: PatternKind::None,
        animated: false,
        speed: 0.0,
        color_cycle: false,
        color_speed: 0.0,
        intensity: 0.0,
        line_spacing: 26.0,
        line_width: 1.0,
    };

    pub fn is_noop(&self) -> bool {
        self.kind == PatternKind::None || self.intensity <= 0.0 || self.line_width <= 0.0
    }

    pub fn needs_continuous_redraw(&self) -> bool {
        if self.is_noop() {
            return false;
        }
        (self.animated && self.speed > 0.0) || self.cycles_color()
    }

    pub fn cycles_color(&self) -> bool {
        self.color_cycle && self.color_speed > 0.0 && !self.is_noop()
    }

    const DRIFT_PER_SECOND: f64 = 0.06;

    const COLOR_REVS_PER_SECOND: f64 = 0.125;

    const CELL_SPACINGS: f32 = 6.0;

    pub fn phase(&self, elapsed_secs: f64) -> f32 {
        if !self.animated || self.speed <= 0.0 || self.is_noop() {
            return 0.0;
        }
        ((elapsed_secs * self.speed as f64 * Self::DRIFT_PER_SECOND) % 1000.0) as f32
    }

    pub fn color_phase(&self, elapsed_secs: f64) -> f32 {
        if !self.cycles_color() {
            return 0.0;
        }
        ((elapsed_secs * self.color_speed as f64 * Self::COLOR_REVS_PER_SECOND).rem_euclid(1.0))
            as f32
    }

    const COLOR_STEPS: f64 = 120.0;

    const SUBSTEPS: f64 = 4.0;

    const FASTEST_REDRAW: Duration = Duration::from_millis(16);
    const SLOWEST_REDRAW: Duration = Duration::from_millis(500);

    pub fn redraw_interval(&self, scale: f32) -> Option<Duration> {
        if !self.needs_continuous_redraw() {
            return None;
        }
        let mut per_second: f64 = 0.0;

        if self.animated && self.speed > 0.0 {
            let cell = (self.line_spacing * scale).max(1.0) * Self::CELL_SPACINGS;
            let pixels_per_second = self.speed as f64 * Self::DRIFT_PER_SECOND * cell as f64;
            per_second = per_second.max(pixels_per_second);
        }
        if self.cycles_color() {
            let steps_per_second =
                self.color_speed as f64 * Self::COLOR_REVS_PER_SECOND * Self::COLOR_STEPS;
            per_second = per_second.max(steps_per_second);
        }

        if !per_second.is_finite() || per_second <= 0.0 {
            return Some(Self::SLOWEST_REDRAW);
        }
        let seconds = (1.0 / (per_second * Self::SUBSTEPS)).clamp(
            Self::FASTEST_REDRAW.as_secs_f64(),
            Self::SLOWEST_REDRAW.as_secs_f64(),
        );
        Some(Duration::from_secs_f64(seconds))
    }

    const DARKEN: f32 = 0.62;

    pub fn line_color(&self, accent: Color, background: Color) -> Color {
        if self.is_noop() {
            return Color::TRANSPARENT;
        }
        accent
            .scaled(Self::DARKEN)
            .mix(background, 0.15)
            .alpha(self.intensity.clamp(0.0, 1.0))
    }

    pub const STOPS: usize = 4;

    pub fn line_stops(&self, accent: &Gradient, background: Color) -> [Color; Self::STOPS] {
        let mut out = [Color::TRANSPARENT; Self::STOPS];
        for (i, slot) in out.iter_mut().enumerate() {
            let t = i as f32 / Self::STOPS as f32;
            *slot = self.line_color(accent.sample_cyclic(t), background);
        }
        out
    }

    pub fn line_at(stops: &[Color; Self::STOPS], t: f32) -> Color {
        let u = t.rem_euclid(1.0) * Self::STOPS as f32;
        let i = (u.floor() as usize) % Self::STOPS;
        stops[i].mix(stops[(i + 1) % Self::STOPS], u - u.floor())
    }

    pub fn without_animation(mut self) -> Self {
        self.animated = false;
        self.color_cycle = false;
        self
    }

    pub fn with_static_lines(mut self) -> Self {
        self.animated = false;
        self.color_cycle = true;
        self
    }

    const LEVELS: f32 = 16.0;

    pub fn coverage(&self, x: f32, y: f32, phase: f32, scale: f32) -> f32 {
        if self.is_noop() {
            return 0.0;
        }
        self.line_coverage(self.height(x, y, phase, scale), scale)
    }

    pub fn cell(&self, scale: f32) -> f32 {
        (self.line_spacing * scale).max(1.0) * Self::CELL_SPACINGS
    }

    pub fn height(&self, x: f32, y: f32, phase: f32, scale: f32) -> f32 {
        let cell = self.cell(scale);
        fbm(x / cell + phase, y / cell)
    }

    pub fn line_coverage(&self, height: f32, scale: f32) -> f32 {
        if self.is_noop() {
            return 0.0;
        }
        let spacing = (self.line_spacing * scale).max(1.0);
        let levels = height * Self::LEVELS;
        let dist = (fract(levels) - 0.5).abs();
        let half_width = ((self.line_width * scale) / spacing).clamp(0.004, 0.4);
        let feather = half_width * 0.9 + 0.015;
        1.0 - smoothstep(half_width, half_width + feather, dist)
    }
}

pub fn ground(base: Color, line: Color) -> Color {
    const TINT: f32 = 0.20;
    const MIX: f32 = 0.30;
    Color {
        r: base.r + (line.r * TINT - base.r) * MIX,
        g: base.g + (line.g * TINT - base.g) * MIX,
        b: base.b + (line.b * TINT - base.b) * MIX,
        a: base.a,
    }
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    if edge1 <= edge0 {
        return if x < edge0 { 0.0 } else { 1.0 };
    }
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn fract(v: f32) -> f32 {
    v - v.floor()
}

fn hash(x: f32, y: f32) -> f32 {
    let mut q = [fract(x * 0.1031), fract(y * 0.1031), fract(x * 0.1031)];
    let d = q[0] * (q[1] + 33.33) + q[1] * (q[2] + 33.33) + q[2] * (q[0] + 33.33);
    q[0] += d;
    q[1] += d;
    q[2] += d;
    fract((q[0] + q[1]) * q[2])
}

fn value_noise(x: f32, y: f32) -> f32 {
    let (ix, iy) = (x.floor(), y.floor());
    let (fx, fy) = (x - ix, y - iy);
    let ux = fx * fx * (3.0 - 2.0 * fx);
    let uy = fy * fy * (3.0 - 2.0 * fy);

    let a = hash(ix, iy);
    let b = hash(ix + 1.0, iy);
    let c = hash(ix, iy + 1.0);
    let d = hash(ix + 1.0, iy + 1.0);
    let top = a + (b - a) * ux;
    let bottom = c + (d - c) * ux;
    top + (bottom - top) * uy
}

fn fbm(mut x: f32, mut y: f32) -> f32 {
    let mut v = 0.0;
    let mut amp = 0.5;
    for _ in 0..4 {
        v += amp * value_noise(x, y);
        x *= 2.03;
        y *= 2.03;
        amp *= 0.5;
    }
    v
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
    fn coverage_stays_inside_zero_to_one() {
        let p = Pattern::default();
        for i in 0..400 {
            let (x, y) = (i as f32 * 3.7, (i % 37) as f32 * 2.3);
            let c = p.coverage(x, y, 0.25, 1.0);
            assert!((0.0..=1.0).contains(&c), "coverage {c} at ({x}, {y})");
        }
    }

    #[test]
    fn a_disabled_pattern_covers_nothing() {
        assert_eq!(Pattern::OFF.coverage(10.0, 10.0, 0.0, 1.0), 0.0);
        let flat = Pattern { intensity: 0.0, ..Default::default() };
        assert_eq!(flat.coverage(10.0, 10.0, 0.0, 1.0), 0.0);
    }

    #[test]
    fn the_pattern_actually_draws_lines_somewhere() {
        let p = Pattern::default();
        let any = (0..2000).any(|i| p.coverage(i as f32 * 1.3, 16.0, 0.0, 1.0) > 0.5);
        assert!(any, "a topographic pattern with no visible contour is not a pattern");
    }

    #[test]
    fn the_phase_moves_the_field() {
        let p = Pattern::default();
        let before: Vec<f32> = (0..64).map(|i| p.coverage(i as f32 * 4.0, 8.0, 0.0, 1.0)).collect();
        let after: Vec<f32> = (0..64).map(|i| p.coverage(i as f32 * 4.0, 8.0, 5.0, 1.0)).collect();
        assert_ne!(before, after);
    }

    #[test]
    fn coverage_is_deterministic() {
        let p = Pattern::default();
        assert_eq!(p.coverage(12.5, 7.5, 0.3, 1.25), p.coverage(12.5, 7.5, 0.3, 1.25));
    }

    #[test]
    fn scaling_keeps_the_pattern_finite() {
        let p = Pattern::default();
        for scale in [0.5, 1.0, 1.5, 2.0, 3.0] {
            let c = p.coverage(100.0, 20.0, 0.0, scale);
            assert!(c.is_finite() && (0.0..=1.0).contains(&c));
        }
    }

    #[test]
    fn a_still_pattern_asks_for_no_frames_at_all() {
        assert_eq!(Pattern::default().without_animation().redraw_interval(1.0), None);
        assert_eq!(Pattern::OFF.redraw_interval(1.0), None);
    }

    #[test]
    fn the_default_pattern_moves_in_steps_small_enough_to_read_as_motion() {
        let interval = Pattern::default().redraw_interval(1.0).expect("it moves");
        assert!(
            interval <= Duration::from_millis(40),
            "a step every {interval:?} reads as a jump rather than as movement"
        );
        assert!(interval >= Pattern::FASTEST_REDRAW, "never faster than a display: {interval:?}");
    }

    #[test]
    fn a_faster_pattern_is_redrawn_more_often() {
        let slow = Pattern { speed: 0.2, color_cycle: false, ..Default::default() };
        let fast = Pattern { speed: 1.0, color_cycle: false, ..Default::default() };
        assert!(fast.redraw_interval(1.0) < slow.redraw_interval(1.0));
    }

    #[test]
    fn wider_spacing_moves_the_field_faster_and_so_is_redrawn_more_often() {
        let tight = Pattern { line_spacing: 8.0, color_cycle: false, ..Default::default() };
        let wide = Pattern { line_spacing: 64.0, color_cycle: false, ..Default::default() };
        assert!(wide.redraw_interval(1.0) < tight.redraw_interval(1.0));
    }

    #[test]
    fn a_doubled_scale_halves_the_interval() {
        let p = Pattern { color_cycle: false, ..Default::default() };
        let one = p.redraw_interval(1.0).unwrap().as_secs_f64();
        let two = p.redraw_interval(2.0).unwrap().as_secs_f64();
        assert!((one / two - 2.0).abs() < 0.01, "{one} vs {two}");
    }

    #[test]
    fn frozen_lines_are_still_redrawn_for_the_colours() {
        let p = Pattern::default().with_static_lines();
        let interval = p.redraw_interval(1.0).expect("the colours travel");
        assert!(interval < Pattern::SLOWEST_REDRAW);
    }

    #[test]
    fn no_pattern_ever_asks_for_more_than_a_display_can_show() {
        for speed in [1.0, 100.0] {
            for spacing in [1.0, 1000.0] {
                for scale in [1.0, 4.0] {
                    let p = Pattern { speed, line_spacing: spacing, color_speed: speed,
                                      ..Default::default() };
                    let interval = p.redraw_interval(scale).unwrap();
                    assert!(interval >= Pattern::FASTEST_REDRAW, "{interval:?}");
                    assert!(interval <= Pattern::SLOWEST_REDRAW, "{interval:?}");
                }
            }
        }
    }

    #[test]
    fn a_nonsense_setting_still_yields_an_interval() {
        let p = Pattern { line_spacing: f32::NAN, ..Default::default() };
        assert!(p.redraw_interval(1.0).is_some());
    }

    #[test]
    fn line_colour_respects_intensity() {
        let p = Pattern::default();
        let c = p.line_color(palette::ACCENT_2, palette::SURFACE);
        assert!((c.a - p.intensity).abs() < 1e-6);
    }

    #[test]
    fn the_lines_are_darker_than_the_accent_they_come_from() {
        let p = Pattern::default();
        let accent = palette::ACCENT_0;
        let line = p.line_color(accent, palette::SURFACE);
        let brightness = |c: crate::Color| c.r + c.g + c.b;
        assert!(brightness(line) < brightness(accent), "the pattern must not read as neon");
    }

    #[test]
    fn the_lines_keep_the_accent_hue_rather_than_going_grey() {
        let p = Pattern::default();
        let line = p.line_color(palette::ACCENT_0, palette::SURFACE);
        assert!(line.b > line.r && line.g > line.r, "the colour has to survive the darkening");
    }

    #[test]
    fn the_stops_differ_so_the_pattern_shifts_hue() {
        let palette = crate::Palette::default();
        let stops = Pattern::default().line_stops(&palette.accent, palette::SURFACE);
        assert_ne!(stops[0], stops[2]);
        assert!(stops[0].b > stops[0].r, "it starts at the cyan end");
        assert!(stops[3].r > stops[0].r, "and reaches magenta");
    }

    #[test]
    fn a_disabled_pattern_has_no_stops_either() {
        let palette = crate::Palette::default();
        let stops = Pattern::OFF.line_stops(&palette.accent, palette::SURFACE);
        assert!(stops.iter().all(|c| *c == Color::TRANSPARENT));
    }

    #[test]
    fn the_colour_loop_wraps_without_a_jump() {
        let palette = crate::Palette::default();
        let stops = Pattern::default().line_stops(&palette.accent, palette::SURFACE);
        let just_before = Pattern::line_at(&stops, 0.999);
        let wrapped = Pattern::line_at(&stops, 1.001);
        assert!((just_before.r - wrapped.r).abs() < 0.02);
        assert!((just_before.g - wrapped.g).abs() < 0.02);
        assert!((just_before.b - wrapped.b).abs() < 0.02);
    }

    #[test]
    fn colours_travel_even_when_the_lines_are_frozen() {
        let p = Pattern::default().with_static_lines();
        assert_eq!(p.phase(4.0), 0.0, "the field must stand still");
        assert!(p.color_phase(4.0) > 0.0, "the colours must not");
        assert!(p.needs_continuous_redraw());
    }

    #[test]
    fn the_kill_switch_stops_both() {
        let p = Pattern::default().without_animation();
        assert_eq!(p.phase(4.0), 0.0);
        assert_eq!(p.color_phase(4.0), 0.0);
        assert!(!p.needs_continuous_redraw());
        assert!(!p.is_noop(), "the pattern is still drawn, just still");
    }
}
