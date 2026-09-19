use crate::{just_run, run};

const SINK: &str = "@DEFAULT_AUDIO_SINK@";
const PULSE_SINK: &str = "@DEFAULT_SINK@";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sound {
    pub percent: u8,
    pub muted: bool,
}

impl Sound {
    pub fn read() -> Option<Self> {
        if let Some(text) = run("wpctl", &["get-volume", SINK]) {
            return from_wpctl(&text);
        }
        let volume = run("pactl", &["get-sink-volume", PULSE_SINK])?;
        let mute = run("pactl", &["get-sink-mute", PULSE_SINK]).unwrap_or_default();
        from_pactl(&volume, &mute)
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
        let wp = format!("{amount}%{sign}");
        if just_run("wpctl", &["set-volume", "--limit", "1.0", SINK, &wp]) {
            return true;
        }
        let pa = format!("{sign}{amount}%");
        just_run("pactl", &["set-sink-volume", PULSE_SINK, &pa])
    }

    pub fn toggle_mute() -> bool {
        if just_run("wpctl", &["set-mute", SINK, "toggle"]) {
            return true;
        }
        just_run("pactl", &["set-sink-mute", PULSE_SINK, "toggle"])
    }

    pub fn set(percent: u8) -> bool {
        let wanted = percent.min(100);
        let wp = format!("{}%", wanted);
        if just_run("wpctl", &["set-volume", SINK, &wp]) {
            return true;
        }
        just_run("pactl", &["set-sink-volume", PULSE_SINK, &wp])
    }

    pub fn label(&self) -> String {
        match self.muted {
            true => String::from("off"),
            false => format!("{}%", self.percent),
        }
    }
}

pub fn from_wpctl(text: &str) -> Option<Sound> {
    let line = text.lines().find(|line| line.contains("Volume:"))?;
    let muted = line.contains("MUTED");
    let number = line.split_whitespace().nth(1)?;
    let share: f32 = number.parse().ok()?;
    Some(Sound { percent: (share * 100.0).round().clamp(0.0, 100.0) as u8, muted })
}

pub fn from_pactl(volume: &str, mute: &str) -> Option<Sound> {
    let percent = volume
        .split('/')
        .map(str::trim)
        .find_map(|part| part.strip_suffix('%'))
        .and_then(|number| number.trim().parse::<u32>().ok())?;
    let muted = mute.to_lowercase().contains("yes");
    Some(Sound { percent: percent.min(100) as u8, muted })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wpctl_tells_us_a_share_and_we_want_a_percentage() {
        let sound = from_wpctl("Volume: 0.65\n").expect("a volume");
        assert_eq!(sound, Sound { percent: 65, muted: false });
    }

    #[test]
    fn wpctl_marks_a_muted_sink() {
        let sound = from_wpctl("Volume: 0.40 [MUTED]\n").expect("a volume");
        assert_eq!(sound, Sound { percent: 40, muted: true });
        assert_eq!(sound.label(), "off");
    }

    #[test]
    fn a_volume_above_one_is_pulled_back_to_a_hundred() {
        let sound = from_wpctl("Volume: 1.35").expect("a volume");
        assert_eq!(sound.percent, 100);
    }

    #[test]
    fn nonsense_from_wpctl_is_no_volume_at_all() {
        assert_eq!(from_wpctl("no sink"), None);
        assert_eq!(from_wpctl("Volume: loud"), None);
    }

    #[test]
    fn pactl_writes_the_percentage_in_its_own_way() {
        let volume = "Volume: front-left: 42281 /  65% / -11.00 dB,   front-right: 42281 /  65%";
        let sound = from_pactl(volume, "Mute: no").expect("a volume");
        assert_eq!(sound, Sound { percent: 65, muted: false });

        let muted = from_pactl(volume, "Mute: yes").expect("a volume");
        assert!(muted.muted);
    }

    #[test]
    fn the_label_reads_as_a_percentage_when_sound_is_on() {
        let sound = Sound { percent: 7, muted: false };
        assert_eq!(sound.label(), "7%");
    }
}
