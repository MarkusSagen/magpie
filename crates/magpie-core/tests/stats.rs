//! Integration tests for Store::stats over a seeded session.

use magpie_core::{open_in_memory, AppInfo, CaptureEvent, Content, ImageStore, StatsRange};

struct Noop;
impl ImageStore for Noop {
    fn put(&self, h: &str, _b: &[u8]) -> std::io::Result<String> {
        Ok(h.to_string())
    }
}
fn ev(text: &str, ms: i64, app: Option<&str>) -> CaptureEvent {
    CaptureEvent {
        content: Content::Text(text.into()),
        source_app: app.map(|a| AppInfo {
            identifier: a.into(),
            display_name: a.into(),
            icon_path: None,
        }),
        copied_at_ms: ms,
    }
}
const DAY: i64 = 86_400_000;

#[test]
fn realistic_session_stats() {
    let s = open_in_memory().unwrap();
    for e in [
        ev("docs/design.md", 100 * DAY + 1, Some("Ghostty")),
        ev("docs/design.md", 100 * DAY + 2, Some("Ghostty")), // re-copy
        ev("https://asana.com/x", 100 * DAY + 3, Some("Vivaldi")),
        ev("#1a2b3c", 101 * DAY + 4, Some("Figma")),
        ev("some note", 101 * DAY + 5, None),
    ] {
        s.ingest(&e, &Noop).unwrap();
    }
    let range = StatsRange { since_ms: Some(100 * DAY), now_ms: 101 * DAY + 100 };
    let st = s.stats(&range, 10).unwrap();

    assert_eq!(st.totals.copies, 5);
    assert_eq!(st.totals.unique_entries, 4);
    assert_eq!(st.totals.distinct_apps, 3);

    assert_eq!(st.most_copied[0].preview, "docs/design.md");
    assert_eq!(st.most_copied[0].count, 2);

    assert_eq!(st.per_app[0].name, "Ghostty"); // 2 copies
    assert!(st.by_type.iter().any(|k| k.kind.as_str() == "link"));
    assert!(st.by_type.iter().any(|k| k.kind.as_str() == "color"));

    assert_eq!(st.over_time.len(), 2); // day 100 and 101
}

#[test]
fn over_time_buckets_partition_the_copies() {
    let s = open_in_memory().unwrap();
    for i in 0..40 {
        s.ingest(
            &ev(&format!("c{}", i % 5), (10 * DAY) + i * 3_600_000, None),
            &Noop,
        )
        .unwrap();
    }
    let range = StatsRange { since_ms: Some(9 * DAY), now_ms: 12 * DAY };
    let st = s.stats(&range, 10).unwrap();
    let bucket_sum: i64 = st.over_time.iter().map(|b| b.count).sum();
    assert_eq!(bucket_sum, st.totals.copies);
}

#[test]
fn empty_db_yields_zeroes() {
    let s = open_in_memory().unwrap();
    let st = s
        .stats(&StatsRange { since_ms: None, now_ms: 1000 }, 10)
        .unwrap();
    assert_eq!(st.totals.copies, 0);
    assert!(st.most_copied.is_empty());
    assert!(st.per_app.is_empty());
    assert!(st.by_type.is_empty());
}
