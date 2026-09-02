//! The shaders Spectre draws its own furniture with.

use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::gles::element::PixelShaderElement;
use smithay::backend::renderer::gles::{
    GlesPixelProgram, GlesRenderer, GlesTexProgram, Uniform, UniformName, UniformType,
};
use smithay::utils::{Logical, Rectangle};
use spectre_theme::{Color, Gradient, Metrics, Palette, Pattern, PatternKind};

const SHADER_SRC: &str = include_str!("pattern.glsl");
const FRAME_SRC: &str = include_str!("frame.glsl");
const ROUNDED_SRC: &str = include_str!("rounded.glsl");

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

/// How much of the colour loop spans one surface.
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

/// Every program Spectre compiles.
///
/// Compilation is fallible on purpose: a machine whose GL driver rejects a
/// shader must still get a desktop, just a plainer one. Each program is
/// optional on its own, so a driver that chokes on one does not cost the
/// others.
#[derive(Debug, Clone)]
pub struct PatternShader {
    program: GlesPixelProgram,
    frame: Option<GlesPixelProgram>,
    rounded: Option<GlesTexProgram>,
}

impl PatternShader {
    /// Compile the shaders for `renderer`.
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

        Some(Self { program, frame, rounded })
    }

    /// The program that rounds a client surface, if it compiled.
    pub fn rounded_program(&self) -> Option<&GlesTexProgram> {
        self.rounded.as_ref()
    }

    /// The window frame: rounded title bar, hairline border and the pattern,
    /// with the client area left transparent.
    ///
    /// Returns `None` when the frame shader is unavailable or the window is
    /// undecorated, so the caller can fall back to plain rectangles.
    #[allow(clippy::too_many_arguments)]
    pub fn frame_element(
        &self,
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
    ) -> Option<PixelShaderElement> {
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

        // Rounded corners and a hollow middle: nothing here is opaque.
        Some(PixelShaderElement::new(
            program.clone(),
            outer,
            None,
            1.0,
            uniforms,
            Kind::Unspecified,
        ))
    }

    /// Build a render element covering `area`.
    ///
    /// `scale` converts the pattern's logical line metrics into the device
    /// pixels the shader works in, so the pattern keeps its density on a HiDPI
    /// output instead of turning into a fine haze.
    ///
    /// Returns `None` when the pattern would draw nothing, which lets the
    /// caller skip the draw call entirely rather than blending a transparent
    /// full-screen quad.
    #[allow(clippy::too_many_arguments)]
    pub fn element(
        &self,
        pattern: &Pattern,
        area: Rectangle<i32, Logical>,
        background: Color,
        accent: &Gradient,
        phase: f32,
        color_phase: f32,
        scale: f64,
    ) -> Option<PixelShaderElement> {
        if pattern.is_noop() || area.size.w <= 0 || area.size.h <= 0 {
            return None;
        }

        let stops = pattern.line_stops(accent, background);
        // Grid is the cheap variant: same shader, but the noise is flattened by
        // pushing the spacing far apart so the level set degenerates to bands.
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

        // The background is opaque, so declaring the whole area opaque lets the
        // damage tracker skip everything behind it.
        let opaque = (background.a >= 1.0).then(|| vec![area]);

        Some(PixelShaderElement::new(
            self.program.clone(),
            area,
            opaque,
            1.0,
            uniforms,
            Kind::Unspecified,
        ))
    }
}
