//! Render-element identities that survive between frames.
//!
//! A damage tracker recognises an element by its [`Id`]. Handing it a fresh
//! `Id::new()` every frame means it can only conclude that everything is new,
//! so the whole output is repainted whether anything moved or not. On a machine
//! without a real GPU that is the difference between a desktop and a slideshow.
//!
//! Each drawn thing therefore asks the cache for its identity under a stable
//! key, and the commit counter only moves when the thing itself changed.

use std::collections::HashMap;

use smithay::backend::renderer::element::solid::SolidColorRenderElement;
use smithay::backend::renderer::element::{Id, Kind};
use smithay::backend::renderer::gles::element::PixelShaderElement;
use smithay::backend::renderer::gles::{GlesPixelProgram, Uniform};
use smithay::backend::renderer::utils::CommitCounter;
use smithay::utils::{Logical, Physical, Rectangle};

/// What a cached element belongs to, so two different things never collide.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Slot {
    /// The flat colour behind everything.
    Backdrop,
    /// The pattern drawn across the desktop.
    DesktopPattern,
    /// A window's frame: rounded title bar, border and pattern.
    Frame(u32),
    /// A rectangle of a window's decorations, numbered within the window.
    Decoration(u32, u8),
}

/// Identities and shader elements kept alive across frames.
#[derive(Debug, Default)]
pub struct RenderCache {
    solids: HashMap<Slot, SolidSlot>,
    shaders: HashMap<Slot, ShaderSlot>,
    /// Slots touched while building the current frame.
    live: Vec<Slot>,
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
}

impl RenderCache {
    /// Start a frame. Slots not asked for before [`RenderCache::end_frame`] are
    /// dropped, so a closed window does not keep its frame alive forever.
    pub fn begin_frame(&mut self) {
        self.live.clear();
    }

    pub fn end_frame(&mut self) {
        let live = &self.live;
        self.solids.retain(|slot, _| live.contains(slot));
        self.shaders.retain(|slot, _| live.contains(slot));
    }

    /// A solid colour rectangle whose identity outlives the frame.
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

    /// A pixel-shader element whose identity outlives the frame.
    ///
    /// The stored element is updated in place, so its commit counter only moves
    /// when the area or a uniform actually differs from the last frame.
    /// `update_uniforms` bumps the counter unconditionally, which is why the
    /// values are kept here and compared first.
    pub fn shader(
        &mut self,
        slot: Slot,
        program: &GlesPixelProgram,
        area: Rectangle<i32, Logical>,
        opaque: Option<Vec<Rectangle<i32, Logical>>>,
        alpha: f32,
        uniforms: Vec<Uniform<'static>>,
        kind: Kind,
    ) -> PixelShaderElement {
        self.live.push(slot);
        match self.shaders.get_mut(&slot) {
            Some(entry) => {
                entry.element.resize(area, opaque);
                if entry.uniforms != uniforms {
                    entry.element.update_uniforms(uniforms.clone());
                    entry.uniforms = uniforms;
                }
                entry.element.clone()
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
                self.shaders.insert(slot, ShaderSlot { element: element.clone(), uniforms });
                element
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

        // A frame that draws something else entirely: the window closed.
        cache.begin_frame();
        cache.solid(Slot::Backdrop, rect(0, 0, 10, 10), [1.0; 4], Kind::Unspecified);
        cache.end_frame();

        cache.begin_frame();
        let again = cache.solid(Slot::Frame(7), rect(0, 0, 10, 10), [1.0; 4], Kind::Unspecified);
        cache.end_frame();
        assert_ne!(first.id(), again.id(), "the slot was released and built anew");
    }
}
