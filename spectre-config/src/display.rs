//! Output resolution and scale.

use serde::{Deserialize, Serialize};

/// `[display]` in the config file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct Display {
    /// `auto`, `1920x1080`, or `1920x1080@60`.
    pub resolution: String,
    /// Fractional output scale. `1.0` is one logical pixel per device pixel.
    pub scale: f64,
}

impl Default for Display {
    fn default() -> Self {
        Self { resolution: String::from("auto"), scale: 1.0 }
    }
}

/// A resolution the user asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WantedMode {
    pub width: i32,
    pub height: i32,
    /// Refresh in whole Hz, when the config named one.
    pub refresh: Option<u32>,
}

impl Display {
    /// The requested mode, or `None` for `auto` and for anything unparsable.
    pub fn wanted_mode(&self) -> Option<WantedMode> {
        parse_mode(&self.resolution)
    }

    /// Scale clamped to what a compositor can actually drive.
    pub fn output_scale(&self) -> f64 {
        if self.scale.is_finite() {
            self.scale.clamp(0.5, 4.0)
        } else {
            1.0
        }
    }
}

fn parse_mode(text: &str) -> Option<WantedMode> {
    let text = text.trim();
    if text.is_empty() || text.eq_ignore_ascii_case("auto") {
        return None;
    }
    let (size, refresh) = match text.split_once('@') {
        Some((size, hz)) => (size, hz.trim().parse::<u32>().ok()),
        None => (text, None),
    };
    let (w, h) = size.split_once(['x', 'X'])?;
    let width = w.trim().parse::<i32>().ok()?;
    let height = h.trim().parse::<i32>().ok()?;
    if width <= 0 || height <= 0 {
        return None;
    }
    Some(WantedMode { width, height, refresh })
}

/// How a resolution is written in the config file.
pub fn format_mode(width: i32, height: i32, refresh: Option<u32>) -> String {
    match refresh {
        Some(hz) => format!("{width}x{height}@{hz}"),
        None => format!("{width}x{height}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_is_the_default_and_means_the_preferred_mode() {
        let display = Display::default();
        assert_eq!(display.resolution, "auto");
        assert!(display.wanted_mode().is_none());
    }

    #[test]
    fn a_plain_resolution_parses() {
        let m = parse_mode("1920x1080").unwrap();
        assert_eq!((m.width, m.height, m.refresh), (1920, 1080, None));
    }

    #[test]
    fn a_refresh_rate_may_be_pinned() {
        let m = parse_mode("2560x1440@144").unwrap();
        assert_eq!((m.width, m.height, m.refresh), (2560, 1440, Some(144)));
    }

    #[test]
    fn whitespace_and_capitals_are_tolerated() {
        assert_eq!(parse_mode(" 1280X800 "), parse_mode("1280x800"));
        assert!(parse_mode("AUTO").is_none());
    }

    #[test]
    fn nonsense_falls_back_to_auto_rather_than_failing_to_start() {
        for text in ["", "big", "1920", "1920x", "x1080", "0x0", "-1920x1080"] {
            assert!(parse_mode(text).is_none(), "{text}");
        }
    }

    #[test]
    fn the_scale_is_clamped_to_something_drivable() {
        assert_eq!(Display { scale: 0.0, ..Default::default() }.output_scale(), 0.5);
        assert_eq!(Display { scale: 99.0, ..Default::default() }.output_scale(), 4.0);
        assert_eq!(Display { scale: f64::NAN, ..Default::default() }.output_scale(), 1.0);
        assert_eq!(Display { scale: 1.5, ..Default::default() }.output_scale(), 1.5);
    }

    #[test]
    fn formatting_round_trips_through_parsing() {
        for (w, h, hz) in [(1920, 1080, None), (1280, 720, Some(60))] {
            let text = format_mode(w, h, hz);
            let parsed = parse_mode(&text).unwrap();
            assert_eq!((parsed.width, parsed.height, parsed.refresh), (w, h, hz));
        }
    }
}
