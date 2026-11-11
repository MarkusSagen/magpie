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
}
