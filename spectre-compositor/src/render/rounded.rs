use smithay::backend::renderer::element::{Element, Id, Kind, RenderElement, UnderlyingStorage};
use smithay::backend::renderer::gles::{GlesError, GlesFrame, GlesRenderer, GlesTexProgram, Uniform};
use smithay::backend::renderer::utils::{CommitCounter, DamageSet, OpaqueRegions};
use smithay::utils::{Buffer, Physical, Rectangle, Scale, Size, Transform};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Corners {
    pub top_left: f32,
    pub top_right: f32,
    pub bottom_right: f32,
    pub bottom_left: f32,
}

impl Corners {
    pub fn uniform(radius: f32) -> Self {
        Self {
            top_left: radius,
            top_right: radius,
            bottom_right: radius,
            bottom_left: radius,
        }
    }

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

#[derive(Debug)]
pub struct RoundedElement<E> {
    element: E,
    program: GlesTexProgram,
    window: Rectangle<i32, Physical>,
    corners: Corners,
    output_scale: f64,
}

impl<E: Element> RoundedElement<E> {
    pub fn new(
        element: E,
        program: Option<&GlesTexProgram>,
        window: Rectangle<i32, Physical>,
        corners: Corners,
        output_scale: f64,
    ) -> Result<Self, E> {
        match program {
            Some(program) if !corners.is_square() && !window.is_empty() => Ok(Self {
                element,
                program: program.clone(),
                window,
                corners,
                output_scale,
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

    fn opaque_regions(&self, scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        let origin = self.element.geometry(scale).loc;
        let window = Rectangle::new(self.window.loc - origin, self.window.size);
        let regions = self.element.opaque_regions(scale);
        let remaining = opaque_after_rounding(&regions, window, self.corners);
        OpaqueRegions::from_slice(&remaining)
    }

    fn alpha(&self) -> f32 {
        self.element.alpha()
    }

    fn kind(&self) -> Kind {
        self.element.kind()
    }
}

type Point = smithay::utils::Point<i32, Physical>;

const FEATHER: i32 = 1;

fn opaque_after_rounding(
    regions: &[Rectangle<i32, Physical>],
    window: Rectangle<i32, Physical>,
    corners: Corners,
) -> Vec<Rectangle<i32, Physical>> {
    let mut result = Vec::new();
    if regions.is_empty() {
        return result;
    }
    let Some(solid) = inset(window, FEATHER) else {
        return result;
    };
    let cut = corner_boxes(window, corners);

    for region in regions {
        let Some(inside) = region.intersection(solid) else {
            continue;
        };
        for piece in inside.subtract_rects(cut.clone()) {
            if !piece.is_empty() {
                result.push(piece);
            }
        }
    }
    result
}

fn inset(rect: Rectangle<i32, Physical>, by: i32) -> Option<Rectangle<i32, Physical>> {
    let width = rect.size.w - 2 * by;
    let height = rect.size.h - 2 * by;
    if width <= 0 || height <= 0 {
        return None;
    }
    let corner = rect.loc + Point::from((by, by));
    Some(Rectangle::new(corner, Size::from((width, height))))
}

fn corner_boxes(window: Rectangle<i32, Physical>, corners: Corners) -> Vec<Rectangle<i32, Physical>> {
    let left = window.loc.x;
    let top = window.loc.y;
    let right = window.loc.x + window.size.w;
    let bottom = window.loc.y + window.size.h;

    let mut boxes = Vec::new();
    add_corner_box(&mut boxes, corners.top_left, left, top, false, false);
    add_corner_box(&mut boxes, corners.top_right, right, top, true, false);
    add_corner_box(&mut boxes, corners.bottom_right, right, bottom, true, true);
    add_corner_box(&mut boxes, corners.bottom_left, left, bottom, false, true);
    boxes
}

fn add_corner_box(
    boxes: &mut Vec<Rectangle<i32, Physical>>,
    radius: f32,
    x: i32,
    y: i32,
    from_right: bool,
    from_bottom: bool,
) {
    if radius <= 0.0 {
        return;
    }
    let side = radius.ceil() as i32 + FEATHER;
    let box_x = if from_right { x - side } else { x };
    let box_y = if from_bottom { y - side } else { y };
    boxes.push(Rectangle::new(Point::from((box_x, box_y)), Size::from((side, side))));
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
        let geometry = self.element.geometry(Scale::from(self.output_scale));
        let mut factor = 1.0f32;
        if geometry.size.w > 0 {
            factor = dst.size.w as f32 / geometry.size.w as f32;
        }
        let offset = self.window.loc - geometry.loc;
        let min_x = offset.x as f32 * factor;
        let min_y = offset.y as f32 * factor;
        let max_x = min_x + self.window.size.w as f32 * factor;
        let max_y = min_y + self.window.size.h as f32 * factor;
        let radii = self.corners.to_array().map(|radius| radius * factor);
        let uniforms = vec![
            Uniform::new("spectre_size", (dst.size.w as f32, dst.size.h as f32)),
            Uniform::new("spectre_window_min", (min_x, min_y)),
            Uniform::new("spectre_window_max", (max_x, max_y)),
            Uniform::new("spectre_radii", radii),
        ];

        frame.override_default_tex_program(self.program.clone(), uniforms);
        let result = self.element.draw(frame, src, dst, damage, opaque_regions);
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
        let c = Corners { top_left: 1.0, top_right: 2.0, bottom_right: 3.0, bottom_left: 4.0 };
        assert_eq!(c.to_array(), [1.0, 2.0, 3.0, 4.0]);
    }
}
