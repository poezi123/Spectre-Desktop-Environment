use spectre_theme::Pattern;

const STEP: i32 = 3;

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

        let columns = (width / STEP + 2) as usize;
        let rows = (height / STEP + 2) as usize;
        let mut field = Vec::with_capacity(columns * rows);
        for row in 0..rows {
            for column in 0..columns {
                let x = (column as i32 * STEP) as f32;
                let y = (row as i32 * STEP) as f32;
                field.push(pattern.height(x, y, phase, scale));
            }
        }

        let fraction = |v: i32| (v % STEP) as f32 / STEP as f32;
        for y in 0..height {
            let row = (y / STEP) as usize;
            let ty = fraction(y);
            for x in 0..width {
                let column = (x / STEP) as usize;
                let tx = fraction(x);
                let at = |row: usize, column: usize| field[row * columns + column];
                let top = lerp(at(row, column), at(row, column + 1), tx);
                let bottom = lerp(at(row + 1, column), at(row + 1, column + 1), tx);
                let c = pattern.line_coverage(lerp(top, bottom, ty), scale);
                self.coverage.push((c.clamp(0.0, 1.0) * 255.0).round() as u8);
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.coverage.is_empty()
    }

    pub fn bytes(&self) -> &[u8] {
        &self.coverage
    }

    pub fn size(&self) -> (i32, i32) {
        (self.width, self.height)
    }

    pub fn at(&self, x: i32, y: i32) -> f32 {
        if x < 0 || y < 0 || x >= self.width || y >= self.height {
            return 0.0;
        }
        let i = y as usize * self.width as usize + x as usize;
        self.coverage.get(i).map_or(0.0, |&c| c as f32 / 255.0)
    }
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
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

#[cfg(test)]
mod accuracy {
    use super::*;

    #[test]
    #[ignore = "measures this machine, not the code"]
    fn how_close_the_interpolation_comes() {
        let pattern = Pattern::default();
        let (w, h) = (900, 386);

        let start = std::time::Instant::now();
        let exact: Vec<u8> = (0..h)
            .flat_map(|y| {
                (0..w).map(move |x| {
                    let c = pattern.coverage(x as f32, y as f32, 0.0, 1.0);
                    (c.clamp(0.0, 1.0) * 255.0).round() as u8
                })
            })
            .collect();
        println!("per pixel: {:?}", start.elapsed());

        let mut mask = PatternMask::new();
        let start = std::time::Instant::now();
        mask.prepare(w, h, &pattern, 0.0, 1.0);
        println!("step {STEP}:   {:?}", start.elapsed());

        let diffs: Vec<i32> = exact
            .iter()
            .zip(&mask.coverage)
            .map(|(a, b)| (*a as i32 - *b as i32).abs())
            .collect();
        let worst = diffs.iter().max().unwrap();
        let mean = diffs.iter().sum::<i32>() as f32 / diffs.len() as f32;
        let visible = diffs.iter().filter(|d| **d > 8).count();
        println!("worst {worst}/255, mean {mean:.2}, {visible} pixels off by more than 8");
    }
}

#[cfg(test)]
mod dump {
    use super::*;

    #[test]
    #[ignore = "writes files, only for looking at"]
    fn dump_masks() {
        let pattern = Pattern::default();
        let (w, h) = (400, 200);
        let exact: Vec<u8> = (0..h)
            .flat_map(|y| {
                (0..w).map(move |x| {
                    let c = pattern.coverage(x as f32, y as f32, 0.0, 1.0);
                    (c.clamp(0.0, 1.0) * 255.0).round() as u8
                })
            })
            .collect();
        let mut mask = PatternMask::new();
        mask.prepare(w, h, &pattern, 0.0, 1.0);
        std::fs::write("/tmp/mask-exact.gray", &exact).unwrap();
        std::fs::write("/tmp/mask-step.gray", &mask.coverage).unwrap();
        println!("{w}x{h}");
    }
}
