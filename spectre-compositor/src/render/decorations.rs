//! Window decorations.
//!
//! Spectre draws a thin outline around every window: flat grey when the window
//! is unfocused, and a teal-to-purple accent gradient when it is focused. The
//! gradient is faked with a handful of solid steps, which costs nothing and is
//! indistinguishable at one or two pixels wide.
//!
//! Title bars with captions and buttons need a text rasteriser and a pointer
//! grab; both arrive with the shell phase. Until then the outline carries the
//! focus signal on its own, which keeps the desktop usable exactly as the
//! project principles require.

use smithay::backend::renderer::element::solid::SolidColorRenderElement;
use smithay::utils::{Logical, Rectangle};
use spectre_theme::{Metrics, Palette};

use super::{horizontal_steps, solid};

/// Number of solid steps used to fake the accent gradient along an edge.
const GRADIENT_STEPS: u32 = 12;

/// Build the outline for one window.
///
/// `geometry` is the window's own rectangle in logical coordinates; the outline
/// is drawn just outside it, so it never covers client content.
pub fn window_outline(
    geometry: Rectangle<i32, Logical>,
    focused: bool,
    palette: &Palette,
    metrics: &Metrics,
    glow: f32,
    scale: f64,
) -> Vec<SolidColorRenderElement> {
    let width = metrics.border_width as i32;
    if width <= 0 || geometry.size.w <= 0 || geometry.size.h <= 0 {
        return Vec::new();
    }

    let outer = Rectangle::new(
        (geometry.loc.x - width, geometry.loc.y - width).into(),
        (geometry.size.w + width * 2, geometry.size.h + width * 2).into(),
    );

    let mut elements = Vec::new();

    if focused {
        // The accent runs left-to-right along the top and bottom edges, and the
        // side edges pick up the colour of the corner they touch, so the whole
        // frame reads as one continuous sweep.
        let top = Rectangle::new(outer.loc, (outer.size.w, width).into());
        let bottom =
            Rectangle::new((outer.loc.x, outer.loc.y + outer.size.h - width).into(), (outer.size.w, width).into());

        for edge in [top, bottom] {
            for (rect, t) in horizontal_steps(edge, GRADIENT_STEPS) {
                elements.extend(solid(rect, palette.accent.sample(t), scale));
            }
        }

        let left = Rectangle::new(
            (outer.loc.x, outer.loc.y + width).into(),
            (width, outer.size.h - width * 2).into(),
        );
        let right = Rectangle::new(
            (outer.loc.x + outer.size.w - width, outer.loc.y + width).into(),
            (width, outer.size.h - width * 2).into(),
        );
        elements.extend(solid(left, palette.accent.sample(0.0), scale));
        elements.extend(solid(right, palette.accent.sample(1.0), scale));

        // Optional glow: one extra ring outside the border at low alpha. Costs a
        // single blended quad per edge and is skipped entirely at glow 0.
        if glow > 0.0 {
            let ring = Rectangle::new(
                (outer.loc.x - width, outer.loc.y - width).into(),
                (outer.size.w + width * 2, outer.size.h + width * 2).into(),
            );
            for (i, edge) in ring_edges(ring, width).into_iter().enumerate() {
                let t = if i % 2 == 0 { 0.25 } else { 0.75 };
                elements.extend(solid(edge, palette.accent_glow(t, glow), scale));
            }
        }
    } else {
        for edge in ring_edges(outer, width) {
            elements.extend(solid(edge, palette.border, scale));
        }
    }

    elements
}

/// The four edges of `rect` as `width`-thick rectangles, without overlapping
/// corners (top and bottom span the full width, sides fill the gap between).
fn ring_edges(rect: Rectangle<i32, Logical>, width: i32) -> [Rectangle<i32, Logical>; 4] {
    let inner_h = (rect.size.h - width * 2).max(0);
    [
        Rectangle::new(rect.loc, (rect.size.w, width).into()),
        Rectangle::new(
            (rect.loc.x, rect.loc.y + rect.size.h - width).into(),
            (rect.size.w, width).into(),
        ),
        Rectangle::new((rect.loc.x, rect.loc.y + width).into(), (width, inner_h).into()),
        Rectangle::new(
            (rect.loc.x + rect.size.w - width, rect.loc.y + width).into(),
            (width, inner_h).into(),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: i32, y: i32, w: i32, h: i32) -> Rectangle<i32, Logical> {
        Rectangle::new((x, y).into(), (w, h).into())
    }

    #[test]
    fn a_zero_width_border_draws_nothing() {
        let metrics = Metrics { border_width: 0, ..Default::default() };
        let out =
            window_outline(rect(0, 0, 100, 100), true, &Palette::default(), &metrics, 1.0, 1.0);
        assert!(out.is_empty());
    }

    #[test]
    fn an_empty_window_draws_nothing() {
        let out = window_outline(
            rect(0, 0, 0, 100),
            true,
            &Palette::default(),
            &Metrics::default(),
            0.0,
            1.0,
        );
        assert!(out.is_empty());
    }

    #[test]
    fn an_unfocused_window_gets_exactly_four_edges() {
        let out = window_outline(
            rect(10, 10, 200, 120),
            false,
            &Palette::default(),
            &Metrics::default(),
            1.0,
            1.0,
        );
        assert_eq!(out.len(), 4, "glow must not apply to unfocused windows");
    }

    #[test]
    fn glow_adds_elements_only_when_enabled() {
        let p = Palette::default();
        let m = Metrics::default();
        let without = window_outline(rect(0, 0, 200, 120), true, &p, &m, 0.0, 1.0).len();
        let with = window_outline(rect(0, 0, 200, 120), true, &p, &m, 1.0, 1.0).len();
        assert!(with > without);
    }

    #[test]
    fn ring_edges_do_not_overlap() {
        let [top, bottom, left, right] = ring_edges(rect(0, 0, 50, 40), 2);
        assert_eq!(top.size, (50, 2).into());
        assert_eq!(bottom.loc.y, 38);
        assert_eq!(left.size, (2, 36).into());
        assert_eq!(right.loc.x, 48);
        let area: i32 = [top, bottom, left, right].iter().map(|r| r.size.w * r.size.h).sum();
        assert_eq!(area, 50 * 40 - 46 * 36, "edges must tile the ring exactly");
    }

    #[test]
    fn a_ring_thinner_than_two_borders_has_no_negative_sides() {
        let [_, _, left, right] = ring_edges(rect(0, 0, 10, 2), 3);
        assert!(left.size.h >= 0 && right.size.h >= 0);
    }
}
