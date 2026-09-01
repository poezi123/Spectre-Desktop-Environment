//! The panel clock.
//!
//! Kept apart from the drawing so the formatting can be tested without a font,
//! a Wayland connection or a particular machine's time zone.

use time::macros::format_description;
use time::{OffsetDateTime, UtcOffset};

/// The two lines the clock widget shows.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Clock {
    /// `HH:MM`.
    pub time: String,
    /// `DD.MM.YY`.
    pub date: String,
}

impl Clock {
    /// Format an instant for display.
    pub fn from_datetime(now: OffsetDateTime) -> Self {
        let time_format = format_description!("[hour]:[minute]");
        let date_format = format_description!("[day].[month].[year repr:last_two]");
        Self {
            time: now.format(time_format).unwrap_or_else(|_| String::from("--:--")),
            date: now.format(date_format).unwrap_or_else(|_| String::from("--.--.--")),
        }
    }

    /// The current local time.
    ///
    /// Falls back to UTC when the local offset cannot be determined, which is
    /// better than a blank clock; the panel is single threaded, so the usual
    /// reason for that failure does not apply here.
    pub fn now() -> Self {
        let now = OffsetDateTime::now_utc();
        let local = UtcOffset::current_local_offset()
            .map(|offset| now.to_offset(offset))
            .unwrap_or(now);
        Self::from_datetime(local)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    #[test]
    fn midnight_and_noon_are_zero_padded_and_unambiguous() {
        assert_eq!(Clock::from_datetime(datetime!(2026-09-01 00:00 UTC)).time, "00:00");
        assert_eq!(Clock::from_datetime(datetime!(2026-09-01 12:00 UTC)).time, "12:00");
        assert_eq!(Clock::from_datetime(datetime!(2026-09-01 23:59 UTC)).time, "23:59");
    }

    #[test]
    fn the_date_line_is_day_first_and_two_digit() {
        let c = Clock::from_datetime(datetime!(2026-09-01 08:05 UTC));
        assert_eq!(c.date, "01.09.26");
    }

    #[test]
    fn both_lines_keep_a_constant_width() {
        // A clock that changes width makes the whole panel shuffle every
        // minute, so the format has to be fixed-width.
        let a = Clock::from_datetime(datetime!(2026-01-01 01:01 UTC));
        let b = Clock::from_datetime(datetime!(2026-12-31 23:59 UTC));
        assert_eq!(a.time.len(), b.time.len());
        assert_eq!(a.date.len(), b.date.len());
    }

    #[test]
    fn the_current_time_is_always_formatted() {
        let c = Clock::now();
        assert_eq!(c.time.len(), 5);
        assert_eq!(c.date.len(), 8);
    }
}
