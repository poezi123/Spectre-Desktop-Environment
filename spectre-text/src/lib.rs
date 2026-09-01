//! Text for the desktop shell.
//!
//! Everything Spectre draws that is not a client surface goes through here:
//! window title captions, panel labels, the clock, the launcher. The output is
//! always a premultiplied RGBA byte buffer, which is what both the compositor's
//! GLES renderer and a software fallback can upload directly.
//!
//! ```no_run
//! use spectre_text::{Label, TextRenderer};
//! use spectre_theme::palette;
//!
//! let mut renderer = TextRenderer::new();
//! let label = Label::new("garuda@spectre: ~").size(13.0).color(palette::TEXT);
//! let image = renderer.rasterise(&label);
//! assert!(image.width > 0);
//! ```

use cosmic_text::{
    Attrs, Buffer, Ellipsize, EllipsizeHeightLimit, Family, FontSystem, Metrics, Shaping,
    SwashCache, Weight, Wrap,
};
use spectre_theme::Color;

/// A rasterised run of text: premultiplied RGBA, top-left origin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    /// `width * height * 4` bytes, premultiplied RGBA.
    pub data: Vec<u8>,
}

impl Image {
    /// A zero-sized image, returned for empty text.
    pub fn empty() -> Self {
        Self { width: 0, height: 0, data: Vec::new() }
    }

    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }

    /// Bytes per row.
    pub fn stride(&self) -> usize {
        self.width as usize * 4
    }
}

/// Which font family to shape with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FontFamily {
    /// The system UI font. Used for title captions and panel labels.
    #[default]
    SansSerif,
    /// Used where columns have to line up: the clock, resource readouts.
    Monospace,
}

impl FontFamily {
    fn to_attrs(self) -> Family<'static> {
        match self {
            FontFamily::SansSerif => Family::SansSerif,
            FontFamily::Monospace => Family::Monospace,
        }
    }
}

/// One run of text to draw.
#[derive(Debug, Clone)]
pub struct Label<'a> {
    pub text: &'a str,
    /// Font size in device pixels. Callers scale this by the output scale.
    pub size_px: f32,
    pub color: Color,
    pub family: FontFamily,
    /// `true` renders semibold, used for the focused window's caption.
    pub bold: bool,
    /// Truncate with an ellipsis beyond this many pixels. `None` never truncates.
    pub max_width: Option<u32>,
    /// Where the ellipsis goes when the text is truncated.
    pub ellipsis: EllipsisSide,
}

/// Which end of the text to drop when it does not fit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EllipsisSide {
    /// `A very long window ti…` - the right choice for window captions.
    #[default]
    End,
    /// `…ong/window/title` - better for paths, where the tail identifies the item.
    Start,
    /// `A very…dow title`.
    Middle,
}

impl EllipsisSide {
    fn to_cosmic(self) -> Ellipsize {
        // One line: title bars and panel labels never wrap.
        let limit = EllipsizeHeightLimit::Lines(1);
        match self {
            EllipsisSide::End => Ellipsize::End(limit),
            EllipsisSide::Start => Ellipsize::Start(limit),
            EllipsisSide::Middle => Ellipsize::Middle(limit),
        }
    }
}

impl<'a> Label<'a> {
    pub fn new(text: &'a str) -> Self {
        Self {
            text,
            size_px: 13.0,
            color: spectre_theme::palette::TEXT,
            family: FontFamily::default(),
            bold: false,
            max_width: None,
            ellipsis: EllipsisSide::default(),
        }
    }

    pub fn size(mut self, size_px: f32) -> Self {
        self.size_px = size_px;
        self
    }

    pub fn color(mut self, color: Color) -> Self {
        self.color = color;
        self
    }

    pub fn family(mut self, family: FontFamily) -> Self {
        self.family = family;
        self
    }

    pub fn bold(mut self, bold: bool) -> Self {
        self.bold = bold;
        self
    }

    pub fn max_width(mut self, max_width: u32) -> Self {
        self.max_width = Some(max_width);
        self
    }

    pub fn ellipsis(mut self, side: EllipsisSide) -> Self {
        self.ellipsis = side;
        self
    }

    fn line_height(&self) -> f32 {
        // 1.3 is the ratio the concept renders use: tight enough for a 32px
        // title bar, loose enough that descenders are not clipped.
        (self.size_px * 1.3).ceil()
    }
}

/// Owns the font database and the glyph cache.
///
/// Construction scans the system fonts, which takes long enough that it must
/// happen once at start-up rather than per frame. Everything after that is
/// cached.
pub struct TextRenderer {
    font_system: FontSystem,
    swash_cache: SwashCache,
}

impl Default for TextRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for TextRenderer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TextRenderer").finish_non_exhaustive()
    }
}

impl TextRenderer {
    pub fn new() -> Self {
        Self { font_system: FontSystem::new(), swash_cache: SwashCache::new() }
    }

    /// Build a renderer over an explicit font set, for tests and for systems
    /// where scanning `/usr/share/fonts` is not wanted.
    pub fn with_fonts(fonts: impl IntoIterator<Item = Vec<u8>>) -> Self {
        let db = cosmic_text::fontdb::Database::new();
        let mut font_system = FontSystem::new_with_locale_and_db("en-US".into(), db);
        for font in fonts {
            font_system.db_mut().load_font_data(font);
        }
        Self { font_system, swash_cache: SwashCache::new() }
    }

    /// Width and height the label will occupy, without rasterising it.
    pub fn measure(&mut self, label: &Label<'_>) -> (u32, u32) {
        if label.text.is_empty() {
            return (0, 0);
        }
        let (buffer, width, height) = self.shape(label);
        drop(buffer);
        (width, height)
    }

    /// Rasterise the label into a premultiplied RGBA image.
    ///
    /// Empty text yields [`Image::empty`] rather than a 1x1 transparent pixel,
    /// so callers can skip the upload entirely.
    pub fn rasterise(&mut self, label: &Label<'_>) -> Image {
        if label.text.is_empty() || label.size_px <= 0.0 {
            return Image::empty();
        }

        let (mut buffer, width, height) = self.shape(label);
        if width == 0 || height == 0 {
            return Image::empty();
        }

        let mut data = vec![0u8; width as usize * height as usize * 4];
        let [tr, tg, tb, ta] = label.color.to_rgba8();
        let text_color = cosmic_text::Color::rgba(tr, tg, tb, ta);

        buffer.draw(
            &mut self.font_system,
            &mut self.swash_cache,
            text_color,
            |x, y, w, h, color| {
                let (r, g, b, a) = (color.r(), color.g(), color.b(), color.a());
                if a == 0 {
                    return;
                }
                for dy in 0..h as i32 {
                    for dx in 0..w as i32 {
                        let px = x + dx;
                        let py = y + dy;
                        if px < 0 || py < 0 || px >= width as i32 || py >= height as i32 {
                            continue;
                        }
                        let idx = (py as usize * width as usize + px as usize) * 4;
                        // Source-over onto whatever is already there, in
                        // premultiplied space. Glyph runs can overlap on
                        // scripts with marks, so this cannot just overwrite.
                        let sa = a as u32;
                        let inv = 255 - sa;
                        let blend = |dst: u8, src: u8| -> u8 {
                            ((src as u32 * sa + dst as u32 * inv) / 255) as u8
                        };
                        data[idx] = blend(data[idx], r);
                        data[idx + 1] = blend(data[idx + 1], g);
                        data[idx + 2] = blend(data[idx + 2], b);
                        data[idx + 3] = (sa + (data[idx + 3] as u32 * inv) / 255).min(255) as u8;
                    }
                }
            },
        );

        Image { width, height, data }
    }

    /// Shape the label and report the pixel box it needs.
    fn shape(&mut self, label: &Label<'_>) -> (Buffer, u32, u32) {
        let metrics = Metrics::new(label.size_px, label.line_height());
        let mut buffer = Buffer::new(&mut self.font_system, metrics);
        let weight = if label.bold { Weight::SEMIBOLD } else { Weight::NORMAL };
        let attrs = Attrs::new().family(label.family.to_attrs()).weight(weight);

        // Title bars and panel labels never wrap; overflow is ellipsised
        // instead, which cosmic-text does during layout so the shaping stays a
        // single pass even for a caption that has to be cut.
        buffer.set_wrap(Wrap::None);
        if label.max_width.is_some() {
            buffer.set_ellipsize(label.ellipsis.to_cosmic());
        }
        buffer.set_size(label.max_width.map(|w| w as f32), Some(label.line_height()));
        buffer.set_text(label.text, &attrs, Shaping::Advanced, None);
        buffer.shape_until_scroll(&mut self.font_system, false);

        let width = buffer
            .layout_runs()
            .map(|run| run.line_w.ceil() as u32)
            .max()
            .unwrap_or(0);
        let width = match label.max_width {
            Some(max) => width.min(max),
            None => width,
        };
        (buffer, width, label.line_height() as u32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_text_rasterises_to_nothing() {
        let mut r = TextRenderer::with_fonts(Vec::<Vec<u8>>::new());
        let image = r.rasterise(&Label::new(""));
        assert!(image.is_empty());
        assert_eq!(r.measure(&Label::new("")), (0, 0));
    }

    #[test]
    fn a_zero_size_label_draws_nothing() {
        let mut r = TextRenderer::with_fonts(Vec::<Vec<u8>>::new());
        assert!(r.rasterise(&Label::new("x").size(0.0)).is_empty());
    }

    #[test]
    fn builder_defaults_match_the_documented_values() {
        let l = Label::new("hello");
        assert_eq!(l.size_px, 13.0);
        assert_eq!(l.family, FontFamily::SansSerif);
        assert!(!l.bold);
        assert_eq!(l.max_width, None);
    }

    #[test]
    fn line_height_leaves_room_for_descenders() {
        let l = Label::new("gjpqy").size(10.0);
        assert!(l.line_height() > 10.0, "a line box the size of the font clips descenders");
    }

    #[test]
    fn ellipsis_sides_map_to_a_single_line_limit() {
        for side in [EllipsisSide::End, EllipsisSide::Start, EllipsisSide::Middle] {
            let e = side.to_cosmic();
            assert!(!matches!(e, Ellipsize::None), "{side:?} must actually ellipsise");
        }
    }

    #[test]
    fn a_label_without_a_width_budget_never_ellipsises() {
        assert_eq!(Label::new("a very long window title").max_width, None);
    }

    /// Needs a real font, so it is skipped on a machine without one rather
    /// than failing: the crate must still be testable in a bare container.
    #[test]
    fn real_text_rasterises_to_visible_pixels() {
        let mut r = TextRenderer::new();
        let label = Label::new("Spectre").size(14.0);
        let (w, h) = r.measure(&label);
        if w == 0 {
            eprintln!("no system font available; skipping");
            return;
        }
        assert!(h >= 14);
        let image = r.rasterise(&label);
        assert_eq!(image.data.len(), image.stride() * image.height as usize);
        assert!(image.data.iter().any(|&b| b != 0), "the glyphs must leave marks");
    }

    #[test]
    fn a_long_caption_is_clipped_to_its_budget() {
        let mut r = TextRenderer::new();
        let text = "A very long window title that will not fit anywhere at all";
        if r.measure(&Label::new(text).size(14.0)).0 == 0 {
            eprintln!("no system font available; skipping");
            return;
        }
        let narrow = Label::new(text).size(14.0).max_width(80);
        assert!(r.measure(&narrow).0 <= 80);
    }

    #[test]
    fn stride_matches_the_width() {
        let img = Image { width: 7, height: 2, data: vec![0; 7 * 2 * 4] };
        assert_eq!(img.stride(), 28);
        assert_eq!(img.data.len(), img.stride() * img.height as usize);
    }
}
