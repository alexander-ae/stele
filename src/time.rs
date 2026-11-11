//! Wall-clock helpers, without a date crate.
//!
//! A mailbox needs two things from a clock: the current second and a readable
//! UTC timestamp. `chrono` would do both, at the cost of a few crates and
//! about 300 KB in the binary; the civil-date conversion below is the part
//! that is actually needed, and it is short enough to test.

use std::time::{SystemTime, UNIX_EPOCH};

/// Seconds since the Unix epoch, UTC.
pub fn now() -> i64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_secs() as i64,
        Err(_) => 0,
    }
}

/// `2026-11-11T03:04:05Z`, the form the message list shows.
pub fn iso8601(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    let (h, mi, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    format!("{y:04}-{m:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
}

/// Howard Hinnant's `civil_from_days`: days since 1970-01-01 to a Gregorian
/// date. It is exact for every date the mailbox will ever hold.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_is_1970() {
        assert_eq!(iso8601(0), "1970-01-01T00:00:00Z");
    }

    #[test]
    fn known_dates() {
        assert_eq!(iso8601(1_000_000_000), "2001-09-09T01:46:40Z");
        assert_eq!(iso8601(1_767_225_600), "2026-01-01T00:00:00Z");
        assert_eq!(iso8601(1_796_947_200), "2026-12-11T00:00:00Z");
    }

    #[test]
    fn leap_day() {
        assert_eq!(iso8601(1_709_164_800), "2024-02-29T00:00:00Z");
    }

    #[test]
    fn dates_before_epoch() {
        assert_eq!(iso8601(-1), "1969-12-31T23:59:59Z");
    }
}
