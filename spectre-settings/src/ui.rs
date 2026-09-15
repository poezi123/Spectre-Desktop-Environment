use spectre_draw::{Canvas, PatternMask, Rect};
use spectre_text::{EllipsisSide, Image, Label, TextRenderer};
use spectre_theme::{Palette, Pattern, Theme};

use crate::model::{Control, Field, Section};

pub const SIDEBAR_WIDTH: i32 = 190;
pub const HEADER_HEIGHT: i32 = 52;
pub const ROW_HEIGHT: i32 = 52;
pub const SECTION_HEIGHT: i32 = 34;
pub const PADDING: i32 = 12;
pub const TITLE_SIZE: f32 = 16.0;
pub const LABEL_SIZE: f32 = 13.0;
pub const HELP_SIZE: f32 = 10.5;
pub const MARKER_WIDTH: i32 = 3;
pub const CONTROL_WIDTH: i32 = 190;
pub const OPTION_HEIGHT: i32 = 26;
pub const MAX_OPTIONS: usize = 8;

pub fn window_size(output_width: i32, output_height: i32) -> (i32, i32) {
    let width = (output_width * 3 / 5).clamp(560, 880).min(output_width.max(1));
    let height = (output_height * 3 / 4).clamp(420, 620).min(output_height.max(1));
    (width, height)
}

pub fn section_rect(index: usize) -> Rect {
    Rect::new(
        PADDING / 2,
        HEADER_HEIGHT + index as i32 * SECTION_HEIGHT,
        SIDEBAR_WIDTH - PADDING,
        SECTION_HEIGHT,
    )
}

pub fn section_at(count: usize, x: i32, y: i32) -> Option<usize> {
    (0..count).find(|&i| section_rect(i).contains(x, y))
}

pub fn row_rect(width: i32, index: usize) -> Rect {
    Rect::new(
        SIDEBAR_WIDTH,
        HEADER_HEIGHT + index as i32 * ROW_HEIGHT,
        (width - SIDEBAR_WIDTH).max(0),
        ROW_HEIGHT,
    )
}

pub fn visible_rows(height: i32) -> usize {
    ((height - HEADER_HEIGHT) / ROW_HEIGHT).max(0) as usize
}

pub fn row_at(width: i32, height: i32, x: i32, y: i32) -> Option<usize> {
    (0..visible_rows(height)).find(|&i| row_rect(width, i).contains(x, y))
}

pub fn control_rect(row: Rect) -> Rect {
    let width = CONTROL_WIDTH.min((row.w - PADDING * 2).max(0));
    Rect::new(row.right() - PADDING - width, row.y + (row.h - 20) / 2, width, 20)
}

pub struct DropdownView<'a> {
    pub row: usize,
    pub options: &'a [String],
    pub selected: usize,
    pub highlighted: usize,
    pub first: usize,
}

pub fn dropdown_rect(width: i32, height: i32, row: usize, count: usize) -> Rect {
    let control = control_rect(row_rect(width, row));
    let visible = count.min(MAX_OPTIONS) as i32;
    let list_height = visible * OPTION_HEIGHT + 4;
    let mut y = control.bottom() + 2;
    if y + list_height > height {
        y = (control.y - 2 - list_height).max(0);
    }
    Rect::new(control.x, y, control.w, list_height)
}

pub fn option_at(list: Rect, first: usize, count: usize, x: i32, y: i32) -> Option<usize> {
    if !list.contains(x, y) {
        return None;
    }
    let index = first + ((y - list.y - 2) / OPTION_HEIGHT).max(0) as usize;
    if index >= count {
        return None;
    }
    Some(index)
}

pub struct Frame<'a> {
    pub theme: &'a Theme,
    pub sections: &'a [Section],
    pub section: usize,
    pub row: usize,
    pub mask: &'a PatternMask,
    pub color_phase: f32,
    pub status: &'a str,
    pub reset_armed: bool,
    pub wallpaper_thumbnail: Option<&'a Image>,
    pub dropdown: Option<DropdownView<'a>>,
}

pub fn settings_pattern(theme: &Theme) -> Pattern {
    Pattern { intensity: theme.window_pattern.intensity * 0.25, ..theme.window_pattern }
}

pub fn draw(canvas: &mut Canvas, text: &mut TextRenderer, frame: &Frame<'_>) {
    let palette = &frame.theme.palette;
    let bounds = canvas.bounds();
    canvas.fill_pattern(bounds, frame.mask, palette.surface, &palette.accent, frame.color_phase);

    draw_sidebar(canvas, text, bounds, frame);
    draw_header(canvas, text, bounds, frame);

    let Some(section) = frame.sections.get(frame.section) else {
        return;
    };
    for (index, row) in section.rows.iter().enumerate().take(visible_rows(bounds.h)) {
        let rect = row_rect(bounds.w, index);
        let selected = index == frame.row;
        if selected {
            canvas.fill_rect(rect, palette.overlay);
            canvas.fill_rect(
                Rect::new(rect.x, rect.y + 6, MARKER_WIDTH, rect.h - 12),
                palette.accent.sample(0.3),
            );
        }

        let x = rect.x + MARKER_WIDTH + PADDING;
        let budget = (control_rect(rect).x - PADDING - x).max(1) as u32;
        let name = Label::new(row.label)
            .size(LABEL_SIZE)
            .color(if selected { palette.text } else { palette.text_dim })
            .bold(selected)
            .max_width(budget)
            .ellipsis(EllipsisSide::End);
        let help = Label::new(row.help)
            .size(HELP_SIZE)
            .color(palette.text_muted)
            .max_width(budget)
            .ellipsis(EllipsisSide::End);
        let name_image = text.rasterise(&name);
        let help_image = text.rasterise(&help);
        let total = name_image.height as i32 + help_image.height as i32;
        let mut y = rect.y + (rect.h - total) / 2;
        canvas.draw_image(x, y, &name_image);
        y += name_image.height as i32;
        canvas.draw_image(x, y, &help_image);

        let mut control = control_rect(rect);
        if row.field == Field::Wallpaper {
            if let Some(image) = frame.wallpaper_thumbnail {
                let y = rect.y + (rect.h - image.height as i32) / 2;
                canvas.draw_image(control.x, y, image);
                let used = image.width as i32 + PADDING;
                control = Rect::new(control.x + used, control.y, (control.w - used).max(0), control.h);
            }
        }
        draw_control(canvas, text, control, &row.control, selected, frame.reset_armed, palette);
    }

    if let Some(view) = frame.dropdown.as_ref() {
        draw_dropdown(canvas, text, bounds, view, palette);
    }
}

fn draw_dropdown(
    canvas: &mut Canvas,
    text: &mut TextRenderer,
    bounds: Rect,
    view: &DropdownView<'_>,
    palette: &Palette,
) {
    let list = dropdown_rect(bounds.w, bounds.h, view.row, view.options.len());
    canvas.fill_rect(list, palette.line);
    canvas.fill_rect(list.inset(1), palette.elevated);

    let visible = view.options.len().min(MAX_OPTIONS);
    for slot in 0..visible {
        let index = view.first + slot;
        let Some(option) = view.options.get(index) else {
            break;
        };
        let item = Rect::new(
            list.x + 2,
            list.y + 2 + slot as i32 * OPTION_HEIGHT,
            list.w - 4,
            OPTION_HEIGHT,
        );
        if index == view.highlighted {
            canvas.fill_rect(item, palette.overlay);
        }
        if index == view.selected {
            let marker = Rect::new(item.x, item.y + 5, MARKER_WIDTH, item.h - 10);
            canvas.fill_rect(marker, palette.accent.sample(0.3));
        }
        let mut color = palette.text_dim;
        if index == view.highlighted || index == view.selected {
            color = palette.text;
        }
        let label = Label::new(option.as_str())
            .size(LABEL_SIZE)
            .color(color)
            .max_width((item.w - 20).max(1) as u32)
            .ellipsis(EllipsisSide::End);
        let image = text.rasterise(&label);
        let y = item.y + (item.h - image.height as i32) / 2;
        canvas.draw_image(item.x + MARKER_WIDTH + 8, y, &image);
    }
}

fn draw_chevron(canvas: &mut Canvas, x: i32, y: i32, color: spectre_theme::Color) {
    for step in 0..4 {
        canvas.fill_rect(Rect::new(x + step, y + step, 7 - step * 2, 1), color);
    }
}

fn draw_header(canvas: &mut Canvas, text: &mut TextRenderer, bounds: Rect, frame: &Frame<'_>) {
    let palette = &frame.theme.palette;
    let title = frame.sections.get(frame.section).map(|s| s.title).unwrap_or("Settings");
    let label = Label::new(title).size(TITLE_SIZE).color(palette.text).bold(true);
    let image = text.rasterise(&label);
    canvas.draw_image(SIDEBAR_WIDTH + PADDING + MARKER_WIDTH, PADDING, &image);

    if !frame.status.is_empty() {
        let status = Label::new(frame.status).size(HELP_SIZE).color(palette.text_muted);
        let image = text.rasterise(&status);
        canvas.draw_image(
            (bounds.w - PADDING - image.width as i32).max(SIDEBAR_WIDTH),
            PADDING + 6,
            &image,
        );
    }
    canvas.fill_rect(Rect::new(SIDEBAR_WIDTH, HEADER_HEIGHT - 1, bounds.w, 1), palette.line);
}

fn draw_sidebar(canvas: &mut Canvas, text: &mut TextRenderer, bounds: Rect, frame: &Frame<'_>) {
    let palette = &frame.theme.palette;
    canvas.fill_rect(Rect::new(0, 0, SIDEBAR_WIDTH, bounds.h), palette.elevated);
    canvas.fill_rect(Rect::new(SIDEBAR_WIDTH - 1, 0, 1, bounds.h), palette.line);

    let brand = Label::new("Spectre").size(TITLE_SIZE).color(palette.text).bold(true);
    let image = text.rasterise(&brand);
    canvas.draw_image(PADDING + MARKER_WIDTH, PADDING, &image);

    for (index, section) in frame.sections.iter().enumerate() {
        let rect = section_rect(index);
        if rect.bottom() > bounds.h {
            break;
        }
        let current = index == frame.section;
        if current {
            canvas.fill_rect(rect, palette.overlay);
            canvas.fill_rect(
                Rect::new(rect.x, rect.y + 4, MARKER_WIDTH, rect.h - 8),
                palette.accent.sample(0.3),
            );
        }
        let label = Label::new(section.title)
            .size(LABEL_SIZE)
            .color(if current { palette.text } else { palette.text_dim })
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

fn draw_control(
    canvas: &mut Canvas,
    text: &mut TextRenderer,
    rect: Rect,
    control: &Control,
    selected: bool,
    armed: bool,
    palette: &Palette,
) {
    match control {
        Control::Toggle(on) => {
            let track = Rect::new(rect.right() - 40, rect.y + 2, 40, 16);
            canvas.fill_rect(track, if *on { palette.accent.sample(0.4) } else { palette.line });
            let knob_x = if *on { track.right() - 15 } else { track.x + 1 };
            canvas.fill_rect(
                Rect::new(knob_x, track.y + 1, 14, 14),
                if *on { palette.text } else { palette.text_muted },
            );
        }
        Control::Choice { index, options } => {
            let value = options.get(*index).map(String::as_str).unwrap_or("-");
            let label = Label::new(value)
                .size(LABEL_SIZE)
                .color(if selected { palette.text } else { palette.text_dim })
                .max_width((rect.w - 28).max(1) as u32)
                .ellipsis(EllipsisSide::Start);
            let image = text.rasterise(&label);
            let y = rect.y + (rect.h - image.height as i32) / 2;
            canvas.draw_image(rect.right() - 14 - image.width as i32, y, &image);
            let mut chevron = palette.text_muted;
            if selected {
                chevron = palette.accent.sample(0.5);
            }
            draw_chevron(canvas, rect.right() - 8, rect.y + rect.h / 2 - 2, chevron);
        }
        Control::Slider { value, label } => {
            let bar = Rect::new(rect.x + 40, rect.y + 7, (rect.w - 40).max(1), 4);
            canvas.fill_rect(bar, palette.line);
            let filled = (bar.w as f32 * value.clamp(0.0, 1.0)).round() as i32;
            let steps = filled.clamp(0, bar.w).max(0);
            for i in 0..steps {
                let t = i as f32 / bar.w.max(1) as f32;
                canvas.fill_rect(Rect::new(bar.x + i, bar.y, 1, bar.h), palette.accent.sample(t));
            }
            let knob = bar.x + steps.clamp(0, bar.w) - 2;
            canvas.fill_rect(
                Rect::new(knob, bar.y - 4, 4, 12),
                if selected { palette.text } else { palette.text_dim },
            );

            let value = Label::new(label)
                .size(HELP_SIZE)
                .color(if selected { palette.text_dim } else { palette.text_muted });
            let image = text.rasterise(&value);
            canvas.draw_image(rect.x, rect.y + (rect.h - image.height as i32) / 2, &image);
        }
        Control::Button { label } => {
            let background = if selected { palette.line } else { palette.elevated };
            canvas.fill_rect(rect, background);

            let caption = if armed { "Click again to confirm" } else { label.as_str() };
            let color = if armed { palette.accent.sample(0.5) } else { palette.text };
            let caption = Label::new(caption)
                .size(HELP_SIZE)
                .color(color)
                .max_width((rect.w - 8).max(1) as u32)
                .ellipsis(EllipsisSide::End);
            let image = text.rasterise(&caption);
            let x = rect.x + (rect.w - image.width as i32) / 2;
            let y = rect.y + (rect.h - image.height as i32) / 2;
            canvas.draw_image(x, y, &image);
        }
    }
}

pub fn slider_value_at(rect: Rect, x: i32) -> f32 {
    let bar_x = rect.x + 40;
    let bar_w = (rect.w - 40).max(1);
    ((x - bar_x) as f32 / bar_w as f32).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_window_fits_any_output() {
        for (w, h) in [(800, 600), (1920, 1080), (640, 480), (3840, 2160)] {
            let (width, height) = window_size(w, h);
            assert!(width > 0 && width <= w.max(560));
            assert!(height > 0 && height <= h.max(420));
        }
    }

    #[test]
    fn rows_start_below_the_header_and_never_reach_the_sidebar() {
        for i in 0..6 {
            let rect = row_rect(700, i);
            assert!(rect.y >= HEADER_HEIGHT);
            assert_eq!(rect.x, SIDEBAR_WIDTH);
        }
    }

    #[test]
    fn rows_do_not_overlap() {
        for i in 0..6 {
            assert_eq!(row_rect(700, i).bottom(), row_rect(700, i + 1).y);
        }
    }

    #[test]
    fn hit_testing_matches_the_drawn_rows() {
        let (w, h) = (700, HEADER_HEIGHT + ROW_HEIGHT * 4);
        for i in 0..4 {
            let rect = row_rect(w, i);
            assert_eq!(row_at(w, h, rect.x + 5, rect.y + 5), Some(i));
        }
        assert_eq!(row_at(w, h, 20, 20), None, "the sidebar is not a row");
        assert_eq!(row_at(w, h, 400, 10), None, "the header is not a row");
    }

    #[test]
    fn hit_testing_matches_the_drawn_sections() {
        for i in 0..4 {
            let rect = section_rect(i);
            assert_eq!(section_at(4, rect.x + 2, rect.y + 2), Some(i));
            assert!(rect.right() <= SIDEBAR_WIDTH);
        }
        assert_eq!(section_at(4, SIDEBAR_WIDTH + 10, 100), None);
    }

    #[test]
    fn the_control_sits_inside_its_row() {
        let row = row_rect(700, 2);
        let control = control_rect(row);
        assert!(control.x >= row.x && control.right() <= row.right());
        assert!(control.y >= row.y && control.bottom() <= row.bottom());
    }

    #[test]
    fn a_click_maps_to_a_position_along_the_slider() {
        let rect = control_rect(row_rect(700, 0));
        assert_eq!(slider_value_at(rect, rect.x), 0.0);
        assert_eq!(slider_value_at(rect, rect.right()), 1.0);
        assert!((slider_value_at(rect, rect.x + 40 + (rect.w - 40) / 2) - 0.5).abs() < 0.02);
        assert_eq!(slider_value_at(rect, rect.x - 100), 0.0, "outside clamps");
    }

    #[test]
    fn a_short_window_shows_no_rows_rather_than_negative_ones() {
        assert_eq!(visible_rows(0), 0);
        assert_eq!(visible_rows(HEADER_HEIGHT), 0);
        assert_eq!(visible_rows(HEADER_HEIGHT + ROW_HEIGHT * 3), 3);
    }
}

#[cfg(test)]
mod dropdown_tests {
    use super::*;

    #[test]
    fn the_list_opens_below_the_row_when_there_is_room() {
        let control = control_rect(row_rect(800, 0));
        let list = dropdown_rect(800, 600, 0, 5);
        assert!(list.y > control.y);
        assert_eq!(list.h, 5 * OPTION_HEIGHT + 4);
    }

    #[test]
    fn the_list_opens_upwards_near_the_bottom() {
        let row = visible_rows(600) - 1;
        let control = control_rect(row_rect(800, row));
        let list = dropdown_rect(800, 600, row, 8);
        assert!(list.bottom() <= control.y);
    }

    #[test]
    fn a_long_list_shows_only_a_few_entries() {
        let list = dropdown_rect(800, 900, 0, 40);
        assert_eq!(list.h, MAX_OPTIONS as i32 * OPTION_HEIGHT + 4);
    }

    #[test]
    fn clicking_an_entry_picks_it_counting_the_scroll() {
        let list = dropdown_rect(800, 900, 0, 40);
        let y = list.y + 2 + OPTION_HEIGHT * 2 + OPTION_HEIGHT / 2;
        assert_eq!(option_at(list, 10, 40, list.x + 10, y), Some(12));
        assert_eq!(option_at(list, 10, 40, list.x - 5, y), None);
    }
}
