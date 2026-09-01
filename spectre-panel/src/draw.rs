//! Painting the panel.
//!
//! Takes the layout's rectangles and turns them into pixels. Nothing here
//! decides *where* anything goes - that is [`crate::layout`] - so a change to
//! the look cannot move a click target.

use spectre_text::{EllipsisSide, Label, TextRenderer};
use spectre_theme::{Color, Palette, Pattern, Theme};

use crate::canvas::{Canvas, Rect};
use crate::layout::{Item, Placed, CHIP_PADDING};

/// Font size for panel labels, in logical pixels.
pub const LABEL_SIZE: f32 = 12.0;
/// Font size for the clock's date line.
pub const DATE_SIZE: f32 = 9.0;
/// Thickness of the accent underline beneath the active workspace.
pub const UNDERLINE: i32 = 2;

/// Glyph used where an icon theme will eventually go. U+25C8 is in every
/// mainstream font; the power symbol is not, so that one is drawn by hand.
pub const LAUNCHER_GLYPH: &str = "\u{25c8}";

/// Everything the panel needs to draw itself that is not in the layout.
pub struct Frame<'a> {
    pub theme: &'a Theme,
    /// Panel-local pointer position, `None` when the pointer is elsewhere.
    pub pointer: Option<(i32, i32)>,
    /// `HH:MM`.
    pub time: &'a str,
    /// The line under the clock.
    pub date: &'a str,
    /// The CPU and memory line.
    pub resources: &'a str,
    pub pattern_phase: f32,
    pub scale: f32,
}

/// Paint the whole panel.
pub fn draw(canvas: &mut Canvas, text: &mut TextRenderer, items: &[Placed], frame: &Frame<'_>) {
    let palette = &frame.theme.palette;
    let bounds = canvas.bounds();

    canvas.fill_pattern(
        bounds,
        &panel_pattern(frame.theme),
        palette.surface,
        palette.accent.sample(0.5),
        frame.pattern_phase,
        frame.scale,
    );

    // A hairline along the top edge separates the panel from the desktop even
    // with every effect switched off.
    canvas.fill_rect(Rect::new(0, 0, bounds.w, 1), palette.line);

    for placed in items {
        let hovered = frame
            .pointer
            .is_some_and(|(x, y)| placed.rect.contains(x, y) && placed.item.is_interactive());
        draw_item(canvas, text, placed, hovered, frame);
    }
}

fn panel_pattern(theme: &Theme) -> Pattern {
    // The panel is a thin strip: the same line spacing that reads well on a
    // title bar reads well here, so it shares the window pattern outright.
    theme.panel_pattern
}

fn draw_item(
    canvas: &mut Canvas,
    text: &mut TextRenderer,
    placed: &Placed,
    hovered: bool,
    frame: &Frame<'_>,
) {
    let palette = &frame.theme.palette;
    let rect = placed.rect;

    match &placed.item {
        Item::Launcher => {
            plate(canvas, rect, hovered, palette);
            centre_label(canvas, text, rect, &Label::new(LAUNCHER_GLYPH)
                .size(LABEL_SIZE + 2.0)
                .color(palette.accent.sample(0.35)));
        }
        Item::Session => {
            plate(canvas, rect, hovered, palette);
            let color = if hovered { palette.danger_hover } else { palette.text_dim };
            power_icon(canvas, rect, color);
        }
        Item::Workspace { index, active, occupied } => {
            plate(canvas, rect, hovered, palette);
            let color = match (active, occupied) {
                (true, _) => palette.text,
                (false, true) => palette.text_dim,
                (false, false) => palette.text_muted,
            };
            centre_label(
                canvas,
                text,
                rect,
                &Label::new(&index.to_string()).size(LABEL_SIZE).color(color).bold(*active),
            );
            if *active {
                accent_underline(canvas, rect, palette);
            } else if *occupied {
                // A dim pip marks a workspace that has windows but is not shown.
                let dot = Rect::new(rect.x + rect.w / 2 - 1, rect.bottom() - UNDERLINE - 2, 2, 2);
                canvas.fill_rect(dot, palette.text_muted);
            }
        }
        Item::Task { title, focused, minimized, .. } => {
            plate(canvas, rect, hovered, palette);
            let color = match (focused, minimized) {
                (true, _) => palette.text,
                (false, true) => palette.text_muted,
                (false, false) => palette.text_dim,
            };
            let budget = (rect.w - CHIP_PADDING).max(1) as u32;
            let label = Label::new(title)
                .size(LABEL_SIZE)
                .color(color)
                .bold(*focused)
                .max_width(budget)
                .ellipsis(EllipsisSide::End);
            let image = text.rasterise(&label);
            let x = rect.x + CHIP_PADDING / 2;
            let y = rect.y + (rect.h - image.height as i32) / 2;
            canvas.draw_image(x, y, &image);
            if *focused {
                accent_underline(canvas, rect, palette);
            }
        }
        Item::Resources => {
            centre_label(
                canvas,
                text,
                rect,
                &Label::new(frame.resources)
                    .size(DATE_SIZE + 1.0)
                    .color(palette.text_dim)
                    .family(spectre_text::FontFamily::Monospace),
            );
        }
        Item::Clock => {
            // Time over date, both monospace so the panel does not twitch as
            // the digits change.
            let time = Label::new(frame.time)
                .size(LABEL_SIZE)
                .color(palette.text)
                .family(spectre_text::FontFamily::Monospace);
            let date = Label::new(frame.date)
                .size(DATE_SIZE)
                .color(palette.text_muted)
                .family(spectre_text::FontFamily::Monospace);

            let time_image = text.rasterise(&time);
            let date_image = text.rasterise(&date);
            let total = time_image.height as i32 + date_image.height as i32;
            let mut y = rect.y + (rect.h - total) / 2;

            let x = rect.x + (rect.w - time_image.width as i32) / 2;
            canvas.draw_image(x, y, &time_image);
            y += time_image.height as i32;
            let x = rect.x + (rect.w - date_image.width as i32) / 2;
            canvas.draw_image(x, y, &date_image);
        }
    }
}

/// A power symbol: a broken ring with a stroke through the gap.
///
/// Drawn rather than typeset because U+23FB is missing from most font stacks -
/// including the one Garuda ships - and a session button that renders as a
/// blank box is worse than one that is a few rectangles.
fn power_icon(canvas: &mut Canvas, rect: Rect, color: Color) {
    let size = rect.w.min(rect.h).clamp(8, 16);
    let cx = rect.x + rect.w / 2;
    let cy = rect.y + rect.h / 2;
    let radius = size / 2;

    // The ring, as a circle of single-pixel steps, with a gap at the top.
    let steps = (radius * 8).max(24);
    for i in 0..steps {
        let angle = std::f32::consts::TAU * i as f32 / steps as f32;
        // Leave the top eighth open, where the stroke goes.
        let from_top = (angle - std::f32::consts::FRAC_PI_2 * 3.0).abs();
        if from_top < 0.45 {
            continue;
        }
        let x = cx + (angle.cos() * radius as f32).round() as i32;
        let y = cy + (angle.sin() * radius as f32).round() as i32;
        canvas.fill_rect(Rect::new(x, y, 1, 1), color);
    }

    // The stroke.
    canvas.fill_rect(Rect::new(cx, cy - radius - 1, 1, radius + 1), color);
}

/// The hover plate behind an interactive item.
fn plate(canvas: &mut Canvas, rect: Rect, hovered: bool, palette: &Palette) {
    if hovered {
        canvas.fill_rect(rect.inset(2), palette.overlay);
    }
}

/// The accent bar marking the active workspace or focused task.
fn accent_underline(canvas: &mut Canvas, rect: Rect, palette: &Palette) {
    let y = rect.bottom() - UNDERLINE;
    let steps = rect.w.clamp(1, 24);
    for i in 0..steps {
        let start = rect.x + (rect.w * i) / steps;
        let end = rect.x + (rect.w * (i + 1)) / steps;
        if end <= start {
            continue;
        }
        let t = if steps == 1 { 0.5 } else { i as f32 / (steps - 1) as f32 };
        canvas.fill_rect(Rect::new(start, y, end - start, UNDERLINE), palette.accent.sample(t));
    }
}

fn centre_label(canvas: &mut Canvas, text: &mut TextRenderer, rect: Rect, label: &Label<'_>) {
    let image = text.rasterise(label);
    let x = rect.x + (rect.w - image.width as i32) / 2;
    let y = rect.y + (rect.h - image.height as i32) / 2;
    canvas.draw_image(x, y, &image);
}

#[cfg(test)]
mod tests {
    use super::*;
    use spectre_theme::palette;

    fn frame<'a>(theme: &'a Theme) -> Frame<'a> {
        Frame {
            theme,
            pointer: None,
            time: "03:04",
            date: "01.09.26",
            resources: "CPU  4%  MEM 1.2G",
            pattern_phase: 0.0,
            scale: 1.0,
        }
    }

    fn painted(canvas: &Canvas) -> usize {
        canvas.as_bytes().chunks_exact(4).filter(|p| p[3] != 0).count()
    }

    #[test]
    fn an_empty_panel_is_still_filled_and_opaque() {
        let theme = Theme::default();
        let mut canvas = Canvas::new(400, 32);
        let mut text = TextRenderer::new();
        draw(&mut canvas, &mut text, &[], &frame(&theme));
        assert_eq!(painted(&canvas), 400 * 32, "every pixel must be opaque");
    }

    #[test]
    fn the_top_hairline_is_drawn() {
        let theme = Theme::default();
        let mut canvas = Canvas::new(40, 32);
        let mut text = TextRenderer::new();
        draw(&mut canvas, &mut text, &[], &frame(&theme));
        let top = &canvas.as_bytes()[0..4];
        let below = &canvas.as_bytes()[(40 * 4 * 4)..(40 * 4 * 4 + 4)];
        assert_ne!(top, below, "the separator must be distinguishable from the panel");
    }

    #[test]
    fn the_active_workspace_gets_an_accent_underline() {
        let theme = Theme::default();
        let mut text = TextRenderer::new();
        let rect = Rect::new(0, 0, 26, 32);

        let mut active = Canvas::new(26, 32);
        active.clear(palette::SURFACE);
        draw_item(
            &mut active,
            &mut text,
            &Placed { item: Item::Workspace { index: 1, active: true, occupied: true }, rect },
            false,
            &frame(&theme),
        );

        let mut inactive = Canvas::new(26, 32);
        inactive.clear(palette::SURFACE);
        draw_item(
            &mut inactive,
            &mut text,
            &Placed { item: Item::Workspace { index: 1, active: false, occupied: true }, rect },
            false,
            &frame(&theme),
        );

        let bottom_row = |c: &Canvas| -> Vec<u8> {
            let start = ((32 - 1) * 26 * 4) as usize;
            c.as_bytes()[start..start + 26 * 4].to_vec()
        };
        assert_ne!(bottom_row(&active), bottom_row(&inactive));
    }

    #[test]
    fn hovering_lights_an_item_up() {
        let theme = Theme::default();
        let mut text = TextRenderer::new();
        let rect = Rect::new(0, 0, 30, 32);
        let placed = Placed { item: Item::Launcher, rect };

        let mut plain = Canvas::new(30, 32);
        plain.clear(palette::SURFACE);
        draw_item(&mut plain, &mut text, &placed, false, &frame(&theme));

        let mut lit = Canvas::new(30, 32);
        lit.clear(palette::SURFACE);
        draw_item(&mut lit, &mut text, &placed, true, &frame(&theme));

        assert_ne!(plain.as_bytes(), lit.as_bytes());
    }

    #[test]
    fn drawing_never_writes_outside_the_canvas() {
        let theme = Theme::default();
        let mut canvas = Canvas::new(20, 32);
        let mut text = TextRenderer::new();
        // An item deliberately hanging off both ends.
        let items = [
            Placed { item: Item::Launcher, rect: Rect::new(-40, 0, 30, 32) },
            Placed { item: Item::Clock, rect: Rect::new(500, 0, 60, 32) },
        ];
        draw(&mut canvas, &mut text, &items, &frame(&theme));
        assert_eq!(canvas.as_bytes().len(), 20 * 32 * 4);
    }
}
