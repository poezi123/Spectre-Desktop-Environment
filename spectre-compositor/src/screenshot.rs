use std::path::{Path, PathBuf};

use image::ImageEncoder;
use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::RenderElement;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::renderer::ExportMem;
use smithay::utils::{Buffer, Physical, Point, Rectangle, Size};
use time::OffsetDateTime;

use crate::render::cube;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wish {
    Screen,
    Window,
    Region(Rectangle<i32, Physical>),
}

pub fn take<E>(
    renderer: &mut GlesRenderer,
    size: Size<i32, Physical>,
    elements: &[E],
    crop: Rectangle<i32, Physical>,
) -> Option<Vec<u8>>
where
    E: RenderElement<GlesRenderer>,
{
    let area = inside(crop, size);
    if area.size.w <= 0 || area.size.h <= 0 {
        return None;
    }
    let texture = cube::capture(renderer, size, elements, [0.0, 0.0, 0.0, 1.0])?;
    let region: Rectangle<i32, Buffer> = Rectangle::new(
        Point::from((area.loc.x, area.loc.y)),
        Size::from((area.size.w, area.size.h)),
    );
    let mapping = match renderer.copy_texture(&texture, region, Fourcc::Abgr8888) {
        Ok(mapping) => mapping,
        Err(err) => {
            tracing::warn!(?err, "could not read the picture back from the graphics card");
            return None;
        }
    };
    let bytes = match renderer.map_texture(&mapping) {
        Ok(bytes) => bytes,
        Err(err) => {
            tracing::warn!(?err, "could not look at the picture that was read back");
            return None;
        }
    };
    encode(bytes, area.size.w as u32, area.size.h as u32)
}

pub fn inside(
    crop: Rectangle<i32, Physical>,
    size: Size<i32, Physical>,
) -> Rectangle<i32, Physical> {
    let whole = Rectangle::from_size(size);
    crop.intersection(whole).unwrap_or(whole)
}

fn encode(bytes: &[u8], width: u32, height: u32) -> Option<Vec<u8>> {
    let wanted = width as usize * height as usize * 4;
    if bytes.len() < wanted {
        tracing::warn!(got = bytes.len(), wanted, "the picture came back too small");
        return None;
    }
    let mut png = Vec::new();
    let encoder = image::codecs::png::PngEncoder::new(&mut png);
    let written = encoder.write_image(
        &bytes[..wanted],
        width,
        height,
        image::ExtendedColorType::Rgba8,
    );
    match written {
        Ok(()) => Some(png),
        Err(err) => {
            tracing::warn!(?err, "could not turn the picture into a PNG");
            None
        }
    }
}

pub fn save(png: &[u8], now: OffsetDateTime) -> Option<PathBuf> {
    let directory = pictures_dir()?;
    if let Err(err) = std::fs::create_dir_all(&directory) {
        tracing::warn!(?err, path = %directory.display(), "could not make the pictures folder");
        return None;
    }
    let path = directory.join(file_name(now));
    if let Err(err) = std::fs::write(&path, png) {
        tracing::warn!(?err, path = %path.display(), "could not write the picture");
        return None;
    }
    Some(path)
}

pub fn file_name(now: OffsetDateTime) -> String {
    let stamp = time::macros::format_description!(
        "[year]-[month]-[day] [hour][minute][second]"
    );
    match now.format(stamp) {
        Ok(text) => format!("Spectre {text}.png"),
        Err(_) => String::from("Spectre.png"),
    }
}

pub fn now() -> OffsetDateTime {
    let utc = OffsetDateTime::now_utc();
    match time::UtcOffset::current_local_offset() {
        Ok(offset) => utc.to_offset(offset),
        Err(_) => utc,
    }
}

fn pictures_dir() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("XDG_PICTURES_DIR") {
        return Some(PathBuf::from(path));
    }
    let home = PathBuf::from(std::env::var_os("HOME")?);
    if let Some(path) = from_user_dirs(&home) {
        return Some(path);
    }
    Some(home.join("Pictures"))
}

fn from_user_dirs(home: &Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(home.join(".config/user-dirs.dirs")).ok()?;
    for line in text.lines() {
        let Some(value) = line.trim().strip_prefix("XDG_PICTURES_DIR=") else {
            continue;
        };
        let value = value.trim().trim_matches('"');
        if value.is_empty() {
            continue;
        }
        let Some(rest) = value.strip_prefix("$HOME/") else {
            return Some(PathBuf::from(value));
        };
        return Some(home.join(rest));
    }
    None
}

pub const FLASH: std::time::Duration = std::time::Duration::from_millis(180);

impl crate::state::Spectre {
    pub fn want_screenshot(&mut self, wish: Wish) {
        self.screenshot = Some(wish);
        self.mark_dirty();
    }

    pub fn serve_screenshot<E>(
        &mut self,
        renderer: &mut GlesRenderer,
        elements: &[E],
        size: Size<i32, Physical>,
        scale: f64,
    ) where
        E: RenderElement<GlesRenderer>,
    {
        let Some(wish) = self.screenshot.take() else {
            return;
        };
        let whole = Rectangle::from_size(size);
        let crop = match wish {
            Wish::Screen => whole,
            Wish::Region(area) => area,
            Wish::Window => self.focused_frame(scale).unwrap_or(whole),
        };
        let Some(png) = take(renderer, size, elements, crop) else {
            return;
        };
        match save(&png, now()) {
            Some(path) => tracing::info!(path = %path.display(), "picture taken"),
            None => tracing::warn!("the picture could not be saved, it is only in the clipboard"),
        }
        self.offer_image(png);
        self.flash = Some(std::time::Instant::now());
        self.mark_dirty();
    }

    fn focused_frame(&self, scale: f64) -> Option<Rectangle<i32, Physical>> {
        let window = self.focus.as_ref()?;
        let space = self.workspaces.active();
        let geometry = space.element_geometry(window)?;
        let metrics = self.config.theme.metrics;
        let decorated = self.is_decorated(window);
        let frame = crate::render::decorations::Frame::new(geometry, &metrics, decorated);
        Some(frame.outer.to_physical_precise_round(scale))
    }

    fn offer_image(&mut self, png: Vec<u8>) {
        let seat = self.seat.clone();
        smithay::wayland::selection::data_device::set_data_device_selection(
            &self.display_handle,
            &seat,
            vec![String::from("image/png")],
            (),
        );
        self.clipboard_image = Some(std::sync::Arc::new(png));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_crop_never_reaches_outside_the_screen() {
        let size = Size::<i32, Physical>::from((1920, 1080));
        let wide = Rectangle::new(Point::from((1800, 1000)), Size::from((400, 400)));
        let area = inside(wide, size);
        assert_eq!(area.loc.x, 1800);
        assert_eq!(area.size.w, 120);
        assert_eq!(area.size.h, 80);

        let elsewhere = Rectangle::new(Point::from((3000, 3000)), Size::from((100, 100)));
        assert_eq!(inside(elsewhere, size), Rectangle::from_size(size), "nothing to crop");
    }

    #[test]
    fn the_file_is_named_after_the_moment_it_was_taken() {
        let moment = time::macros::datetime!(2026 - 09 - 19 22:41:03 UTC);
        assert_eq!(file_name(moment), "Spectre 2026-09-19 224103.png");
    }

    #[test]
    fn the_pictures_folder_comes_from_the_user_dirs_file() {
        let home = std::env::temp_dir().join("spectre-pictures-test");
        std::fs::create_dir_all(home.join(".config")).unwrap();
        std::fs::write(
            home.join(".config/user-dirs.dirs"),
            "XDG_DESKTOP_DIR=\"$HOME/Schreibtisch\"\nXDG_PICTURES_DIR=\"$HOME/Bilder\"\n",
        )
        .unwrap();
        assert_eq!(from_user_dirs(&home), Some(home.join("Bilder")));
        std::fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn a_missing_user_dirs_file_is_no_reason_to_fail() {
        let nowhere = std::env::temp_dir().join("spectre-has-no-user-dirs");
        assert_eq!(from_user_dirs(&nowhere), None);
    }
}
