use std::f32::consts::PI;

use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::element::texture::{TextureBuffer, TextureRenderElement};
use smithay::backend::renderer::element::{Element, Id, Kind, RenderElement, UnderlyingStorage};
use smithay::backend::renderer::gles::{
    GlesError, GlesFrame, GlesRenderer, GlesTexProgram, GlesTexture, Uniform,
};
use smithay::backend::renderer::utils::{CommitCounter, DamageSet, OpaqueRegions};
use smithay::backend::renderer::{Bind, Offscreen};
use smithay::utils::{Buffer, Logical, Physical, Point, Rectangle, Scale, Size, Transform};

pub const SNAPSHOT_SCALE: f64 = 0.5;

pub const FACE_SIZE: f32 = 0.55;

pub fn capture<E>(
    renderer: &mut GlesRenderer,
    size: Size<i32, Physical>,
    elements: &[E],
) -> Option<GlesTexture>
where
    E: RenderElement<GlesRenderer>,
{
    if size.w <= 0 || size.h <= 0 {
        return None;
    }
    let buffer_size: Size<i32, Buffer> = Size::from((size.w, size.h));
    let mut texture = match Offscreen::<GlesTexture>::create_buffer(renderer, Fourcc::Abgr8888, buffer_size) {
        Ok(texture) => texture,
        Err(err) => {
            tracing::warn!(?err, "could not create a workspace snapshot");
            return None;
        }
    };

    let mut framebuffer = match renderer.bind(&mut texture) {
        Ok(framebuffer) => framebuffer,
        Err(err) => {
            tracing::warn!(?err, "could not draw into a workspace snapshot");
            return None;
        }
    };
    let mut tracker = OutputDamageTracker::new(size, 1.0, Transform::Normal);
    if let Err(err) = tracker.render_output(renderer, &mut framebuffer, 0, elements, [0.02, 0.02, 0.03, 1.0]) {
        tracing::warn!(?err, "could not render a workspace snapshot");
        return None;
    }
    drop(framebuffer);
    Some(texture)
}

pub fn apothem(faces: usize) -> f32 {
    if faces < 3 {
        return 0.5;
    }
    0.5 / (PI / faces as f32).tan()
}

#[derive(Debug, Clone, Copy)]
pub struct FaceView {
    pub angle: f32,
    pub faces: usize,
    pub aspect: f32,
    pub flip: bool,
}

#[derive(Debug)]
pub struct CubeFace {
    element: TextureRenderElement<GlesTexture>,
    program: GlesTexProgram,
    uniforms: Vec<Uniform<'static>>,
    commit: CommitCounter,
}

impl CubeFace {
    pub fn new(
        buffer: &TextureBuffer<GlesTexture>,
        texture_size: Size<i32, Physical>,
        program: &GlesTexProgram,
        output: Size<i32, Logical>,
        view: FaceView,
        commit: CommitCounter,
    ) -> Self {
        let whole_texture = Rectangle::<f64, Logical>::from_size(Size::from((
            texture_size.w as f64,
            texture_size.h as f64,
        )));
        let element = TextureRenderElement::from_texture_buffer(
            Point::<f64, Physical>::from((0.0, 0.0)),
            buffer,
            None,
            Some(whole_texture),
            Some(output),
            Kind::Unspecified,
        );

        let apothem = apothem(view.faces);
        let flip = if view.flip { 1.0 } else { 0.0 };
        let uniforms = vec![
            Uniform::new("spectre_angle", view.angle),
            Uniform::new("spectre_apothem", apothem),
            Uniform::new("spectre_camera", apothem + 1.8),
            Uniform::new("spectre_scale", FACE_SIZE),
            Uniform::new("spectre_aspect", view.aspect),
            Uniform::new("spectre_flip", flip),
        ];
        Self { element, program: program.clone(), uniforms, commit }
    }
}

impl Element for CubeFace {
    fn id(&self) -> &Id {
        self.element.id()
    }

    fn current_commit(&self) -> CommitCounter {
        self.commit
    }

    fn src(&self) -> Rectangle<f64, Buffer> {
        self.element.src()
    }

    fn geometry(&self, scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.element.geometry(scale)
    }

    fn location(&self, scale: Scale<f64>) -> Point<i32, Physical> {
        self.element.location(scale)
    }

    fn transform(&self) -> Transform {
        self.element.transform()
    }

    fn damage_since(&self, scale: Scale<f64>, commit: Option<CommitCounter>) -> DamageSet<i32, Physical> {
        if commit == Some(self.commit) {
            return DamageSet::default();
        }
        let size = self.geometry(scale).size;
        DamageSet::from_slice(&[Rectangle::from_size(size)])
    }

    fn opaque_regions(&self, _scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        OpaqueRegions::default()
    }

    fn alpha(&self) -> f32 {
        self.element.alpha()
    }

    fn kind(&self) -> Kind {
        self.element.kind()
    }
}

impl RenderElement<GlesRenderer> for CubeFace {
    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
    ) -> Result<(), GlesError> {
        frame.override_default_tex_program(self.program.clone(), self.uniforms.clone());
        let result = RenderElement::<GlesRenderer>::draw(&self.element, frame, src, dst, damage, opaque_regions);
        frame.clear_tex_program_override();
        result
    }

    fn underlying_storage(&self, renderer: &mut GlesRenderer) -> Option<UnderlyingStorage<'_>> {
        let _ = renderer;
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn four_workspaces_make_a_cube() {
        assert!((apothem(4) - 0.5).abs() < 1e-5);
    }

    #[test]
    fn more_workspaces_push_the_faces_further_out() {
        assert!(apothem(6) > apothem(4));
        assert!(apothem(9) > apothem(6));
    }

    #[test]
    fn one_or_two_workspaces_still_have_a_finite_shape() {
        assert!(apothem(1).is_finite());
        assert!(apothem(2).is_finite());
    }
}
