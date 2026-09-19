use spectre_draw::{Canvas, PatternMask, Rect};
use spectre_text::{FontFamily, Label, TextRenderer};
use spectre_theme::Theme;

pub const CARD_WIDTH: i32 = 420;
pub const CARD_HEIGHT: i32 = 260;
pub const FIELD_HEIGHT: i32 = 40;
pub const DOT_SIZE: i32 = 8;
pub const DOT_GAP: i32 = 6;
pub const MAX_DOTS: usize = 24;

pub struct Screen<'a> {
    pub theme: &'a Theme,
    pub mask: &'a PatternMask,
    pub color_phase: f32,
    pub time: &'a str,
    pub date: &'a str,
    pub user: &'a str,
    pub typed: usize,
    pub message: &'a str,
    pub working: bool,
}

pub fn card(width: i32, height: i32, scale: i32) -> Rect {
    let card_width = (CARD_WIDTH * scale).min(width - 40 * scale).max(120);
    let card_height = (CARD_HEIGHT * scale).min(height - 40 * scale).max(120);
    Rect::new((width - card_width) / 2, (height - card_height) / 2, card_width, card_height)
}

pub fn field(card: Rect, scale: i32) -> Rect {
    let inset = 24 * scale;
    let height = FIELD_HEIGHT * scale;
    let y = card.bottom() - inset - height;
    Rect::new(card.x + inset, y, (card.w - inset * 2).max(1), height)
}

pub fn dots(field: Rect, typed: usize, scale: i32) -> Vec<Rect> {
    let shown = typed.min(MAX_DOTS);
    let size = DOT_SIZE * scale;
    let gap = DOT_GAP * scale;
    let total = shown as i32 * size + (shown as i32 - 1).max(0) * gap;
    let start = field.x + (field.w - total).max(0) / 2;
    let y = field.y + (field.h - size) / 2;
    (0..shown)
        .map(|index| Rect::new(start + index as i32 * (size + gap), y, size, size))
        .collect()
}

pub fn draw(canvas: &mut Canvas, text: &mut TextRenderer, screen: &Screen<'_>, scale: i32) {
    let palette = &screen.theme.palette;
    let bounds = canvas.bounds();
    canvas.fill_pattern(bounds, screen.mask, palette.base, &palette.accent, screen.color_phase);

    let card = card(canvas.width(), canvas.height(), scale);
    canvas.fill_rect(card, palette.surface);
    let hairline = Rect::new(card.x, card.y, card.w, scale.max(1));
    canvas.fill_rect(hairline, palette.accent.sample(0.5));

    let logo = spectre_draw::logo((34 * scale) as u32);
    canvas.draw_image(card.x + (card.w - logo.width as i32) / 2, card.y + 22 * scale, &logo);

    let clock = Label::new(screen.time)
        .size(34.0 * scale as f32)
        .color(palette.text)
        .family(FontFamily::Monospace);
    let image = text.rasterise(&clock);
    canvas.draw_image(card.x + (card.w - image.width as i32) / 2, card.y + 66 * scale, &image);

    let under = format!("{}  ·  {}", screen.date, screen.user);
    let day = Label::new(&under).size(11.0 * scale as f32).color(palette.text_muted);
    let image = text.rasterise(&day);
    canvas.draw_image(card.x + (card.w - image.width as i32) / 2, card.y + 112 * scale, &image);

    let field = field(card, scale);
    canvas.fill_rect(field, palette.base);
    let edge = match screen.working {
        true => palette.accent.sample(0.5),
        false => palette.line,
    };
    canvas.fill_rect(Rect::new(field.x, field.bottom() - scale.max(1), field.w, scale.max(1)), edge);

    if screen.typed == 0 && !screen.working {
        let hint = Label::new("Enter your password")
            .size(12.0 * scale as f32)
            .color(palette.text_muted);
        let image = text.rasterise(&hint);
        let y = field.y + (field.h - image.height as i32) / 2;
        canvas.draw_image(field.x + (field.w - image.width as i32) / 2, y, &image);
    }
    for dot in dots(field, screen.typed, scale) {
        canvas.fill_rect(dot, palette.text_dim);
    }

    if !screen.message.is_empty() {
        let note = Label::new(screen.message)
            .size(11.0 * scale as f32)
            .color(palette.accent.sample(0.0));
        let image = text.rasterise(&note);
        let y = field.bottom() + 8 * scale;
        canvas.draw_image(field.x + (field.w - image.width as i32) / 2, y, &image);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_card_sits_in_the_middle_of_the_screen() {
        let card = card(1920, 1080, 1);
        assert_eq!(card.x + card.w / 2, 960);
        assert_eq!(card.y + card.h / 2, 540);
        assert_eq!(card.w, CARD_WIDTH);
    }

    #[test]
    fn a_tiny_screen_still_gets_a_card_that_fits() {
        let card = card(300, 200, 1);
        assert!(card.w <= 300 && card.h <= 200);
        assert!(card.w >= 120 && card.h >= 120);
    }

    #[test]
    fn the_field_stays_inside_the_card() {
        let card = card(1920, 1080, 1);
        let field = field(card, 1);
        assert!(field.x >= card.x && field.right() <= card.right());
        assert!(field.bottom() <= card.bottom());
    }

    #[test]
    fn the_dots_are_centred_and_counted() {
        let card = card(1920, 1080, 1);
        let field = field(card, 1);
        assert!(dots(field, 0, 1).is_empty());
        assert_eq!(dots(field, 5, 1).len(), 5);

        let five = dots(field, 5, 1);
        let left = five.first().expect("a dot").x;
        let right = five.last().expect("a dot").right();
        let middle = (left + right) / 2;
        assert!((middle - (field.x + field.w / 2)).abs() <= 2, "middle was {middle}");
    }

    #[test]
    fn a_very_long_password_stops_adding_dots() {
        let card = card(1920, 1080, 1);
        let field = field(card, 1);
        assert_eq!(dots(field, 500, 1).len(), MAX_DOTS);
        for dot in dots(field, 500, 1) {
            assert!(dot.x >= field.x, "a dot escaped the field");
        }
    }
}
