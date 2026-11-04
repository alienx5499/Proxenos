//! Unix seconds to an RFC 3339 UTC string, for Switch's `expires_at`.

use proxenos_domain::timestamp::Timestamp;

use crate::chat::format::civil;

/// `YYYY-MM-DDTHH:MM:SSZ`, proleptic Gregorian.
pub fn format(t: Timestamp) -> String {
    let secs = t.seconds();
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let (y, m, d) = civil(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3_600,
        rem % 3_600 / 60,
        rem % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// (year, month, day, hour, minute, second) read back from the string.
    fn parts(t: i64) -> [i64; 6] {
        let s = format(Timestamp(t));
        assert_eq!(s.len(), 20, "{s}");
        assert_eq!(
            (&s[4..5], &s[7..8], &s[10..11], &s[19..]),
            ("-", "-", "T", "Z")
        );
        let n = |r: std::ops::Range<usize>| s[r].parse::<i64>().unwrap();
        [n(0..4), n(5..7), n(8..10), n(11..13), n(14..16), n(17..19)]
    }

    #[test]
    fn epoch_start_and_time_of_day() {
        let [_, m, d, h, mi, s] = parts(0);
        assert_eq!([m, d, h, mi, s], [1, 1, 0, 0, 0]);
        let [_, m, d, h, mi, s] = parts(86_399);
        assert_eq!([m, d, h, mi, s], [1, 1, 23, 59, 59]);
        assert_eq!(parts(86_400)[2], 2);
        assert_eq!(parts(3_723)[3..], [1, 2, 3]);
    }

    #[test]
    fn years_months_and_leap_days_roll_over() {
        let y0 = parts(0)[0];
        // Two common years after the epoch start comes a leap year.
        let leap = (365 + 365) * 86_400;
        assert_eq!(parts(leap)[..3], [y0 + 2, 1, 1]);
        assert_eq!(parts(leap + 59 * 86_400)[..3], [y0 + 2, 2, 29]);
        assert_eq!(parts(leap + 60 * 86_400)[..3], [y0 + 2, 3, 1]);
        assert_eq!(parts(365 * 86_400 - 1)[..3], [y0, 12, 31]);
    }
}
