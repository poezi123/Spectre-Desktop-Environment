use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum WallpaperMode {
    #[default]
    Fill,
    Fit,
    Stretch,
    Center,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct Desktop {
    pub wallpaper: Option<PathBuf>,
    pub wallpaper_mode: WallpaperMode,
}

impl Desktop {
    pub fn wallpaper_path(&self) -> Option<&Path> {
        self.wallpaper.as_deref().filter(|p| p.is_file())
    }
}

pub fn wallpaper_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        dirs.push(home.join("Pictures/Wallpapers"));
        dirs.push(home.join("Pictures"));
        dirs.push(home.join(".local/share/wallpapers"));
    }
    dirs.push(PathBuf::from("/usr/share/backgrounds"));
    dirs.push(PathBuf::from("/usr/share/wallpapers"));
    dirs
}

pub fn is_image(path: &Path) -> bool {
    let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
        return false;
    };
    matches!(ext.to_ascii_lowercase().as_str(), "png" | "jpg" | "jpeg")
}

pub fn find_wallpapers() -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = Vec::new();
    for dir in wallpaper_dirs() {
        collect_images(&dir, 1, &mut found);
    }
    found.sort();
    found.dedup();
    found
}

fn collect_images(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if depth > 0 {
                collect_images(&path, depth - 1, out);
            }
        } else if is_image(&path) {
            out.push(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_decodable_extensions_count_as_images() {
        assert!(is_image(Path::new("/a/b.png")));
        assert!(is_image(Path::new("/a/B.JPG")));
        assert!(!is_image(Path::new("/a/b.svg")));
        assert!(!is_image(Path::new("/a/b")));
    }

    #[test]
    fn a_wallpaper_that_is_not_there_is_not_offered() {
        let desktop = Desktop {
            wallpaper: Some(PathBuf::from("/nowhere/at/all.png")),
            ..Desktop::default()
        };
        assert!(desktop.wallpaper_path().is_none());
    }

    #[test]
    fn the_default_desktop_is_the_spectre_black() {
        let desktop = Desktop::default();
        assert!(desktop.wallpaper.is_none());
        assert_eq!(desktop.wallpaper_mode, WallpaperMode::Fill);
    }

    #[test]
    fn the_search_path_starts_in_the_users_own_pictures() {
        std::env::set_var("HOME", "/home/tester");
        let dirs = wallpaper_dirs();
        assert_eq!(dirs[0], PathBuf::from("/home/tester/Pictures/Wallpapers"));
        assert!(dirs.contains(&PathBuf::from("/usr/share/backgrounds")));
    }
}
