//! Rendering: the element types Spectre adds on top of client surfaces.

pub mod decorations;
mod pattern;
mod text;

pub use decorations::{Frame, Part};
pub use pattern::PatternShader;
pub use text::TextCache;

use smithay::backend::renderer::element::memory::MemoryRenderBufferRenderElement;
use smithay::backend::renderer::element::solid::SolidColorRenderElement;
use smithay::backend::renderer::element::surface::WaylandSurfaceRenderElement;
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::gles::element::PixelShaderElement;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::desktop::space::{space_render_elements, SpaceRenderElements};
use smithay::output::Output;
use smithay::render_elements;
use smithay::utils::{Logical, Physical, Point, Rectangle, Scale};
use spectre_theme::Color;

use crate::state::Spectre;

/// Client surfaces and everything Spectre stacks around them.
type WindowElement = SpaceRenderElements<GlesRenderer, WaylandSurfaceRenderElement<GlesRenderer>>;

render_elements! {
    /// Everything drawn on an output: client surfaces plus the desktop
    /// backdrop, window outlines and the focus accent.
    pub SpectreElement<=GlesRenderer>;
    Space = WindowElement,
    Text = MemoryRenderBufferRenderElement<GlesRenderer>,
    Solid = SolidColorRenderElement,
    Pattern = PixelShaderElement,
}

/// Caption font size, in logical pixels.
const CAPTION_SIZE: f32 = 13.0;
/// Glyphs drawn on the title bar buttons.
const MINIMIZE_GLYPH: &str = "\u{2212}";
const MAXIMIZE_GLYPH: &str = "\u{25a1}";
const RESTORE_GLYPH: &str = "\u{2750}";
const CLOSE_GLYPH: &str = "\u{2715}";

/// Build the full front-to-back element list for one output.
///
/// Order matters and is deliberate:
/// 1. client surfaces and layer shells, as smithay stacks them,
/// 2. window outlines *below* the surfaces, so a window on top correctly
///    covers the outline of a window underneath it,
/// 3. the desktop backdrop last, at the very bottom.
pub fn output_elements(
    state: &Spectre,
    output: &Output,
    renderer: &mut GlesRenderer,
    shader: Option<&PatternShader>,
) -> Vec<SpectreElement> {
    let scale = output.current_scale().fractional_scale();
    let theme = &state.config.theme;
    let metrics = theme.metrics;
    let glow = state.config.effects.rgb_glow;

    let mut elements: Vec<SpectreElement> = Vec::new();

    let space = state.workspaces.active();
    match space_render_elements(renderer, [space], output, 1.0) {
        Ok(surfaces) => elements.extend(surfaces.into_iter().map(SpectreElement::Space)),
        Err(err) => tracing::warn!(?err, "failed to collect surface elements"),
    }

    let pointer = state.pointer.current_location();
    let phase = state.pattern_phase();
    let mut cache = state.text.borrow_mut();

    for window in space.elements() {
        let Some(geometry) = space.element_geometry(window) else {
            continue;
        };
        let focused = state.focus.as_ref() == Some(window);
        let frame = Frame::new(geometry, &metrics, state.is_decorated(window));
        let hovered = decorations::part_at(&frame, &metrics, pointer);

        if frame.is_decorated() {
            elements.extend(
                decoration_text(state, &frame, window, focused, hovered, &mut cache, renderer, scale)
                    .into_iter()
                    .map(SpectreElement::Text),
            );
        }

        // The Spectre Pattern goes in before the frame so it lands *above* the
        // title bar background and *below* the caption: contour lines read as
        // texture in the bar rather than as marks drawn over the text.
        if frame.is_decorated() {
            if let Some(pattern) = shader.and_then(|shader| {
                shader.element(
                    &theme.window_pattern,
                    frame.titlebar,
                    theme.palette.titlebar(focused),
                    theme.palette.accent.sample(if focused { 0.5 } else { 0.0 }),
                    if focused { phase } else { 0.0 },
                    scale,
                )
            }) {
                elements.push(SpectreElement::Pattern(pattern));
            }
        }

        elements.extend(
            decorations::frame_elements(
                &frame,
                &metrics,
                &theme.palette,
                focused,
                hovered,
                if focused { glow } else { 0.0 },
                scale,
            )
            .into_iter()
            .map(SpectreElement::Solid),
        );
    }
    drop(cache);

    if let Some(area) = state.workspaces.output_geometry(output) {
        let backdrop = shader.and_then(|shader| {
            shader.element(
                &theme.desktop_pattern,
                area,
                theme.palette.base,
                theme.palette.accent.sample(0.5),
                state.pattern_phase(),
                scale,
            )
        });
        match backdrop {
            Some(element) => elements.push(SpectreElement::Pattern(element)),
            None => elements.extend(solid(area, theme.palette.base, scale).map(SpectreElement::Solid)),
        }
    }

    elements
}

/// The caption and the button glyphs for one window.
#[allow(clippy::too_many_arguments)]
fn decoration_text(
    state: &Spectre,
    frame: &Frame,
    window: &smithay::desktop::Window,
    focused: bool,
    hovered: Option<Part>,
    cache: &mut TextCache,
    renderer: &mut GlesRenderer,
    scale: f64,
) -> Vec<MemoryRenderBufferRenderElement<GlesRenderer>> {
    use spectre_text::Label;

    let theme = &state.config.theme;
    let metrics = theme.metrics;
    let mut out = Vec::new();

    // Caption, centred in the space the buttons leave over.
    let area = decorations::caption_area(frame, &metrics);
    if area.size.w > 0 {
        let title = state.window_title(window);
        let label = Label::new(&title)
            .size(CAPTION_SIZE)
            .color(theme.palette.titlebar_text(focused))
            .bold(focused)
            .max_width(area.size.w as u32);
        let size = cache.measure(&label);
        let location = Point::from((
            area.loc.x + (area.size.w - size.w).max(0) / 2,
            area.loc.y + (area.size.h - size.h).max(0) / 2,
        ));
        out.extend(cache.element(renderer, &label, location, scale));
    }

    // Button glyphs, centred in their hit boxes.
    let maximized = state.is_maximized(window);
    for (part, rect) in decorations::buttons(frame, &metrics) {
        let glyph = match part {
            Part::Minimize => MINIMIZE_GLYPH,
            Part::Maximize if maximized => RESTORE_GLYPH,
            Part::Maximize => MAXIMIZE_GLYPH,
            Part::Close => CLOSE_GLYPH,
            Part::Titlebar | Part::Border => continue,
        };
        let color = match (hovered == Some(part), part) {
            (true, Part::Close) => theme.palette.text,
            (true, _) => theme.palette.text,
            (false, _) if focused => theme.palette.text_dim,
            (false, _) => theme.palette.text_muted,
        };
        let label = Label::new(glyph).size(CAPTION_SIZE).color(color);
        let size = cache.measure(&label);
        let location = Point::from((
            rect.loc.x + (rect.size.w - size.w).max(0) / 2,
            rect.loc.y + (rect.size.h - size.h).max(0) / 2,
        ));
        out.extend(cache.element(renderer, &label, location, scale));
    }

    out
}

/// A filled rectangle in logical coordinates.
///
/// Kept as a helper because decoration drawing needs a dozen of these per
/// window and the physical conversion is easy to get subtly wrong.
pub fn solid(
    area: Rectangle<i32, Logical>,
    color: Color,
    scale: f64,
) -> Option<SolidColorRenderElement> {
    if area.size.w <= 0 || area.size.h <= 0 || color.a <= 0.0 {
        return None;
    }
    let scale = Scale::from(scale);
    let geometry: Rectangle<i32, Physical> = area.to_physical_precise_round(scale);
    Some(SolidColorRenderElement::new(
        smithay::backend::renderer::element::Id::new(),
        geometry,
        smithay::backend::renderer::utils::CommitCounter::default(),
        color.to_premultiplied(),
        Kind::Unspecified,
    ))
}

/// Split `area` into `segments` horizontal strips.
///
/// Used to fake a gradient with solid rectangles on the accent bar: a real
/// gradient shader is not worth a second program for a 1px line, and eight
/// steps are indistinguishable at that thickness.
pub fn horizontal_steps(
    area: Rectangle<i32, Logical>,
    segments: u32,
) -> impl Iterator<Item = (Rectangle<i32, Logical>, f32)> {
    let segments = segments.max(1);
    let width = area.size.w;
    (0..segments).filter_map(move |i| {
        let start = (width as i64 * i as i64 / segments as i64) as i32;
        let end = (width as i64 * (i + 1) as i64 / segments as i64) as i32;
        if end <= start {
            return None;
        }
        let rect = Rectangle::new(
            (area.loc.x + start, area.loc.y).into(),
            (end - start, area.size.h).into(),
        );
        let t = if segments == 1 { 0.5 } else { i as f32 / (segments - 1) as f32 };
        Some((rect, t))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: i32, y: i32, w: i32, h: i32) -> Rectangle<i32, Logical> {
        Rectangle::new((x, y).into(), (w, h).into())
    }

    #[test]
    fn empty_rectangles_produce_no_element() {
        assert!(solid(rect(0, 0, 0, 10), Color::hex(0xffffff), 1.0).is_none());
        assert!(solid(rect(0, 0, 10, 0), Color::hex(0xffffff), 1.0).is_none());
    }

    #[test]
    fn fully_transparent_colours_produce_no_element() {
        assert!(solid(rect(0, 0, 10, 10), Color::TRANSPARENT, 1.0).is_none());
    }

    #[test]
    fn steps_tile_the_area_exactly() {
        let area = rect(10, 4, 101, 2);
        let steps: Vec<_> = horizontal_steps(area, 8).collect();
        assert_eq!(steps.first().unwrap().0.loc.x, 10);
        let last = steps.last().unwrap().0;
        assert_eq!(last.loc.x + last.size.w, 111, "must reach the right edge");
        let covered: i32 = steps.iter().map(|(r, _)| r.size.w).sum();
        assert_eq!(covered, area.size.w, "no gaps and no overlap");
    }

    #[test]
    fn steps_walk_the_gradient_from_zero_to_one() {
        let steps: Vec<_> = horizontal_steps(rect(0, 0, 80, 1), 8).collect();
        assert_eq!(steps.first().unwrap().1, 0.0);
        assert_eq!(steps.last().unwrap().1, 1.0);
    }

    #[test]
    fn a_narrow_area_never_yields_zero_width_rectangles() {
        for (r, _) in horizontal_steps(rect(0, 0, 3, 1), 8) {
            assert!(r.size.w > 0);
        }
    }
}
