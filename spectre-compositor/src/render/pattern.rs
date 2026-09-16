use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::gles::element::PixelShaderElement;
use smithay::backend::renderer::gles::{
    GlesPixelProgram, GlesRenderer, GlesTexProgram, Uniform, UniformName, UniformType,
};
use smithay::utils::{Logical, Point, Rectangle, Size};

use super::{Banded, RenderCache, Slot};
use spectre_theme::{Color, Gradient, Metrics, Palette, Pattern, PatternKind};

const SHADER_SRC: &str = include_str!("pattern.glsl");
const FRAME_SRC: &str = include_str!("frame.glsl");
const ROUNDED_SRC: &str = include_str!("rounded.glsl");
const CONTOUR_SRC: &str = include_str!("contour.glsl");
const CUBE_SRC: &str = include_str!("cube.glsl");
const SHADOW_SRC: &str = include_str!("shadow.glsl");

const FRAME_UNIFORMS: &[(&str, UniformType)] = &[
    ("spectre_radius", UniformType::_1f),
    ("spectre_border", UniformType::_1f),
    ("spectre_titlebar", UniformType::_1f),
    ("spectre_bg", UniformType::_4f),
    ("spectre_edge", UniformType::_4f),
    ("spectre_line_0", UniformType::_4f),
    ("spectre_line_1", UniformType::_4f),
    ("spectre_line_2", UniformType::_4f),
    ("spectre_line_3", UniformType::_4f),
    ("spectre_color_phase", UniformType::_1f),
    ("spectre_color_span", UniformType::_1f),
    ("spectre_phase", UniformType::_1f),
    ("spectre_spacing", UniformType::_1f),
    ("spectre_line_width", UniformType::_1f),
];

const CONTOUR_UNIFORMS: &[(&str, UniformType)] = &[
    ("spectre_line_0", UniformType::_4f),
    ("spectre_line_1", UniformType::_4f),
    ("spectre_line_2", UniformType::_4f),
    ("spectre_line_3", UniformType::_4f),
    ("spectre_color_phase", UniformType::_1f),
    ("spectre_color_span", UniformType::_1f),
    ("spectre_bg", UniformType::_4f),
    ("spectre_uv_origin", UniformType::_1f),
    ("spectre_uv_span", UniformType::_1f),
];

pub const SHADOW_SPREAD: i32 = 18;

pub const SHADOW_DROP: i32 = 6;

const SHADOW_STRENGTH: f32 = 0.6;

const SHADOW_UNIFORMS: &[(&str, UniformType)] = &[
    ("spectre_radius", UniformType::_1f),
    ("spectre_spread", UniformType::_1f),
    ("spectre_strength", UniformType::_1f),
    ("spectre_drop", UniformType::_1f),
];

const CUBE_UNIFORMS: &[(&str, UniformType)] = &[
    ("spectre_angle", UniformType::_1f),
    ("spectre_apothem", UniformType::_1f),
    ("spectre_camera", UniformType::_1f),
    ("spectre_scale", UniformType::_1f),
    ("spectre_aspect", UniformType::_1f),
    ("spectre_flip", UniformType::_1f),
];

const ROUNDED_UNIFORMS: &[(&str, UniformType)] = &[
    ("spectre_size", UniformType::_2f),
    ("spectre_window_min", UniformType::_2f),
    ("spectre_window_max", UniformType::_2f),
    ("spectre_radii", UniformType::_4f),
];

const UNIFORMS: &[(&str, UniformType)] = &[
    ("spectre_phase", UniformType::_1f),
    ("spectre_spacing", UniformType::_1f),
    ("spectre_line_width", UniformType::_1f),
    ("spectre_line_0", UniformType::_4f),
    ("spectre_line_1", UniformType::_4f),
    ("spectre_line_2", UniformType::_4f),
    ("spectre_line_3", UniformType::_4f),
    ("spectre_color_phase", UniformType::_1f),
    ("spectre_color_span", UniformType::_1f),
    ("spectre_bg", UniformType::_4f),
];

const COLOR_SPAN: f32 = 1.0;

fn stop_uniforms(stops: &[Color; Pattern::STOPS], color_phase: f32) -> Vec<Uniform<'static>> {
    vec![
        Uniform::new("spectre_line_0", stops[0].to_array()),
        Uniform::new("spectre_line_1", stops[1].to_array()),
        Uniform::new("spectre_line_2", stops[2].to_array()),
        Uniform::new("spectre_line_3", stops[3].to_array()),
        Uniform::new("spectre_color_phase", color_phase),
        Uniform::new("spectre_color_span", COLOR_SPAN),
    ]
}

#[derive(Debug, Clone)]
pub struct PatternShader {
    program: GlesPixelProgram,
    frame: Option<GlesPixelProgram>,
    rounded: Option<GlesTexProgram>,
    contour: Option<GlesTexProgram>,
    cube: Option<GlesTexProgram>,
    shadow: Option<GlesPixelProgram>,
}

impl PatternShader {
    pub fn compile(renderer: &mut GlesRenderer) -> Option<Self> {
        let names = |list: &[(&'static str, UniformType)]| -> Vec<UniformName<'static>> {
            list.iter().map(|(n, t)| UniformName::new(*n, *t)).collect()
        };

        let program = match renderer.compile_custom_pixel_shader(SHADER_SRC, &names(UNIFORMS)) {
            Ok(program) => program,
            Err(err) => {
                tracing::warn!(
                    ?err,
                    "the Spectre Pattern shader did not compile; \
                     falling back to flat surfaces"
                );
                return None;
            }
        };

        let frame = renderer
            .compile_custom_pixel_shader(FRAME_SRC, &names(FRAME_UNIFORMS))
            .inspect_err(|err| {
                tracing::warn!(?err, "the window frame shader did not compile; \
                                      windows will have square corners")
            })
            .ok();

        let rounded = renderer
            .compile_custom_texture_shader(ROUNDED_SRC, &names(ROUNDED_UNIFORMS))
            .inspect_err(|err| {
                tracing::warn!(?err, "the corner rounding shader did not compile")
            })
            .ok();

        let contour = renderer
            .compile_custom_texture_shader(CONTOUR_SRC, &names(CONTOUR_UNIFORMS))
            .inspect_err(|err| {
                tracing::warn!(?err, "the contour colouring shader did not compile; \
                                      the desktop pattern falls back to the slow path")
            })
            .ok();

        let cube = renderer
            .compile_custom_texture_shader(CUBE_SRC, &names(CUBE_UNIFORMS))
            .inspect_err(|err| tracing::warn!(?err, "the workspace cube shader did not compile"))
            .ok();

        let shadow = renderer
            .compile_custom_pixel_shader(SHADOW_SRC, &names(SHADOW_UNIFORMS))
            .inspect_err(|err| tracing::warn!(?err, "the window shadow shader did not compile"))
            .ok();

        Some(Self { program, frame, rounded, contour, cube, shadow })
    }

    pub fn shadow_element(
        &self,
        cache: &mut RenderCache,
        slot: Slot,
        outer: Rectangle<i32, Logical>,
        metrics: &Metrics,
        alpha: f32,
        scale: f64,
    ) -> Option<PixelShaderElement> {
        let program = self.shadow.as_ref()?;
        if outer.size.w <= 0 || outer.size.h <= 0 {
            return None;
        }
        let area = Rectangle::new(
            Point::from((outer.loc.x - SHADOW_SPREAD, outer.loc.y - SHADOW_SPREAD)),
            Size::from((outer.size.w + SHADOW_SPREAD * 2, outer.size.h + SHADOW_SPREAD * 2)),
        );
        let uniforms = vec![
            Uniform::new("spectre_radius", (metrics.corner_radius as f64 * scale) as f32),
            Uniform::new("spectre_spread", (SHADOW_SPREAD as f64 * scale) as f32),
            Uniform::new("spectre_strength", SHADOW_STRENGTH),
            Uniform::new("spectre_drop", (SHADOW_DROP as f64 * scale) as f32),
        ];
        let (element, _) =
            cache.shader(slot, program, area, None, alpha, uniforms, Kind::Unspecified);
        Some(element)
    }

    pub fn rounded_program(&self) -> Option<&GlesTexProgram> {
        self.rounded.as_ref()
    }

    pub fn cube_program(&self) -> Option<&GlesTexProgram> {
        self.cube.as_ref()
    }

    pub fn contour_program(&self) -> Option<&GlesTexProgram> {
        self.contour.as_ref()
    }

    pub const COLOR_SPAN: f32 = COLOR_SPAN;

    #[allow(clippy::too_many_arguments)]
    pub fn frame_element(
        &self,
        cache: &mut RenderCache,
        slot: Slot,
        outer: Rectangle<i32, Logical>,
        titlebar_height: i32,
        metrics: &Metrics,
        palette: &Palette,
        pattern: &Pattern,
        accent: &Gradient,
        focused: bool,
        phase: f32,
        color_phase: f32,
        alpha: f32,
        scale: f64,
    ) -> Option<Banded<PixelShaderElement>> {
        let program = self.frame.as_ref()?;
        if outer.size.w <= 0 || outer.size.h <= 0 || titlebar_height <= 0 {
            return None;
        }

        let background = palette.titlebar(focused).alpha(alpha);
        let edge = palette.window_border(focused).alpha(alpha);
        let stops = if pattern.is_noop() {
            [Color::TRANSPARENT; Pattern::STOPS]
        } else {
            pattern.line_stops(accent, background)
        };
        let spacing = (pattern.line_spacing as f64 * scale).max(1.0) as f32;

        let mut uniforms = vec![
            Uniform::new("spectre_radius", (metrics.corner_radius as f64 * scale) as f32),
            Uniform::new("spectre_border", (metrics.border_width as f64 * scale) as f32),
            Uniform::new("spectre_titlebar", (titlebar_height as f64 * scale) as f32),
            Uniform::new("spectre_bg", background.to_array()),
            Uniform::new("spectre_edge", edge.to_array()),
            Uniform::new("spectre_phase", phase),
            Uniform::new("spectre_spacing", if pattern.is_noop() { 0.0 } else { spacing }),
            Uniform::new(
                "spectre_line_width",
                (pattern.line_width as f64 * scale).max(0.5) as f32,
            ),
        ];
        uniforms.extend(stop_uniforms(&stops, color_phase));

        let (element, moved) =
            cache.shader(slot, program, outer, None, 1.0, uniforms, Kind::Unspecified);
        let band = (!moved).then(|| {
            let height = ((titlebar_height as f64 * scale).ceil() as i32).max(1);
            let width = (outer.size.w as f64 * scale).ceil() as i32;
            Rectangle::new(Point::from((0, 0)), Size::from((width, height)))
        });
        Some(Banded::new(element, band))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn element(
        &self,
        cache: &mut RenderCache,
        slot: Slot,
        pattern: &Pattern,
        area: Rectangle<i32, Logical>,
        background: Color,
        accent: &Gradient,
        phase: f32,
        color_phase: f32,
        scale: f64,
    ) -> Option<Banded<PixelShaderElement>> {
        if pattern.is_noop() || area.size.w <= 0 || area.size.h <= 0 {
            return None;
        }

        let stops = pattern.line_stops(accent, background);
        let spacing = match pattern.kind {
            PatternKind::Grid => pattern.line_spacing * 0.5,
            _ => pattern.line_spacing,
        } as f64
            * scale;

        let mut uniforms = vec![
            Uniform::new("spectre_phase", phase),
            Uniform::new("spectre_spacing", spacing.max(1.0) as f32),
            Uniform::new("spectre_line_width", (pattern.line_width as f64 * scale).max(0.5) as f32),
            Uniform::new("spectre_bg", background.to_array()),
        ];
        uniforms.extend(stop_uniforms(&stops, color_phase));

        let opaque = (background.a >= 1.0).then(|| vec![area]);

        let (element, _) =
            cache.shader(slot, &self.program, area, opaque, 1.0, uniforms, Kind::Unspecified);
        Some(Banded::whole(element))
    }
}
