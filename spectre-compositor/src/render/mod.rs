pub mod cursor;
pub mod decorations;
mod pattern;
mod banded;
mod cache;
mod contour;
mod rounded;
mod text;
mod wallpaper;

pub use cursor::CursorImage;
pub use decorations::{Frame, Part};
pub use pattern::PatternShader;
pub use banded::Banded;
pub use cache::{RenderCache, Slot};
pub use contour::{ContourField, Contoured};
pub use rounded::{Corners, RoundedElement};
pub use text::TextCache;
pub use wallpaper::Wallpaper;

use smithay::backend::renderer::element::memory::MemoryRenderBufferRenderElement;
use smithay::backend::renderer::element::solid::SolidColorRenderElement;
use smithay::backend::renderer::element::surface::WaylandSurfaceRenderElement;
use smithay::backend::renderer::element::utils::{
    Relocate, RelocateRenderElement, RescaleRenderElement,
};
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::gles::element::PixelShaderElement;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::renderer::element::AsRenderElements;
use smithay::desktop::layer_map_for_output;
use smithay::wayland::shell::wlr_layer::Layer as WlrLayer;
use smithay::output::Output;
use smithay::render_elements;
use smithay::utils::{Logical, Physical, Point, Rectangle, Scale};
use spectre_theme::Color;

use crate::state::Spectre;

type SurfaceElement = WaylandSurfaceRenderElement<GlesRenderer>;

render_elements! {
    pub WorkspaceElement<=GlesRenderer>;
    Surface = SurfaceElement,
    Rounded = RoundedElement<SurfaceElement>,
    Text = MemoryRenderBufferRenderElement<GlesRenderer>,
    Solid = SolidColorRenderElement,
    Pattern = Banded<PixelShaderElement>,
    Contour = Contoured<MemoryRenderBufferRenderElement<GlesRenderer>>,
}

type MovedElement = RelocateRenderElement<RescaleRenderElement<WorkspaceElement>>;

render_elements! {
    pub SpectreElement<=GlesRenderer>;
    Plain = WorkspaceElement,
    Moved = MovedElement,
}

const CAPTION_SIZE: f32 = 13.0;
const MINIMIZE_GLYPH: &str = "\u{2212}";
const MAXIMIZE_GLYPH: &str = "\u{25a1}";
const RESTORE_GLYPH: &str = "\u{2750}";
const CLOSE_GLYPH: &str = "\u{2715}";

pub fn output_elements(
    state: &Spectre,
    output: &Output,
    renderer: &mut GlesRenderer,
    shader: Option<&PatternShader>,
    cache: &mut RenderCache,
) -> Vec<SpectreElement> {
    cache.begin_frame();
    let elements = build_output_elements(state, output, renderer, shader, cache);
    cache.end_frame();
    elements
}

fn build_output_elements(
    state: &Spectre,
    output: &Output,
    renderer: &mut GlesRenderer,
    shader: Option<&PatternShader>,
    cache: &mut RenderCache,
) -> Vec<SpectreElement> {
    let scale = output.current_scale().fractional_scale();
    let theme = &state.config.theme;
    let geometry = state.workspaces.output_geometry(output);
    let width = geometry.map(|g| g.size.w).unwrap_or(0);

    let mut elements: Vec<SpectreElement> = Vec::new();

    elements.extend(
        cursor_elements(state, output, renderer, scale)
            .into_iter()
            .map(SpectreElement::Plain),
    );

    elements.extend(
        layer_elements(output, renderer, scale, true)
            .into_iter()
            .map(WorkspaceElement::Surface)
            .map(SpectreElement::Plain),
    );

    match state.transition.as_ref() {
        Some(transition) => {
            let (from, to) = transition.placements(std::time::Instant::now(), width);
            tracing::trace!(?from, ?to, width, "transition frame");
            for (index, placement) in [(transition.to, to), (transition.from, from)] {
                if !placement.is_visible() {
                    continue;
                }
                let workspace =
                    workspace_elements(state, output, renderer, shader, cache, index, placement.alpha);
                elements.extend(
                    workspace
                        .into_iter()
                        .map(|element| move_element(element, placement, scale))
                        .map(SpectreElement::Moved),
                );
            }
        }
        None => {
            let index = state.workspaces.active_index();
            elements.extend(
                workspace_elements(state, output, renderer, shader, cache, index, 1.0)
                    .into_iter()
                    .map(SpectreElement::Plain),
            );
        }
    }

    elements.extend(
        layer_elements(output, renderer, scale, false)
            .into_iter()
            .map(WorkspaceElement::Surface)
            .map(SpectreElement::Plain),
    );

    if let Some(area) = geometry {
        if let Some(element) = wallpaper_element(state, renderer, area, scale) {
            elements.push(SpectreElement::Plain(WorkspaceElement::Text(element)));
            return elements;
        }
        let backdrop = contour_element(state, renderer, cache, shader, area, scale);
        match backdrop {
            Some(element) => elements.push(SpectreElement::Plain(element)),
            None => {
                let drawn = shader.and_then(|shader| {
                    shader.element(
                        cache,
                        Slot::DesktopPattern,
                        &theme.desktop_pattern,
                        area,
                        theme.palette.base,
                        &theme.palette.accent,
                        state.desktop_phase(),
                        state.desktop_color_phase(),
                        scale,
                    )
                });
                elements.extend(drawn.map(WorkspaceElement::Pattern).map(SpectreElement::Plain));
            }
        }

        let physical: Rectangle<i32, Physical> =
            area.to_physical_precise_round(Scale::from(scale));
        let base = theme.palette.base.to_premultiplied();
        let ground = cache.solid(Slot::Backdrop, physical, base, Kind::Unspecified);
        elements.push(SpectreElement::Plain(WorkspaceElement::Solid(ground)));
    }

    elements
}

pub fn dump_scene(elements: &[SpectreElement], scale: f64) {
    if std::env::var_os("SPECTRE_DUMP_SCENE").is_none() {
        return;
    }
    use smithay::backend::renderer::element::Element;
    let scale = Scale::from(scale);
    for (i, element) in elements.iter().enumerate() {
        tracing::debug!(
            i,
            geo = ?element.geometry(scale),
            opaque = element.opaque_regions(scale).len(),
            "scene element"
        );
    }
    tracing::debug!(count = elements.len(), "scene end");
}

fn contour_element(
    state: &Spectre,
    renderer: &mut GlesRenderer,
    cache: &mut RenderCache,
    shader: Option<&PatternShader>,
    area: Rectangle<i32, Logical>,
    scale: f64,
) -> Option<WorkspaceElement> {
    let theme = &state.config.theme;
    let pattern = &theme.desktop_pattern;
    if std::env::var_os("SPECTRE_NO_CONTOUR").is_some() {
        return None;
    }
    let program = shader?.contour_program()?;

    let physical: Rectangle<i32, Physical> = area.to_physical_precise_round(Scale::from(scale));
    let field = ContourField::prepare(
        cache.contour(),
        pattern,
        physical.size,
        scale,
        state.desktop_phase(),
    )?;

    let element = MemoryRenderBufferRenderElement::from_buffer(
        renderer,
        physical.loc.to_f64(),
        field.buffer(),
        None,
        Some(field.source()),
        Some(area.size),
        Kind::Unspecified,
    )
    .ok()?;

    let stops = pattern.line_stops(&theme.palette.accent, theme.palette.base);
    Contoured::new(
        element,
        Some(program),
        field.commit(),
        &stops,
        state.desktop_color_phase(),
        PatternShader::COLOR_SPAN,
        theme.palette.base,
        field.uv_window(),
    )
    .ok()
    .map(WorkspaceElement::Contour)
}

fn cursor_elements(
    state: &Spectre,
    output: &Output,
    renderer: &mut GlesRenderer,
    scale: f64,
) -> Vec<WorkspaceElement> {
    use smithay::input::pointer::CursorImageStatus;

    let Some(area) = state.workspaces.output_geometry(output) else {
        return Vec::new();
    };
    let pointer = state.pointer_position();
    if !area.to_f64().contains(pointer) {
        return Vec::new();
    }
    let local = pointer - area.loc.to_f64();

    match &state.cursor_status {
        CursorImageStatus::Hidden => Vec::new(),
        CursorImageStatus::Surface(surface) => {
            let hotspot = cursor_hotspot(surface);
            let location = (local - hotspot.to_f64()).to_physical_precise_round(scale);
            smithay::backend::renderer::element::surface::render_elements_from_surface_tree(
                renderer,
                surface,
                location,
                Scale::from(scale),
                1.0,
                Kind::Unspecified,
            )
            .into_iter()
            .map(WorkspaceElement::Surface)
            .collect()
        }
        CursorImageStatus::Named(_) => {
            let Some(image) = state.cursor.as_ref() else {
                return Vec::new();
            };
            let location: Point<i32, Physical> = local.to_physical_precise_round(scale);
            let location = location - Point::from(image.hotspot);
            MemoryRenderBufferRenderElement::from_buffer(
                renderer,
                location.to_f64(),
                &image.buffer,
                None,
                None,
                None,
                Kind::Unspecified,
            )
            .ok()
            .map(WorkspaceElement::Text)
            .into_iter()
            .collect()
        }
    }
}

fn cursor_hotspot(surface: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface) -> Point<i32, Logical> {
    use smithay::input::pointer::CursorImageSurfaceData;
    smithay::wayland::compositor::with_states(surface, |states| {
        states
            .data_map
            .get::<CursorImageSurfaceData>()
            .map(|data| data.lock().unwrap().hotspot)
            .unwrap_or_default()
    })
}

fn wallpaper_element(
    state: &Spectre,
    renderer: &mut GlesRenderer,
    area: Rectangle<i32, Logical>,
    scale: f64,
) -> Option<MemoryRenderBufferRenderElement<GlesRenderer>> {
    let wallpaper = state.wallpaper.as_ref()?;
    MemoryRenderBufferRenderElement::from_buffer(
        renderer,
        area.loc.to_physical_precise_round(scale),
        &wallpaper.buffer,
        None,
        None,
        None,
        Kind::Unspecified,
    )
    .ok()
}

fn accent_for(theme: &spectre_theme::Theme, focused: bool) -> spectre_theme::Gradient {
    if focused {
        theme.palette.accent.clone()
    } else {
        theme.palette.accent.scaled(0.35)
    }
}

fn layer_elements(
    output: &Output,
    renderer: &mut GlesRenderer,
    scale: f64,
    upper: bool,
) -> Vec<SurfaceElement> {
    let map = layer_map_for_output(output);
    let mut elements = Vec::new();

    for layer in map.layers().rev() {
        let is_upper = matches!(layer.layer(), WlrLayer::Overlay | WlrLayer::Top);
        if is_upper != upper {
            continue;
        }
        let Some(geometry) = map.layer_geometry(layer) else {
            continue;
        };
        elements.extend(AsRenderElements::<GlesRenderer>::render_elements::<SurfaceElement>(
            layer,
            renderer,
            geometry.loc.to_physical_precise_round(scale),
            Scale::from(scale),
            1.0,
        ));
    }
    elements
}

fn move_element(
    element: WorkspaceElement,
    placement: crate::transition::Placement,
    scale: f64,
) -> MovedElement {
    let scaled = RescaleRenderElement::from_element(
        element,
        Point::<i32, Physical>::from((0, 0)),
        placement.scale,
    );
    let offset = Point::<i32, Physical>::from(((placement.offset_x as f64 * scale) as i32, 0));
    RelocateRenderElement::from_element(scaled, offset, Relocate::Relative)
}

fn workspace_elements(
    state: &Spectre,
    output: &Output,
    renderer: &mut GlesRenderer,
    shader: Option<&PatternShader>,
    cache: &mut RenderCache,
    index: usize,
    alpha: f32,
) -> Vec<WorkspaceElement> {
    let scale = output.current_scale().fractional_scale();
    let theme = &state.config.theme;
    let metrics = theme.metrics;

    let Some(space) = state.workspaces.get(index) else {
        return Vec::new();
    };
    let Some(region) = space.output_geometry(output) else {
        return Vec::new();
    };

    let pointer = state.pointer_position();
    let phase = state.pattern_phase();
    let color_phase = state.color_phase();
    let mut text = state.text.borrow_mut();
    let mut elements: Vec<WorkspaceElement> = Vec::new();

    for window in space.elements().rev() {
        let Some(geometry) = space.element_geometry(window) else {
            continue;
        };
        let Some(location) = space.element_location(window) else {
            continue;
        };
        let focused = state.focus.as_ref() == Some(window);
        let decorated = state.is_decorated(window);
        let key = element_key(window);

        let local = Rectangle::new(geometry.loc - region.loc, geometry.size);
        let frame = Frame::new(local, &metrics, decorated);
        let hovered = decorations::part_at(
            &Frame::new(geometry, &metrics, decorated),
            &metrics,
            pointer,
        );

        if decorated {
            elements.extend(
                decoration_text(
                    state, &frame, window, focused, hovered, &mut text, renderer, scale, alpha,
                )
                .into_iter()
                .map(WorkspaceElement::Text),
            );
            elements.extend(
                decorations::button_plates(
                    cache, key, &frame, &metrics, &theme.palette, hovered, alpha, scale,
                )
                .into_iter()
                .map(WorkspaceElement::Solid),
            );
        }

        let radius = (metrics.corner_radius as f64 * scale) as f32;
        let corners = if decorated {
            Corners::bottom(radius)
        } else {
            Corners::uniform(radius)
        };
        let window_physical: Rectangle<i32, Physical> = local.to_physical_precise_round(scale);
        let render_location = location - window.geometry().loc - region.loc;

        let surfaces = AsRenderElements::<GlesRenderer>::render_elements::<SurfaceElement>(
            window,
            renderer,
            render_location.to_physical_precise_round(scale),
            Scale::from(scale),
            alpha,
        );
        for surface in surfaces {
            match RoundedElement::new(
                surface,
                shader.and_then(PatternShader::rounded_program),
                window_physical,
                corners,
            ) {
                Ok(rounded) => elements.push(WorkspaceElement::Rounded(rounded)),
                Err(plain) => elements.push(WorkspaceElement::Surface(plain)),
            }
        }

        if !decorated {
            continue;
        }

        let titlebar_height = frame.titlebar.size.h + frame.border;
        let drawn = shader.and_then(|shader| {
            shader.frame_element(
                cache,
                Slot::Frame(key),
                frame.outer,
                titlebar_height,
                &metrics,
                &theme.palette,
                &theme.window_pattern,
                &accent_for(theme, focused),
                focused,
                if focused { phase } else { 0.0 },
                color_phase,
                alpha,
                scale,
            )
        });
        match drawn {
            Some(element) => elements.push(WorkspaceElement::Pattern(element)),
            None => elements.extend(
                decorations::fallback_frame(
                    cache, key, &frame, &theme.palette, focused, alpha, scale,
                )
                .into_iter()
                .map(WorkspaceElement::Solid),
            ),
        }
    }
    drop(text);

    elements
}

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
    alpha: f32,
) -> Vec<MemoryRenderBufferRenderElement<GlesRenderer>> {
    use spectre_text::Label;

    let theme = &state.config.theme;
    let metrics = theme.metrics;
    let mut out = Vec::new();

    let area = decorations::caption_area(frame, &metrics);
    if area.size.w > 0 {
        let title = state.window_title(window);
        let label = Label::new(&title)
            .size(CAPTION_SIZE)
            .color(theme.palette.titlebar_text(focused).alpha(alpha))
            .bold(focused)
            .max_width(area.size.w as u32);
        let size = cache.measure(&label);
        let location = Point::from((
            area.loc.x + (area.size.w - size.w).max(0) / 2,
            area.loc.y + (area.size.h - size.h).max(0) / 2,
        ));
        out.extend(cache.element(renderer, &label, location, scale));
    }

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
        let label = Label::new(glyph).size(CAPTION_SIZE).color(color.alpha(alpha));
        let size = cache.measure(&label);
        let location = Point::from((
            rect.loc.x + (rect.size.w - size.w).max(0) / 2,
            rect.loc.y + (rect.size.h - size.h).max(0) / 2,
        ));
        out.extend(cache.element(renderer, &label, location, scale));
    }

    out
}

fn element_key(window: &smithay::desktop::Window) -> u32 {
    use smithay::reexports::wayland_server::Resource;
    use smithay::wayland::shell::xdg::ToplevelSurface;
    window
        .toplevel()
        .map(ToplevelSurface::wl_surface)
        .map(|s| s.id().protocol_id())
        .unwrap_or(0)
}

pub fn solid(
    cache: &mut RenderCache,
    slot: Slot,
    area: Rectangle<i32, Logical>,
    color: Color,
    scale: f64,
) -> Option<SolidColorRenderElement> {
    if area.size.w <= 0 || area.size.h <= 0 || color.a <= 0.0 {
        return None;
    }
    let scale = Scale::from(scale);
    let geometry: Rectangle<i32, Physical> = area.to_physical_precise_round(scale);
    Some(cache.solid(slot, geometry, color.to_premultiplied(), Kind::Unspecified))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: i32, y: i32, w: i32, h: i32) -> Rectangle<i32, Logical> {
        Rectangle::new((x, y).into(), (w, h).into())
    }

    #[test]
    fn empty_rectangles_produce_no_element() {
        assert!(solid(&mut RenderCache::default(), Slot::Backdrop, rect(0, 0, 0, 10), Color::hex(0xffffff), 1.0).is_none());
        assert!(solid(&mut RenderCache::default(), Slot::Backdrop, rect(0, 0, 10, 0), Color::hex(0xffffff), 1.0).is_none());
    }

    #[test]
    fn fully_transparent_colours_produce_no_element() {
        assert!(solid(&mut RenderCache::default(), Slot::Backdrop, rect(0, 0, 10, 10), Color::TRANSPARENT, 1.0).is_none());
    }

}
