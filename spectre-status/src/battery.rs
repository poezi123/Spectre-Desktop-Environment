use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Battery {
    pub percent: u8,
    pub charging: bool,
    pub full: bool,
}

impl Battery {
    pub fn read() -> Option<Self> {
        let path = first_battery(Path::new("/sys/class/power_supply"))?;
        let capacity = std::fs::read_to_string(path.join("capacity")).ok()?;
        let status = std::fs::read_to_string(path.join("status")).unwrap_or_default();
        parse(&capacity, &status)
    }

    pub fn label(&self) -> String {
        match self.charging {
            true => format!("{}% +", self.percent),
            false => format!("{}%", self.percent),
        }
    }

    pub fn low(&self) -> bool {
        !self.charging && self.percent <= 15
    }
}

fn first_battery(directory: &Path) -> Option<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(directory)
        .ok()?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            let name = path.file_name().and_then(|name| name.to_str()).unwrap_or("");
            name.starts_with("BAT") && path.join("capacity").exists()
        })
        .collect();
    found.sort();
    found.into_iter().next()
}

pub fn parse(capacity: &str, status: &str) -> Option<Battery> {
    let percent: u32 = capacity.trim().parse().ok()?;
    let status = status.trim().to_lowercase();
    Some(Battery {
        percent: percent.min(100) as u8,
        charging: status == "charging",
        full: status == "full" || percent >= 100,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_discharging_battery_reads_as_a_plain_percentage() {
        let battery = parse("83\n", "Discharging\n").expect("a battery");
        assert_eq!(battery, Battery { percent: 83, charging: false, full: false });
        assert_eq!(battery.label(), "83%");
        assert!(!battery.low());
    }

    #[test]
    fn a_charging_battery_says_so() {
        let battery = parse("42", "Charging").expect("a battery");
        assert!(battery.charging);
        assert_eq!(battery.label(), "42% +");
        assert!(!battery.low(), "while it charges, nothing is urgent");
    }

    #[test]
    fn a_nearly_empty_battery_is_worth_a_warning() {
        let battery = parse("9", "Discharging").expect("a battery");
        assert!(battery.low());
    }

    #[test]
    fn a_full_battery_is_marked_full_whatever_the_status_says() {
        assert!(parse("100", "Not charging").expect("a battery").full);
        assert!(parse("97", "Full").expect("a battery").full);
    }

    #[test]
    fn a_machine_without_a_battery_reports_nothing() {
        assert_eq!(parse("", ""), None);
        assert_eq!(parse("a lot", "Full"), None);
        assert_eq!(first_battery(Path::new("/nowhere-at-all")), None);
    }
}
