//! The desktop wallpaper.
//!
//! Decoded once, scaled to the output on the CPU and handed to the renderer as
//! a memory buffer, so only one copy at output resolution is ever kept.

use image::imageops::FilterType;
use image::GenericImageView;
use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::memory::MemoryRenderBuffer;
use smithay::utils::{Size, Transform};
use spectre_config::WallpaperMode;

pub struct Wallpaper {
    pub buffer: MemoryRenderBuffer,
    /// The output size it was prepared for.
    pub size: (i32, i32),
    /// What it was prepared from, so a reload can skip identical work.
    pub source: (std::path::PathBuf, WallpaperMode),
}

impl std::fmt::Debug for Wallpaper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Wallpaper").field("size", &self.size).field("source", &self.source).finish()
    }
}

impl Wallpaper {
    /// Decode `path` and fit it to a `width` x `height` output.
    pub fn load(
        path: &std::path::Path,
        mode: WallpaperMode,
        width: i32,
        height: i32,
    ) -> Option<Self> {
        if width <= 0 || height <= 0 {
            return None;
        }
        let image = match image::open(path) {
            Ok(image) => image,
            Err(err) => {
                tracing::warn!(?err, path = %path.display(), "could not read the wallpaper");
                return None;
            }
        };

        let pixels = fit(image, mode, width as u32, height as u32);
        let buffer = MemoryRenderBuffer::from_slice(
            &pixels,
            Fourcc::Argb8888,
            Size::from((width, height)),
            1,
            Transform::Normal,
            None,
        );
        Some(Self {
            buffer,
            size: (width, height),
            source: (path.to_owned(), mode),
        })
    }

    /// Whether this wallpaper still matches what is configured.
    pub fn matches(
        &self,
        path: &std::path::Path,
        mode: WallpaperMode,
        width: i32,
        height: i32,
    ) -> bool {
        self.size == (width, height) && self.source.0 == path && self.source.1 == mode
    }
}

/// Scale and crop `image` into a `width` x `height` `Argb8888` buffer.
fn fit(image: image::DynamicImage, mode: WallpaperMode, width: u32, height: u32) -> Vec<u8> {
    let (iw, ih) = image.dimensions();
    let scale = match mode {
        WallpaperMode::Fill => (width as f32 / iw as f32).max(height as f32 / ih as f32),
        WallpaperMode::Fit => (width as f32 / iw as f32).min(height as f32 / ih as f32),
        WallpaperMode::Stretch => 0.0,
        WallpaperMode::Center => 1.0,
    };

    let scaled = if mode == WallpaperMode::Stretch {
        image.resize_exact(width.max(1), height.max(1), FilterType::Triangle)
    } else if mode == WallpaperMode::Center {
        image
    } else {
        let w = ((iw as f32 * scale).round() as u32).max(1);
        let h = ((ih as f32 * scale).round() as u32).max(1);
        image.resize_exact(w, h, FilterType::Triangle)
    };

    let (sw, sh) = scaled.dimensions();
    let rgba = scaled.to_rgba8();
    let raw = rgba.as_raw();
    // Centre the scaled image on the output; whatever falls outside is cropped,
    // whatever is missing stays black.
    let offset_x = (width as i64 - sw as i64) / 2;
    let offset_y = (height as i64 - sh as i64) / 2;

    let mut out = vec![0u8; width as usize * height as usize * 4];
    for y in 0..height as i64 {
        let src_y = y - offset_y;
        if src_y < 0 || src_y >= sh as i64 {
            continue;
        }
        for x in 0..width as i64 {
            let src_x = x - offset_x;
            if src_x < 0 || src_x >= sw as i64 {
                continue;
            }
            let src = (src_y as usize * sw as usize + src_x as usize) * 4;
            let dst = (y as usize * width as usize + x as usize) * 4;
            let (r, g, b, a) = (raw[src], raw[src + 1], raw[src + 2], raw[src + 3]);
            // Argb8888 is [B, G, R, A] in memory, premultiplied.
            let m = |c: u8| ((c as u16 * a as u16) / 255) as u8;
            out[dst] = m(b);
            out[dst + 1] = m(g);
            out[dst + 2] = m(r);
            out[dst + 3] = a;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(w: u32, h: u32) -> image::DynamicImage {
        let mut buffer = image::RgbaImage::new(w, h);
        for (x, y, pixel) in buffer.enumerate_pixels_mut() {
            *pixel = image::Rgba([(x % 256) as u8, (y % 256) as u8, 128, 255]);
        }
        image::DynamicImage::ImageRgba8(buffer)
    }

    fn opaque(pixels: &[u8]) -> usize {
        pixels.chunks(4).filter(|p| p[3] != 0).count()
    }

    #[test]
    fn every_mode_produces_a_buffer_of_the_output_size() {
        for mode in [
            WallpaperMode::Fill,
            WallpaperMode::Fit,
            WallpaperMode::Stretch,
            WallpaperMode::Center,
        ] {
            let out = fit(source(64, 32), mode, 100, 50);
            assert_eq!(out.len(), 100 * 50 * 4, "{mode:?}");
        }
    }

    #[test]
    fn filling_leaves_no_gaps() {
        let out = fit(source(64, 32), WallpaperMode::Fill, 100, 50);
        assert_eq!(opaque(&out), 100 * 50, "fill must cover the whole output");
    }

    #[test]
    fn fitting_letterboxes_rather_than_cropping() {
        let out = fit(source(64, 16), WallpaperMode::Fit, 100, 100);
        assert!(opaque(&out) < 100 * 100, "a wide image must leave bars");
        assert!(opaque(&out) > 0);
    }

    #[test]
    fn a_smaller_image_is_centred_rather_than_blown_up() {
        let out = fit(source(10, 10), WallpaperMode::Center, 100, 100);
        assert_eq!(opaque(&out), 100, "only the image's own pixels are drawn");
        let at = |x: usize, y: usize| out[(y * 100 + x) * 4 + 3];
        assert_eq!(at(50, 50), 255, "the centre is covered");
        assert_eq!(at(0, 0), 0, "the corners are not");
    }

    #[test]
    fn stretching_covers_the_output_whatever_the_aspect() {
        let out = fit(source(8, 40), WallpaperMode::Stretch, 90, 30);
        assert_eq!(opaque(&out), 90 * 30);
    }

    #[test]
    fn an_output_with_no_area_is_refused_rather_than_divided_by() {
        assert!(Wallpaper::load(std::path::Path::new("/nowhere.png"), WallpaperMode::Fill, 0, 0)
            .is_none());
    }
}
