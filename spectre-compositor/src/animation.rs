use std::time::{Duration, Instant};

use smithay::utils::{Logical, Rectangle};

const OPEN_DURATION: Duration = Duration::from_millis(220);

const CLOSE_DURATION: Duration = Duration::from_millis(170);

const SMALLEST: f64 = 0.85;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pop {
    started: Instant,
    duration: Duration,
    opening: bool,
}

impl Pop {
    pub fn opening(now: Instant, speed: f32) -> Self {
        Self { started: now, duration: scaled(OPEN_DURATION, speed), opening: true }
    }

    pub fn closing(now: Instant, speed: f32) -> Self {
        Self { started: now, duration: scaled(CLOSE_DURATION, speed), opening: false }
    }

    fn progress(&self, now: Instant) -> f32 {
        let elapsed = now.saturating_duration_since(self.started).as_secs_f32();
        let total = self.duration.as_secs_f32();
        if total <= 0.0 {
            return 1.0;
        }
        (elapsed / total).clamp(0.0, 1.0)
    }

    pub fn is_done(&self, now: Instant) -> bool {
        self.progress(now) >= 1.0
    }

    pub fn scale(&self, now: Instant) -> f64 {
        let t = self.progress(now) as f64;
        if self.opening {
            SMALLEST + (1.0 - SMALLEST) * ease_out_back(t)
        } else {
            1.0 - (1.0 - SMALLEST) * t * t
        }
    }

    pub fn alpha(&self, now: Instant) -> f32 {
        let t = self.progress(now);
        if self.opening {
            1.0 - (1.0 - t) * (1.0 - t)
        } else {
            1.0 - t
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Closing {
    pub key: u32,
    pub workspace: usize,
    pub outer: Rectangle<i32, Logical>,
    pub pop: Pop,
}

fn scaled(duration: Duration, speed: f32) -> Duration {
    let speed = if speed.is_finite() { speed.clamp(0.25, 4.0) } else { 1.0 };
    duration.div_f32(speed)
}

fn ease_out_back(t: f64) -> f64 {
    let overshoot = 1.4;
    let u = t - 1.0;
    1.0 + (overshoot + 1.0) * u * u * u + overshoot * u * u
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(now: Instant, millis: u64) -> Instant {
        now + Duration::from_millis(millis)
    }

    #[test]
    fn an_opening_window_starts_small_and_invisible() {
        let now = Instant::now();
        let pop = Pop::opening(now, 1.0);
        assert!((pop.scale(now) - SMALLEST).abs() < 1e-6);
        assert!(pop.alpha(now) < 1e-6);
    }

    #[test]
    fn an_opening_window_ends_at_full_size() {
        let now = Instant::now();
        let pop = Pop::opening(now, 1.0);
        let end = at(now, 400);
        assert!(pop.is_done(end));
        assert!((pop.scale(end) - 1.0).abs() < 1e-6);
        assert!((pop.alpha(end) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn the_pop_overshoots_a_little_on_the_way() {
        let now = Instant::now();
        let pop = Pop::opening(now, 1.0);
        let largest = (0..=220).map(|ms| pop.scale(at(now, ms))).fold(0.0, f64::max);
        assert!(largest > 1.0, "a pop without overshoot reads as a fade");
        assert!(largest < 1.05, "and too much reads as a bounce");
    }

    #[test]
    fn a_closing_window_shrinks_and_fades_out() {
        let now = Instant::now();
        let pop = Pop::closing(now, 1.0);
        assert!((pop.scale(now) - 1.0).abs() < 1e-6);
        let end = at(now, 400);
        assert!(pop.is_done(end));
        assert!(pop.alpha(end) < 1e-6);
        assert!((pop.scale(end) - SMALLEST).abs() < 1e-6);
    }

    #[test]
    fn a_faster_setting_finishes_sooner() {
        let now = Instant::now();
        let slow = Pop::opening(now, 0.5);
        let fast = Pop::opening(now, 2.0);
        let middle = at(now, 150);
        assert!(fast.is_done(middle));
        assert!(!slow.is_done(middle));
    }

    #[test]
    fn a_nonsense_speed_still_animates() {
        let now = Instant::now();
        for speed in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            let pop = Pop::opening(now, speed);
            assert!(pop.is_done(at(now, 2000)), "speed {speed}");
        }
    }
}
