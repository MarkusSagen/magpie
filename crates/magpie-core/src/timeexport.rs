//! Time-tracking views over `time_entries` (Phase 17/18b leftovers): which entries
//! belong to a project, per-project totals, and a CSV export. Pure.
//!
//! A time entry belongs to a project when its task title carries `#proj/<slug>` (the
//! tag the Projects view shows) or `#<slug>` — the same "first #tag = project"
//! convention the Time tab's This-week card uses. No schema change.

use crate::notes::TimeEntry;
use crate::projects::slugify;

/// The `#tags` in a task title, lowercased and without the `#`.
pub fn entry_tags(title: &str) -> Vec<String> {
    title
        .split_whitespace()
        .filter_map(|w| w.strip_prefix('#'))
        .map(|t| {
            t.trim_end_matches([',', '.', ';', ':'])
                .to_ascii_lowercase()
        })
        .filter(|t| !t.is_empty())
        .collect()
}

/// Whether an entry titled `title` counts toward project `project_name`.
pub fn belongs_to_project(title: &str, project_name: &str) -> bool {
    let slug = slugify(project_name);
    if slug.is_empty() {
        return false;
    }
    let proj = format!("proj/{slug}");
    entry_tags(title).iter().any(|t| *t == proj || *t == slug)
}

/// Tracked ms for `project_name` over entries starting at or after `since_ms`; a
/// running entry counts up to `now_ms`.
pub fn project_ms(entries: &[TimeEntry], project_name: &str, since_ms: i64, now_ms: i64) -> i64 {
    entries
        .iter()
        .filter(|e| e.start_ms >= since_ms && belongs_to_project(&e.task_title, project_name))
        .map(|e| (e.end_ms.unwrap_or(now_ms) - e.start_ms).max(0))
        .sum()
}

/// `11h 20m`, `45m`, `0m`.
pub fn fmt_hm(ms: i64) -> String {
    let m = ms.max(0) / 60_000;
    if m >= 60 {
        format!("{}h {:02}m", m / 60, m % 60)
    } else {
        format!("{m}m")
    }
}

fn iso_utc(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|d| d.format("%Y-%m-%dT%H:%M:%SZ").to_string())
        .unwrap_or_default()
}

/// RFC 4180 field: quoted when it contains a comma, quote or line break.
fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// All `entries` as CSV: `start_utc,end_utc,duration_min,description,project,tags`.
/// Project = the first #tag (a `proj/` prefix dropped), tags = the rest. A running
/// entry has an empty `end_utc` and its duration measured to `now_ms`.
pub fn to_csv(entries: &[TimeEntry], now_ms: i64) -> String {
    let mut out = String::from("start_utc,end_utc,duration_min,description,project,tags\n");
    for e in entries {
        let tags = entry_tags(&e.task_title);
        let project = tags
            .first()
            .map(|t| t.strip_prefix("proj/").unwrap_or(t).to_string())
            .unwrap_or_default();
        let rest = tags.iter().skip(1).cloned().collect::<Vec<_>>().join(" ");
        let mins = (e.end_ms.unwrap_or(now_ms) - e.start_ms).max(0) as f64 / 60_000.0;
        let row = [
            iso_utc(e.start_ms),
            e.end_ms.map(iso_utc).unwrap_or_default(),
            format!("{mins:.1}"),
            csv_field(&e.task_title),
            csv_field(&project),
            csv_field(&rest),
        ];
        out.push_str(&row.join(","));
        out.push('\n');
    }
    out
}

/// A manually entered time span (Time tab quick-add).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManualEntry {
    pub title: String,
    pub start_ms: i64,
    pub end_ms: i64,
}

const MIN_MS: i64 = 60_000;
const DAY_MS: i64 = 86_400_000;

/// `1h30m` / `90m` / `1.5h` / `2h` → minutes.
fn duration_token(t: &str) -> Option<f64> {
    let t = t.to_ascii_lowercase();
    if let Some((h, m)) = t.split_once('h') {
        if !m.is_empty() {
            let m = m.strip_suffix('m')?;
            return Some(h.parse::<f64>().ok()? * 60.0 + m.parse::<f64>().ok()?);
        }
        return Some(h.parse::<f64>().ok()? * 60.0);
    }
    t.strip_suffix('m')?.parse::<f64>().ok()
}

/// `9:00` → minutes after midnight.
fn clock_token(t: &str) -> Option<i64> {
    let (h, m) = t.split_once(':')?;
    let (h, m) = (h.parse::<i64>().ok()?, m.parse::<i64>().ok()?);
    ((0..24).contains(&h) && (0..60).contains(&m)).then_some(h * 60 + m)
}

/// Days since 1970-01-01 for a proleptic Gregorian date (Hinnant's algorithm).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Inverse of [`days_from_civil`].
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// `2026-11-10` → local day number.
fn parse_ymd(t: &str) -> Option<i64> {
    let mut it = t.split('-');
    let (y, m, d) = (
        it.next()?.parse().ok()?,
        it.next()?.parse().ok()?,
        it.next()?.parse().ok()?,
    );
    if it.next().is_some() || !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let day = days_from_civil(y, m, d);
    // Reject impossible dates like 2026-02-31 (they'd roll into March).
    (civil_from_days(day) == (y, m, d)).then_some(day)
}

/// Render an entry back into quick-add syntax (for editing): same local day as now →
/// `title 9:00-10:30`; yesterday → `title yesterday 9:00-10:30`; older →
/// `title 2026-11-10 9:00-10:30`. Times are local (`offset_min`).
pub fn format_manual_entry(
    title: &str,
    start_ms: i64,
    end_ms: i64,
    now_ms: i64,
    offset_min: i64,
) -> String {
    let off = offset_min * MIN_MS;
    let day = (start_ms + off).div_euclid(DAY_MS);
    let today = (now_ms + off).div_euclid(DAY_MS);
    let clock = |ms: i64| {
        let mins = (ms + off - day * DAY_MS)
            .div_euclid(MIN_MS)
            .clamp(0, 24 * 60 - 1);
        format!("{}:{:02}", mins / 60, mins % 60)
    };
    let when = if day == today {
        String::new()
    } else if day == today - 1 {
        "yesterday ".into()
    } else {
        let (y, m, d) = civil_from_days(day);
        format!("{y:04}-{m:02}-{d:02} ")
    };
    format!("{title} {when}{}-{}", clock(start_ms), clock(end_ms))
}

/// Parse a Time-tab quick-add: `<description> <when>`, where `<when>` is a trailing
/// duration ending now (`1h30m`, `90m`, `1.5h`, `1h 30m`) or a clock range today
/// (`9:00-10:30`), optionally prefixed `yesterday` / `today`. `offset_min` is the
/// local UTC offset, used to place clock ranges on the local day.
pub fn parse_manual_entry(
    input: &str,
    now_ms: i64,
    offset_min: i64,
) -> Result<ManualEntry, String> {
    let mut toks: Vec<&str> = input.split_whitespace().collect();
    let last = *toks
        .last()
        .ok_or("type a description and a time, e.g. \"Review PR 45m\"")?;
    let (start_ms, end_ms) = if let Some((a, b)) = last.split_once(['-', '–']) {
        let (Some(a), Some(b)) = (clock_token(a), clock_token(b)) else {
            return Err(format!("couldn't read the time range \"{last}\""));
        };
        toks.pop();
        let off = offset_min * MIN_MS;
        let today = (now_ms + off).div_euclid(DAY_MS);
        let mut day = today;
        match toks.last().map(|t| t.to_ascii_lowercase()) {
            Some(t) if t == "yesterday" => {
                day = today - 1;
                toks.pop();
            }
            Some(t) if t == "today" => {
                toks.pop();
            }
            Some(t) => {
                if let Some(d) = parse_ymd(&t) {
                    day = d;
                    toks.pop();
                }
            }
            None => {}
        }
        let local_midnight = day * DAY_MS;
        let start = local_midnight + a * MIN_MS - off;
        let end = local_midnight + b * MIN_MS - off;
        if end <= start {
            return Err("the end time must be after the start".into());
        }
        (start, end)
    } else {
        let mut mins = 0.0;
        let mut any = false;
        while let Some(t) = toks.last() {
            match duration_token(t) {
                Some(m) => {
                    mins += m;
                    any = true;
                    toks.pop();
                }
                None => break,
            }
        }
        if !any {
            return Err("end with a duration (45m, 1h30m) or a range (9:00-10:30)".into());
        }
        let dur = (mins * MIN_MS as f64).round() as i64;
        if dur <= 0 {
            return Err("the duration must be more than zero".into());
        }
        (now_ms - dur, now_ms)
    };
    if end_ms > now_ms {
        return Err("that ends in the future".into());
    }
    if end_ms - start_ms > DAY_MS {
        return Err("an entry can't be longer than 24 hours".into());
    }
    let title = toks.join(" ");
    if title.is_empty() {
        return Err("add a description before the time".into());
    }
    Ok(ManualEntry {
        title,
        start_ms,
        end_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(title: &str, start: i64, end: Option<i64>) -> TimeEntry {
        TimeEntry {
            id: 0,
            task_title: title.into(),
            start_ms: start,
            end_ms: end,
        }
    }

    #[test]
    fn project_rule_table() {
        for (title, want) in [
            ("Fix sync #proj/magpie", true),
            ("Fix sync #magpie #ui", true),
            ("Fix sync #MAGPIE", true),
            ("Fix sync #proj/magpie-web", false),
            ("Fix sync #ui", false),
            ("magpie without a tag", false),
        ] {
            assert_eq!(belongs_to_project(title, "Magpie"), want, "{title}");
        }
        assert!(belongs_to_project(
            "x #proj/home-renovation",
            "Home renovation"
        ));
        assert!(!belongs_to_project("x #proj/", ""));
    }

    #[test]
    fn project_ms_window_and_running_clip() {
        let h = 3_600_000;
        let entries = [
            e("a #magpie", 10 * h, Some(12 * h)), // 2h, in window
            e("b #magpie", h, Some(5 * h)),       // before window
            e("c #other", 11 * h, Some(13 * h)),  // other project
            e("d #proj/magpie", 14 * h, None),    // running → clipped to now
        ];
        let ms = project_ms(&entries, "magpie", 9 * h, 15 * h);
        assert_eq!(ms, 3 * h);
        assert_eq!(fmt_hm(ms + 20 * 60_000), "3h 20m");
        assert_eq!(fmt_hm(45 * 60_000), "45m");
    }

    #[test]
    fn csv_header_quoting_and_running() {
        let entries = [
            e(
                "Write \"spec\", v2 #proj/magpie #docs",
                0,
                Some(90 * 60_000),
            ),
            e("Live #blog", 60_000, None),
        ];
        let csv = to_csv(&entries, 31 * 60_000);
        let lines: Vec<&str> = csv.lines().collect();
        assert_eq!(
            lines[0],
            "start_utc,end_utc,duration_min,description,project,tags"
        );
        assert_eq!(
            lines[1],
            "1970-01-01T00:00:00Z,1970-01-01T01:30:00Z,90.0,\"Write \"\"spec\"\", v2 #proj/magpie #docs\",magpie,docs"
        );
        assert_eq!(lines[2], "1970-01-01T00:01:00Z,,30.0,Live #blog,blog,");
    }

    // now = 1970-01-02 12:00 UTC; local offset +60 (CET) → local 13:00.
    const NOW: i64 = DAY_MS + 12 * 3_600_000;

    #[test]
    fn manual_durations_end_now() {
        let m = parse_manual_entry("Review PR #magpie 1h30m", NOW, 60).unwrap();
        assert_eq!(m.title, "Review PR #magpie");
        assert_eq!((m.start_ms, m.end_ms), (NOW - 90 * MIN_MS, NOW));
        for (inp, mins) in [
            ("x 90m", 90),
            ("x 1.5h", 90),
            ("x 1h 30m", 90),
            ("x 2h", 120),
            ("x 45m", 45),
        ] {
            let m = parse_manual_entry(inp, NOW, 0).unwrap();
            assert_eq!(m.end_ms - m.start_ms, mins * MIN_MS, "{inp}");
        }
    }

    #[test]
    fn manual_clock_ranges_use_local_day() {
        // Local (CET) 9:00-10:30 today = 08:00-09:30 UTC on day 1.
        let m = parse_manual_entry("Standup 9:00-10:30", NOW, 60).unwrap();
        assert_eq!(m.start_ms, DAY_MS + 8 * 3_600_000);
        assert_eq!(m.end_ms, DAY_MS + 9 * 3_600_000 + 30 * MIN_MS);
        let y = parse_manual_entry("Deep work yesterday 14:00-15:00", NOW, 60).unwrap();
        assert_eq!(y.title, "Deep work");
        assert_eq!(y.start_ms, 13 * 3_600_000);
        let t = parse_manual_entry("Lunch today 11:00–11:45", NOW, 60).unwrap();
        assert_eq!(t.title, "Lunch");
    }

    #[test]
    fn manual_entry_errors() {
        for inp in [
            "",
            "Just words",
            "45m",
            "x 10:00-9:00",
            "x 14:00-15:00", // ends after local 13:00 now → future
            "x 25h",
            "x 9:99-10:00",
            "x 0m",
        ] {
            assert!(parse_manual_entry(inp, NOW, 60).is_err(), "{inp:?}");
        }
    }

    #[test]
    fn civil_dates_round_trip() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2026, 11, 14), 20_771);
        for d in [-1000, 0, 59, 60, 20_771, 40_000] {
            let (y, m, dd) = civil_from_days(d);
            assert_eq!(days_from_civil(y, m, dd), d);
        }
        assert!(parse_ymd("2026-02-31").is_none());
        assert!(parse_ymd("2026-13-01").is_none());
        assert_eq!(parse_ymd("2024-02-29"), Some(days_from_civil(2024, 2, 29)));
    }

    #[test]
    fn manual_entry_explicit_date() {
        // NOW = day 1 (1970-01-02) 13:00 CET; 1970-01-01 is yesterday, 1969-12-31 older.
        let m = parse_manual_entry("Old task 1969-12-31 9:00-10:00", NOW, 60).unwrap();
        assert_eq!(m.title, "Old task");
        assert_eq!(m.start_ms, -DAY_MS + 8 * 3_600_000);
    }

    #[test]
    fn format_round_trips_through_parse() {
        for (start, end) in [
            (DAY_MS + 8 * 3_600_000, DAY_MS + 9 * 3_600_000 + 30 * MIN_MS), // today
            (13 * 3_600_000, 14 * 3_600_000),                               // yesterday
            (-DAY_MS + 8 * 3_600_000, -DAY_MS + 10 * 3_600_000),            // older
        ] {
            let text = format_manual_entry("Review PR #magpie", start, end, NOW, 60);
            let back = parse_manual_entry(&text, NOW, 60).unwrap();
            assert_eq!(
                (back.title.as_str(), back.start_ms, back.end_ms),
                ("Review PR #magpie", start, end),
                "{text}"
            );
        }
        assert_eq!(
            format_manual_entry("x", DAY_MS + 8 * 3_600_000, DAY_MS + 9 * 3_600_000, NOW, 60),
            "x 9:00-10:00"
        );
        assert!(
            format_manual_entry("x", 13 * 3_600_000, 14 * 3_600_000, NOW, 60).contains("yesterday")
        );
        assert!(format_manual_entry(
            "x",
            -DAY_MS + 8 * 3_600_000,
            -DAY_MS + 9 * 3_600_000,
            NOW,
            60
        )
        .contains("1969-12-31"));
    }
}
