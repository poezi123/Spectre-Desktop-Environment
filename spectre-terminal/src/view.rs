use alacritty_terminal::index::Point;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::vte::ansi::{Color as AnsiColor, NamedColor};
use spectre_draw::{Canvas, Rect};
use spectre_text::{FontFamily, Label, TextRenderer};
use spectre_theme::{Color, Palette, Theme};

use crate::session::Session;

pub const PADDING: i32 = 10;

const CUBE: [u8; 6] = [0, 95, 135, 175, 215, 255];

pub struct Metrics {
    pub cell_width: i32,
    pub cell_height: i32,
    pub font_px: f32,
}

impl Metrics {
    pub fn measure(text: &mut TextRenderer, font_px: f32) -> Self {
        let sample = Label::new("MMMMMMMMMM").size(font_px).family(FontFamily::Monospace);
        let (width, height) = text.measure(&sample);
        let cell_width = ((width as f32 / 10.0).ceil() as i32).max(1);
        let cell_height = (height as i32).max(1);
        Self { cell_width, cell_height, font_px }
    }

    pub fn columns(&self, width: i32) -> usize {
        (((width - PADDING * 2) / self.cell_width).max(1)) as usize
    }

    pub fn lines(&self, height: i32) -> usize {
        (((height - PADDING * 2) / self.cell_height).max(1)) as usize
    }
}

pub fn draw(
    canvas: &mut Canvas,
    text: &mut TextRenderer,
    session: &Session,
    theme: &Theme,
    metrics: &Metrics,
) {
    let palette = &theme.palette;
    canvas.clear(palette.base);

    let content = session.term.renderable_content();
    let offset = content.display_offset as i32;
    let selection = content.selection;
    let cursor = content.cursor.point;
    let mut under_cursor = ' ';
    let mut run = Run::new();

    for indexed in content.display_iter {
        let cell = indexed.cell;
        let point = indexed.point;
        if point == cursor {
            under_cursor = cell.c;
        }
        let mut foreground = resolve(cell.fg, palette, palette.text);
        let mut background = resolve(cell.bg, palette, palette.base);
        let picked = match selection {
            Some(range) => range.contains(point),
            None => false,
        };
        if picked {
            std::mem::swap(&mut foreground, &mut background);
        }
        let style = Style {
            line: point.line.0 + offset,
            foreground,
            background,
            bold: cell.flags.contains(Flags::BOLD),
        };
        if !run.accepts(&style, point.column.0) {
            run.flush(canvas, text, metrics);
            run.start(style, point.column.0);
        }
        run.push(cell.c);
    }
    run.flush(canvas, text, metrics);
    draw_cursor(canvas, text, cursor, offset, under_cursor, palette, metrics);
}

fn draw_cursor(
    canvas: &mut Canvas,
    text: &mut TextRenderer,
    cursor: Point,
    offset: i32,
    glyph: char,
    palette: &Palette,
    metrics: &Metrics,
) {
    let row = cursor.line.0 + offset;
    if row < 0 {
        return;
    }
    let x = PADDING + cursor.column.0 as i32 * metrics.cell_width;
    let y = PADDING + row * metrics.cell_height;
    let block = Rect::new(x, y, metrics.cell_width, metrics.cell_height);
    canvas.fill_rect(block, palette.accent.sample(0.5));

    if glyph == ' ' || glyph == '\0' {
        return;
    }
    let mut letters = String::new();
    letters.push(glyph);
    let label = Label::new(&letters)
        .size(metrics.font_px)
        .family(FontFamily::Monospace)
        .color(palette.base);
    let image = text.rasterise(&label);
    canvas.draw_image(x, y, &image);
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Style {
    line: i32,
    foreground: Color,
    background: Color,
    bold: bool,
}

struct Run {
    style: Option<Style>,
    column: usize,
    text: String,
}

impl Run {
    fn new() -> Self {
        Self { style: None, column: 0, text: String::new() }
    }

    fn accepts(&self, style: &Style, column: usize) -> bool {
        let Some(current) = self.style.as_ref() else {
            return false;
        };
        current == style && self.column + self.text.chars().count() == column
    }

    fn start(&mut self, style: Style, column: usize) {
        self.style = Some(style);
        self.column = column;
        self.text.clear();
    }

    fn push(&mut self, glyph: char) {
        self.text.push(glyph);
    }

    fn flush(&mut self, canvas: &mut Canvas, text: &mut TextRenderer, metrics: &Metrics) {
        let Some(style) = self.style else {
            return;
        };
        if self.text.is_empty() || style.line < 0 {
            self.text.clear();
            return;
        }

        let x = PADDING + self.column as i32 * metrics.cell_width;
        let y = PADDING + style.line * metrics.cell_height;
        let width = self.text.chars().count() as i32 * metrics.cell_width;
        canvas.fill_rect(Rect::new(x, y, width, metrics.cell_height), style.background);

        if self.text.trim().is_empty() {
            self.text.clear();
            return;
        }
        let label = Label::new(&self.text)
            .size(metrics.font_px)
            .family(FontFamily::Monospace)
            .color(style.foreground)
            .bold(style.bold);
        let image = text.rasterise(&label);
        canvas.draw_image(x, y, &image);
        self.text.clear();
    }
}

fn resolve(color: AnsiColor, palette: &Palette, fallback: Color) -> Color {
    match color {
        AnsiColor::Named(named) => named_color(named, palette, fallback),
        AnsiColor::Spec(rgb) => from_rgb(rgb.r, rgb.g, rgb.b),
        AnsiColor::Indexed(index) => indexed_color(index, palette, fallback),
    }
}

fn named_color(named: NamedColor, palette: &Palette, fallback: Color) -> Color {
    match named {
        NamedColor::Foreground => palette.text,
        NamedColor::Background => palette.base,
        NamedColor::Cursor => palette.text,
        NamedColor::Black => Color::hex(0x1a1a20),
        NamedColor::Red => Color::hex(0xe06c75),
        NamedColor::Green => Color::hex(0x8fbf7f),
        NamedColor::Yellow => Color::hex(0xd8c58a),
        NamedColor::Blue => Color::hex(0x7aa2f7),
        NamedColor::Magenta => Color::hex(0xc678dd),
        NamedColor::Cyan => Color::hex(0x6bc7c7),
        NamedColor::White => Color::hex(0xc8c8d0),
        NamedColor::BrightBlack => Color::hex(0x565662),
        NamedColor::BrightRed => Color::hex(0xff8b94),
        NamedColor::BrightGreen => Color::hex(0xa9d89a),
        NamedColor::BrightYellow => Color::hex(0xf0dca6),
        NamedColor::BrightBlue => Color::hex(0x9ab8ff),
        NamedColor::BrightMagenta => Color::hex(0xdb9bef),
        NamedColor::BrightCyan => Color::hex(0x8fe1e1),
        NamedColor::BrightWhite => Color::hex(0xe6e6ec),
        _ => fallback,
    }
}

fn indexed_color(index: u8, palette: &Palette, fallback: Color) -> Color {
    if index < 16 {
        return named_color(base_name(index), palette, fallback);
    }
    if index < 232 {
        let step = index - 16;
        let red = CUBE[(step / 36) as usize];
        let green = CUBE[((step % 36) / 6) as usize];
        let blue = CUBE[(step % 6) as usize];
        return from_rgb(red, green, blue);
    }
    let level = 8u16 + (index as u16 - 232) * 10;
    let level = level.min(255) as u8;
    from_rgb(level, level, level)
}

fn base_name(index: u8) -> NamedColor {
    match index {
        0 => NamedColor::Black,
        1 => NamedColor::Red,
        2 => NamedColor::Green,
        3 => NamedColor::Yellow,
        4 => NamedColor::Blue,
        5 => NamedColor::Magenta,
        6 => NamedColor::Cyan,
        7 => NamedColor::White,
        8 => NamedColor::BrightBlack,
        9 => NamedColor::BrightRed,
        10 => NamedColor::BrightGreen,
        11 => NamedColor::BrightYellow,
        12 => NamedColor::BrightBlue,
        13 => NamedColor::BrightMagenta,
        14 => NamedColor::BrightCyan,
        _ => NamedColor::BrightWhite,
    }
}

fn from_rgb(red: u8, green: u8, blue: u8) -> Color {
    let packed = ((red as u32) << 16) | ((green as u32) << 8) | blue as u32;
    Color::hex(packed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metrics() -> Metrics {
        Metrics { cell_width: 9, cell_height: 18, font_px: 14.0 }
    }

    #[test]
    fn the_grid_fits_the_window_minus_the_padding() {
        let m = metrics();
        assert_eq!(m.columns(9 * 80 + PADDING * 2), 80);
        assert_eq!(m.lines(18 * 24 + PADDING * 2), 24);
    }

    #[test]
    fn a_tiny_window_still_has_one_cell() {
        let m = metrics();
        assert_eq!(m.columns(1), 1);
        assert_eq!(m.lines(1), 1);
    }

    #[test]
    fn the_colour_cube_matches_the_usual_one() {
        let palette = Palette::default();
        let grey = indexed_color(244, &palette, palette.text);
        assert_eq!(grey.to_rgba8()[0], grey.to_rgba8()[2], "the grey ramp has no hue");
        let red = indexed_color(196, &palette, palette.text);
        let [r, g, b, _] = red.to_rgba8();
        assert_eq!((r, g, b), (255, 0, 0));
    }
}
