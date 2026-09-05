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
use smithay::utils::{Buffer, Physical, Rectangle, Scale, Size, Transform};

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

    /// What is left opaque once the corners are cut away.
    ///
    /// Everything the curve touches has to be given up, or a renderer that
    /// skipped what is behind it would leave the desktop unpainted exactly
    /// where the corner shows it through. The middle of the window, though, is
    /// as opaque as the surface underneath was - and saying so is what lets the
    /// damage tracker leave the animated desktop pattern alone while a window
    /// covers it, instead of recomputing the whole contour field behind
    /// every frame of every window.
    fn opaque_regions(&self, scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        // The window rectangle is output-relative, opaque regions are
        // element-relative; the shader does the same translation in `draw`.
        let origin = self.element.geometry(scale).loc;
        let window = Rectangle::new(self.window.loc - origin, self.window.size);
        opaque_after_rounding(&self.element.opaque_regions(scale), window, self.corners)
            .into_iter()
            .collect()
    }

    fn alpha(&self) -> f32 {
        self.element.alpha()
    }

    fn kind(&self) -> Kind {
        self.element.kind()
    }
}

type Point = smithay::utils::Point<i32, Physical>;

/// How far inside its own edge the rounding shader has stopped feathering.
///
/// `rounded.glsl` blends the mask over `AA = 0.8` device pixels either side of
/// the curve, so a fragment less than a pixel from the edge is not fully
/// opaque. A whole pixel of margin is the cheapest way to be certain, and one
/// pixel of extra repainting around a window costs nothing.
const FEATHER: i32 = 1;

/// The part of `regions` that survives rounding `window` with `corners`.
///
/// All rectangles are element-local. The result is conservative on purpose:
/// claiming a pixel is opaque when the shader has made it translucent shows up
/// as a hole in the desktop, so every doubtful pixel is given up instead.
fn opaque_after_rounding(
    regions: &[Rectangle<i32, Physical>],
    window: Rectangle<i32, Physical>,
    corners: Corners,
) -> Vec<Rectangle<i32, Physical>> {
    if regions.is_empty() {
        return Vec::new();
    }
    // Outside the window rectangle the shader's mask is zero: a subsurface
    // reaching past the toplevel is clipped away entirely.
    let Some(solid) = inset(window, FEATHER) else {
        return Vec::new();
    };
    let cut = corner_boxes(window, corners);
    regions
        .iter()
        .filter_map(|region| region.intersection(solid))
        .flat_map(|region| region.subtract_rects(cut.iter().copied()))
        .filter(|region| !region.is_empty())
        .collect()
}

/// `rect` shrunk by `by` on every side, or `None` when nothing is left.
fn inset(rect: Rectangle<i32, Physical>, by: i32) -> Option<Rectangle<i32, Physical>> {
    // `Size` refuses to hold a negative number, so the check comes first.
    let (w, h) = (rect.size.w - 2 * by, rect.size.h - 2 * by);
    (w > 0 && h > 0)
        .then(|| Rectangle::new(rect.loc + Point::from((by, by)), Size::from((w, h))))
}

/// The four squares a rounded corner can reach into, largest radius first in
/// each corner. A corner with no radius contributes nothing.
fn corner_boxes(
    window: Rectangle<i32, Physical>,
    corners: Corners,
) -> Vec<Rectangle<i32, Physical>> {
    let (w, h) = (window.size.w, window.size.h);
    let side = |radius: f32| (radius.ceil() as i32).saturating_add(FEATHER).max(0);
    [
        (side(corners.top_left), (0, 0)),
        (side(corners.top_right), (w, 0)),
        (side(corners.bottom_right), (w, h)),
        (side(corners.bottom_left), (0, h)),
    ]
    .into_iter()
    .filter(|(side, _)| *side > FEATHER)
    .map(|(side, (x, y))| {
        // Anchor the square at the corner it belongs to: an x of `w` means the
        // right edge, so the square hangs to the left of it.
        let loc = Point::from((
            window.loc.x + if x == 0 { 0 } else { w - side },
            window.loc.y + if y == 0 { 0 } else { h - side },
        ));
        Rectangle::new(loc, Size::from((side, side)))
    })
    .collect()
}

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

    fn rect(x: i32, y: i32, w: i32, h: i32) -> Rectangle<i32, Physical> {
        Rectangle::new((x, y).into(), (w, h).into())
    }

    /// True when every rectangle in `regions` avoids the pixel at `x`, `y`.
    fn free_of(regions: &[Rectangle<i32, Physical>], x: i32, y: i32) -> bool {
        !regions.iter().any(|r| r.contains(Point::from((x, y))))
    }

    fn area(regions: &[Rectangle<i32, Physical>]) -> i32 {
        regions.iter().map(|r| r.size.w * r.size.h).sum()
    }

    #[test]
    fn the_middle_of_a_window_stays_opaque() {
        let window = rect(0, 0, 400, 300);
        let regions = opaque_after_rounding(&[window], window, Corners::uniform(8.0));
        assert!(!regions.is_empty(), "a covered desktop must not be redrawn behind the window");
        assert!(!free_of(&regions, 200, 150), "the centre is as opaque as the surface was");
    }

    #[test]
    fn the_rounded_corners_are_given_up() {
        let window = rect(0, 0, 400, 300);
        let regions = opaque_after_rounding(&[window], window, Corners::uniform(8.0));
        for (x, y) in [(0, 0), (399, 0), (399, 299), (0, 299), (4, 4), (395, 295)] {
            assert!(free_of(&regions, x, y), "the curve shows the desktop through ({x}, {y})");
        }
    }

    #[test]
    fn a_square_top_keeps_its_corners() {
        let window = rect(0, 0, 400, 300);
        let regions = opaque_after_rounding(&[window], window, Corners::bottom(8.0));
        // One pixel of feather is still given up along every edge.
        assert!(!free_of(&regions, 1, 1), "a title bar has already squared this corner");
        assert!(free_of(&regions, 1, 298), "but the bottom is still rounded");
    }

    #[test]
    fn a_translucent_surface_stays_translucent() {
        let window = rect(0, 0, 400, 300);
        assert!(opaque_after_rounding(&[], window, Corners::uniform(8.0)).is_empty());
    }

    #[test]
    fn a_subsurface_reaching_past_the_window_is_clipped() {
        let window = rect(0, 0, 400, 300);
        let regions = opaque_after_rounding(&[rect(-50, -50, 500, 400)], window, Corners::uniform(8.0));
        assert!(free_of(&regions, -1, 150), "nothing outside the window is drawn at all");
        assert!(free_of(&regions, 400, 150));
        assert!(!free_of(&regions, 200, 150));
    }

    #[test]
    fn a_window_smaller_than_its_own_feather_claims_nothing() {
        for size in [0, 1, 2] {
            let window = rect(0, 0, size, size);
            assert!(opaque_after_rounding(&[window], window, Corners::uniform(8.0)).is_empty());
        }
    }

    #[test]
    fn a_bigger_radius_gives_up_more() {
        let window = rect(0, 0, 400, 300);
        let small = area(&opaque_after_rounding(&[window], window, Corners::uniform(4.0)));
        let large = area(&opaque_after_rounding(&[window], window, Corners::uniform(24.0)));
        assert!(large < small);
        assert!(large > 0, "a rounded window is still mostly solid");
    }

    #[test]
    fn the_regions_never_overlap() {
        let window = rect(0, 0, 400, 300);
        let regions = opaque_after_rounding(&[window], window, Corners::uniform(12.0));
        for (i, a) in regions.iter().enumerate() {
            for b in &regions[i + 1..] {
                assert!(a.intersection(*b).is_none(), "{a:?} and {b:?} overlap");
            }
        }
    }

    #[test]
    fn the_radii_reach_the_shader_in_the_documented_order() {
        // top-left, top-right, bottom-right, bottom-left, matching rounded.glsl.
        let c = Corners { top_left: 1.0, top_right: 2.0, bottom_right: 3.0, bottom_left: 4.0 };
        assert_eq!(c.to_array(), [1.0, 2.0, 3.0, 4.0]);
    }
}
