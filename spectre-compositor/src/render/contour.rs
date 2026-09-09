use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::memory::MemoryRenderBuffer;
use smithay::backend::renderer::element::{Element, Id, Kind, RenderElement, UnderlyingStorage};
use smithay::backend::renderer::gles::{GlesError, GlesFrame, GlesRenderer, GlesTexProgram, Uniform};
use smithay::backend::renderer::utils::{CommitCounter, OpaqueRegions};
use smithay::utils::{Buffer, Physical, Point, Rectangle, Scale, Size, Transform};
use spectre_draw::PatternMask;
use spectre_theme::{Color, Pattern};

const MARGIN: i32 = 512;

#[derive(Debug)]
pub struct ContourField {
    buffer: MemoryRenderBuffer,
    size: Size<i32, Physical>,
    pattern: Pattern,
    scale: f64,
    output: Size<i32, Physical>,
    origin: f64,
    commit: CommitCounter,
    offset: f64,
}

impl ContourField {
    pub fn prepare<'a>(
        held: &'a mut Option<Self>,
        pattern: &Pattern,
        output: Size<i32, Physical>,
        scale: f64,
        phase: f32,
    ) -> Option<&'a mut Self> {
        if pattern.is_noop() || output.w <= 0 || output.h <= 0 {
            *held = None;
            return None;
        }
        let wanted = field_offset(pattern, scale, phase);
        let reusable = held.as_ref().is_some_and(|field| {
            field.pattern == *pattern
                && field.scale == scale
                && field.output == output
                && field.holds(wanted)
        });
        if !reusable {
            *held = Self::bake(pattern, output, scale, wanted);
        }
        let field = held.as_mut()?;
        field.look_at(wanted);
        Some(field)
    }

    fn holds(&self, offset: f64) -> bool {
        let local = offset - self.origin;
        local >= 0.0 && local + self.output.w as f64 <= self.size.w as f64
    }

    fn look_at(&mut self, offset: f64) {
        let local = (offset - self.origin).clamp(0.0, (self.size.w - self.output.w).max(0) as f64);
        if (local - self.offset).abs() >= 0.1 {
            self.offset = local;
            self.commit.increment();
        }
    }

    fn bake(
        pattern: &Pattern,
        output: Size<i32, Physical>,
        scale: f64,
        origin: f64,
    ) -> Option<Self> {
        let margin = if pattern.animated && pattern.speed > 0.0 { MARGIN } else { 0 };
        let size = Size::from((output.w + margin, output.h));

        let mut mask = PatternMask::new();
        let cell = cell_size(pattern, scale);
        mask.prepare(size.w, size.h, pattern, (origin / cell) as f32, scale as f32);
        let coverage = mask.bytes();
        if coverage.is_empty() {
            return None;
        }

        let mut pixels = vec![0u8; (size.w * size.h * 4) as usize];
        for (out, &c) in pixels.chunks_exact_mut(4).zip(coverage) {
            out.copy_from_slice(&[c, c, c, c]);
        }

        let buffer = MemoryRenderBuffer::from_slice(
            &pixels,
            Fourcc::Argb8888,
            (size.w, size.h),
            1,
            Transform::Normal,
            None,
        );
        Some(Self {
            buffer,
            size,
            pattern: *pattern,
            scale,
            output,
            origin,
            commit: CommitCounter::default(),
            offset: 0.0,
        })
    }

    pub fn buffer(&self) -> &MemoryRenderBuffer {
        &self.buffer
    }

    pub fn commit(&self) -> CommitCounter {
        self.commit
    }

    pub fn source(&self) -> Rectangle<f64, smithay::utils::Logical> {
        Rectangle::new(
            Point::from((self.offset, 0.0)),
            Size::from((self.output.w as f64, self.output.h as f64)),
        )
    }

    pub fn uv_window(&self) -> (f32, f32) {
        let width = self.size.w.max(1) as f32;
        (self.offset as f32 / width, self.output.w as f32 / width)
    }
}

fn cell_size(pattern: &Pattern, scale: f64) -> f64 {
    (pattern.line_spacing as f64 * scale).max(1.0) * 6.0
}

fn field_offset(pattern: &Pattern, scale: f64, phase: f32) -> f64 {
    (phase as f64 * cell_size(pattern, scale)).max(0.0)
}

#[derive(Debug)]
pub struct Contoured<E> {
    element: E,
    program: GlesTexProgram,
    commit: CommitCounter,
    uniforms: Vec<Uniform<'static>>,
    opaque: bool,
}

impl<E: Element> Contoured<E> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        element: E,
        program: Option<&GlesTexProgram>,
        commit: CommitCounter,
        stops: &[Color; Pattern::STOPS],
        color_phase: f32,
        color_span: f32,
        background: Color,
        uv_window: (f32, f32),
    ) -> Result<Self, E> {
        let Some(program) = program else {
            return Err(element);
        };
        let uniforms = vec![
            Uniform::new("spectre_line_0", stops[0].to_array()),
            Uniform::new("spectre_line_1", stops[1].to_array()),
            Uniform::new("spectre_line_2", stops[2].to_array()),
            Uniform::new("spectre_line_3", stops[3].to_array()),
            Uniform::new("spectre_color_phase", color_phase),
            Uniform::new("spectre_color_span", color_span),
            Uniform::new("spectre_bg", background.to_array()),
            Uniform::new("spectre_uv_origin", uv_window.0),
            Uniform::new("spectre_uv_span", uv_window.1),
        ];
        Ok(Self {
            element,
            program: program.clone(),
            commit,
            uniforms,
            opaque: background.a >= 1.0,
        })
    }
}

impl<E: Element> Element for Contoured<E> {
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

    fn opaque_regions(&self, scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        if !self.opaque {
            return OpaqueRegions::default();
        }
        let geometry = self.geometry(scale);
        OpaqueRegions::from_slice(&[Rectangle::from_size(geometry.size)])
    }

    fn alpha(&self) -> f32 {
        self.element.alpha()
    }

    fn kind(&self) -> Kind {
        self.element.kind()
    }
}

impl<E> RenderElement<GlesRenderer> for Contoured<E>
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
        frame.override_default_tex_program(self.program.clone(), self.uniforms.clone());
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

    fn output() -> Size<i32, Physical> {
        Size::from((1280, 800))
    }

    fn moving() -> Pattern {
        Pattern::default()
    }

    #[test]
    fn a_still_field_is_baked_no_wider_than_the_screen() {
        let pattern = Pattern::default().without_animation();
        let field = ContourField::bake(&pattern, output(), 1.0, 0.0).expect("it draws something");
        assert_eq!(field.size.w, 1280, "a field that cannot move needs no room to move in");
    }

    #[test]
    fn a_moving_field_is_baked_wider_so_it_has_somewhere_to_scroll() {
        let field = ContourField::bake(&moving(), output(), 1.0, 0.0).expect("it draws something");
        assert_eq!(field.size.w, 1280 + MARGIN);
    }

    #[test]
    fn a_pattern_that_draws_nothing_is_not_baked_at_all() {
        let mut held = None;
        assert!(ContourField::prepare(&mut held, &Pattern::OFF, output(), 1.0, 0.0).is_none());
        assert!(held.is_none());
    }

    #[test]
    fn the_same_phase_neither_bakes_again_nor_damages_anything() {
        let mut held = None;
        ContourField::prepare(&mut held, &moving(), output(), 1.0, 0.0);
        let (origin, commit) = {
            let field = held.as_ref().unwrap();
            (field.origin, field.commit())
        };
        ContourField::prepare(&mut held, &moving(), output(), 1.0, 0.0);
        let field = held.as_ref().unwrap();
        assert_eq!(field.origin, origin, "nothing changed, so nothing was baked again");
        assert_eq!(field.commit(), commit, "and the desktop is not repainted");
    }

    #[test]
    fn scrolling_inside_the_texture_moves_the_source_and_damages_the_element() {
        let mut held = None;
        ContourField::prepare(&mut held, &moving(), output(), 1.0, 0.0);
        let before = held.as_ref().unwrap().commit();
        let origin = held.as_ref().unwrap().origin;

        let cell = cell_size(&moving(), 1.0);
        let phase = ((origin + 100.0) / cell) as f32;
        ContourField::prepare(&mut held, &moving(), output(), 1.0, phase);
        let field = held.as_ref().unwrap();
        assert!((field.source().loc.x - 100.0).abs() < 1.0, "{:?}", field.source());
        assert_ne!(field.commit(), before, "a scrolled field has to be redrawn");
    }

    #[test]
    fn scrolling_past_the_margin_bakes_again_rather_than_running_off_the_edge() {
        let mut held = None;
        ContourField::prepare(&mut held, &moving(), output(), 1.0, 0.0);
        let cell = cell_size(&moving(), 1.0);
        let far = ((MARGIN as f64 + 400.0) / cell) as f32;
        ContourField::prepare(&mut held, &moving(), output(), 1.0, far);
        let field = held.as_ref().unwrap();
        assert!(field.holds(field.origin), "the new bake has to cover where we are looking");
        assert_eq!(field.source().loc.x, 0.0, "and start at the left edge of it");
    }

    #[test]
    fn a_new_output_size_is_baked_again() {
        let mut held = None;
        ContourField::prepare(&mut held, &moving(), output(), 1.0, 0.0);
        ContourField::prepare(&mut held, &moving(), Size::from((1920, 1080)), 1.0, 0.0);
        assert_eq!(held.as_ref().unwrap().size.h, 1080);
    }

    #[test]
    fn the_visible_window_is_reported_as_a_fraction_of_the_texture() {
        let mut held = None;
        ContourField::prepare(&mut held, &moving(), output(), 1.0, 0.0);
        let (origin, span) = held.as_ref().unwrap().uv_window();
        assert_eq!(origin, 0.0);
        assert!((span - 1280.0 / (1280.0 + MARGIN as f32)).abs() < 1e-6);
    }

    #[test]
    fn the_field_offset_follows_the_line_spacing() {
        let pattern = Pattern { line_spacing: 10.0, ..Pattern::default() };
        assert_eq!(field_offset(&pattern, 1.0, 1.0), 60.0);
        assert_eq!(field_offset(&pattern, 2.0, 1.0), 120.0);
    }
}
