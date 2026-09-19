use std::path::{Path, PathBuf};

use crate::just_run;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Brightness {
    pub percent: u8,
}

impl Brightness {
    pub fn read() -> Option<Self> {
        let path = first_backlight(Path::new("/sys/class/backlight"))?;
        let now = std::fs::read_to_string(path.join("brightness")).ok()?;
        let most = std::fs::read_to_string(path.join("max_brightness")).ok()?;
        parse(&now, &most)
    }

    pub fn change(step: i32) -> bool {
        if step == 0 {
            return false;
        }
        let sign = match step > 0 {
            true => '+',
            false => '-',
        };
        let amount = step.unsigned_abs();
        if just_run("brightnessctl", &["set", &format!("{amount}%{sign}")]) {
            return true;
        }
        if just_run("light", &[&format!("-{sign}"), &amount.to_string()]) {
            return true;
        }
        write_sysfs(step)
    }

    pub fn label(&self) -> String {
        format!("{}%", self.percent)
    }
}

fn write_sysfs(step: i32) -> bool {
    let Some(path) = first_backlight(Path::new("/sys/class/backlight")) else {
        return false;
    };
    let Some(now) = std::fs::read_to_string(path.join("brightness"))
        .ok()
        .and_then(|text| text.trim().parse::<i64>().ok())
    else {
        return false;
    };
    let Some(most) = std::fs::read_to_string(path.join("max_brightness"))
        .ok()
        .and_then(|text| text.trim().parse::<i64>().ok())
    else {
        return false;
    };
    let wanted = wanted_value(now, most, step);
    match std::fs::write(path.join("brightness"), wanted.to_string()) {
        Ok(()) => true,
        Err(err) => {
            tracing::debug!(?err, "the backlight cannot be written to by this program");
            false
        }
    }
}

pub fn wanted_value(now: i64, most: i64, step: i32) -> i64 {
    if most <= 0 {
        return now;
    }
    let change = most * step as i64 / 100;
    let change = match change {
        0 if step > 0 => 1,
        0 if step < 0 => -1,
        change => change,
    };
    (now + change).clamp(1, most)
}

fn first_backlight(directory: &Path) -> Option<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(directory)
        .ok()?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.join("brightness").exists())
        .collect();
    found.sort();
    found.into_iter().next()
}

pub fn parse(now: &str, most: &str) -> Option<Brightness> {
    let now: f32 = now.trim().parse().ok()?;
    let most: f32 = most.trim().parse().ok()?;
    if most <= 0.0 {
        return None;
    }
    let share = (now / most).clamp(0.0, 1.0);
    Some(Brightness { percent: (share * 100.0).round() as u8 })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_backlight_is_read_as_a_percentage_of_its_maximum() {
        assert_eq!(parse("120", "240"), Some(Brightness { percent: 50 }));
        assert_eq!(parse("0", "255"), Some(Brightness { percent: 0 }));
        assert_eq!(parse("255", "255"), Some(Brightness { percent: 100 }));
    }

    #[test]
    fn a_machine_without_a_backlight_reports_nothing() {
        assert_eq!(parse("10", "0"), None);
        assert_eq!(parse("", "255"), None);
        assert_eq!(first_backlight(Path::new("/nowhere-at-all")), None);
    }

    #[test]
    fn a_step_moves_by_that_share_of_the_whole_range() {
        assert_eq!(wanted_value(100, 1000, 10), 200);
        assert_eq!(wanted_value(100, 1000, -10), 1);
    }

    #[test]
    fn the_screen_never_goes_fully_dark_and_never_past_the_maximum() {
        assert_eq!(wanted_value(5, 100, -50), 1, "a black screen is not helpful");
        assert_eq!(wanted_value(95, 100, 50), 100);
    }

    #[test]
    fn a_tiny_range_still_moves_by_one_step() {
        assert_eq!(wanted_value(3, 7, 5), 4);
        assert_eq!(wanted_value(3, 7, -5), 2);
    }
}
