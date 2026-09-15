use smithay::backend::renderer::element::solid::SolidColorRenderElement;
use smithay::utils::{Logical, Point, Rectangle, Size};
use spectre_theme::{Metrics, Palette};

use super::{solid, RenderCache, Slot};

const BUTTON_GAP: i32 = 2;
const TITLEBAR_PADDING: i32 = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Frame {
    pub window: Rectangle<i32, Logical>,
    pub titlebar: Rectangle<i32, Logical>,
    pub outer: Rectangle<i32, Logical>,
    pub border: i32,
}

impl Frame {
    pub fn new(window: Rectangle<i32, Logical>, metrics: &Metrics, decorated: bool) -> Self {
        let border = if decorated { metrics.border_width as i32 } else { 0 };
        let title_h = if decorated { metrics.titlebar_height as i32 } else { 0 };

        let titlebar = Rectangle::new(
            Point::from((window.loc.x, window.loc.y - title_h)),
            Size::from((window.size.w, title_h)),
        );
        let outer = Rectangle::new(
            Point::from((window.loc.x - border, window.loc.y - title_h - border)),
            Size::from((
                window.size.w + border * 2,
                window.size.h + title_h + border * 2,
            )),
        );

        Self { window, titlebar, outer, border }
    }

    pub fn is_decorated(&self) -> bool {
        self.titlebar.size.h > 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Part {
    Titlebar,
    Minimize,
    Maximize,
    Close,
    Border,
}

pub fn buttons(frame: &Frame, metrics: &Metrics) -> Vec<(Part, Rectangle<i32, Logical>)> {
    let size = metrics.button_size as i32;
    let bar = frame.titlebar;
    if bar.size.h <= 0 || size <= 0 {
        return Vec::new();
    }

    let order = [Part::Close, Part::Maximize, Part::Minimize];
    let needed = order.len() as i32 * size + (order.len() as i32 - 1) * BUTTON_GAP;
    if bar.size.w < needed + TITLEBAR_PADDING * 2 {
        return Vec::new();
    }

    let y = bar.loc.y + (bar.size.h - size) / 2;
    let mut x = bar.loc.x + bar.size.w - TITLEBAR_PADDING - size;

    let mut out = Vec::with_capacity(order.len());
    for part in order {
        out.push((part, Rectangle::new(Point::from((x, y)), Size::from((size, size)))));
        x -= size + BUTTON_GAP;
    }
    out
}

pub fn caption_area(frame: &Frame, metrics: &Metrics) -> Rectangle<i32, Logical> {
    let bar = frame.titlebar;
    let buttons = buttons(frame, metrics);
    let right = buttons
        .iter()
        .map(|(_, r)| r.loc.x)
        .min()
        .unwrap_or(bar.loc.x + bar.size.w - TITLEBAR_PADDING);

    let left = bar.loc.x + TITLEBAR_PADDING;
    let width = (right - BUTTON_GAP - left).max(0);
    Rectangle::new(Point::from((left, bar.loc.y)), Size::from((width, bar.size.h)))
}

pub const RESIZE_MARGIN: i32 = 6;

pub const RESIZE_CORNER: i32 = 18;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Edges {
    pub left: bool,
    pub right: bool,
    pub top: bool,
    pub bottom: bool,
}

impl Edges {
    pub fn is_empty(&self) -> bool {
        !self.left && !self.right && !self.top && !self.bottom
    }
}

pub fn edges_at(frame: &Frame, point: Point<f64, Logical>) -> Option<Edges> {
    let p = Point::<i32, Logical>::from((point.x.floor() as i32, point.y.floor() as i32));
    let outer = frame.outer;
    let reach = Rectangle::new(
        Point::from((outer.loc.x - RESIZE_MARGIN, outer.loc.y - RESIZE_MARGIN)),
        Size::from((outer.size.w + RESIZE_MARGIN * 2, outer.size.h + RESIZE_MARGIN * 2)),
    );
    if !reach.contains(p) || frame.window.contains(p) || frame.titlebar.contains(p) {
        return None;
    }

    let mut content_top = frame.window.loc.y;
    if frame.is_decorated() {
        content_top = frame.titlebar.loc.y;
    }
    let mut edges = Edges {
        left: p.x < frame.window.loc.x,
        right: p.x >= frame.window.loc.x + frame.window.size.w,
        top: p.y < content_top,
        bottom: p.y >= frame.window.loc.y + frame.window.size.h,
    };
    if edges.left || edges.right {
        if p.y < outer.loc.y + RESIZE_CORNER {
            edges.top = true;
        }
        if p.y >= outer.loc.y + outer.size.h - RESIZE_CORNER {
            edges.bottom = true;
        }
    }
    if edges.top || edges.bottom {
        if p.x < outer.loc.x + RESIZE_CORNER {
            edges.left = true;
        }
        if p.x >= outer.loc.x + outer.size.w - RESIZE_CORNER {
            edges.right = true;
        }
    }
    if edges.is_empty() {
        return None;
    }
    Some(edges)
}

pub fn part_at(
    frame: &Frame,
    metrics: &Metrics,
    point: Point<f64, Logical>,
) -> Option<Part> {
    let p = Point::<i32, Logical>::from((point.x.floor() as i32, point.y.floor() as i32));
    if !frame.outer.contains(p) || frame.window.contains(p) {
        return None;
    }

    for (part, rect) in buttons(frame, metrics) {
        if rect.contains(p) {
            return Some(part);
        }
    }
    if frame.titlebar.contains(p) {
        return Some(Part::Titlebar);
    }
    Some(Part::Border)
}

#[allow(clippy::too_many_arguments)]
pub fn button_plates(
    cache: &mut RenderCache,
    key: u32,
    frame: &Frame,
    metrics: &Metrics,
    palette: &Palette,
    hovered: Option<Part>,
    alpha: f32,
    scale: f64,
) -> Vec<SolidColorRenderElement> {
    let mut elements = Vec::new();
    for (index, (part, rect)) in buttons(frame, metrics).into_iter().enumerate() {
        let Some(color) = button_background(part, hovered, palette) else {
            continue;
        };
        let slot = Slot::Decoration(key, index as u8);
        elements.extend(solid(cache, slot, rect, faded(color, alpha), scale));
    }
    elements
}

pub fn fallback_frame(
    cache: &mut RenderCache,
    key: u32,
    frame: &Frame,
    palette: &Palette,
    focused: bool,
    alpha: f32,
    scale: f64,
) -> Vec<SolidColorRenderElement> {
    let mut elements = Vec::new();
    if frame.outer.size.w <= 0 || frame.outer.size.h <= 0 {
        return elements;
    }

    const FIRST: u8 = 16;
    if frame.is_decorated() {
        let slot = Slot::Decoration(key, FIRST);
        let color = faded(palette.titlebar(focused), alpha);
        elements.extend(solid(cache, slot, frame.titlebar, color, scale));
    }
    if frame.border > 0 {
        let color = faded(palette.window_border(focused), alpha);
        for (index, edge) in ring_edges(frame.outer, frame.border).into_iter().enumerate() {
            let slot = Slot::Decoration(key, FIRST + 1 + index as u8);
            elements.extend(solid(cache, slot, edge, color, scale));
        }
    }
    elements
}

fn faded(color: spectre_theme::Color, alpha: f32) -> spectre_theme::Color {
    color.alpha(color.a * alpha.clamp(0.0, 1.0))
}

fn button_background(part: Part, hovered: Option<Part>, palette: &Palette) -> Option<spectre_theme::Color> {
    if hovered != Some(part) {
        return None;
    }
    Some(match part {
        Part::Close => palette.danger,
        _ => palette.overlay,
    })
}

fn ring_edges(rect: Rectangle<i32, Logical>, width: i32) -> [Rectangle<i32, Logical>; 4] {
    let inner_h = (rect.size.h - width * 2).max(0);
    [
        Rectangle::new(rect.loc, Size::from((rect.size.w, width))),
        Rectangle::new(
            Point::from((rect.loc.x, rect.loc.y + rect.size.h - width)),
            Size::from((rect.size.w, width)),
        ),
        Rectangle::new(
            Point::from((rect.loc.x, rect.loc.y + width)),
            Size::from((width, inner_h)),
        ),
        Rectangle::new(
            Point::from((rect.loc.x + rect.size.w - width, rect.loc.y + width)),
            Size::from((width, inner_h)),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: i32, y: i32, w: i32, h: i32) -> Rectangle<i32, Logical> {
        Rectangle::new(Point::from((x, y)), Size::from((w, h)))
    }

    fn frame(w: i32, h: i32) -> Frame {
        Frame::new(rect(100, 140, w, h), &Metrics::default(), true)
    }

    #[test]
    fn the_titlebar_sits_directly_above_the_surface() {
        let m = Metrics::default();
        let f = frame(400, 300);
        assert_eq!(f.titlebar.loc.y + f.titlebar.size.h, f.window.loc.y);
        assert_eq!(f.titlebar.size.h, m.titlebar_height as i32);
        assert_eq!(f.titlebar.size.w, f.window.size.w);
    }

    #[test]
    fn the_outer_rectangle_contains_everything() {
        let f = frame(400, 300);
        assert!(f.outer.contains_rect(f.window));
        assert!(f.outer.contains_rect(f.titlebar));
        assert_eq!(f.window.loc.y - f.outer.loc.y, f.titlebar.size.h + f.border);
    }

    #[test]
    fn an_undecorated_window_has_no_frame_at_all() {
        let f = Frame::new(rect(0, 0, 400, 300), &Metrics::default(), false);
        assert!(!f.is_decorated());
        assert_eq!(f.outer, f.window);
        assert!(buttons(&f, &Metrics::default()).is_empty());
        assert_eq!(part_at(&f, &Metrics::default(), (10.0, 10.0).into()), None);
    }

    #[test]
    fn buttons_are_right_aligned_close_outermost() {
        let m = Metrics::default();
        let f = frame(400, 300);
        let b = buttons(&f, &m);
        assert_eq!(b.len(), 3);
        assert_eq!(b[0].0, Part::Close);
        let close = b[0].1;
        assert_eq!(close.loc.x + close.size.w, f.titlebar.loc.x + f.titlebar.size.w - 8);
        assert!(b[1].1.loc.x + b[1].1.size.w <= b[0].1.loc.x);
        assert!(b[2].1.loc.x + b[2].1.size.w <= b[1].1.loc.x);
    }

    #[test]
    fn buttons_are_vertically_centred_in_the_bar() {
        let m = Metrics::default();
        let f = frame(400, 300);
        let (_, close) = buttons(&f, &m)[0];
        let above = close.loc.y - f.titlebar.loc.y;
        let below = (f.titlebar.loc.y + f.titlebar.size.h) - (close.loc.y + close.size.h);
        assert!((above - below).abs() <= 1);
    }

    #[test]
    fn a_narrow_window_drops_its_buttons_rather_than_overflowing() {
        let m = Metrics::default();
        let f = frame(40, 300);
        assert!(buttons(&f, &m).is_empty(), "buttons must never be drawn outside the frame");
        assert!(caption_area(&f, &m).size.w >= 0);
    }

    #[test]
    fn the_caption_never_runs_under_the_buttons() {
        let m = Metrics::default();
        let f = frame(400, 300);
        let caption = caption_area(&f, &m);
        let leftmost_button = buttons(&f, &m).iter().map(|(_, r)| r.loc.x).min().unwrap();
        assert!(caption.loc.x + caption.size.w <= leftmost_button);
    }

    #[test]
    fn hit_testing_agrees_with_the_drawn_buttons() {
        let m = Metrics::default();
        let f = frame(400, 300);
        for (part, rect) in buttons(&f, &m) {
            let centre = Point::<f64, Logical>::from((
                (rect.loc.x + rect.size.w / 2) as f64,
                (rect.loc.y + rect.size.h / 2) as f64,
            ));
            assert_eq!(part_at(&f, &m, centre), Some(part), "{part:?}");
        }
    }

    #[test]
    fn the_client_surface_is_never_claimed_by_the_frame() {
        let m = Metrics::default();
        let f = frame(400, 300);
        let inside = Point::<f64, Logical>::from((200.0, 200.0));
        assert!(f.window.contains(Point::<i32, Logical>::from((200, 200))));
        assert_eq!(part_at(&f, &m, inside), None);
    }

    #[test]
    fn empty_space_on_the_bar_is_a_drag_handle() {
        let m = Metrics::default();
        let f = frame(400, 300);
        let p = Point::<f64, Logical>::from((f.titlebar.loc.x as f64 + 20.0, f.titlebar.loc.y as f64 + 8.0));
        assert_eq!(part_at(&f, &m, p), Some(Part::Titlebar));
    }

    #[test]
    fn points_outside_the_frame_belong_to_nobody() {
        let m = Metrics::default();
        let f = frame(400, 300);
        assert_eq!(part_at(&f, &m, (0.0, 0.0).into()), None);
        assert_eq!(part_at(&f, &m, (10_000.0, 10_000.0).into()), None);
    }

    #[test]
    fn only_the_hovered_button_gets_a_plate() {
        let p = Palette::default();
        assert_eq!(button_background(Part::Close, None, &p), None);
        assert_eq!(button_background(Part::Close, Some(Part::Minimize), &p), None);
        assert_eq!(button_background(Part::Close, Some(Part::Close), &p), Some(p.danger));
        assert_eq!(button_background(Part::Minimize, Some(Part::Minimize), &p), Some(p.overlay));
    }

    #[test]
    fn only_the_hovered_button_gets_a_plate_element() {
        let m = Metrics::default();
        let p = Palette::default();
        let f = frame(400, 300);
        assert!(button_plates(&mut RenderCache::default(), 1, &f, &m, &p, None, 1.0, 1.0).is_empty());
        assert_eq!(button_plates(&mut RenderCache::default(), 1, &f, &m, &p, Some(Part::Close), 1.0, 1.0).len(), 1);
    }

    #[test]
    fn the_fallback_frame_draws_a_bar_and_four_edges() {
        let p = Palette::default();
        let f = frame(400, 300);
        assert_eq!(fallback_frame(&mut RenderCache::default(), 1, &f, &p, true, 1.0, 1.0).len(), 5);
    }

    #[test]
    fn an_undecorated_window_has_nothing_to_fall_back_to() {
        let p = Palette::default();
        let f = Frame::new(rect(0, 0, 400, 300), &Metrics::default(), false);
        assert!(fallback_frame(&mut RenderCache::default(), 1, &f, &p, true, 1.0, 1.0).is_empty());
    }

    #[test]
    fn ring_edges_tile_the_ring_without_overlapping() {
        let [top, bottom, left, right] = ring_edges(rect(0, 0, 50, 40), 2);
        let area: i32 = [top, bottom, left, right].iter().map(|r| r.size.w * r.size.h).sum();
        assert_eq!(area, 50 * 40 - 46 * 36);
    }
}

#[cfg(test)]
mod edge_tests {
    use super::*;

    fn frame() -> Frame {
        let window = Rectangle::new(Point::from((100, 130)), Size::from((400, 300)));
        let titlebar = Rectangle::new(Point::from((100, 100)), Size::from((400, 30)));
        let outer = Rectangle::new(Point::from((98, 98)), Size::from((404, 334)));
        Frame { window, titlebar, outer, border: 2 }
    }

    fn at(x: f64, y: f64) -> Option<Edges> {
        edges_at(&frame(), Point::from((x, y)))
    }

    #[test]
    fn the_right_border_resizes_sideways() {
        assert_eq!(at(503.0, 250.0), Some(Edges { right: true, ..Edges::default() }));
    }

    #[test]
    fn a_few_pixels_outside_still_count() {
        assert_eq!(at(94.0, 250.0), Some(Edges { left: true, ..Edges::default() }));
        assert_eq!(at(300.0, 436.0), Some(Edges { bottom: true, ..Edges::default() }));
    }

    #[test]
    fn corners_resize_both_ways() {
        let bottom_right = Edges { right: true, bottom: true, ..Edges::default() };
        let top_left = Edges { left: true, top: true, ..Edges::default() };
        assert_eq!(at(500.5, 431.0), Some(bottom_right));
        assert_eq!(at(95.0, 95.0), Some(top_left));
    }

    #[test]
    fn the_title_bar_and_the_content_do_not_resize() {
        assert_eq!(at(300.0, 115.0), None);
        assert_eq!(at(300.0, 250.0), None);
        assert_eq!(at(300.0, 60.0), None);
    }

    #[test]
    fn the_top_border_above_the_title_bar_resizes() {
        assert_eq!(at(300.0, 98.0), Some(Edges { top: true, ..Edges::default() }));
    }
}
