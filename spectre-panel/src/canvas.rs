//! A minimal software canvas.
//!
//! The panel is a strip a few hundred kilobytes in size, redrawn a handful of
//! times a second. That is far below the point where a GPU context would pay
//! for itself, so everything is composited here on the CPU, straight into the
//! shared-memory buffer the compositor reads.
//!
//! Pixels are stored as `Argb8888` - what `wl_shm` expects - which on a
//! little-endian machine is `[B, G, R, A]` in memory, premultiplied.

use spectre_text::Image;
use spectre_theme::{Color, Pattern};

/// An integer rectangle in panel-local pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }

    pub fn is_empty(&self) -> bool {
        self.w <= 0 || self.h <= 0
    }

    pub fn right(&self) -> i32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> i32 {
        self.y + self.h
    }

    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x && y >= self.y && x < self.right() && y < self.bottom()
    }

    /// Shrink on every side by `amount`, never past zero.
    pub fn inset(&self, amount: i32) -> Rect {
        Rect::new(
            self.x + amount,
            self.y + amount,
            (self.w - amount * 2).max(0),
            (self.h - amount * 2).max(0),
        )
    }

    /// The overlap of two rectangles, empty when they do not touch.
    pub fn intersect(&self, other: &Rect) -> Rect {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        Rect::new(
            x,
            y,
            (self.right().min(other.right()) - x).max(0),
            (self.bottom().min(other.bottom()) - y).max(0),
        )
    }
}

/// A premultiplied ARGB pixel buffer.
pub struct Canvas {
    width: i32,
    height: i32,
    pixels: Vec<u8>,
}

impl Canvas {
    pub fn new(width: i32, height: i32) -> Self {
        let (width, height) = (width.max(0), height.max(0));
        Self { width, height, pixels: vec![0; (width * height * 4) as usize] }
    }

    /// Used by the tests, and by any future widget that needs to know how much
    /// room it has without going through `bounds`.
    #[allow(dead_code)]
    pub fn width(&self) -> i32 {
        self.width
    }

    #[allow(dead_code)]
    pub fn height(&self) -> i32 {
        self.height
    }

    pub fn bounds(&self) -> Rect {
        Rect::new(0, 0, self.width, self.height)
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.pixels
    }

    /// Resize, discarding the contents. Returns `true` if anything changed.
    pub fn resize(&mut self, width: i32, height: i32) -> bool {
        let (width, height) = (width.max(0), height.max(0));
        if width == self.width && height == self.height {
            return false;
        }
        self.width = width;
        self.height = height;
        self.pixels = vec![0; (width * height * 4) as usize];
        true
    }

    /// Overwrite every pixel, ignoring what was there.
    #[allow(dead_code)]
    pub fn clear(&mut self, color: Color) {
        let [b, g, r, a] = to_argb(color);
        for chunk in self.pixels.chunks_exact_mut(4) {
            chunk[0] = b;
            chunk[1] = g;
            chunk[2] = r;
            chunk[3] = a;
        }
    }

    /// Blend `color` over the rectangle, clipped to the canvas.
    pub fn fill_rect(&mut self, rect: Rect, color: Color) {
        if color.a <= 0.0 {
            return;
        }
        let area = rect.intersect(&self.bounds());
        if area.is_empty() {
            return;
        }
        let [b, g, r, a] = to_argb(color);
        for y in area.y..area.bottom() {
            for x in area.x..area.right() {
                self.blend(x, y, b, g, r, a);
            }
        }
    }

    /// Draw a rasterised text image with its top-left at `(x, y)`.
    pub fn draw_image(&mut self, x: i32, y: i32, image: &Image) {
        if image.is_empty() {
            return;
        }
        let stride = image.stride();
        for row in 0..image.height as i32 {
            let ty = y + row;
            if ty < 0 || ty >= self.height {
                continue;
            }
            for col in 0..image.width as i32 {
                let tx = x + col;
                if tx < 0 || tx >= self.width {
                    continue;
                }
                let i = row as usize * stride + col as usize * 4;
                // spectre-text hands back premultiplied RGBA.
                let (r, g, b, a) =
                    (image.data[i], image.data[i + 1], image.data[i + 2], image.data[i + 3]);
                if a == 0 {
                    continue;
                }
                self.blend(tx, ty, b, g, r, a);
            }
        }
    }

    /// Fill `rect` with the Spectre Pattern over `background`.
    ///
    /// Uses [`Pattern::coverage`], the same field the compositor's shader
    /// evaluates, so the panel and the window title bars show one pattern
    /// rather than two that merely look similar.
    #[allow(clippy::too_many_arguments)]
    pub fn fill_pattern(
        &mut self,
        rect: Rect,
        pattern: &Pattern,
        background: Color,
        accent: Color,
        phase: f32,
        scale: f32,
    ) {
        self.fill_rect(rect, background);
        if pattern.is_noop() {
            return;
        }
        let line = pattern.line_color(accent, background);
        let area = rect.intersect(&self.bounds());
        if area.is_empty() {
            return;
        }

        for y in area.y..area.bottom() {
            for x in area.x..area.right() {
                let coverage = pattern.coverage(
                    (x - rect.x) as f32,
                    (y - rect.y) as f32,
                    phase,
                    scale,
                );
                if coverage <= 0.0 {
                    continue;
                }
                let color = line.alpha(line.a * coverage);
                let [b, g, r, a] = to_argb(color);
                self.blend(x, y, b, g, r, a);
            }
        }
    }

    /// Source-over blend one premultiplied pixel.
    #[inline]
    fn blend(&mut self, x: i32, y: i32, b: u8, g: u8, r: u8, a: u8) {
        let idx = ((y * self.width + x) * 4) as usize;
        if a == 255 {
            self.pixels[idx] = b;
            self.pixels[idx + 1] = g;
            self.pixels[idx + 2] = r;
            self.pixels[idx + 3] = 255;
            return;
        }
        let inv = 255 - a as u32;
        let mix = |src: u8, dst: u8| -> u8 { (src as u32 + (dst as u32 * inv) / 255).min(255) as u8 };
        self.pixels[idx] = mix(b, self.pixels[idx]);
        self.pixels[idx + 1] = mix(g, self.pixels[idx + 1]);
        self.pixels[idx + 2] = mix(r, self.pixels[idx + 2]);
        self.pixels[idx + 3] = mix(a, self.pixels[idx + 3]);
    }
}

/// Premultiplied `[B, G, R, A]`, the byte order `Argb8888` has in memory.
fn to_argb(color: Color) -> [u8; 4] {
    let a = color.a.clamp(0.0, 1.0);
    let c = |v: f32| ((v.clamp(0.0, 1.0) * a) * 255.0 + 0.5) as u8;
    [c(color.b), c(color.g), c(color.r), (a * 255.0 + 0.5) as u8]
}

#[cfg(test)]
mod tests {
    use super::*;
    use spectre_theme::palette;

    fn pixel(canvas: &Canvas, x: i32, y: i32) -> [u8; 4] {
        let i = ((y * canvas.width() + x) * 4) as usize;
        [canvas.as_bytes()[i], canvas.as_bytes()[i + 1], canvas.as_bytes()[i + 2], canvas.as_bytes()[i + 3]]
    }

    #[test]
    fn a_new_canvas_is_transparent_and_correctly_sized() {
        let c = Canvas::new(10, 4);
        assert_eq!(c.as_bytes().len(), 10 * 4 * 4);
        assert!(c.as_bytes().iter().all(|&b| b == 0));
    }

    #[test]
    fn clear_writes_every_pixel() {
        let mut c = Canvas::new(3, 2);
        c.clear(palette::BASE);
        let [b, g, r, a] = to_argb(palette::BASE);
        assert_eq!(pixel(&c, 2, 1), [b, g, r, a]);
    }

    #[test]
    fn fills_are_clipped_to_the_canvas() {
        let mut c = Canvas::new(8, 8);
        // Entirely outside, and straddling the edge: neither may panic.
        c.fill_rect(Rect::new(100, 100, 10, 10), palette::TEXT);
        c.fill_rect(Rect::new(-5, -5, 8, 8), palette::TEXT);
        assert_ne!(pixel(&c, 0, 0), [0, 0, 0, 0], "the overlapping part is drawn");
        assert_eq!(pixel(&c, 7, 7), [0, 0, 0, 0], "the far corner stays untouched");
    }

    #[test]
    fn an_empty_rectangle_draws_nothing() {
        let mut c = Canvas::new(4, 4);
        c.fill_rect(Rect::new(1, 1, 0, 5), palette::TEXT);
        c.fill_rect(Rect::new(1, 1, 5, -1), palette::TEXT);
        assert!(c.as_bytes().iter().all(|&b| b == 0));
    }

    #[test]
    fn a_fully_transparent_colour_is_a_no_op() {
        let mut c = Canvas::new(2, 2);
        c.fill_rect(c.bounds(), palette::TEXT.alpha(0.0));
        assert!(c.as_bytes().iter().all(|&b| b == 0));
    }

    #[test]
    fn half_alpha_white_over_black_lands_in_the_middle() {
        let mut c = Canvas::new(1, 1);
        c.clear(spectre_theme::Color::hex(0x000000));
        c.fill_rect(c.bounds(), spectre_theme::Color::hex(0xffffff).alpha(0.5));
        let [b, g, r, a] = pixel(&c, 0, 0);
        assert_eq!(a, 255);
        for channel in [b, g, r] {
            assert!((channel as i32 - 128).abs() <= 2, "got {channel}");
        }
    }

    #[test]
    fn rectangles_intersect_and_inset_sanely() {
        let a = Rect::new(0, 0, 10, 10);
        assert_eq!(a.intersect(&Rect::new(5, 5, 10, 10)), Rect::new(5, 5, 5, 5));
        assert!(a.intersect(&Rect::new(50, 50, 1, 1)).is_empty());
        assert_eq!(a.inset(2), Rect::new(2, 2, 6, 6));
        assert!(a.inset(50).is_empty(), "an over-inset rectangle must not go negative");
    }

    #[test]
    fn contains_uses_a_half_open_range() {
        let r = Rect::new(0, 0, 4, 4);
        assert!(r.contains(0, 0));
        assert!(r.contains(3, 3));
        assert!(!r.contains(4, 0), "the right edge is exclusive");
        assert!(!r.contains(-1, 0));
    }

    #[test]
    fn resizing_reports_only_real_changes() {
        let mut c = Canvas::new(4, 4);
        assert!(!c.resize(4, 4));
        assert!(c.resize(8, 2));
        assert_eq!(c.as_bytes().len(), 8 * 2 * 4);
    }

    #[test]
    fn the_pattern_leaves_marks_but_keeps_the_background() {
        let mut c = Canvas::new(120, 32);
        let pattern = Pattern::default();
        c.fill_pattern(c.bounds(), &pattern, palette::SURFACE, palette::ACCENT_2, 0.0, 1.0);
        let bg = to_argb(palette::SURFACE);
        let bytes = c.as_bytes();
        assert!(bytes.chunks_exact(4).any(|p| p != bg), "the contour lines must be visible");
        assert!(bytes.chunks_exact(4).all(|p| p[3] == 255), "the panel must stay opaque");
    }

    #[test]
    fn a_disabled_pattern_leaves_a_flat_fill() {
        let mut c = Canvas::new(40, 8);
        c.fill_pattern(c.bounds(), &Pattern::OFF, palette::SURFACE, palette::ACCENT_2, 0.0, 1.0);
        let bg = to_argb(palette::SURFACE);
        assert!(c.as_bytes().chunks_exact(4).all(|p| p == bg));
    }

    #[test]
    fn images_drawn_off_canvas_are_clipped_not_panicked() {
        let mut c = Canvas::new(4, 4);
        let image = Image { width: 8, height: 8, data: vec![255; 8 * 8 * 4] };
        c.draw_image(-3, -3, &image);
        c.draw_image(3, 3, &image);
        c.draw_image(100, 100, &image);
        assert_ne!(pixel(&c, 0, 0), [0, 0, 0, 0]);
    }

    #[test]
    fn an_empty_image_draws_nothing() {
        let mut c = Canvas::new(4, 4);
        c.draw_image(0, 0, &Image::empty());
        assert!(c.as_bytes().iter().all(|&b| b == 0));
    }
}
