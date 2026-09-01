//! Backends: the two ways Spectre can get a screen and input devices.
//!
//! `winit` runs the compositor as a window inside an existing session, which is
//! how development and the VM smoke test work. `udev` drives KMS/DRM directly
//! and is the real desktop session.

#[cfg(feature = "udev")]
pub mod udev;
#[cfg(feature = "winit")]
pub mod winit;

/// Which backend to start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    /// Nested inside another Wayland or X11 session.
    Winit,
    /// Native KMS/DRM session on a TTY.
    Udev,
}

impl Backend {
    /// Pick a backend from the environment.
    ///
    /// Being inside a session means a nested window is almost certainly what
    /// the user wants; starting a DRM session from inside another compositor
    /// would fight it for the display.
    pub fn detect() -> Backend {
        let nested = std::env::var_os("WAYLAND_DISPLAY").is_some()
            || std::env::var_os("DISPLAY").is_some();
        if nested {
            Backend::Winit
        } else {
            Backend::Udev
        }
    }

    pub fn parse(name: &str) -> Option<Backend> {
        match name.to_ascii_lowercase().as_str() {
            "winit" | "nested" | "window" => Some(Backend::Winit),
            "udev" | "drm" | "tty" | "native" => Some(Backend::Udev),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Backend::Winit => "winit",
            Backend::Udev => "udev",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_every_documented_alias() {
        for name in ["winit", "WINIT", "nested", "window"] {
            assert_eq!(Backend::parse(name), Some(Backend::Winit), "{name}");
        }
        for name in ["udev", "drm", "tty", "native"] {
            assert_eq!(Backend::parse(name), Some(Backend::Udev), "{name}");
        }
    }

    #[test]
    fn rejects_unknown_backends() {
        assert_eq!(Backend::parse("x11"), None);
        assert_eq!(Backend::parse(""), None);
    }

    #[test]
    fn names_round_trip() {
        for backend in [Backend::Winit, Backend::Udev] {
            assert_eq!(Backend::parse(backend.name()), Some(backend));
        }
    }
}
