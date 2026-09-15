use std::collections::HashMap;

use smithay::backend::renderer::element::memory::{
    MemoryRenderBuffer, MemoryRenderBufferRenderElement,
};
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::utils::{Logical, Point, Rectangle, Size, Transform};
use spectre_text::{Image, Label, TextRenderer};

const CAPACITY: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Key {
    text: String,
    size_px: u32,
    max_width: u32,
    color: [u8; 4],
    bold: bool,
}

impl Key {
    fn new(label: &Label<'_>) -> Self {
        Self {
            text: label.text.to_owned(),
            size_px: (label.size_px * 4.0).round() as u32,
            max_width: label.max_width.unwrap_or(0),
            color: label.color.to_rgba8(),
            bold: label.bold,
        }
    }
}

struct Entry {
    buffer: MemoryRenderBuffer,
    size: Size<i32, Logical>,
    last_used: u64,
}

pub struct TextCache {
    text: TextRenderer,
    entries: HashMap<Key, Entry>,
    tick: u64,
}

impl std::fmt::Debug for TextCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TextCache").field("entries", &self.entries.len()).finish()
    }
}

impl Default for TextCache {
    fn default() -> Self {
        Self::new()
    }
}

impl TextCache {
    pub fn new() -> Self {
        Self { text: TextRenderer::new(), entries: HashMap::new(), tick: 0 }
    }

    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn measure(&mut self, label: &Label<'_>) -> Size<i32, Logical> {
        let (w, h) = self.text.measure(label);
        Size::from((w as i32, h as i32))
    }

    pub fn element(
        &mut self,
        renderer: &mut GlesRenderer,
        label: &Label<'_>,
        location: Point<i32, Logical>,
        scale: f64,
        alpha: f32,
    ) -> Option<MemoryRenderBufferRenderElement<GlesRenderer>> {
        let (buffer, size) = self.entry(label)?;

        let physical = Point::<f64, smithay::utils::Physical>::from((
            location.x as f64 * scale,
            location.y as f64 * scale,
        ));
        let src = Rectangle::new(
            Point::from((0.0, 0.0)),
            Size::from((size.w as f64, size.h as f64)),
        );

        match MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            physical,
            &buffer,
            Some(alpha),
            Some(src),
            Some(size),
            Kind::Unspecified,
        ) {
            Ok(element) => Some(element),
            Err(err) => {
                tracing::warn!(?err, "could not upload a text label");
                None
            }
        }
    }

    fn entry(&mut self, label: &Label<'_>) -> Option<(MemoryRenderBuffer, Size<i32, Logical>)> {
        if label.text.trim().is_empty() {
            return None;
        }

        self.tick += 1;
        let key = Key::new(label);

        if let Some(entry) = self.entries.get_mut(&key) {
            entry.last_used = self.tick;
            return Some((entry.buffer.clone(), entry.size));
        }

        let image = self.text.rasterise(label);
        if image.is_empty() {
            return None;
        }

        let buffer = to_render_buffer(&image);
        let size = Size::from((image.width as i32, image.height as i32));
        self.evict_if_full();
        self.entries.insert(key, Entry { buffer: buffer.clone(), size, last_used: self.tick });
        Some((buffer, size))
    }

    fn evict_if_full(&mut self) {
        if self.entries.len() < CAPACITY {
            return;
        }
        if let Some(oldest) =
            self.entries.iter().min_by_key(|(_, e)| e.last_used).map(|(k, _)| k.clone())
        {
            self.entries.remove(&oldest);
        }
    }
}

fn to_render_buffer(image: &Image) -> MemoryRenderBuffer {
    MemoryRenderBuffer::from_slice(
        &image.data,
        smithay::backend::allocator::Fourcc::Abgr8888,
        (image.width as i32, image.height as i32),
        1,
        Transform::Normal,
        None,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use spectre_theme::palette;

    #[test]
    fn keys_separate_the_things_that_change_the_pixels() {
        let base = Label::new("Konsole");
        let a = Key::new(&base);
        assert_eq!(a, Key::new(&Label::new("Konsole")));
        assert_ne!(a, Key::new(&Label::new("konsole")));
        assert_ne!(a, Key::new(&base.clone().bold(true)));
        assert_ne!(a, Key::new(&base.clone().size(20.0)));
        assert_ne!(a, Key::new(&base.clone().color(palette::TEXT_DIM)));
        assert_ne!(a, Key::new(&base.clone().max_width(100)));
    }

    #[test]
    fn nearly_identical_sizes_still_share_a_key() {
        let a = Key::new(&Label::new("x").size(13.0));
        let b = Key::new(&Label::new("x").size(13.01));
        assert_eq!(a, b);
    }

    #[test]
    fn blank_labels_are_never_cached() {
        let mut cache = TextCache::new();
        assert!(cache.entry(&Label::new("")).is_none());
        assert!(cache.entry(&Label::new("   ")).is_none());
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn repeated_labels_reuse_one_entry() {
        let mut cache = TextCache::new();
        if cache.entry(&Label::new("Spectre")).is_none() {
            eprintln!("no system font available; skipping");
            return;
        }
        for _ in 0..10 {
            cache.entry(&Label::new("Spectre"));
        }
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn the_cache_stays_bounded() {
        let mut cache = TextCache::new();
        if cache.entry(&Label::new("probe")).is_none() {
            eprintln!("no system font available; skipping");
            return;
        }
        for i in 0..(CAPACITY * 2) {
            let text = format!("window {i}");
            cache.entry(&Label::new(&text));
        }
        assert!(cache.len() <= CAPACITY, "cached {} labels", cache.len());
    }
}
