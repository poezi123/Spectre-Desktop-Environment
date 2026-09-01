//! Notification popup layout and painting.
//!
//! One layer surface holds the whole stack, newest at the top. Card heights
//! depend on how much text there is, so the geometry is computed once per frame
//! and reused for both drawing and hit testing.

use spectre_draw::{Canvas, Rect};
use spectre_text::{EllipsisSide, FontFamily, Label, TextRenderer};
use spectre_theme::{Color, Palette, Theme};

use crate::model::{Notification, Urgency};

/// Card width, in logical pixels.
pub const CARD_WIDTH: i32 = 380;
/// Gap between cards.
pub const CARD_GAP: i32 = 8;
/// Distance from the screen edge.
pub const SCREEN_MARGIN: i32 = 12;
/// Padding inside a card.
pub const PADDING: i32 = 12;
/// Width of the urgency bar down the leading edge.
pub const ACCENT_WIDTH: i32 = 3;

pub const APP_SIZE: f32 = 9.5;
pub const SUMMARY_SIZE: f32 = 13.0;
pub const BODY_SIZE: f32 = 11.5;
/// How many lines of body text a card will show.
pub const BODY_LINES: u16 = 3;

/// A card and the notification it shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Card {
    pub id: crate::model::Id,
    pub rect: Rect,
}

/// Measure and stack the cards, newest at the top.
///
/// `measure_body` reports how tall the body text will be at the card's inner
/// width, so a one-line body does not reserve room for three.
pub fn layout(
    notifications: &[&Notification],
    mut measure_body: impl FnMut(&str) -> i32,
) -> Vec<Card> {
    let mut cards = Vec::with_capacity(notifications.len());
    let mut y = SCREEN_MARGIN;

    for notification in notifications {
        let height = card_height(notification, &mut measure_body);
        cards.push(Card {
            id: notification.id,
            rect: Rect::new(SCREEN_MARGIN, y, CARD_WIDTH, height),
        });
        y += height + CARD_GAP;
    }
    cards
}

/// Height of one card.
fn card_height(notification: &Notification, measure_body: &mut impl FnMut(&str) -> i32) -> i32 {
    let app_line = (APP_SIZE * 1.3).ceil() as i32;
    let summary_line = (SUMMARY_SIZE * 1.3).ceil() as i32;
    let body = if notification.body.trim().is_empty() {
        0
    } else {
        measure_body(&notification.body) + 4
    };
    PADDING * 2 + app_line + 2 + summary_line + body
}

/// The surface the whole stack needs.
pub fn surface_size(cards: &[Card]) -> (i32, i32) {
    let width = CARD_WIDTH + SCREEN_MARGIN * 2;
    let height = cards
        .last()
        .map(|card| card.rect.bottom() + SCREEN_MARGIN)
        .unwrap_or(0);
    (width, height.max(0))
}

/// The card at a surface-local point.
pub fn card_at(cards: &[Card], x: i32, y: i32) -> Option<&Card> {
    cards.iter().find(|card| card.rect.contains(x, y))
}

/// The colour of a card's urgency bar.
pub fn urgency_color(urgency: Urgency, palette: &Palette) -> Color {
    match urgency {
        Urgency::Low => palette.accent.sample(0.0),
        Urgency::Normal => palette.accent.sample(0.6),
        Urgency::Critical => palette.danger,
    }
}

/// Paint the stack.
pub fn draw(
    canvas: &mut Canvas,
    text: &mut TextRenderer,
    theme: &Theme,
    notifications: &[&Notification],
    cards: &[Card],
    hovered: Option<crate::model::Id>,
    scale: f32,
) {
    let palette = &theme.palette;
    // The surface is a transparent sheet; only the cards are painted.
    canvas.clear(Color::TRANSPARENT);

    for (notification, card) in notifications.iter().zip(cards) {
        draw_card(canvas, text, palette, notification, card, hovered == Some(card.id), scale);
    }
}

fn draw_card(
    canvas: &mut Canvas,
    text: &mut TextRenderer,
    palette: &Palette,
    notification: &Notification,
    card: &Card,
    hovered: bool,
    scale: f32,
) {
    let rect = card.rect;
    canvas.fill_rect(rect, if hovered { palette.elevated } else { palette.surface });

    // A hairline frame, and the urgency bar down the leading edge.
    for edge in [
        Rect::new(rect.x, rect.y, rect.w, 1),
        Rect::new(rect.x, rect.bottom() - 1, rect.w, 1),
        Rect::new(rect.right() - 1, rect.y, 1, rect.h),
    ] {
        canvas.fill_rect(edge, palette.line);
    }
    canvas.fill_rect(
        Rect::new(rect.x, rect.y, ACCENT_WIDTH, rect.h),
        urgency_color(notification.urgency, palette),
    );

    let x = rect.x + ACCENT_WIDTH + PADDING;
    let budget = (rect.right() - PADDING - x).max(1) as u32;
    let mut y = rect.y + PADDING;

    let app = Label::new(&notification.app_name)
        .size(APP_SIZE * scale)
        .color(palette.text_muted)
        .family(FontFamily::Monospace)
        .max_width(budget);
    let app_image = text.rasterise(&app);
    canvas.draw_image(x, y, &app_image);
    y += app_image.height as i32 + 2;

    let summary = Label::new(&notification.summary)
        .size(SUMMARY_SIZE * scale)
        .color(palette.text)
        .bold(true)
        .max_width(budget);
    let summary_image = text.rasterise(&summary);
    canvas.draw_image(x, y, &summary_image);
    y += summary_image.height as i32;

    if !notification.body.trim().is_empty() {
        y += 4;
        let body = Label::new(&notification.body)
            .size(BODY_SIZE * scale)
            .color(palette.text_dim)
            .max_width(budget)
            .max_lines(BODY_LINES)
            .ellipsis(EllipsisSide::End);
        let body_image = text.rasterise(&body);
        canvas.draw_image(x, y, &body_image);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
        fn note(id: u32, body: &str, urgency: Urgency) -> Notification {
        Notification {
            id,
            app_name: "test".into(),
            summary: "Summary".into(),
            body: body.into(),
            urgency,
            expires_at: None,
        }
    }

    /// Pretend every body is one line tall.
    fn one_line(_: &str) -> i32 {
        15
    }

    #[test]
    fn an_empty_stack_needs_no_surface() {
        let cards = layout(&[], one_line);
        assert!(cards.is_empty());
        assert_eq!(surface_size(&cards).1, 0);
    }

    #[test]
    fn cards_stack_downwards_without_overlapping() {
        let notes = [note(1, "a", Urgency::Normal), note(2, "b", Urgency::Low)];
        let refs: Vec<&Notification> = notes.iter().collect();
        let cards = layout(&refs, one_line);

        assert_eq!(cards.len(), 2);
        assert_eq!(cards[0].rect.y, SCREEN_MARGIN);
        assert_eq!(cards[1].rect.y, cards[0].rect.bottom() + CARD_GAP);
        assert!(cards[0].rect.bottom() <= cards[1].rect.y);
    }

    #[test]
    fn the_surface_encloses_every_card() {
        let notes = [note(1, "a", Urgency::Normal), note(2, "b", Urgency::Normal)];
        let refs: Vec<&Notification> = notes.iter().collect();
        let cards = layout(&refs, one_line);
        let (width, height) = surface_size(&cards);

        for card in &cards {
            assert!(card.rect.right() <= width);
            assert!(card.rect.bottom() <= height);
        }
    }

    #[test]
    fn a_notification_without_a_body_gets_a_shorter_card() {
        let with = note(1, "some body text", Urgency::Normal);
        let without = note(2, "", Urgency::Normal);
        let refs = [&with, &without];
        let cards = layout(&refs, one_line);
        assert!(cards[1].rect.h < cards[0].rect.h);
    }

    #[test]
    fn a_body_of_only_whitespace_counts_as_none() {
        let blank = note(1, "   \n  ", Urgency::Normal);
        let empty = note(2, "", Urgency::Normal);
        let refs = [&blank, &empty];
        let cards = layout(&refs, one_line);
        assert_eq!(cards[0].rect.h, cards[1].rect.h);
    }

    #[test]
    fn a_taller_body_makes_a_taller_card() {
        let note = note(1, "long", Urgency::Normal);
        let refs = [&note];
        let short = layout(&refs, |_| 15)[0].rect.h;
        let tall = layout(&refs, |_| 45)[0].rect.h;
        assert_eq!(tall - short, 30);
    }

    #[test]
    fn hit_testing_finds_the_card_that_was_drawn() {
        let notes = [note(7, "a", Urgency::Normal), note(9, "b", Urgency::Normal)];
        let refs: Vec<&Notification> = notes.iter().collect();
        let cards = layout(&refs, one_line);

        for card in &cards {
            let hit = card_at(&cards, card.rect.x + 5, card.rect.y + 5);
            assert_eq!(hit.map(|c| c.id), Some(card.id));
        }
        // The gap between cards belongs to nobody.
        assert!(card_at(&cards, cards[0].rect.x + 5, cards[0].rect.bottom() + 2).is_none());
        assert!(card_at(&cards, 0, 0).is_none());
    }

    #[test]
    fn urgency_is_visible_in_the_colour() {
        let p = Palette::default();
        let low = urgency_color(Urgency::Low, &p);
        let normal = urgency_color(Urgency::Normal, &p);
        let critical = urgency_color(Urgency::Critical, &p);
        assert_ne!(low, normal);
        assert_eq!(critical, p.danger, "a critical alert must read as an alert");
    }

    #[test]
    fn cards_are_painted_and_the_sheet_around_them_is_not() {
        let notes = [note(1, "body", Urgency::Normal)];
        let refs: Vec<&Notification> = notes.iter().collect();
        let cards = layout(&refs, one_line);
        let (width, height) = surface_size(&cards);

        let mut canvas = Canvas::new(width, height);
        let mut text = TextRenderer::new();
        draw(&mut canvas, &mut text, &Theme::default(), &refs, &cards, None, 1.0);

        let pixel = |x: i32, y: i32| -> [u8; 4] {
            let i = ((y * width + x) * 4) as usize;
            canvas.as_bytes()[i..i + 4].try_into().unwrap()
        };
        assert_eq!(pixel(0, 0), [0, 0, 0, 0], "the margin must stay transparent");
        let inside = cards[0].rect;
        assert_ne!(pixel(inside.x + 1, inside.y + 4), [0, 0, 0, 0], "the card must be painted");
    }
}
