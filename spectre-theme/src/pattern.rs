//! Parameters for the Spectre Pattern — the animated topographic contour lines
//! that run behind title bars, the panel and the lock screen.
//!
//! This module owns no rendering code. It only describes *what* to draw, so the
//! compositor's GLES shader, the panel's software fallback and any future
//! Vulkan path stay in agreement. Keeping the description declarative is also
//! what makes the "Performance" profile cheap: the same struct just reports
//! `animated == false` and the shader stops sampling time.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::color::{Color, Gradient};

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
    /// Whether the contour colours travel along the accent gradient. Separate
    /// from [`Pattern::animated`]: moving the colour is far cheaper than moving
    /// the noise field, so the two are switched independently.
    pub color_cycle: bool,
    /// Colour cycle rate as a 0..1 knob.
    pub color_speed: f32,
    /// Line opacity as a 0..1 knob.
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
            color_cycle: true,
            color_speed: 0.5,
            intensity: 0.55,
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
        color_cycle: false,
        color_speed: 0.0,
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
        if self.is_noop() {
            return false;
        }
        (self.animated && self.speed > 0.0) || self.cycles_color()
    }

    /// True when the colours travel even though the lines may be standing still.
    pub fn cycles_color(&self) -> bool {
        self.color_cycle && self.color_speed > 0.0 && !self.is_noop()
    }

    /// Cycles of the noise field per second at `speed == 1.0`. Slow enough to
    /// read as ambient movement rather than as something happening.
    const DRIFT_PER_SECOND: f64 = 0.06;

    /// Revolutions of the colour loop per second at `color_speed == 1.0`.
    const COLOR_REVS_PER_SECOND: f64 = 0.125;

    /// How many noise cells one contour spacing is worth. The shaders divide by
    /// the same number; it is what keeps the ridges broad enough to read as a
    /// contour map at panel height as well as full screen.
    const CELL_SPACINGS: f32 = 6.0;

    /// Phase to feed the shader at `elapsed` seconds since compositor start.
    ///
    /// Wrapped into `0.0..1000.0` so an f32 uniform keeps its precision on a
    /// machine that has been up for weeks.
    pub fn phase(&self, elapsed_secs: f64) -> f32 {
        if !self.animated || self.speed <= 0.0 || self.is_noop() {
            return 0.0;
        }
        ((elapsed_secs * self.speed as f64 * Self::DRIFT_PER_SECOND) % 1000.0) as f32
    }

    /// Where the colour cycle stands at `elapsed` seconds, in `0.0..1.0`.
    /// One revolution per 8 seconds at `color_speed = 1.0`.
    pub fn color_phase(&self, elapsed_secs: f64) -> f32 {
        if !self.cycles_color() {
            return 0.0;
        }
        ((elapsed_secs * self.color_speed as f64 * Self::COLOR_REVS_PER_SECOND).rem_euclid(1.0))
            as f32
    }

    /// Samples of one colour revolution that still read as a continuous sweep.
    ///
    /// The colour travels along a gradient rather than across pixels, so the
    /// criterion is not travel but banding: below this many steps the sweep
    /// starts to look like it is changing in jumps.
    const COLOR_STEPS: f64 = 120.0;

    /// Fastest and slowest a moving pattern is redrawn, whatever the settings
    /// say. The lower bound is there so a pattern turned up to its limit cannot
    /// ask for more frames than a display can show; the upper bound keeps a
    /// pattern turned right down still visibly creeping.
    const FASTEST_REDRAW: Duration = Duration::from_millis(16);
    const SLOWEST_REDRAW: Duration = Duration::from_millis(500);

    /// How long this pattern may be left alone before the picture changes.
    ///
    /// Both movements are far slower than a display refresh, so a frame per
    /// vblank draws the same image several times over - and on a machine
    /// without a GPU each of those frames is a repaint of the whole surface.
    ///
    /// The field drifts [`Pattern::DRIFT_PER_SECOND`] noise cells a second at
    /// full speed and a cell is `line_spacing * CELL_SPACINGS` device pixels
    /// across, which gives the travel in pixels per second: one frame per pixel
    /// of travel is all the movement there is to show. The colours are judged
    /// by [`Pattern::COLOR_STEPS`] instead, since they move through a gradient
    /// rather than across the screen.
    ///
    /// `None` means the pattern is standing still and needs no frames at all.
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
        let seconds = (1.0 / per_second).clamp(
            Self::FASTEST_REDRAW.as_secs_f64(),
            Self::SLOWEST_REDRAW.as_secs_f64(),
        );
        Some(Duration::from_secs_f64(seconds))
    }

    /// How far the accent is darkened before it is drawn as a contour line.
    ///
    /// The pattern is where Spectre's colour lives, but it is texture in the
    /// material rather than a graphic laid on top: at full accent brightness
    /// the lines stop reading as topography and start reading as neon.
    const DARKEN: f32 = 0.62;

    /// Colour of the contour lines over `background`.
    pub fn line_color(&self, accent: Color, background: Color) -> Color {
        if self.is_noop() {
            return Color::TRANSPARENT;
        }
        accent
            .scaled(Self::DARKEN)
            .mix(background, 0.15)
            .alpha(self.intensity.clamp(0.0, 1.0))
    }

    /// How many accent stops the renderers carry, matching the shader uniforms.
    pub const STOPS: usize = 4;

    /// The accent resampled to [`Pattern::STOPS`] contour-line colours.
    pub fn line_stops(&self, accent: &Gradient, background: Color) -> [Color; Self::STOPS] {
        let mut out = [Color::TRANSPARENT; Self::STOPS];
        for (i, slot) in out.iter_mut().enumerate() {
            let t = i as f32 / Self::STOPS as f32;
            *slot = self.line_color(accent.sample_cyclic(t), background);
        }
        out
    }

    /// The line colour at `t` along the loop, wrapping. The CPU twin of the
    /// shaders' `spectre_line_at`.
    pub fn line_at(stops: &[Color; Self::STOPS], t: f32) -> Color {
        let u = t.rem_euclid(1.0) * Self::STOPS as f32;
        let i = (u.floor() as usize) % Self::STOPS;
        stops[i].mix(stops[(i + 1) % Self::STOPS], u - u.floor())
    }

    /// Force the pattern static, keeping it visible. Used by the Performance
    /// profile and by the global animation kill switch.
    pub fn without_animation(mut self) -> Self {
        self.animated = false;
        self.color_cycle = false;
        self
    }

    /// Freeze the contour field but keep the colours travelling.
    pub fn with_static_lines(mut self) -> Self {
        self.animated = false;
        self.color_cycle = true;
        self
    }

    /// Line coverage at a device pixel, in `0.0..=1.0`.
    ///
    /// This is the CPU twin of `spectre-compositor`'s `pattern.glsl`, for
    /// surfaces drawn in software - the panel, and any renderer without a GPU.
    /// The two must stay in step: the constants below are the same ones the
    /// shader uses, and changing one without the other makes the panel and the
    /// title bars disagree about what the Spectre Pattern looks like.
    pub fn coverage(&self, x: f32, y: f32, phase: f32, scale: f32) -> f32 {
        if self.is_noop() {
            return 0.0;
        }
        let spacing = (self.line_spacing * scale).max(1.0);
        let cell = spacing * Self::CELL_SPACINGS;
        let q = (x / cell + phase, y / cell);
        let height = fbm(q.0, q.1);

        let levels = height * 16.0;
        let dist = (fract(levels) - 0.5).abs();
        let half_width = ((self.line_width * scale) / spacing).clamp(0.004, 0.4);
        let feather = half_width * 0.9 + 0.015;
        1.0 - smoothstep(half_width, half_width + feather, dist)
    }
}

/// The ground the lines sit on, tinted by the colour passing overhead.
///
/// Twin of `spectre_ground` in the shaders: near black at the cyan end of the
/// accent, a deep magenta at the other, which is what gives the bar its depth
/// instead of a flat fill.
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

/// GLSL's `fract`: always the positive fractional part, unlike `f32::fract`.
fn fract(v: f32) -> f32 {
    v - v.floor()
}

/// Twin of the shaders' `hash`. Polynomial rather than sine-based, because the
/// same function runs per pixel on the CPU for the panel.
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

/// Four octaves, matching the shader.
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
    fn the_default_pattern_is_redrawn_far_slower_than_a_display_refreshes() {
        let interval = Pattern::default().redraw_interval(1.0).expect("it moves");
        assert!(
            interval > Duration::from_millis(66),
            "sixty frames a second buys nothing that can be seen: {interval:?}"
        );
        assert!(interval <= Pattern::SLOWEST_REDRAW);
    }

    #[test]
    fn a_faster_pattern_is_redrawn_more_often() {
        let slow = Pattern { speed: 0.2, color_cycle: false, ..Default::default() };
        let fast = Pattern { speed: 1.0, color_cycle: false, ..Default::default() };
        assert!(fast.redraw_interval(1.0) < slow.redraw_interval(1.0));
    }

    #[test]
    fn wider_spacing_moves_the_field_faster_and_so_is_redrawn_more_often() {
        // A cell is measured in contour spacings, so wider lines mean the field
        // travels further per second in device pixels.
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
        // ACCENT_0 is teal: blue and green well above red.
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
