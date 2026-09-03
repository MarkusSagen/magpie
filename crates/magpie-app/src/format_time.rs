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
pub fn civil_from_days(z: i64) -> (i64, u32, u32) {
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

const DAY_MS: i64 = 86_400_000;

/// Days since 1970-01-01 for a civil date (Howard Hinnant's algorithm; inverse of
/// `civil_from_days`).
pub fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// Day-of-week for a day number, 0 = Sunday (1970-01-01 was a Thursday).
fn weekday(days: i64) -> i64 {
    (days.rem_euclid(7) + 4) % 7
}

/// Resolve a `@due` token (without the `@`) to a day-epoch (midnight UTC), using
/// `now_ms` for relative dates. Supports today/tomorrow/tmr, mon..sun (next
/// occurrence), and absolute YYYY-MM-DD.
pub fn parse_due(token: &str, now_ms: i64) -> Option<i64> {
    let t = token.to_ascii_lowercase();
    let today = now_ms.div_euclid(DAY_MS);
    let wd = |target: i64| {
        let cur = weekday(today);
        let mut delta = (target - cur).rem_euclid(7);
        if delta == 0 {
            delta = 7;
        } // "@mon" means the NEXT Monday, not today
        today + delta
    };
    let day = match t.as_str() {
        "today" => today,
        "tomorrow" | "tmr" => today + 1,
        "sun" => wd(0),
        "mon" => wd(1),
        "tue" => wd(2),
        "wed" => wd(3),
        "thu" => wd(4),
        "fri" => wd(5),
        "sat" => wd(6),
        _ => {
            let mut it = t.split('-');
            let y: i64 = it.next()?.parse().ok()?;
            let m: i64 = it.next()?.parse().ok()?;
            let d: i64 = it.next()?.parse().ok()?;
            if it.next().is_some() || !(1..=12).contains(&m) || !(1..=31).contains(&d) {
                return None;
            }
            days_from_civil(y, m, d)
        }
    };
    Some(day * DAY_MS)
}

/// Parse a clock time into minutes past midnight (0..=1439). Accepts "HH:MM" (24h),
/// "H:MMam|pm", and "Ham|Hpm" (e.g. "9am", "5pm", "9:30am"). A bare number without
/// `:` or an am/pm suffix is NOT a time (returns None), so it can't eat titles.
pub fn parse_time(token: &str) -> Option<i64> {
    let t = token.trim().to_ascii_lowercase();
    let (body, pm) = if let Some(b) = t.strip_suffix("am") {
        (b.trim(), Some(false))
    } else if let Some(b) = t.strip_suffix("pm") {
        (b.trim(), Some(true))
    } else {
        (t.as_str(), None)
    };
    let (h, m) = match body.split_once(':') {
        Some((hh, mm)) => (
            hh.trim().parse::<i64>().ok()?,
            mm.trim().parse::<i64>().ok()?,
        ),
        None => {
            pm?;
            (body.parse::<i64>().ok()?, 0)
        }
    };
    if !(0..=59).contains(&m) {
        return None;
    }
    let h = match pm {
        Some(is_pm) => {
            if !(1..=12).contains(&h) {
                return None;
            }
            let base = if h == 12 { 0 } else { h };
            if is_pm {
                base + 12
            } else {
                base
            }
        }
        None => {
            if !(0..=23).contains(&h) {
                return None;
            }
            h
        }
    };
    Some(h * 60 + m)
}

/// Human duration for tracked time: "2h 5m", "45m", "30s"; "" for under a second.
pub fn fmt_duration(ms: i64) -> String {
    if ms < 1000 {
        return String::new();
    }
    let secs = ms / 1000;
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    if h > 0 {
        format!("{h}h {m}m")
    } else if m > 0 {
        format!("{m}m")
    } else {
        format!("{s}s")
    }
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

    #[test]
    fn days_from_civil_roundtrips() {
        assert_eq!(super::days_from_civil(1970, 1, 1), 0);
        for z in [-1000i64, 0, 1, 12000, 20000] {
            let (y, m, d) = super::civil_from_days(z);
            assert_eq!(super::days_from_civil(y, m as i64, d as i64), z);
        }
    }

    #[test]
    fn parse_due_relative_and_absolute() {
        const DAY: i64 = 86_400_000;
        let now = 20_000 * DAY + 5_000; // arbitrary mid-day
        assert_eq!(super::parse_due("today", now), Some(20_000 * DAY));
        assert_eq!(super::parse_due("tomorrow", now), Some(20_001 * DAY));
        assert_eq!(
            super::parse_due("2026-09-10", now),
            Some(super::days_from_civil(2026, 9, 10) * DAY)
        );
        assert!(super::parse_due("notadate", now).is_none());
    }

    #[test]
    fn parse_time_formats() {
        use super::parse_time;
        assert_eq!(parse_time("14:00"), Some(840));
        assert_eq!(parse_time("9am"), Some(540));
        assert_eq!(parse_time("5pm"), Some(1020));
        assert_eq!(parse_time("9:30am"), Some(570));
        assert_eq!(parse_time("12am"), Some(0));
        assert_eq!(parse_time("12pm"), Some(720));
        assert_eq!(parse_time("5"), None);
        assert_eq!(parse_time("24:00"), None);
        assert_eq!(parse_time("banana"), None);
    }

    #[test]
    fn fmt_duration_units() {
        use super::fmt_duration;
        assert_eq!(fmt_duration(0), "");
        assert_eq!(fmt_duration(500), "");
        assert_eq!(fmt_duration(30_000), "30s");
        assert_eq!(fmt_duration(90_000), "1m");
        assert_eq!(fmt_duration(3_600_000 + 300_000), "1h 5m");
    }
}
