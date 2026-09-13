use std::collections::HashMap;
use std::path::{Path, PathBuf};

use spectre_text::Image;

pub fn load(path: &Path, max: u32) -> Option<Image> {
    if max == 0 {
        return None;
    }
    let decoded = image::open(path).ok()?.to_rgba8();
    let (width, height) = decoded.dimensions();
    if width == 0 || height == 0 {
        return None;
    }
    let (out_width, out_height) = fitted_size(width, height, max);
    let source = premultiplied(decoded.as_raw());
    Some(shrink(&source, width, height, out_width, out_height))
}

fn fitted_size(width: u32, height: u32, max: u32) -> (u32, u32) {
    let largest = width.max(height);
    if largest <= max {
        return (width, height);
    }
    let out_width = (width as u64 * max as u64 / largest as u64).max(1) as u32;
    let out_height = (height as u64 * max as u64 / largest as u64).max(1) as u32;
    (out_width, out_height)
}

fn premultiplied(rgba: &[u8]) -> Vec<u8> {
    let mut out = rgba.to_vec();
    for pixel in out.chunks_exact_mut(4) {
        let alpha = pixel[3] as u32;
        pixel[0] = (pixel[0] as u32 * alpha / 255) as u8;
        pixel[1] = (pixel[1] as u32 * alpha / 255) as u8;
        pixel[2] = (pixel[2] as u32 * alpha / 255) as u8;
    }
    out
}

fn shrink(source: &[u8], width: u32, height: u32, out_width: u32, out_height: u32) -> Image {
    let mut data = vec![0u8; out_width as usize * out_height as usize * 4];

    for out_y in 0..out_height {
        let top = out_y * height / out_height;
        let bottom = ((out_y + 1) * height / out_height).max(top + 1);

        for out_x in 0..out_width {
            let left = out_x * width / out_width;
            let right = ((out_x + 1) * width / out_width).max(left + 1);

            let mut sum = [0u32; 4];
            let mut count = 0u32;
            for y in top..bottom {
                for x in left..right {
                    let i = (y as usize * width as usize + x as usize) * 4;
                    sum[0] += source[i] as u32;
                    sum[1] += source[i + 1] as u32;
                    sum[2] += source[i + 2] as u32;
                    sum[3] += source[i + 3] as u32;
                    count += 1;
                }
            }

            let out = (out_y as usize * out_width as usize + out_x as usize) * 4;
            data[out] = (sum[0] / count) as u8;
            data[out + 1] = (sum[1] / count) as u8;
            data[out + 2] = (sum[2] / count) as u8;
            data[out + 3] = (sum[3] / count) as u8;
        }
    }

    Image { width: out_width, height: out_height, data }
}

#[derive(Default)]
pub struct Cache {
    loaded: HashMap<PathBuf, Option<Image>>,
}

impl Cache {
    pub fn get(&mut self, path: &Path, max: u32) -> Option<&Image> {
        if !self.loaded.contains_key(path) {
            let thumbnail = load(path, max);
            self.loaded.insert(path.to_path_buf(), thumbnail);
        }
        match self.loaded.get(path) {
            Some(Some(thumbnail)) => Some(thumbnail),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "spectre-thumbnail-{name}-{}",
                std::process::id()
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn file(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn write_png(path: &Path, width: u32, height: u32, color: [u8; 4]) {
        let picture = image::RgbaImage::from_pixel(width, height, image::Rgba(color));
        picture.save(path).unwrap();
    }

    #[test]
    fn a_missing_file_has_no_thumbnail() {
        assert!(load(Path::new("/nonexistent/wallpaper.png"), 64).is_none());
    }

    #[test]
    fn a_file_that_is_not_a_picture_has_no_thumbnail() {
        let dir = TempDir::new("garbage");
        let path = dir.file("broken.png");
        std::fs::write(&path, b"this is not a png").unwrap();
        assert!(load(&path, 64).is_none());
    }

    #[test]
    fn a_wide_picture_keeps_its_shape() {
        let dir = TempDir::new("wide");
        let path = dir.file("wide.png");
        write_png(&path, 100, 50, [255, 255, 255, 255]);
        let thumbnail = load(&path, 20).unwrap();
        assert_eq!((thumbnail.width, thumbnail.height), (20, 10));
        assert_eq!(thumbnail.data.len(), 20 * 10 * 4);
    }

    #[test]
    fn a_small_picture_is_not_blown_up() {
        let dir = TempDir::new("small");
        let path = dir.file("small.png");
        write_png(&path, 10, 5, [255, 255, 255, 255]);
        let thumbnail = load(&path, 20).unwrap();
        assert_eq!((thumbnail.width, thumbnail.height), (10, 5));
    }

    #[test]
    fn a_solid_colour_stays_that_colour() {
        let dir = TempDir::new("solid");
        let path = dir.file("solid.png");
        write_png(&path, 90, 60, [10, 200, 30, 255]);
        let thumbnail = load(&path, 16).unwrap();
        assert_eq!(&thumbnail.data[0..4], &[10, 200, 30, 255]);
    }

    #[test]
    fn transparent_pixels_come_out_premultiplied() {
        let dir = TempDir::new("alpha");
        let path = dir.file("alpha.png");
        write_png(&path, 4, 4, [255, 0, 0, 128]);
        let thumbnail = load(&path, 4).unwrap();
        assert_eq!(&thumbnail.data[0..4], &[128, 0, 0, 128]);
    }

    #[test]
    fn the_cache_reads_a_file_only_once() {
        let dir = TempDir::new("cache");
        let path = dir.file("once.png");
        write_png(&path, 40, 40, [255, 255, 255, 255]);

        let mut cache = Cache::default();
        assert!(cache.get(&path, 16).is_some());
        std::fs::remove_file(&path).unwrap();
        assert!(cache.get(&path, 16).is_some(), "the second call must not touch the disk");
    }

    #[test]
    fn the_cache_remembers_a_file_it_could_not_read() {
        let mut cache = Cache::default();
        let path = Path::new("/nonexistent/nothing.png");
        assert!(cache.get(path, 16).is_none());
        assert!(cache.get(path, 16).is_none());
    }
}
