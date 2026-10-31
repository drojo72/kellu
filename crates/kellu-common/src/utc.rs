//! UTC timestamps without a dependency.
//!
//! Logs and session files use UTC: local time needs the platform time-zone database, which is
//! racy in multithreaded programs and not worth a dependency here.

use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

/// A calendar date and time of day in UTC, with second precision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UtcStamp {
    /// Year (for example 2026).
    pub year: i64,
    /// Month, 1 to 12.
    pub month: u8,
    /// Day of month, 1 to 31.
    pub day: u8,
    /// Hour, 0 to 23.
    pub hour: u8,
    /// Minute, 0 to 59.
    pub minute: u8,
    /// Second, 0 to 59.
    pub second: u8,
}

impl UtcStamp {
    /// The current time (the epoch if the system clock is before 1970).
    #[must_use]
    pub fn now() -> Self {
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .and_then(|d| i64::try_from(d.as_secs()).ok())
            .unwrap_or(0);
        Self::from_unix(secs)
    }

    /// Convert seconds since the Unix epoch (civil-from-days algorithm by Howard Hinnant).
    #[must_use]
    pub fn from_unix(secs: i64) -> Self {
        let days = secs.div_euclid(86_400);
        let rem = secs.rem_euclid(86_400);
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z.rem_euclid(146_097);
        let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = doy - (153 * mp + 2) / 5 + 1;
        let month = if mp < 10 { mp + 3 } else { mp - 9 };
        let year = yoe + era * 400 + i64::from(month <= 2);
        let small = |value: i64| u8::try_from(value).unwrap_or(0);
        Self {
            year,
            month: small(month),
            day: small(day),
            hour: small(rem / 3_600),
            minute: small(rem % 3_600 / 60),
            second: small(rem % 60),
        }
    }

    /// `YYYY-MM-DD`.
    #[must_use]
    pub fn date(&self) -> String {
        format!("{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }

    /// `HH:MM:SS`.
    #[must_use]
    pub fn time(&self) -> String {
        format!("{:02}:{:02}:{:02}", self.hour, self.minute, self.second)
    }

    /// `YYYY-MM-DD-HHMMSS`, used in file names.
    #[must_use]
    pub fn compact(&self) -> String {
        format!(
            "{}-{:02}{:02}{:02}",
            self.date(),
            self.hour,
            self.minute,
            self.second
        )
    }
}

impl fmt::Display for UtcStamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.date(), self.time())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_instants_convert_correctly() {
        assert_eq!(UtcStamp::from_unix(0).to_string(), "1970-01-01 00:00:00");
        assert_eq!(UtcStamp::from_unix(951_782_400).date(), "2000-02-29");
        assert_eq!(
            UtcStamp::from_unix(1_700_000_000).to_string(),
            "2023-11-14 22:13:20"
        );
        assert_eq!(UtcStamp::from_unix(1_782_864_000).date(), "2026-07-01");
        assert_eq!(
            UtcStamp::from_unix(4_102_444_799).to_string(),
            "2099-12-31 23:59:59"
        );
    }

    #[test]
    fn negative_seconds_reach_before_the_epoch() {
        assert_eq!(UtcStamp::from_unix(-1).to_string(), "1969-12-31 23:59:59");
    }

    #[test]
    fn compact_form_is_filename_friendly() {
        assert_eq!(
            UtcStamp::from_unix(1_700_000_000).compact(),
            "2023-11-14-221320"
        );
    }

    #[test]
    fn now_is_after_2025() {
        assert!(UtcStamp::now().year >= 2025);
    }
}
