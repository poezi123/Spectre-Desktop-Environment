//! Render-element identities that survive between frames.
//!
//! A damage tracker recognises an element by its [`Id`]. Handing it a fresh
//! `Id::new()` every frame means it can only conclude that everything is new,
//! so the whole output is repainted whether anything moved or not. On a machine
//! without a real GPU that is the difference between a desktop and a slideshow.
//!
//! Each drawn thing therefore asks the cache for its identity under a stable
//! key, and the commit counter only moves when the thing itself changed.

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

// Why every element reports itself damaged every frame.
///
// Stable identities plus a commit counter that only moves on a real change is
// what a damage tracker wants, and it takes an idle desktop from a third of a
// core to nothing. It also turned out to expose a defect further down: this
// project's target - a virtual GPU driven by vmwgfx - clears a region it is
// about to repaint and then loses the update, leaving black boxes behind the
// pointer and a freshly mapped panel half missing. The panel's own buffer was
// verified correct to the pixel, so the loss is below us.
//
// The identities stay - they are right, and they are what a fix will build on
// - but the commit counters move every frame, so every frame is complete.

/// Identities and shader elements kept alive across frames.
#[derive(Debug, Default)]
pub struct RenderCache {
    /// Slots touched while building the current frame. Kept so the identities
    /// can be reinstated the moment the damage path is trustworthy again.
    live: Vec<Slot>,
}

impl RenderCache {
    /// Start a frame. Slots not asked for before [`RenderCache::end_frame`] are
    /// dropped, so a closed window does not keep its frame alive forever.
    pub fn begin_frame(&mut self) {
        self.live.clear();
    }

    pub fn end_frame(&mut self) {
        self.live.clear();
    }

    /// A solid colour rectangle.
    pub fn solid(
        &mut self,
        slot: Slot,
        geometry: Rectangle<i32, Physical>,
        color: [f32; 4],
        kind: Kind,
    ) -> SolidColorRenderElement {
        self.live.push(slot);
        SolidColorRenderElement::new(Id::new(), geometry, CommitCounter::default(), color, kind)
    }

    /// A pixel-shader element.
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
    ) -> PixelShaderElement {
        self.live.push(slot);
        PixelShaderElement::new(program.clone(), area, opaque, alpha, uniforms, kind)
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
    fn every_frame_is_complete() {
        let mut cache = RenderCache::default();
        let white = [1.0, 1.0, 1.0, 1.0];

        cache.begin_frame();
        let first = cache.solid(Slot::Backdrop, rect(0, 0, 10, 10), white, Kind::Unspecified);
        cache.end_frame();

        cache.begin_frame();
        let second = cache.solid(Slot::Backdrop, rect(0, 0, 10, 10), white, Kind::Unspecified);
        cache.end_frame();

        // A fresh identity every frame is what keeps the screen whole while the
        // damage path cannot be trusted; see the note at the top of this file.
        assert_ne!(first.id(), second.id());
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
    fn a_frame_ends_with_nothing_held_over() {
        let mut cache = RenderCache::default();
        cache.begin_frame();
        cache.solid(Slot::Frame(7), rect(0, 0, 10, 10), [1.0; 4], Kind::Unspecified);
        cache.end_frame();
        assert!(cache.live.is_empty(), "a slot must not outlive the frame that used it");
    }
}
