pub mod cursor;
pub mod decorations;
mod pattern;
mod banded;
mod cache;
mod contour;
mod cube;
mod rounded;
mod text;
mod wallpaper;

pub use cursor::{CursorImage, ResizeCursors};
pub use decorations::{Edges, Frame, Part};
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
use smithay::backend::renderer::element::texture::{TextureBuffer, TextureRenderElement};
use smithay::backend::renderer::gles::element::PixelShaderElement;
use smithay::backend::renderer::gles::GlesTexture;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::renderer::element::AsRenderElements;
use smithay::desktop::layer_map_for_output;
use smithay::wayland::shell::wlr_layer::Layer as WlrLayer;
use smithay::output::Output;
use smithay::render_elements;
use smithay::utils::{Logical, Physical, Point, Rectangle, Scale, Size, Transform};
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
    Face = cube::CubeFace,
    Snapshot = TextureRenderElement<GlesTexture>,
}

type MovedElement = RelocateRenderElement<RescaleRenderElement<WorkspaceElement>>;

pub const LAUNCHER_NAMESPACE: &str = "spectre-launcher";

const LAUNCHER_KEY: u32 = u32::MAX;

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

    if let Some(overview) = state.overview.as_ref() {
        return overview_elements(state, overview, output, renderer, shader, cache, scale);
    }
    let turning = state.transition.is_some()
        && state.config.effects.workspace_transition == spectre_config::WorkspaceTransition::Cube;
    if !turning && cache.snapshot_size().w != 0 {
        cache.set_snapshots(Vec::new(), Size::from((0, 0)));
    }

    upload_wallpaper(state, renderer, cache);
    capture_closing(state, output, renderer, cache, scale);

    let mut elements: Vec<SpectreElement> = Vec::new();

    let cursor = cursor_elements(state, output, renderer, scale);
    cache.set_cursor_elements(cursor.len());
    elements.extend(cursor.into_iter().map(SpectreElement::Plain));

    let menu = menu_elements(state, output, renderer, cache, scale);
    elements.extend(menu.into_iter().map(SpectreElement::Plain));

    elements.extend(layer_elements(state, output, renderer, cache, scale, true));

    match state.transition.as_ref() {
        Some(transition) if turning => {
            if let Some(cube) =
                cube_switch_elements(state, transition, output, renderer, shader, cache, scale)
            {
                elements.extend(cube);
                return elements;
            }
        }
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
            elements.extend(active_workspace_elements(state, output, renderer, shader, cache, index, scale));
        }
    }

    elements.extend(layer_elements(state, output, renderer, cache, scale, false));

    if let Some(area) = geometry {
        if let Some(element) = wallpaper_element(state, cache, area, scale) {
            elements.push(SpectreElement::Plain(WorkspaceElement::Snapshot(element)));
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

fn overview_elements(
    state: &Spectre,
    overview: &crate::overview::Overview,
    output: &Output,
    renderer: &mut GlesRenderer,
    shader: Option<&PatternShader>,
    cache: &mut RenderCache,
    scale: f64,
) -> Vec<SpectreElement> {
    let mut elements: Vec<SpectreElement> = Vec::new();
    let cursor = cursor_elements(state, output, renderer, scale);
    cache.set_cursor_elements(cursor.len());
    elements.extend(cursor.into_iter().map(SpectreElement::Plain));

    let Some(area) = state.workspaces.output_geometry(output) else {
        return elements;
    };
    let Some(program) = shader.and_then(PatternShader::cube_program) else {
        return elements;
    };
    let physical: Rectangle<i32, Physical> = area.to_physical_precise_round(Scale::from(scale));

    if cache.snapshot_size().w == 0 {
        let (snapshots, size) = capture_workspaces(state, output, renderer, shader, cache, overview.faces(), scale);
        cache.set_snapshots(snapshots, size);
    }

    let now = std::time::Instant::now();
    let commit = cache.face_commit(overview.angle(now));
    let texture_size = cache.snapshot_size();
    let aspect = area.size.h as f32 / area.size.w.max(1) as f32;
    for index in 0..overview.faces() {
        if !overview.is_visible(index, now) {
            continue;
        }
        let Some(buffer) = cache.snapshots().get(index) else {
            continue;
        };
        let view = cube::FaceView {
            angle: overview.face_angle(index, now),
            faces: overview.faces(),
            aspect,
            flip: false,
            zoom: cube::FACE_SIZE,
        };
        let face = cube::CubeFace::new(buffer, texture_size, program, area.size, view, commit);
        elements.push(SpectreElement::Plain(WorkspaceElement::Face(face)));
    }

    let ground = cache.solid(Slot::Backdrop, physical, [0.02, 0.02, 0.03, 1.0], Kind::Unspecified);
    elements.push(SpectreElement::Plain(WorkspaceElement::Solid(ground)));
    elements
}

fn cube_switch_elements(
    state: &Spectre,
    transition: &crate::transition::Transition,
    output: &Output,
    renderer: &mut GlesRenderer,
    shader: Option<&PatternShader>,
    cache: &mut RenderCache,
    scale: f64,
) -> Option<Vec<SpectreElement>> {
    let program = shader.and_then(PatternShader::cube_program)?;
    let area = state.workspaces.output_geometry(output)?;
    let faces = state.workspaces.count();
    if faces < 2 {
        return None;
    }

    if cache.snapshot_size().w == 0 {
        let (snapshots, size) =
            capture_workspaces(state, output, renderer, shader, cache, faces, scale);
        cache.set_snapshots(snapshots, size);
    }

    let now = std::time::Instant::now();
    let position = transition.cube_position(faces, now);
    let zoom = transition.cube_zoom(now);
    let commit = cache.face_commit(position);
    let texture_size = cache.snapshot_size();
    let aspect = area.size.h as f32 / area.size.w.max(1) as f32;

    let mut elements = Vec::new();
    for index in [transition.from, transition.to] {
        let Some(buffer) = cache.snapshots().get(index) else {
            continue;
        };
        let angle = (index as f32 - position) * std::f32::consts::TAU / faces as f32;
        if angle.cos() <= 0.001 {
            continue;
        }
        let view = cube::FaceView { angle, faces, aspect, flip: false, zoom };
        let face = cube::CubeFace::new(buffer, texture_size, program, area.size, view, commit);
        elements.push(SpectreElement::Plain(WorkspaceElement::Face(face)));
    }

    let physical: Rectangle<i32, Physical> = area.to_physical_precise_round(Scale::from(scale));
    let ground = cache.solid(Slot::Backdrop, physical, [0.02, 0.02, 0.03, 1.0], Kind::Unspecified);
    elements.push(SpectreElement::Plain(WorkspaceElement::Solid(ground)));
    Some(elements)
}

fn capture_workspaces(
    state: &Spectre,
    output: &Output,
    renderer: &mut GlesRenderer,
    shader: Option<&PatternShader>,
    cache: &mut RenderCache,
    faces: usize,
    scale: f64,
) -> (
    Vec<smithay::backend::renderer::element::texture::TextureBuffer<smithay::backend::renderer::gles::GlesTexture>>,
    Size<i32, Physical>,
) {
    let Some(area) = state.workspaces.output_geometry(output) else {
        return (Vec::new(), Size::from((1, 1)));
    };
    let physical: Rectangle<i32, Physical> = area.to_physical_precise_round(Scale::from(scale));
    let small = Size::from((
        (physical.size.w as f64 * cube::SNAPSHOT_SCALE).round() as i32,
        (physical.size.h as f64 * cube::SNAPSHOT_SCALE).round() as i32,
    ));

    let mut textures = Vec::new();
    for index in 0..faces {
        let mut scene = workspace_elements(state, output, renderer, shader, cache, index, 1.0);
        match wallpaper_element(state, cache, area, scale) {
            Some(wallpaper) => scene.push(WorkspaceElement::Snapshot(wallpaper)),
            None => {
                if let Some(backdrop) = contour_element(state, renderer, cache, shader, area, scale) {
                    scene.push(backdrop);
                }
            }
        }
        let base = state.config.theme.palette.base.to_premultiplied();
        let ground = cache.solid(Slot::Backdrop, physical, base, Kind::Unspecified);
        scene.push(WorkspaceElement::Solid(ground));

        let mut shrunk = Vec::new();
        for element in scene {
            let origin = Point::<i32, Physical>::from((0, 0));
            shrunk.push(RescaleRenderElement::from_element(element, origin, cube::SNAPSHOT_SCALE));
        }

        let Some(texture) = cube::capture(renderer, small, &shrunk, [0.02, 0.02, 0.03, 1.0]) else {
            break;
        };
        let buffer = smithay::backend::renderer::element::texture::TextureBuffer::from_texture(
            renderer,
            texture,
            1,
            Transform::Normal,
            None,
        );
        textures.push(buffer);
    }
    (textures, small)
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
    contour::update(cache.contour(), pattern, physical.size, scale, state.desktop_phase());
    let field = cache.contour().as_ref()?;

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
        CursorImageStatus::Named(icon) => {
            let Some(image) = named_cursor(state, *icon) else {
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

fn menu_elements(
    state: &Spectre,
    output: &Output,
    renderer: &mut GlesRenderer,
    cache: &mut RenderCache,
    scale: f64,
) -> Vec<WorkspaceElement> {
    use spectre_text::Label;

    let mut elements = Vec::new();
    let Some(menu) = state.window_menu.as_ref() else {
        return elements;
    };
    let Some(area) = state.workspaces.output_geometry(output) else {
        return elements;
    };
    let palette = &state.config.theme.palette;

    let mut text = state.text.borrow_mut();
    for (index, item) in menu.items.iter().enumerate() {
        let rect = on_output(menu.item_rect(index), area);
        let mut color = palette.text_dim;
        if menu.hovered == Some(index) {
            color = palette.text;
        }
        let label = Label::new(item.label()).size(CAPTION_SIZE).color(color);
        let size = text.measure(&label);
        let location = Point::from((rect.loc.x + 12, rect.loc.y + (rect.size.h - size.h).max(0) / 2));
        if let Some(element) = text.element(renderer, &label, location, scale, 1.0) {
            elements.push(WorkspaceElement::Text(element));
        }
    }
    drop(text);

    if let Some(index) = menu.hovered {
        let rect = on_output(menu.item_rect(index), area);
        if let Some(element) = solid(cache, Slot::Menu(2), rect, palette.overlay, scale) {
            elements.push(WorkspaceElement::Solid(element));
        }
    }
    let outer = on_output(menu.rect(), area);
    let inner = Rectangle::new(
        Point::from((outer.loc.x + 1, outer.loc.y + 1)),
        Size::from((outer.size.w - 2, outer.size.h - 2)),
    );
    if let Some(element) = solid(cache, Slot::Menu(1), inner, palette.elevated, scale) {
        elements.push(WorkspaceElement::Solid(element));
    }
    if let Some(element) = solid(cache, Slot::Menu(0), outer, palette.line, scale) {
        elements.push(WorkspaceElement::Solid(element));
    }
    elements
}

fn on_output(rect: Rectangle<i32, Logical>, area: Rectangle<i32, Logical>) -> Rectangle<i32, Logical> {
    Rectangle::new(rect.loc - area.loc, rect.size)
}

fn named_cursor(state: &Spectre, icon: smithay::input::pointer::CursorIcon) -> Option<&CursorImage> {
    use smithay::input::pointer::CursorIcon;
    let Some(resize) = state.resize_cursors.as_ref() else {
        return state.cursor.as_ref();
    };
    match icon {
        CursorIcon::EwResize | CursorIcon::EResize | CursorIcon::WResize | CursorIcon::ColResize => {
            Some(&resize.horizontal)
        }
        CursorIcon::NsResize | CursorIcon::NResize | CursorIcon::SResize | CursorIcon::RowResize => {
            Some(&resize.vertical)
        }
        CursorIcon::NwseResize | CursorIcon::NwResize | CursorIcon::SeResize => Some(&resize.falling),
        CursorIcon::NeswResize | CursorIcon::NeResize | CursorIcon::SwResize => Some(&resize.rising),
        _ => state.cursor.as_ref(),
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

fn upload_wallpaper(state: &Spectre, renderer: &mut GlesRenderer, cache: &mut RenderCache) {
    use smithay::backend::renderer::ImportMem;

    let Some(wallpaper) = state.wallpaper.as_ref() else {
        return;
    };
    let key = wallpaper.key();
    if cache.wallpaper_key() == Some(key.as_str()) {
        return;
    }
    let Some(pixels) = wallpaper.pixels() else {
        return;
    };
    let format = smithay::backend::allocator::Fourcc::Argb8888;
    let size = smithay::utils::Size::<i32, smithay::utils::Buffer>::from(wallpaper.size);
    let texture = match renderer.import_memory(&pixels, format, size, false) {
        Ok(texture) => texture,
        Err(err) => {
            tracing::warn!(?err, "could not upload the wallpaper");
            return;
        }
    };
    let buffer = TextureBuffer::from_texture(renderer, texture, 1, Transform::Normal, None);
    cache.set_wallpaper(buffer, key);

    let Some((soft, soft_size)) = wallpaper::blurred(&pixels, wallpaper.size) else {
        return;
    };
    let size = smithay::utils::Size::<i32, smithay::utils::Buffer>::from(soft_size);
    let Ok(texture) = renderer.import_memory(&soft, format, size, false) else {
        tracing::warn!("could not upload the blurred wallpaper");
        return;
    };
    let buffer = TextureBuffer::from_texture(renderer, texture, 1, Transform::Normal, None);
    cache.set_blur(buffer, soft_size);
}

fn wallpaper_element(
    state: &Spectre,
    cache: &RenderCache,
    area: Rectangle<i32, Logical>,
    scale: f64,
) -> Option<TextureRenderElement<GlesTexture>> {
    state.wallpaper.as_ref()?;
    let buffer = cache.wallpaper()?;
    let location: Point<i32, Physical> = area.loc.to_physical_precise_round(scale);
    Some(TextureRenderElement::from_texture_buffer(
        location.to_f64(),
        buffer,
        None,
        None,
        None,
        Kind::Unspecified,
    ))
}

fn blur_element(
    state: &Spectre,
    cache: &RenderCache,
    area: Rectangle<i32, Logical>,
    behind: Rectangle<i32, Logical>,
    scale: f64,
) -> Option<WorkspaceElement> {
    if !state.config.effects.blur {
        return None;
    }
    state.wallpaper.as_ref()?;
    let buffer = cache.blur()?;
    let (soft_width, soft_height) = cache.blur_size();
    if area.size.w <= 0 || area.size.h <= 0 {
        return None;
    }
    let across = soft_width as f64 / area.size.w as f64;
    let down = soft_height as f64 / area.size.h as f64;
    let left = (behind.loc.x - area.loc.x) as f64 * across;
    let top = (behind.loc.y - area.loc.y) as f64 * down;
    let src = Rectangle::new(
        Point::<f64, Logical>::from((left, top)),
        smithay::utils::Size::<f64, Logical>::from((
            behind.size.w as f64 * across,
            behind.size.h as f64 * down,
        )),
    );
    let location: Point<i32, Physical> = behind.loc.to_physical_precise_round(scale);
    let element = TextureRenderElement::from_texture_buffer(
        location.to_f64(),
        buffer,
        None,
        Some(src),
        Some(behind.size),
        Kind::Unspecified,
    );
    Some(WorkspaceElement::Snapshot(element))
}

fn accent_for(theme: &spectre_theme::Theme, focused: bool) -> spectre_theme::Gradient {
    if focused {
        theme.palette.accent.clone()
    } else {
        theme.palette.accent.scaled(0.35)
    }
}

fn layer_elements(
    state: &Spectre,
    output: &Output,
    renderer: &mut GlesRenderer,
    cache: &mut RenderCache,
    scale: f64,
    upper: bool,
) -> Vec<SpectreElement> {
    let now = std::time::Instant::now();
    let mut elements = Vec::new();

    if upper {
        if let Some(element) = launcher_closing_element(state, output, cache, scale, now) {
            elements.push(element);
        }
    }

    let map = layer_map_for_output(output);
    for layer in map.layers().rev() {
        let is_upper = matches!(layer.layer(), WlrLayer::Overlay | WlrLayer::Top);
        if is_upper != upper {
            continue;
        }
        let Some(geometry) = map.layer_geometry(layer) else {
            continue;
        };
        let location = geometry.loc.to_physical_precise_round(scale);
        let is_launcher = layer.namespace() == LAUNCHER_NAMESPACE;

        let mut slide = None;
        if is_launcher {
            slide = state.launcher_opening;
        }
        let mut alpha = 1.0;
        if let Some(slide) = slide {
            alpha = slide.alpha(now);
        }

        let mut frosted = None;
        if let Some(area) = state.workspaces.output_geometry(output) {
            frosted = blur_element(state, cache, area, geometry, scale);
        }

        let surfaces = AsRenderElements::<GlesRenderer>::render_elements::<SurfaceElement>(
            layer,
            renderer,
            location,
            Scale::from(scale),
            alpha,
        );
        if is_launcher && state.config.effects.window_animations {
            let copy = AsRenderElements::<GlesRenderer>::render_elements::<SurfaceElement>(
                layer,
                renderer,
                location,
                Scale::from(scale),
                1.0,
            );
            let copy = copy.into_iter().map(WorkspaceElement::Surface).collect();
            cache.remember(LAUNCHER_KEY, copy);
            cache.set_launcher_geometry(geometry);
        }

        let Some(slide) = slide else {
            for surface in surfaces {
                elements.push(SpectreElement::Plain(WorkspaceElement::Surface(surface)));
            }
            if let Some(element) = frosted {
                elements.push(SpectreElement::Plain(element));
            }
            continue;
        };
        let down = slide.offset(now) * scale;
        for surface in surfaces {
            elements.push(SpectreElement::Moved(shift(WorkspaceElement::Surface(surface), down)));
        }
        if let Some(element) = frosted {
            elements.push(SpectreElement::Moved(shift(element, down)));
        }
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
    let mut elements = Vec::new();
    let Some(space) = state.workspaces.get(index) else {
        return elements;
    };
    let Some(region) = space.output_geometry(output) else {
        return elements;
    };
    let mut text = state.text.borrow_mut();
    for window in space.elements().rev() {
        let built = window_elements(
            state, output, renderer, shader, cache, space, region, window, alpha, &mut text,
        );
        elements.extend(built);
    }
    elements
}

fn active_workspace_elements(
    state: &Spectre,
    output: &Output,
    renderer: &mut GlesRenderer,
    shader: Option<&PatternShader>,
    cache: &mut RenderCache,
    index: usize,
    scale: f64,
) -> Vec<SpectreElement> {
    let mut elements = Vec::new();
    let Some(space) = state.workspaces.get(index) else {
        return elements;
    };
    let Some(region) = space.output_geometry(output) else {
        return elements;
    };
    let now = std::time::Instant::now();

    for closing in &state.closing {
        if closing.workspace != index {
            continue;
        }
        let local = Rectangle::new(closing.outer.loc - region.loc, closing.outer.size);
        let alpha = closing.pop.alpha(now);
        let Some(snapshot) = snapshot_element(cache, closing.key, local, scale, alpha) else {
            continue;
        };
        let physical: Rectangle<i32, Physical> = local.to_physical_precise_round(scale);
        let center = physical.loc + Point::from((physical.size.w / 2, physical.size.h / 2));
        elements.push(SpectreElement::Moved(scale_about(snapshot, center, closing.pop.scale(now))));
    }

    let remember = state.config.effects.window_animations;
    let metrics = state.config.theme.metrics;
    let mut text = state.text.borrow_mut();
    for window in space.elements().rev() {
        let pop = state.opening_pop(window);
        let mut alpha = 1.0;
        if let Some(pop) = pop {
            alpha = pop.alpha(now);
        }
        let built = window_elements(
            state, output, renderer, shader, cache, space, region, window, alpha, &mut text,
        );
        if remember {
            let copy = window_elements(
                state, output, renderer, shader, cache, space, region, window, 1.0, &mut text,
            );
            cache.remember(element_key(window), copy);
        }

        let Some(pop) = pop else {
            elements.extend(built.into_iter().map(SpectreElement::Plain));
            continue;
        };
        let Some(geometry) = space.element_geometry(window) else {
            continue;
        };
        let outer = Frame::new(geometry, &metrics, state.is_decorated(window)).outer;
        let local = Rectangle::new(outer.loc - region.loc, outer.size);
        let physical: Rectangle<i32, Physical> = local.to_physical_precise_round(scale);
        let center = physical.loc + Point::from((physical.size.w / 2, physical.size.h / 2));
        for element in built {
            elements.push(SpectreElement::Moved(scale_about(element, center, pop.scale(now))));
        }
    }
    elements
}

fn capture_closing(
    state: &Spectre,
    output: &Output,
    renderer: &mut GlesRenderer,
    cache: &mut RenderCache,
    scale: f64,
) {
    let mut keys = Vec::new();
    for closing in &state.closing {
        keys.push(closing.key);
        let Some(space) = state.workspaces.get(closing.workspace) else {
            continue;
        };
        let Some(region) = space.output_geometry(output) else {
            continue;
        };
        let local = Rectangle::new(closing.outer.loc - region.loc, closing.outer.size);
        capture_once(renderer, cache, closing.key, local, scale);
    }
    if let Some(closing) = state.launcher_closing.as_ref() {
        keys.push(LAUNCHER_KEY);
        if &closing.output == output {
            if let Some(geometry) = cache.launcher_geometry() {
                capture_once(renderer, cache, LAUNCHER_KEY, geometry, scale);
            }
        }
    }
    cache.keep_closing_snapshots(&keys);
    cache.forget_remembered();
}

fn capture_once(
    renderer: &mut GlesRenderer,
    cache: &mut RenderCache,
    key: u32,
    local: Rectangle<i32, Logical>,
    scale: f64,
) {
    if cache.closing_snapshot(key).is_some() {
        return;
    }
    let Some(remembered) = cache.take_remembered(key) else {
        return;
    };
    let physical: Rectangle<i32, Physical> = local.to_physical_precise_round(scale);
    let offset = Point::<i32, Physical>::from((-physical.loc.x, -physical.loc.y));
    let mut moved = Vec::new();
    for element in remembered {
        moved.push(RelocateRenderElement::from_element(element, offset, Relocate::Relative));
    }
    let Some(texture) = cube::capture(renderer, physical.size, &moved, [0.0, 0.0, 0.0, 0.0]) else {
        return;
    };
    let snapshot = TextureBuffer::from_texture(renderer, texture, 1, Transform::Normal, None);
    cache.set_closing_snapshot(key, snapshot);
}

fn snapshot_element(
    cache: &RenderCache,
    key: u32,
    local: Rectangle<i32, Logical>,
    scale: f64,
    alpha: f32,
) -> Option<WorkspaceElement> {
    let snapshot = cache.closing_snapshot(key)?;
    let physical: Rectangle<i32, Physical> = local.to_physical_precise_round(scale);
    let element = TextureRenderElement::from_texture_buffer(
        physical.loc.to_f64(),
        snapshot,
        Some(alpha),
        None,
        None,
        Kind::Unspecified,
    );
    Some(WorkspaceElement::Snapshot(element))
}

fn launcher_closing_element(
    state: &Spectre,
    output: &Output,
    cache: &RenderCache,
    scale: f64,
    now: std::time::Instant,
) -> Option<SpectreElement> {
    let closing = state.launcher_closing.as_ref()?;
    if &closing.output != output {
        return None;
    }
    let geometry = cache.launcher_geometry()?;
    let alpha = closing.slide.alpha(now);
    let snapshot = snapshot_element(cache, LAUNCHER_KEY, geometry, scale, alpha)?;
    let down = closing.slide.offset(now) * scale;
    Some(SpectreElement::Moved(shift(snapshot, down)))
}

fn shift(element: WorkspaceElement, down: f64) -> MovedElement {
    let kept = RescaleRenderElement::from_element(element, Point::<i32, Physical>::from((0, 0)), 1.0);
    let offset = Point::<i32, Physical>::from((0, down.round() as i32));
    RelocateRenderElement::from_element(kept, offset, Relocate::Relative)
}

fn scale_about(element: WorkspaceElement, center: Point<i32, Physical>, factor: f64) -> MovedElement {
    let scaled = RescaleRenderElement::from_element(element, center, factor);
    RelocateRenderElement::from_element(scaled, Point::<i32, Physical>::from((0, 0)), Relocate::Relative)
}

#[allow(clippy::too_many_arguments)]
fn window_elements(
    state: &Spectre,
    output: &Output,
    renderer: &mut GlesRenderer,
    shader: Option<&PatternShader>,
    cache: &mut RenderCache,
    space: &smithay::desktop::Space<smithay::desktop::Window>,
    region: Rectangle<i32, Logical>,
    window: &smithay::desktop::Window,
    alpha: f32,
    text: &mut TextCache,
) -> Vec<WorkspaceElement> {
    let scale = output.current_scale().fractional_scale();
    let theme = &state.config.theme;
    let metrics = theme.metrics;
    let pointer = state.pointer_position();
    let phase = state.pattern_phase();
    let color_phase = state.color_phase();
    let mut elements: Vec<WorkspaceElement> = Vec::new();

    let Some(geometry) = space.element_geometry(window) else {
        return elements;
    };
    let Some(location) = space.element_location(window) else {
        return elements;
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
            decoration_text(state, &frame, window, focused, hovered, text, renderer, scale, alpha)
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
            scale,
        ) {
            Ok(rounded) => elements.push(WorkspaceElement::Rounded(rounded)),
            Err(plain) => elements.push(WorkspaceElement::Surface(plain)),
        }
    }

    if !decorated {
        return elements;
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
            decorations::fallback_frame(cache, key, &frame, &theme.palette, focused, alpha, scale)
                .into_iter()
                .map(WorkspaceElement::Solid),
        ),
    }

    if state.config.effects.shadows {
        if let Some(shader) = shader {
            let shadow =
                shader.shadow_element(cache, Slot::Shadow(key), frame.outer, &metrics, alpha, scale);
            if let Some(element) = shadow {
                elements.push(WorkspaceElement::Pattern(Banded::whole(element)));
            }
        }
    }
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
            .color(theme.palette.titlebar_text(focused))
            .bold(focused)
            .max_width(area.size.w as u32);
        let size = cache.measure(&label);
        let location = Point::from((
            area.loc.x + (area.size.w - size.w).max(0) / 2,
            area.loc.y + (area.size.h - size.h).max(0) / 2,
        ));
        out.extend(cache.element(renderer, &label, location, scale, alpha));
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
        let label = Label::new(glyph).size(CAPTION_SIZE).color(color);
        let size = cache.measure(&label);
        let location = Point::from((
            rect.loc.x + (rect.size.w - size.w).max(0) / 2,
            rect.loc.y + (rect.size.h - size.h).max(0) / 2,
        ));
        out.extend(cache.element(renderer, &label, location, scale, alpha));
    }

    out
}

pub fn element_key(window: &smithay::desktop::Window) -> u32 {
    use smithay::reexports::wayland_server::Resource;
    if let Some(toplevel) = window.toplevel() {
        return toplevel.wl_surface().id().protocol_id();
    }
    if let Some(x11) = window.x11_surface() {
        return x11.window_id();
    }
    0
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
