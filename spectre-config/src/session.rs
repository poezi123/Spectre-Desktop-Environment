use std::time::Duration;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct Session {
    pub lock_command: String,
    pub blank_after: u32,
    pub lock_after: u32,
}

impl Default for Session {
    fn default() -> Self {
        Self { lock_command: String::from("spectre-lock"), blank_after: 300, lock_after: 0 }
    }
}

impl Session {
    pub fn lock_command(&self) -> Option<&str> {
        let command = self.lock_command.trim();
        match command.is_empty() {
            true => None,
            false => Some(command),
        }
    }

    pub fn blank_after(&self) -> Option<Duration> {
        after(self.blank_after)
    }

    pub fn lock_after(&self) -> Option<Duration> {
        after(self.lock_after)
    }
}

fn after(seconds: u32) -> Option<Duration> {
    match seconds {
        0 => None,
        seconds => Some(Duration::from_secs(seconds.max(5) as u64)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_screen_goes_dark_after_five_minutes_and_stays_unlocked() {
        let session = Session::default();
        assert_eq!(session.blank_after(), Some(Duration::from_secs(300)));
        assert_eq!(session.lock_after(), None);
        assert_eq!(session.lock_command(), Some("spectre-lock"));
    }

    #[test]
    fn a_zero_means_never() {
        let session = Session { blank_after: 0, lock_after: 0, ..Session::default() };
        assert_eq!(session.blank_after(), None);
        assert_eq!(session.lock_after(), None);
    }

    #[test]
    fn a_very_short_time_is_pulled_up_to_something_usable() {
        let session = Session { blank_after: 1, ..Session::default() };
        assert_eq!(session.blank_after(), Some(Duration::from_secs(5)));
    }

    #[test]
    fn no_lock_program_means_the_session_cannot_be_locked() {
        let session = Session { lock_command: String::from("  "), ..Session::default() };
        assert_eq!(session.lock_command(), None);
    }
}
