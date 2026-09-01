//! Compiling and instantiating the Spectre Pattern shader.

use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::gles::element::PixelShaderElement;
use smithay::backend::renderer::gles::{
    GlesPixelProgram, GlesRenderer, Uniform, UniformName, UniformType,
};
use smithay::utils::{Logical, Rectangle};
use spectre_theme::{Color, Pattern, PatternKind};

const SHADER_SRC: &str = include_str!("pattern.glsl");

const UNIFORMS: &[(&str, UniformType)] = &[
    ("spectre_phase", UniformType::_1f),
    ("spectre_spacing", UniformType::_1f),
    ("spectre_line_width", UniformType::_1f),
    ("spectre_line", UniformType::_4f),
    ("spectre_bg", UniformType::_4f),
];

/// The compiled pattern program.
///
/// Compilation is fallible on purpose: a machine whose GL driver rejects the
/// shader must still get a desktop, just a flat one. Callers hold an
/// `Option<PatternShader>` and fall back to a solid colour.
#[derive(Debug, Clone)]
pub struct PatternShader {
    program: GlesPixelProgram,
}

impl PatternShader {
    /// Compile the pattern shader for `renderer`.
    pub fn compile(renderer: &mut GlesRenderer) -> Option<Self> {
        let names: Vec<UniformName<'_>> =
            UNIFORMS.iter().map(|(n, t)| UniformName::new(*n, *t)).collect();

        match renderer.compile_custom_pixel_shader(SHADER_SRC, &names) {
            Ok(program) => Some(Self { program }),
            Err(err) => {
                tracing::warn!(
                    ?err,
                    "the Spectre Pattern shader did not compile; \
                     falling back to flat surfaces"
                );
                None
            }
        }
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
    pub fn element(
        &self,
        pattern: &Pattern,
        area: Rectangle<i32, Logical>,
        background: Color,
        accent: Color,
        phase: f32,
        scale: f64,
    ) -> Option<PixelShaderElement> {
        if pattern.is_noop() || area.size.w <= 0 || area.size.h <= 0 {
            return None;
        }

        let line = pattern.line_color(accent, background);
        // Grid is the cheap variant: same shader, but the noise is flattened by
        // pushing the spacing far apart so the level set degenerates to bands.
        let spacing = match pattern.kind {
            PatternKind::Grid => pattern.line_spacing * 0.5,
            _ => pattern.line_spacing,
        } as f64
            * scale;

        let uniforms = vec![
            Uniform::new("spectre_phase", phase),
            Uniform::new("spectre_spacing", spacing.max(1.0) as f32),
            Uniform::new("spectre_line_width", (pattern.line_width as f64 * scale).max(0.5) as f32),
            Uniform::new("spectre_line", line.to_array()),
            Uniform::new("spectre_bg", background.to_array()),
        ];

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
