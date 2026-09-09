use time::macros::format_description;
use time::{OffsetDateTime, UtcOffset};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Clock {
    pub time: String,
    pub date: String,
}

impl Clock {
    pub fn from_datetime(now: OffsetDateTime) -> Self {
        let time_format = format_description!("[hour]:[minute]");
        let date_format = format_description!("[day].[month].[year repr:last_two]");
        Self {
            time: now.format(time_format).unwrap_or_else(|_| String::from("--:--")),
            date: now.format(date_format).unwrap_or_else(|_| String::from("--.--.--")),
        }
    }

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
