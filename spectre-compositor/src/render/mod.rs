//! Rendering: the element types Spectre adds on top of client surfaces.

pub mod cursor;
pub mod decorations;
mod pattern;
mod banded;
mod cache;
mod rounded;
mod text;
mod wallpaper;

pub use cursor::CursorImage;
pub use decorations::{Frame, Part};
pub use pattern::PatternShader;
pub use banded::Banded;
pub use cache::{RenderCache, Slot};
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

/// A client surface, whether it belongs to a window or a layer shell.
type SurfaceElement = WaylandSurfaceRenderElement<GlesRenderer>;

render_elements! {
    /// Everything one workspace contributes: its client surfaces plus the
    /// decorations Spectre draws around them.
    pub WorkspaceElement<=GlesRenderer>;
    Surface = SurfaceElement,
    Rounded = RoundedElement<SurfaceElement>,
    Text = MemoryRenderBufferRenderElement<GlesRenderer>,
    Solid = SolidColorRenderElement,
    Pattern = Banded<PixelShaderElement>,
}

/// A workspace element moved and scaled for a transition.
type MovedElement = RelocateRenderElement<RescaleRenderElement<WorkspaceElement>>;

render_elements! {
    /// Everything drawn on an output. A workspace at rest is drawn as-is; one
    /// taking part in a transition is offset and scaled first.
    pub SpectreElement<=GlesRenderer>;
    Plain = WorkspaceElement,
    Moved = MovedElement,
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

    // The pointer sits above everything, including the panel.
    elements.extend(
        cursor_elements(state, output, renderer, scale)
            .into_iter()
            .map(SpectreElement::Plain),
    );

    // Panels and other layer surfaces belong to the output, not to a
    // workspace: they stay put while workspaces move past underneath, and they
    // must be collected once, because two copies of the same surface in one
    // frame confuses damage tracking.
    elements.extend(
        layer_elements(output, renderer, scale, true)
            .into_iter()
            .map(WorkspaceElement::Surface)
            .map(SpectreElement::Plain),
    );

    match state.transition.as_ref() {
        // A switch is in progress: draw the workspace being entered over the
        // one being left, each where the transition says it belongs.
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

    // The backdrop is not part of any workspace: it stays put while they move.
    if let Some(area) = geometry {
        // A wallpaper replaces the flat backdrop; the desktop pattern is only
        // drawn when there is none, since contour lines over a photograph read
        // as dirt on the screen.
        if let Some(element) = wallpaper_element(state, renderer, area, scale) {
            elements.push(SpectreElement::Plain(WorkspaceElement::Text(element)));
            return elements;
        }
        let backdrop = shader.and_then(|shader| {
            shader.element(
                cache,
                Slot::DesktopPattern,
                &theme.desktop_pattern,
                area,
                theme.palette.base,
                &theme.palette.accent,
                state.pattern_phase(),
                state.color_phase(),
                scale,
            )
        });
        let element = backdrop.map(WorkspaceElement::Pattern);
        elements.extend(element.map(SpectreElement::Plain));

        // Under everything: a flat ground for the corners a wallpaper cannot fill.
        let physical: Rectangle<i32, Physical> =
            area.to_physical_precise_round(Scale::from(scale));
        let base = theme.palette.base.to_premultiplied();
        let ground = cache.solid(Slot::Backdrop, physical, base, Kind::Unspecified);
        elements.push(SpectreElement::Plain(WorkspaceElement::Solid(ground)));
    }

    elements
}

/// The pointer, drawn from the client's cursor surface or from Spectre's own
/// arrow when nothing has asked for one.
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
                // Composited rather than handed to the hardware cursor plane:
                // on the virtual GPUs this project targets the plane's position
                // is not always committed, and a pointer that does not move is
                // worse than one that costs a textured quad.
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

/// Where a client's cursor surface wants its tip.
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

/// The wallpaper, if one is loaded for this output size.
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

/// The accent a window's pattern is drawn with.
///
/// An unfocused window keeps the pattern but loses most of its colour, which
/// is what tells the two apart now that the frame itself is plain.
fn accent_for(theme: &spectre_theme::Theme, focused: bool) -> spectre_theme::Gradient {
    if focused {
        theme.palette.accent.clone()
    } else {
        theme.palette.accent.scaled(0.35)
    }
}

/// Layer surfaces for an output.
///
/// `upper` selects the overlay and top layers, which sit above windows; `false`
/// selects bottom and background, which sit below them.
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

/// Offset and scale one element for a transition.
fn move_element(
    element: WorkspaceElement,
    placement: crate::transition::Placement,
    scale: f64,
) -> MovedElement {
    // Scaling happens about the output's origin, then the result is shifted;
    // doing it the other way round would make the offset scale too.
    let scaled = RescaleRenderElement::from_element(
        element,
        Point::<i32, Physical>::from((0, 0)),
        placement.scale,
    );
    let offset = Point::<i32, Physical>::from(((placement.offset_x as f64 * scale) as i32, 0));
    RelocateRenderElement::from_element(scaled, offset, Relocate::Relative)
}

/// Everything one workspace draws, front to back.
///
/// `alpha` fades the whole workspace, which is what the fade and depth
/// transitions use.
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

    // Front to back, which is the order the renderer wants.
    for window in space.elements().rev() {
        let Some(geometry) = space.element_geometry(window) else {
            continue;
        };
        let Some(location) = space.element_location(window) else {
            continue;
        };
        let focused = state.focus.as_ref() == Some(window);
        let decorated = state.is_decorated(window);
        // The surface's protocol id is stable for as long as the window lives,
        // which is exactly how long its cached render elements should live.
        let key = element_key(window);

        // Everything below works in output-local coordinates.
        let local = Rectangle::new(geometry.loc - region.loc, geometry.size);
        let frame = Frame::new(local, &metrics, decorated);
        // Hit testing still happens in global coordinates, so the hovered part
        // is worked out from the untranslated frame.
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

        // The client's own surfaces, clipped to the window's rounded corners.
        let radius = (metrics.corner_radius as f64 * scale) as f32;
        let corners = if decorated {
            // The frame has already rounded the top two.
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

        // The frame itself: rounded title bar, hairline border, pattern.
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
            // No frame shader: square corners rather than no frame at all.
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
    alpha: f32,
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

/// A stable per-window key for [`Slot`], taken from the toplevel surface.
fn element_key(window: &smithay::desktop::Window) -> u32 {
    use smithay::reexports::wayland_server::Resource;
    use smithay::wayland::shell::xdg::ToplevelSurface;
    window
        .toplevel()
        .map(ToplevelSurface::wl_surface)
        .map(|s| s.id().protocol_id())
        .unwrap_or(0)
}

/// A filled rectangle in logical coordinates.
///
/// Kept as a helper because decoration drawing needs a dozen of these per
/// window and the physical conversion is easy to get subtly wrong.
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
