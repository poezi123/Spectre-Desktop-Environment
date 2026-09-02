//! Cached contour coverage for the software pattern.
//!
//! Evaluating four octaves of noise per pixel is the expensive half of the
//! Spectre Pattern, and it only changes when the field scrolls. Caching it
//! means a surface whose lines stand still and whose colours travel is repainted
//! for the cost of a blend per pixel.

use spectre_theme::Pattern;

#[derive(Debug, Clone)]
pub struct PatternMask {
    width: i32,
    height: i32,
    phase: f32,
    scale: f32,
    pub(crate) pattern: Pattern,
    coverage: Vec<u8>,
}

impl Default for PatternMask {
    fn default() -> Self {
        Self::new()
    }
}

impl PatternMask {
    pub fn new() -> Self {
        Self {
            width: 0,
            height: 0,
            phase: 0.0,
            scale: 1.0,
            pattern: Pattern::OFF,
            coverage: Vec::new(),
        }
    }

    /// Recompute the mask if anything it depends on changed.
    pub fn prepare(
        &mut self,
        width: i32,
        height: i32,
        pattern: &Pattern,
        phase: f32,
        scale: f32,
    ) {
        let unchanged = self.width == width
            && self.height == height
            && self.phase == phase
            && self.scale == scale
            && &self.pattern == pattern;
        if unchanged {
            return;
        }

        self.width = width;
        self.height = height;
        self.phase = phase;
        self.scale = scale;
        self.pattern = *pattern;

        if width <= 0 || height <= 0 || pattern.is_noop() {
            self.coverage.clear();
            return;
        }

        self.coverage.clear();
        self.coverage.reserve(width as usize * height as usize);
        for y in 0..height {
            for x in 0..width {
                let c = pattern.coverage(x as f32, y as f32, phase, scale);
                self.coverage.push((c.clamp(0.0, 1.0) * 255.0).round() as u8);
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.coverage.is_empty()
    }

    /// Coverage at a mask-local pixel, in `0.0..=1.0`.
    pub fn at(&self, x: i32, y: i32) -> f32 {
        if x < 0 || y < 0 || x >= self.width || y >= self.height {
            return 0.0;
        }
        let i = y as usize * self.width as usize + x as usize;
        self.coverage.get(i).map_or(0.0, |&c| c as f32 / 255.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_switched_off_pattern_caches_nothing() {
        let mut mask = PatternMask::new();
        mask.prepare(40, 10, &Pattern::OFF, 0.0, 1.0);
        assert!(mask.is_empty());
        assert_eq!(mask.at(3, 3), 0.0);
    }

    #[test]
    fn the_mask_covers_the_area_it_was_asked_for() {
        let mut mask = PatternMask::new();
        mask.prepare(40, 10, &Pattern::default(), 0.0, 1.0);
        assert!(!mask.is_empty());
        assert_eq!(mask.coverage.len(), 400);
        assert!(mask.coverage.iter().any(|&c| c > 0), "no lines landed in the mask");
    }

    #[test]
    fn sampling_outside_the_mask_is_zero_rather_than_a_panic() {
        let mut mask = PatternMask::new();
        mask.prepare(8, 8, &Pattern::default(), 0.0, 1.0);
        assert_eq!(mask.at(-1, 0), 0.0);
        assert_eq!(mask.at(0, 99), 0.0);
    }

    #[test]
    fn preparing_twice_with_the_same_inputs_keeps_the_cache() {
        let mut mask = PatternMask::new();
        let pattern = Pattern::default();
        mask.prepare(16, 16, &pattern, 0.25, 1.0);
        let first = mask.coverage.clone();
        mask.prepare(16, 16, &pattern, 0.25, 1.0);
        assert_eq!(first, mask.coverage);
    }

    #[test]
    fn a_new_phase_moves_the_lines() {
        let mut mask = PatternMask::new();
        let pattern = Pattern::default();
        mask.prepare(32, 32, &pattern, 0.0, 1.0);
        let first = mask.coverage.clone();
        mask.prepare(32, 32, &pattern, 0.5, 1.0);
        assert_ne!(first, mask.coverage);
    }
}
