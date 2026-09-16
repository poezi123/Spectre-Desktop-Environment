use std::collections::HashMap;

use smithay::backend::renderer::element::solid::SolidColorRenderElement;
use smithay::backend::renderer::element::{Id, Kind};
use smithay::backend::renderer::gles::element::PixelShaderElement;
use smithay::backend::renderer::gles::{GlesPixelProgram, GlesTexture, Uniform};
use smithay::backend::renderer::utils::CommitCounter;
use smithay::backend::renderer::element::texture::TextureBuffer;
use smithay::utils::{Logical, Physical, Rectangle, Size};

use super::{ContourField, WorkspaceElement};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Slot {
    Backdrop,
    DesktopPattern,
    Frame(u32),
    Decoration(u32, u8),
    Menu(u8),
    Shadow(u32),
}

#[derive(Debug, Default)]
pub struct RenderCache {
    solids: HashMap<Slot, SolidSlot>,
    shaders: HashMap<Slot, ShaderSlot>,
    live: Vec<Slot>,
    contour: Option<ContourField>,
    snapshots: Vec<TextureBuffer<GlesTexture>>,
    snapshot_size: Size<i32, Physical>,
    face_commit: CommitCounter,
    face_angle: f32,
    cursor_elements: usize,
    remembered: Remembered,
    closing: HashMap<u32, TextureBuffer<GlesTexture>>,
    launcher_geometry: Option<Rectangle<i32, Logical>>,
    wallpaper: Option<TextureBuffer<GlesTexture>>,
    wallpaper_key: Option<String>,
}

#[derive(Default)]
struct Remembered(HashMap<u32, Vec<WorkspaceElement>>);

impl std::fmt::Debug for Remembered {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Remembered({} windows)", self.0.len())
    }
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
    alpha: f32,
}

impl RenderCache {
    pub fn remember(&mut self, key: u32, elements: Vec<WorkspaceElement>) {
        self.remembered.0.insert(key, elements);
    }

    pub fn take_remembered(&mut self, key: u32) -> Option<Vec<WorkspaceElement>> {
        self.remembered.0.remove(&key)
    }

    pub fn forget_remembered(&mut self) {
        self.remembered.0.clear();
    }

    pub fn wallpaper(&self) -> Option<&TextureBuffer<GlesTexture>> {
        self.wallpaper.as_ref()
    }

    pub fn wallpaper_key(&self) -> Option<&str> {
        self.wallpaper_key.as_deref()
    }

    pub fn set_wallpaper(&mut self, buffer: TextureBuffer<GlesTexture>, key: String) {
        self.wallpaper = Some(buffer);
        self.wallpaper_key = Some(key);
    }

    pub fn launcher_geometry(&self) -> Option<Rectangle<i32, Logical>> {
        self.launcher_geometry
    }

    pub fn set_launcher_geometry(&mut self, geometry: Rectangle<i32, Logical>) {
        self.launcher_geometry = Some(geometry);
    }

    pub fn closing_snapshot(&self, key: u32) -> Option<&TextureBuffer<GlesTexture>> {
        self.closing.get(&key)
    }

    pub fn set_closing_snapshot(&mut self, key: u32, snapshot: TextureBuffer<GlesTexture>) {
        self.closing.insert(key, snapshot);
    }

    pub fn keep_closing_snapshots(&mut self, keys: &[u32]) {
        self.closing.retain(|key, _| keys.contains(key));
    }

    pub fn cursor_elements(&self) -> usize {
        self.cursor_elements
    }

    pub fn set_cursor_elements(&mut self, count: usize) {
        self.cursor_elements = count;
    }

    pub fn snapshots(&self) -> &[TextureBuffer<GlesTexture>] {
        &self.snapshots
    }

    pub fn snapshot_size(&self) -> Size<i32, Physical> {
        self.snapshot_size
    }

    pub fn set_snapshots(&mut self, snapshots: Vec<TextureBuffer<GlesTexture>>, size: Size<i32, Physical>) {
        self.snapshots = snapshots;
        self.snapshot_size = size;
    }

    pub fn face_commit(&mut self, angle: f32) -> CommitCounter {
        if (angle - self.face_angle).abs() > 0.00001 {
            self.face_angle = angle;
            self.face_commit.increment();
        }
        self.face_commit
    }

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
        let faded = match self.shaders.get(&slot) {
            Some(entry) => entry.alpha != alpha,
            None => false,
        };
        if faded {
            self.shaders.remove(&slot);
        }
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
                let slot_entry = ShaderSlot { element: element.clone(), uniforms, area, alpha };
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
