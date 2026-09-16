use std::cell::RefCell;
use std::path::{Path, PathBuf};

use image::imageops::FilterType;
use image::GenericImageView;
use spectre_config::WallpaperMode;

pub struct Wallpaper {
    pixels: RefCell<Option<Vec<u8>>>,
    pub size: (i32, i32),
    pub source: (PathBuf, WallpaperMode),
}

impl std::fmt::Debug for Wallpaper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Wallpaper").field("size", &self.size).field("source", &self.source).finish()
    }
}

impl Wallpaper {
    pub fn load(path: &Path, mode: WallpaperMode, width: i32, height: i32) -> Option<Self> {
        let pixels = load_pixels(path, mode, width, height)?;
        Some(Self {
            pixels: RefCell::new(Some(pixels)),
            size: (width, height),
            source: (path.to_owned(), mode),
        })
    }

    pub fn key(&self) -> String {
        let (path, mode) = &self.source;
        format!("{}|{:?}|{}x{}", path.display(), mode, self.size.0, self.size.1)
    }

    pub fn pixels(&self) -> Option<Vec<u8>> {
        let taken = self.pixels.borrow_mut().take();
        if taken.is_some() {
            return taken;
        }
        let (path, mode) = &self.source;
        load_pixels(path, *mode, self.size.0, self.size.1)
    }

    pub fn matches(
        &self,
        path: &Path,
        mode: WallpaperMode,
        width: i32,
        height: i32,
    ) -> bool {
        self.size == (width, height) && self.source.0 == path && self.source.1 == mode
    }
}

pub const BLUR_SHRINK: i32 = 4;

const BLUR_RADIUS: i32 = 3;

pub fn blurred(pixels: &[u8], size: (i32, i32)) -> Option<(Vec<u8>, (i32, i32))> {
    if size.0 <= 0 || size.1 <= 0 {
        return None;
    }
    if pixels.len() < (size.0 as usize * size.1 as usize * 4) {
        return None;
    }
    let width = (size.0 / BLUR_SHRINK).max(1);
    let height = (size.1 / BLUR_SHRINK).max(1);
    let mut small = shrink(pixels, size, width, height);
    soften(&mut small, width, height);
    soften(&mut small, width, height);
    Some((small, (width, height)))
}

fn shrink(pixels: &[u8], size: (i32, i32), width: i32, height: i32) -> Vec<u8> {
    let mut out = vec![0u8; width as usize * height as usize * 4];
    let count = (BLUR_SHRINK * BLUR_SHRINK) as u32;
    for y in 0..height {
        for x in 0..width {
            let mut sums = [0u32; 4];
            for step_y in 0..BLUR_SHRINK {
                for step_x in 0..BLUR_SHRINK {
                    let source_x = (x * BLUR_SHRINK + step_x).min(size.0 - 1);
                    let source_y = (y * BLUR_SHRINK + step_y).min(size.1 - 1);
                    let at = (source_y as usize * size.0 as usize + source_x as usize) * 4;
                    for channel in 0..4 {
                        sums[channel] += pixels[at + channel] as u32;
                    }
                }
            }
            let at = (y as usize * width as usize + x as usize) * 4;
            for channel in 0..4 {
                out[at + channel] = (sums[channel] / count) as u8;
            }
        }
    }
    out
}

fn soften(pixels: &mut [u8], width: i32, height: i32) {
    let count = (BLUR_RADIUS * 2 + 1) as u32;

    let source = pixels.to_vec();
    for y in 0..height {
        for x in 0..width {
            let mut sums = [0u32; 4];
            for step in -BLUR_RADIUS..=BLUR_RADIUS {
                let near = (x + step).clamp(0, width - 1);
                let at = (y as usize * width as usize + near as usize) * 4;
                for channel in 0..4 {
                    sums[channel] += source[at + channel] as u32;
                }
            }
            let at = (y as usize * width as usize + x as usize) * 4;
            for channel in 0..4 {
                pixels[at + channel] = (sums[channel] / count) as u8;
            }
        }
    }

    let source = pixels.to_vec();
    for y in 0..height {
        for x in 0..width {
            let mut sums = [0u32; 4];
            for step in -BLUR_RADIUS..=BLUR_RADIUS {
                let near = (y + step).clamp(0, height - 1);
                let at = (near as usize * width as usize + x as usize) * 4;
                for channel in 0..4 {
                    sums[channel] += source[at + channel] as u32;
                }
            }
            let at = (y as usize * width as usize + x as usize) * 4;
            for channel in 0..4 {
                pixels[at + channel] = (sums[channel] / count) as u8;
            }
        }
    }
}

fn load_pixels(path: &Path, mode: WallpaperMode, width: i32, height: i32) -> Option<Vec<u8>> {
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
    Some(fit(image, mode, width as u32, height as u32))
}

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
        assert!(Wallpaper::load(Path::new("/nowhere.png"), WallpaperMode::Fill, 0, 0).is_none());
    }

    #[test]
    fn the_blurred_copy_is_smaller_and_evens_the_picture_out() {
        let sharp = fit(source(64, 64), WallpaperMode::Stretch, 64, 64);
        let (soft, size) = blurred(&sharp, (64, 64)).expect("a blurred copy");
        assert_eq!(size, (16, 16));
        assert_eq!(soft.len(), 16 * 16 * 4);

        let spread = |pixels: &[u8]| {
            let reds: Vec<u8> = pixels.iter().skip(2).step_by(4).copied().collect();
            let high = reds.iter().copied().max().unwrap_or(0) as i32;
            let low = reds.iter().copied().min().unwrap_or(0) as i32;
            high - low
        };
        assert!(spread(&soft) < spread(&sharp), "blurring must flatten the picture");
    }

    #[test]
    fn a_picture_with_no_area_has_no_blurred_copy() {
        assert!(blurred(&[], (0, 0)).is_none());
    }

    #[test]
    fn the_pixels_are_handed_out_once_and_read_again_after_that() {
        let directory = std::env::temp_dir().join("spectre-wallpaper-test");
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("wallpaper.png");
        source(8, 8).save(&path).unwrap();

        let wallpaper = Wallpaper::load(&path, WallpaperMode::Fill, 20, 10).unwrap();
        let first = wallpaper.pixels().expect("the pixels it was loaded with");
        let again = wallpaper.pixels().expect("read from disk a second time");
        assert_eq!(first.len(), 20 * 10 * 4);
        assert_eq!(first, again, "the picture must not change when it is read again");
        std::fs::remove_file(&path).unwrap();
    }
}
