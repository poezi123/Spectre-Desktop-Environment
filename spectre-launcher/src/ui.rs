//! Launcher layout and painting.
//!
//! The geometry is a pure function of the window size and the number of
//! results, so the row a click lands on and the row that was drawn are always
//! the same row.

use spectre_draw::{Canvas, Rect};
use spectre_text::{EllipsisSide, Label, TextRenderer};
use spectre_theme::{Palette, Pattern, Theme};

use crate::category::Category;
use crate::entry::Entry;

/// Height of the search field.
pub const SEARCH_HEIGHT: i32 = 46;
/// Height of one result row.
pub const ROW_HEIGHT: i32 = 40;
/// Padding inside the launcher window.
pub const PADDING: i32 = 10;
/// Font size of a result's name and of the query.
pub const NAME_SIZE: f32 = 14.0;
/// Font size of a result's description.
pub const COMMENT_SIZE: f32 = 10.5;
/// Width of the accent bar marking the selected row.
pub const MARKER_WIDTH: i32 = 3;
/// Width of the category column.
pub const SIDEBAR_WIDTH: i32 = 176;
/// Height of one category row.
pub const CATEGORY_HEIGHT: i32 = 30;

/// Preferred launcher size for an output of the given size.
///
/// Clamped so it neither swallows a small screen nor floats as a postage stamp
/// on a large one.
pub fn window_size(output_width: i32, output_height: i32, rows: i32) -> (i32, i32) {
    let width = (output_width * 3 / 5).clamp(480, 900).min(output_width.max(1));
    let content = SEARCH_HEIGHT + rows.max(1) * ROW_HEIGHT + PADDING * 2;
    let wanted = content.max(SEARCH_HEIGHT + PADDING * 2 + CATEGORY_HEIGHT * 8);
    let height = wanted.min((output_height * 3 / 4).max(SEARCH_HEIGHT + PADDING * 2));
    (width, height)
}

/// The category column.
pub fn sidebar_rect(height: i32) -> Rect {
    Rect::new(0, 0, SIDEBAR_WIDTH, height)
}

/// The rectangle of the `index`-th category.
pub fn category_rect(index: usize) -> Rect {
    Rect::new(
        PADDING / 2,
        PADDING + index as i32 * CATEGORY_HEIGHT,
        SIDEBAR_WIDTH - PADDING,
        CATEGORY_HEIGHT,
    )
}

/// Which category a point falls on.
pub fn category_at(count: usize, height: i32, x: i32, y: i32) -> Option<usize> {
    (0..count).find(|&i| {
        let rect = category_rect(i);
        rect.bottom() <= height && rect.contains(x, y)
    })
}

/// How many categories fit in a window of this height.
pub fn visible_categories(height: i32) -> usize {
    ((height - PADDING * 2) / CATEGORY_HEIGHT).max(0) as usize
}

/// How many result rows fit in a window of this height.
pub fn visible_rows(height: i32) -> usize {
    ((height - SEARCH_HEIGHT - PADDING * 2) / ROW_HEIGHT).max(0) as usize
}

/// The rectangle of the `index`-th visible row.
pub fn row_rect(width: i32, index: usize) -> Rect {
    Rect::new(
        SIDEBAR_WIDTH + PADDING,
        PADDING + SEARCH_HEIGHT + index as i32 * ROW_HEIGHT,
        (width - SIDEBAR_WIDTH - PADDING * 2).max(0),
        ROW_HEIGHT,
    )
}

/// Which visible row a point falls on.
pub fn row_at(width: i32, height: i32, x: i32, y: i32) -> Option<usize> {
    let rows = visible_rows(height);
    (0..rows).find(|&index| row_rect(width, index).contains(x, y))
}

/// The window scrolled so `selected` is on screen.
///
/// Returns the index of the first row to draw.
pub fn scroll_offset(selected: usize, rows: usize, previous: usize) -> usize {
    if rows == 0 {
        return 0;
    }
    if selected < previous {
        selected
    } else if selected >= previous + rows {
        selected + 1 - rows
    } else {
        previous
    }
}

/// Everything the launcher needs painting.
pub struct Frame<'a> {
    pub theme: &'a Theme,
    pub query: &'a str,
    pub results: &'a [&'a Entry],
    pub selected: usize,
    pub offset: usize,
    /// The categories in the column, and which one is showing.
    pub categories: &'a [Category],
    pub category: usize,
    pub mask: &'a spectre_draw::PatternMask,
    pub color_phase: f32,
}

/// Paint the launcher.
pub fn draw(canvas: &mut Canvas, text: &mut TextRenderer, frame: &Frame<'_>) {
    let palette = &frame.theme.palette;
    let bounds = canvas.bounds();
    let width = bounds.w;

    canvas.fill_pattern(bounds, frame.mask, palette.surface, &palette.accent, frame.color_phase);
    draw_border(canvas, bounds, palette);
    draw_sidebar(canvas, text, bounds.h, frame);
    draw_search(canvas, text, width, frame);

    let rows = visible_rows(bounds.h);
    for (row, entry) in frame.results.iter().skip(frame.offset).take(rows).enumerate() {
        let absolute = frame.offset + row;
        draw_row(canvas, text, row_rect(width, row), entry, absolute == frame.selected, palette);
    }

    if frame.results.is_empty() {
        let what = if frame.query.is_empty() { "Nothing here" } else { "No matches" };
        let label = Label::new(what).size(NAME_SIZE).color(palette.text_muted);
        let image = text.rasterise(&label);
        let x = SIDEBAR_WIDTH + (width - SIDEBAR_WIDTH - image.width as i32) / 2;
        let y = PADDING + SEARCH_HEIGHT + ROW_HEIGHT / 2 - image.height as i32 / 2;
        canvas.draw_image(x.max(SIDEBAR_WIDTH), y, &image);
    }
}

fn draw_sidebar(canvas: &mut Canvas, text: &mut TextRenderer, height: i32, frame: &Frame<'_>) {
    let palette = &frame.theme.palette;
    let column = sidebar_rect(height);
    canvas.fill_rect(column, palette.elevated);
    canvas.fill_rect(Rect::new(column.right() - 1, 0, 1, height), palette.line);

    let searching = !frame.query.trim().is_empty();
    for (index, category) in frame.categories.iter().enumerate().take(visible_categories(height)) {
        let rect = category_rect(index);
        let current = !searching && index == frame.category;
        if current {
            canvas.fill_rect(rect, palette.overlay);
            canvas.fill_rect(
                Rect::new(rect.x, rect.y + 4, MARKER_WIDTH, rect.h - 8),
                palette.accent.sample(0.3),
            );
        }
        let color = match (current, searching) {
            (true, _) => palette.text,
            (_, true) => palette.text_muted,
            _ => palette.text_dim,
        };
        let label = Label::new(category.label())
            .size(NAME_SIZE - 1.0)
            .color(color)
            .bold(current)
            .max_width((rect.w - MARKER_WIDTH - 14).max(1) as u32)
            .ellipsis(EllipsisSide::End);
        let image = text.rasterise(&label);
        canvas.draw_image(
            rect.x + MARKER_WIDTH + 8,
            rect.y + (rect.h - image.height as i32) / 2,
            &image,
        );
    }
}

/// The pattern behind the menu, held well back: this surface is a wall of
/// text, and at title bar intensity the contours read through it.
pub fn launcher_pattern(theme: &Theme) -> Pattern {
    Pattern { intensity: theme.window_pattern.intensity * 0.3, ..theme.window_pattern }
}

/// A one pixel accent frame, so the launcher reads as its own surface.
fn draw_border(canvas: &mut Canvas, bounds: Rect, palette: &Palette) {
    let steps = bounds.w.clamp(1, 48);
    for i in 0..steps {
        let start = bounds.x + (bounds.w * i) / steps;
        let end = bounds.x + (bounds.w * (i + 1)) / steps;
        if end <= start {
            continue;
        }
        let t = if steps == 1 { 0.5 } else { i as f32 / (steps - 1) as f32 };
        let color = palette.accent.sample(t);
        canvas.fill_rect(Rect::new(start, bounds.y, end - start, 1), color);
        canvas.fill_rect(Rect::new(start, bounds.bottom() - 1, end - start, 1), color);
    }
    canvas.fill_rect(Rect::new(bounds.x, bounds.y, 1, bounds.h), palette.accent.sample(0.0));
    canvas.fill_rect(
        Rect::new(bounds.right() - 1, bounds.y, 1, bounds.h),
        palette.accent.sample(1.0),
    );
}

fn draw_search(canvas: &mut Canvas, text: &mut TextRenderer, width: i32, frame: &Frame<'_>) {
    let palette = &frame.theme.palette;
    let field = Rect::new(
        SIDEBAR_WIDTH + PADDING,
        PADDING,
        (width - SIDEBAR_WIDTH - PADDING * 2).max(0),
        SEARCH_HEIGHT - PADDING,
    );
    canvas.fill_rect(field, palette.elevated);

    let prompt = Label::new("\u{203a}").size(NAME_SIZE + 2.0).color(palette.accent.sample(0.2));
    let prompt_image = text.rasterise(&prompt);
    canvas.draw_image(
        field.x + 10,
        field.y + (field.h - prompt_image.height as i32) / 2,
        &prompt_image,
    );

    let text_x = field.x + 10 + prompt_image.width as i32 + 8;
    let budget = (field.right() - 10 - text_x).max(1) as u32;

    if frame.query.is_empty() {
        let hint = Label::new("Type to search").size(NAME_SIZE).color(palette.text_muted);
        let image = text.rasterise(&hint);
        canvas.draw_image(text_x, field.y + (field.h - image.height as i32) / 2, &image);
        // The caret sits where the first character will land.
        canvas.fill_rect(
            Rect::new(text_x - 3, field.y + 8, 1, field.h - 16),
            palette.accent.sample(0.5),
        );
        return;
    }

    let label = Label::new(frame.query)
        .size(NAME_SIZE)
        .color(palette.text)
        .max_width(budget)
        .ellipsis(EllipsisSide::Start);
    let image = text.rasterise(&label);
    let y = field.y + (field.h - image.height as i32) / 2;
    canvas.draw_image(text_x, y, &image);
    canvas.fill_rect(
        Rect::new(text_x + image.width as i32 + 2, field.y + 8, 1, field.h - 16),
        palette.accent.sample(0.5),
    );
}

fn draw_row(
    canvas: &mut Canvas,
    text: &mut TextRenderer,
    rect: Rect,
    entry: &Entry,
    selected: bool,
    palette: &Palette,
) {
    if selected {
        canvas.fill_rect(rect, palette.overlay);
        // A short accent bar on the leading edge, rather than a full-width
        // highlight: black first, RGB second.
        canvas.fill_rect(Rect::new(rect.x, rect.y + 4, MARKER_WIDTH, rect.h - 8), palette.accent.sample(0.3));
    }

    let x = rect.x + MARKER_WIDTH + 10;
    let budget = (rect.right() - 10 - x).max(1) as u32;
    let has_comment = !entry.comment.is_empty();

    let name = Label::new(&entry.name)
        .size(NAME_SIZE)
        .color(if selected { palette.text } else { palette.text_dim })
        .bold(selected)
        .max_width(budget);
    let name_image = text.rasterise(&name);

    if has_comment {
        let comment = Label::new(&entry.comment)
            .size(COMMENT_SIZE)
            .color(palette.text_muted)
            .max_width(budget);
        let comment_image = text.rasterise(&comment);
        let total = name_image.height as i32 + comment_image.height as i32;
        let mut y = rect.y + (rect.h - total) / 2;
        canvas.draw_image(x, y, &name_image);
        y += name_image.height as i32;
        canvas.draw_image(x, y, &comment_image);
    } else {
        let y = rect.y + (rect.h - name_image.height as i32) / 2;
        canvas.draw_image(x, y, &name_image);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_window_stays_a_sensible_size_on_any_output() {
        for (w, h) in [(800, 600), (1920, 1080), (3840, 2160), (320, 240)] {
            let (width, height) = window_size(w, h, 8);
            assert!(width >= 480.min(w) && width <= 900, "{w}x{h} gave width {width}");
            assert!(height > 0 && height <= h.max(1), "{w}x{h} gave height {height}");
        }
    }

    #[test]
    fn a_tiny_output_still_gets_a_usable_window() {
        let (width, height) = window_size(200, 100, 8);
        assert!(width > 0 && height > 0);
        assert!(width <= 480, "the window must not exceed a 200px wide output by much");
    }

    #[test]
    fn rows_fit_the_space_that_is_left() {
        let height = PADDING * 2 + SEARCH_HEIGHT + ROW_HEIGHT * 5;
        assert_eq!(visible_rows(height), 5);
        assert_eq!(visible_rows(SEARCH_HEIGHT), 0, "no room means no rows");
        assert_eq!(visible_rows(0), 0);
    }

    #[test]
    fn rows_do_not_overlap_and_stay_inside_the_window() {
        let width = 500;
        for i in 0..6 {
            let a = row_rect(width, i);
            let b = row_rect(width, i + 1);
            assert_eq!(a.bottom(), b.y);
            assert!(a.x >= 0 && a.right() <= width);
        }
    }

    #[test]
    fn hit_testing_matches_the_drawn_rows() {
        let width = 500;
        let height = PADDING * 2 + SEARCH_HEIGHT + ROW_HEIGHT * 4;
        for i in 0..4 {
            let rect = row_rect(width, i);
            assert_eq!(row_at(width, height, rect.x + 5, rect.y + 5), Some(i));
        }
        // The search field is not a row.
        assert_eq!(row_at(width, height, 20, PADDING + 2), None);
        // Past the last row.
        assert_eq!(row_at(width, height, 20, height - 1), None);
    }

    #[test]
    fn the_category_column_and_the_rows_never_overlap() {
        let width = 700;
        let height = PADDING * 2 + SEARCH_HEIGHT + ROW_HEIGHT * 6;
        for i in 0..6 {
            assert!(row_rect(width, i).x >= SIDEBAR_WIDTH);
        }
        for i in 0..visible_categories(height) {
            assert!(category_rect(i).right() <= SIDEBAR_WIDTH);
        }
    }

    #[test]
    fn hit_testing_matches_the_drawn_categories() {
        let height = PADDING * 2 + CATEGORY_HEIGHT * 6;
        for i in 0..5 {
            let rect = category_rect(i);
            assert_eq!(category_at(5, height, rect.x + 2, rect.y + 2), Some(i));
        }
        assert_eq!(category_at(5, height, SIDEBAR_WIDTH + 40, 60), None);
    }

    #[test]
    fn categories_that_do_not_fit_are_not_clickable() {
        let height = PADDING * 2 + CATEGORY_HEIGHT * 2;
        let rect = category_rect(5);
        assert_eq!(category_at(11, height, rect.x + 2, rect.y + 2), None);
    }

    #[test]
    fn scrolling_follows_the_selection_down() {
        assert_eq!(scroll_offset(0, 5, 0), 0);
        assert_eq!(scroll_offset(4, 5, 0), 0, "still on screen");
        assert_eq!(scroll_offset(5, 5, 0), 1, "one past the bottom scrolls by one");
        assert_eq!(scroll_offset(9, 5, 0), 5);
    }

    #[test]
    fn scrolling_follows_the_selection_up() {
        assert_eq!(scroll_offset(3, 5, 5), 3);
        assert_eq!(scroll_offset(0, 5, 5), 0);
    }

    #[test]
    fn scrolling_a_window_with_no_rows_does_not_divide_by_zero() {
        assert_eq!(scroll_offset(7, 0, 3), 0);
    }
}
