use std::collections::HashMap;

use smithay::backend::renderer::element::solid::SolidColorRenderElement;
use smithay::backend::renderer::element::{Id, Kind};
use smithay::backend::renderer::gles::element::PixelShaderElement;
use smithay::backend::renderer::gles::{GlesPixelProgram, Uniform};
use smithay::backend::renderer::utils::CommitCounter;
use smithay::utils::{Logical, Physical, Rectangle};

use super::ContourField;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Slot {
    Backdrop,
    DesktopPattern,
    Frame(u32),
    Decoration(u32, u8),
}

#[derive(Debug, Default)]
pub struct RenderCache {
    solids: HashMap<Slot, SolidSlot>,
    shaders: HashMap<Slot, ShaderSlot>,
    live: Vec<Slot>,
    contour: Option<ContourField>,
}

#[derive(Debug)]
struct SolidSlot {
    id: Id,
    commit: CommitCounter,
    color: [f32; 4],
    geometry: Rectangle<i32, Physical>,
}

#[derive(Debug)]
struct ShaderSlot {
    element: PixelShaderElement,
    uniforms: Vec<Uniform<'static>>,
    area: Rectangle<i32, Logical>,
}

impl RenderCache {
    pub fn contour(&mut self) -> &mut Option<ContourField> {
        &mut self.contour
    }

    pub fn begin_frame(&mut self) {
        self.live.clear();
    }

    pub fn end_frame(&mut self) {
        let live = &self.live;
        self.solids.retain(|slot, _| live.contains(slot));
        self.shaders.retain(|slot, _| live.contains(slot));
    }

    pub fn solid(
        &mut self,
        slot: Slot,
        geometry: Rectangle<i32, Physical>,
        color: [f32; 4],
        kind: Kind,
    ) -> SolidColorRenderElement {
        self.live.push(slot);
        let entry = self.solids.entry(slot).or_insert_with(|| SolidSlot {
            id: Id::new(),
            commit: CommitCounter::default(),
            color,
            geometry,
        });
        if entry.color != color || entry.geometry != geometry {
            entry.color = color;
            entry.geometry = geometry;
            entry.commit.increment();
        }
        SolidColorRenderElement::new(entry.id.clone(), geometry, entry.commit, color, kind)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn shader(
        &mut self,
        slot: Slot,
        program: &GlesPixelProgram,
        area: Rectangle<i32, Logical>,
        opaque: Option<Vec<Rectangle<i32, Logical>>>,
        alpha: f32,
        uniforms: Vec<Uniform<'static>>,
        kind: Kind,
    ) -> (PixelShaderElement, bool) {
        self.live.push(slot);
        match self.shaders.get_mut(&slot) {
            Some(entry) => {
                let moved = entry.area != area;
                entry.area = area;
                entry.element.resize(area, opaque);
                if entry.uniforms != uniforms {
                    entry.element.update_uniforms(uniforms.clone());
                    entry.uniforms = uniforms;
                }
                (entry.element.clone(), moved)
            }
            None => {
                let element = PixelShaderElement::new(
                    program.clone(),
                    area,
                    opaque,
                    alpha,
                    uniforms.clone(),
                    kind,
                );
                let slot_entry = ShaderSlot { element: element.clone(), uniforms, area };
                self.shaders.insert(slot, slot_entry);
                (element, true)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use smithay::backend::renderer::element::Element;

    fn rect(x: i32, y: i32, w: i32, h: i32) -> Rectangle<i32, Physical> {
        Rectangle::new((x, y).into(), (w, h).into())
    }

    #[test]
    fn the_same_rectangle_keeps_its_identity_and_its_commit() {
        let mut cache = RenderCache::default();
        let white = [1.0, 1.0, 1.0, 1.0];

        cache.begin_frame();
        let first = cache.solid(Slot::Backdrop, rect(0, 0, 10, 10), white, Kind::Unspecified);
        cache.end_frame();

        cache.begin_frame();
        let second = cache.solid(Slot::Backdrop, rect(0, 0, 10, 10), white, Kind::Unspecified);
        cache.end_frame();

        assert_eq!(first.id(), second.id(), "a new id would damage the whole output");
        assert_eq!(
            first.current_commit(),
            second.current_commit(),
            "nothing changed, so nothing is damaged"
        );
    }

    #[test]
    fn a_changed_colour_moves_the_commit_counter() {
        let mut cache = RenderCache::default();
        cache.begin_frame();
        let first = cache.solid(Slot::Backdrop, rect(0, 0, 10, 10), [0.0; 4], Kind::Unspecified);
        cache.end_frame();
        cache.begin_frame();
        let second = cache.solid(Slot::Backdrop, rect(0, 0, 10, 10), [1.0; 4], Kind::Unspecified);
        cache.end_frame();

        assert_eq!(first.id(), second.id());
        assert_ne!(first.current_commit(), second.current_commit());
    }

    #[test]
    fn a_moved_rectangle_is_damaged() {
        let mut cache = RenderCache::default();
        cache.begin_frame();
        let first = cache.solid(Slot::Backdrop, rect(0, 0, 10, 10), [1.0; 4], Kind::Unspecified);
        cache.end_frame();
        cache.begin_frame();
        let second = cache.solid(Slot::Backdrop, rect(5, 0, 10, 10), [1.0; 4], Kind::Unspecified);
        cache.end_frame();

        assert_ne!(first.current_commit(), second.current_commit());
    }

    #[test]
    fn two_slots_never_share_an_identity() {
        let mut cache = RenderCache::default();
        cache.begin_frame();
        let a = cache.solid(Slot::Backdrop, rect(0, 0, 10, 10), [1.0; 4], Kind::Unspecified);
        let b = cache.solid(Slot::Frame(1), rect(0, 0, 10, 10), [1.0; 4], Kind::Unspecified);
        cache.end_frame();
        assert_ne!(a.id(), b.id());
    }

    #[test]
    fn a_slot_nobody_asked_for_is_dropped() {
        let mut cache = RenderCache::default();
        cache.begin_frame();
        let first = cache.solid(Slot::Frame(7), rect(0, 0, 10, 10), [1.0; 4], Kind::Unspecified);
        cache.end_frame();

        cache.begin_frame();
        cache.solid(Slot::Backdrop, rect(0, 0, 10, 10), [1.0; 4], Kind::Unspecified);
        cache.end_frame();

        cache.begin_frame();
        let again = cache.solid(Slot::Frame(7), rect(0, 0, 10, 10), [1.0; 4], Kind::Unspecified);
        cache.end_frame();
        assert_ne!(first.id(), again.id(), "the slot was released and built anew");
    }
}
