//! Rounding the corners of a client surface.
//!
//! A window's own texture cannot be masked from outside, so the mask is applied
//! by swapping the renderer's texture shader for the duration of that window's
//! draw. The replacement takes the window rectangle as a uniform, which means
//! every surface belonging to the window - toplevel and subsurfaces alike - is
//! clipped by the same curve rather than each rounding itself.

use smithay::backend::renderer::element::{Element, Id, Kind, RenderElement, UnderlyingStorage};
use smithay::backend::renderer::gles::{GlesError, GlesFrame, GlesRenderer, GlesTexProgram, Uniform};
use smithay::backend::renderer::utils::{CommitCounter, DamageSet, OpaqueRegions};
use smithay::utils::{Buffer, Physical, Rectangle, Scale, Transform};

/// Corner radii in device pixels: top-left, top-right, bottom-right, bottom-left.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Corners {
    pub top_left: f32,
    pub top_right: f32,
    pub bottom_right: f32,
    pub bottom_left: f32,
}

impl Corners {
    /// The same radius on every corner.
    pub fn uniform(radius: f32) -> Self {
        Self {
            top_left: radius,
            top_right: radius,
            bottom_right: radius,
            bottom_left: radius,
        }
    }

    /// Rounded at the bottom only, for a surface sitting under a title bar that
    /// has already rounded the top two.
    pub fn bottom(radius: f32) -> Self {
        Self { top_left: 0.0, top_right: 0.0, bottom_right: radius, bottom_left: radius }
    }

    pub fn is_square(&self) -> bool {
        self.top_left <= 0.0
            && self.top_right <= 0.0
            && self.bottom_right <= 0.0
            && self.bottom_left <= 0.0
    }

    fn to_array(self) -> [f32; 4] {
        [self.top_left, self.top_right, self.bottom_right, self.bottom_left]
    }
}

/// Wraps a surface element so it is drawn through the rounding shader.
#[derive(Debug)]
pub struct RoundedElement<E> {
    element: E,
    program: GlesTexProgram,
    /// The window's rectangle in the same physical space as the element's own.
    window: Rectangle<i32, Physical>,
    corners: Corners,
}

impl<E: Element> RoundedElement<E> {
    /// Round `element` against `window`, or hand it back unchanged when there
    /// is nothing to round.
    pub fn new(
        element: E,
        program: Option<&GlesTexProgram>,
        window: Rectangle<i32, Physical>,
        corners: Corners,
    ) -> Result<Self, E> {
        match program {
            Some(program) if !corners.is_square() && !window.is_empty() => Ok(Self {
                element,
                program: program.clone(),
                window,
                corners,
            }),
            _ => Err(element),
        }
    }
}

impl<E: Element> Element for RoundedElement<E> {
    fn id(&self) -> &Id {
        self.element.id()
    }

    fn current_commit(&self) -> CommitCounter {
        self.element.current_commit()
    }

    fn src(&self) -> Rectangle<f64, Buffer> {
        self.element.src()
    }

    fn geometry(&self, scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.element.geometry(scale)
    }

    fn location(&self, scale: Scale<f64>) -> Point {
        self.element.location(scale)
    }

    fn transform(&self) -> Transform {
        self.element.transform()
    }

    fn damage_since(&self, scale: Scale<f64>, commit: Option<CommitCounter>) -> DamageSet<i32, Physical> {
        self.element.damage_since(scale, commit)
    }

    /// Nothing is opaque any more: the corners are cut away, and a renderer
    /// that skipped what is behind them would leave the desktop unpainted
    /// exactly where the curve shows it.
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

type Point = smithay::utils::Point<i32, Physical>;

impl<E> RenderElement<GlesRenderer> for RoundedElement<E>
where
    E: RenderElement<GlesRenderer>,
{
    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
    ) -> Result<(), GlesError> {
        // The shader works in coordinates local to the element being drawn, so
        // the window rectangle is translated into that space here.
        let min = self.window.loc - dst.loc;
        let max = min + self.window.size;
        let uniforms = vec![
            Uniform::new("spectre_size", (dst.size.w as f32, dst.size.h as f32)),
            Uniform::new("spectre_window_min", (min.x as f32, min.y as f32)),
            Uniform::new("spectre_window_max", (max.x as f32, max.y as f32)),
            Uniform::new("spectre_radii", self.corners.to_array()),
        ];

        frame.override_default_tex_program(self.program.clone(), uniforms);
        let result = self.element.draw(frame, src, dst, damage, opaque_regions);
        // Reset even on failure: leaving the override in place would round
        // every surface drawn after this one for the rest of the frame.
        frame.clear_tex_program_override();
        result
    }

    fn underlying_storage(&self, renderer: &mut GlesRenderer) -> Option<UnderlyingStorage<'_>> {
        // Direct scan-out would bypass the shader and show square corners.
        let _ = renderer;
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_uniform_radius_rounds_every_corner() {
        let c = Corners::uniform(8.0);
        assert_eq!(c.to_array(), [8.0; 4]);
        assert!(!c.is_square());
    }

    #[test]
    fn the_bottom_variant_leaves_the_top_alone() {
        let c = Corners::bottom(8.0);
        assert_eq!(c.top_left, 0.0);
        assert_eq!(c.top_right, 0.0);
        assert_eq!(c.bottom_left, 8.0);
        assert_eq!(c.bottom_right, 8.0);
        assert!(!c.is_square());
    }

    #[test]
    fn a_zero_radius_counts_as_square() {
        assert!(Corners::uniform(0.0).is_square());
        assert!(Corners::bottom(0.0).is_square());
        assert!(Corners::uniform(-1.0).is_square());
    }

    #[test]
    fn the_radii_reach_the_shader_in_the_documented_order() {
        // top-left, top-right, bottom-right, bottom-left, matching rounded.glsl.
        let c = Corners { top_left: 1.0, top_right: 2.0, bottom_right: 3.0, bottom_left: 4.0 };
        assert_eq!(c.to_array(), [1.0, 2.0, 3.0, 4.0]);
    }
}
