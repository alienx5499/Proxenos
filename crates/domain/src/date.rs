//! Calendar day as days since the Unix epoch; ordered, no time of day.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::timestamp::Timestamp;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Date(i32);

impl Date {
    pub fn from_days(days: i32) -> Self {
        Self(days)
    }

    pub fn days(self) -> i32 {
        self.0
    }

    /// The UTC calendar day containing `t`.
    pub fn containing(t: Timestamp) -> Self {
        Self(t.seconds().div_euclid(86_400) as i32)
    }

    /// ISO `YYYY-MM-DD` (proleptic Gregorian).
    pub fn iso(self) -> String {
        let (y, m, d) = civil(i64::from(self.0));
        format!("{y:04}-{m:02}-{d:02}")
    }

    /// Parses `YYYY-MM-DD`; `None` for any other shape or an impossible day.
    pub fn from_iso(s: &str) -> Option<Self> {
        let b = s.as_bytes();
        if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
            return None;
        }
        let digits = |r: std::ops::Range<usize>| {
            b[r.clone()]
                .iter()
                .all(u8::is_ascii_digit)
                .then(|| s[r].parse::<i64>().ok())?
        };
        let (y, m, d) = (digits(0..4)?, digits(5..7)?, digits(8..10)?);
        if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
            return None;
        }
        let days = days_from_civil(y, m, d);
        // A day past the month's end rolls over, so the round trip rejects it.
        (civil(days) == (y, m, d)).then_some(Self(days as i32))
    }
}

/// Days since the epoch to (year, month, day); the standard era-based conversion.
pub fn civil(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// Inverse of `civil` for in-range months and days.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * ((m + 9) % 12) + 2) / 5 + d - 1;
    era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468
}

impl fmt::Display for Date {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "day {}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orders_by_day() {
        assert!(Date::from_days(10) < Date::from_days(11));
        assert_eq!(Date::from_days(-3).days(), -3);
        assert_eq!(serde_json::to_string(&Date(42)).unwrap(), "42");
    }

    #[test]
    fn iso_round_trips_and_orders() {
        let mut prev = String::new();
        for days in -800..40_000 {
            let s = Date(days).iso();
            assert_eq!(s.len(), 10);
            assert_eq!(Date::from_iso(&s), Some(Date(days)), "{s}");
            assert!(days < 0 || s > prev, "{s} after {prev}");
            prev = s;
        }
        assert!(Date(0).iso().ends_with("-01-01"));
        assert_eq!(Date(31).iso()[4..], *"-02-01");
    }

    #[test]
    fn from_iso_rejects_impossible_and_malformed() {
        let leap = 400; // divisible by 400, so it has a 29th of February
        assert!(Date::from_iso(&format!("{leap:04}-02-29")).is_some());
        assert_eq!(
            Date::from_iso(&format!("{leap:04}-03-01")).unwrap().iso()[4..],
            *"-03-01"
        );
        for bad in [
            format!("{:04}-02-29", leap + 1),
            format!("{leap:04}-04-31"),
            format!("{leap:04}-13-01"),
            format!("{leap:04}-00-10"),
            format!("{leap:04}-01-00"),
            format!("{leap:04}-1-01"),
            format!("{leap:04}/01/01"),
            format!("{leap:05}-01-1"),
            "abcd-ef-gh".into(),
            "329".into(),
        ] {
            assert_eq!(Date::from_iso(&bad), None, "{bad}");
        }
    }

    #[test]
    fn containing_floors_to_the_utc_day() {
        assert_eq!(Date::containing(Timestamp(1_000)), Date(0));
        assert_eq!(Date::containing(Timestamp(86_400 * 3 + 5)), Date(3));
        assert_eq!(Date::containing(Timestamp(-1)), Date(-1));
    }
}
