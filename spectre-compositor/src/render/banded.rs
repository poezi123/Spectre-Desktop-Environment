//! Restricting an element's damage to the band it actually paints.
//!
//! The window frame is one element the size of the whole window, because the
//! rounded corners and the border need the full rectangle to measure against.
//! Only the title bar has anything moving in it, though, and reporting the
//! whole window as damaged makes every animated frame recomposite the client's
//! surface underneath - hundreds of thousands of pixels for a strip of
//! twenty-odd. The band is what changed; the rest is redrawn only when the
//! element itself moves or is resized.

use smithay::backend::renderer::element::{Element, Id, Kind, RenderElement, UnderlyingStorage};
use smithay::backend::renderer::gles::{GlesError, GlesFrame, GlesRenderer};
use smithay::backend::renderer::utils::{CommitCounter, DamageSet, OpaqueRegions};
use smithay::utils::{Buffer, Physical, Point, Rectangle, Scale, Transform};

/// Wraps an element so only part of it is reported as damaged.
#[derive(Debug)]
pub struct Banded<E> {
    element: E,
    /// The part that changes, in coordinates local to the element. `None`
    /// reports whatever the element itself reports.
    band: Option<Rectangle<i32, Physical>>,
}

impl<E: Element> Banded<E> {
    pub fn new(element: E, band: Option<Rectangle<i32, Physical>>) -> Self {
        Self { element, band }
    }

    /// Report everything, for an element that paints all of itself.
    pub fn whole(element: E) -> Self {
        Self { element, band: None }
    }
}

impl<E: Element> Element for Banded<E> {
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

    fn location(&self, scale: Scale<f64>) -> Point<i32, Physical> {
        self.element.location(scale)
    }

    fn transform(&self) -> Transform {
        self.element.transform()
    }

    fn damage_since(
        &self,
        scale: Scale<f64>,
        commit: Option<CommitCounter>,
    ) -> DamageSet<i32, Physical> {
        let damage = self.element.damage_since(scale, commit);
        // An element the tracker has not seen before has to be drawn whole,
        // whatever the band says, or the first frame comes up with holes.
        let Some(band) = self.band.filter(|_| commit.is_some()) else {
            return damage;
        };
        let clipped: Vec<_> = damage.iter().filter_map(|rect| rect.intersection(band)).collect();
        DamageSet::from_slice(&clipped)
    }

    fn opaque_regions(&self, scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        self.element.opaque_regions(scale)
    }

    fn alpha(&self) -> f32 {
        self.element.alpha()
    }

    fn kind(&self) -> Kind {
        self.element.kind()
    }
}

impl<E> RenderElement<GlesRenderer> for Banded<E>
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
        self.element.draw(frame, src, dst, damage, opaque_regions)
    }

    fn underlying_storage(&self, renderer: &mut GlesRenderer) -> Option<UnderlyingStorage<'_>> {
        self.element.underlying_storage(renderer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use smithay::backend::renderer::element::solid::SolidColorRenderElement;

    fn rect(x: i32, y: i32, w: i32, h: i32) -> Rectangle<i32, Physical> {
        Rectangle::new((x, y).into(), (w, h).into())
    }

    /// An element that has changed since the tracker last looked at it.
    fn element() -> SolidColorRenderElement {
        let mut commit = CommitCounter::default();
        commit.increment();
        SolidColorRenderElement::new(
            Id::new(),
            rect(0, 0, 900, 400),
            commit,
            [1.0; 4],
            Kind::Unspecified,
        )
    }

    /// What the tracker saw last time: one commit behind.
    fn last_seen() -> Option<CommitCounter> {
        Some(CommitCounter::default())
    }

    fn scale() -> Scale<f64> {
        Scale::from(1.0)
    }

    #[test]
    fn a_band_cuts_the_damage_down_to_the_part_that_moves() {
        let banded = Banded::new(element(), Some(rect(0, 0, 900, 24)));
        let damage = banded.damage_since(scale(), last_seen());
        assert_eq!(damage.len(), 1);
        assert_eq!(damage[0], rect(0, 0, 900, 24), "only the title bar is redrawn");
    }

    #[test]
    fn an_element_the_tracker_has_not_seen_is_drawn_whole() {
        let banded = Banded::new(element(), Some(rect(0, 0, 900, 24)));
        let damage = banded.damage_since(scale(), None);
        assert_eq!(damage[0].size, rect(0, 0, 900, 400).size, "a first frame has no holes");
    }

    #[test]
    fn without_a_band_nothing_is_held_back() {
        let plain: Vec<_> = element().damage_since(scale(), last_seen()).into_iter().collect();
        let banded: Vec<_> =
            Banded::whole(element()).damage_since(scale(), last_seen()).into_iter().collect();
        assert_eq!(banded, plain);
    }

    #[test]
    fn the_wrapper_keeps_the_element_it_wraps() {
        let inner = element();
        let id = inner.id().clone();
        let banded = Banded::whole(inner);
        assert_eq!(banded.id(), &id, "a new identity would damage the whole output");
        assert_eq!(banded.geometry(scale()), rect(0, 0, 900, 400));
    }
}
