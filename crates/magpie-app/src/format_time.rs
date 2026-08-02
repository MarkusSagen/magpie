//! Human-friendly relative time formatting (no chrono dependency).

/// Human relative time: "just now", "5m", "3h", "yesterday", "5d", else an
/// absolute `YYYY-MM-DD` date (UTC). `then_ms`/`now_ms` are epoch milliseconds.
pub fn relative_time(then_ms: i64, now_ms: i64) -> String {
    let diff = now_ms - then_ms;
    const S: i64 = 1000;
    const MIN: i64 = 60 * S;
    const HOUR: i64 = 60 * MIN;
    const DAY: i64 = 24 * HOUR;
    if diff < MIN {
        "just now".to_string()
    } else if diff < HOUR {
        format!("{}m", diff / MIN)
    } else if diff < DAY {
        format!("{}h", diff / HOUR)
    } else if diff < 2 * DAY {
        "yesterday".to_string()
    } else if diff < 30 * DAY {
        format!("{}d", diff / DAY)
    } else {
        let (y, m, d) = civil_from_days(then_ms.div_euclid(DAY));
        format!("{y:04}-{m:02}-{d:02}")
    }
}

/// Absolute UTC date `YYYY-MM-DD` for an epoch-millis timestamp.
pub fn abs_date(ms: i64) -> String {
    let (y, m, d) = civil_from_days(ms.div_euclid(86_400_000));
    format!("{y:04}-{m:02}-{d:02}")
}

/// Howard Hinnant's days-from-epoch → civil (y, m, d) algorithm, proleptic
/// Gregorian, UTC. `z` is days since 1970-01-01.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::relative_time;
    const S: i64 = 1000;
    const MIN: i64 = 60 * S;
    const HOUR: i64 = 60 * MIN;
    const DAY: i64 = 24 * HOUR;

    #[test]
    fn buckets() {
        let now = 1_000 * DAY; // far from epoch
        assert_eq!(relative_time(now, now), "just now");
        assert_eq!(relative_time(now - 30 * S, now), "just now"); // <60s
        assert_eq!(relative_time(now - 5 * MIN, now), "5m");
        assert_eq!(relative_time(now - 3 * HOUR, now), "3h");
        assert_eq!(relative_time(now - 26 * HOUR, now), "yesterday");
        assert_eq!(relative_time(now - 5 * DAY, now), "5d");
    }

    #[test]
    fn future_clock_skew_is_just_now() {
        let now = 1_000 * DAY;
        assert_eq!(relative_time(now + 10 * S, now), "just now");
    }

    #[test]
    fn abs_date_formats_epoch_day() {
        assert_eq!(super::abs_date(1_000 * DAY), "1972-09-27");
    }

    #[test]
    fn old_shows_date_yyyy_mm_dd() {
        let now = 1_000 * DAY;
        let out = relative_time(now - 40 * DAY, now);
        assert_eq!(out.len(), 10); // YYYY-MM-DD
        assert_eq!(out.as_bytes()[4], b'-');
        assert_eq!(out.as_bytes()[7], b'-');
    }
}
