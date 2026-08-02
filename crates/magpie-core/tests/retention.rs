//! Integration tests for retention enforcement.

use magpie_core::{
    default_query, open_in_memory, CaptureEvent, Content, ImageStore, RetentionPolicy,
};

struct Noop;
impl ImageStore for Noop {
    fn put(&self, h: &str, _b: &[u8]) -> std::io::Result<String> {
        Ok(h.to_string())
    }
}
fn text(s: &str, ms: i64) -> CaptureEvent {
    CaptureEvent { content: Content::Text(s.into()), source_app: None, copied_at_ms: ms }
}

#[test]
fn deletion_cascades_to_copy_events_and_search() {
    let s = open_in_memory().unwrap();
    s.ingest(&text("gone", 1), &Noop).unwrap();
    s.ingest(&text("gone", 2), &Noop).unwrap(); // dup -> 2 copy_events
    s.ingest(&text("kept", 9_500), &Noop).unwrap();

    let policy = RetentionPolicy { max_entries: None, max_age_ms: Some(1_000), max_image_bytes: None };
    let r = s.enforce_retention(&policy, 10_000).unwrap();
    assert_eq!(r.entries_deleted, 1);

    let remaining: Vec<String> = s.recent(100).unwrap().into_iter().map(|e| e.full_text).collect();
    assert_eq!(remaining, vec!["kept".to_string()]);

    // FTS search no longer finds the deleted content...
    let mut q = default_query();
    q.text = "gone".into();
    assert_eq!(s.search(&q).unwrap().len(), 0);
    // ...and the surviving entry is still searchable (index consistent).
    q.text = "kept".into();
    assert_eq!(s.search(&q).unwrap().len(), 1);
}

#[test]
fn combined_policy_unions_victims() {
    let s = open_in_memory().unwrap();
    s.ingest(&text("old", 1), &Noop).unwrap();
    for i in 0..5 {
        s.ingest(&text(&format!("r{i}"), 10_000 + i), &Noop).unwrap();
    }
    // now=2_000, age=1_000 -> cutoff 1_000: deletes "old"(1); the recents (10_000+)
    // survive age, and count keeps only the newest 2 -> deletes 3 recents.
    let policy = RetentionPolicy { max_entries: Some(2), max_age_ms: Some(1_000), max_image_bytes: None };
    let r = s.enforce_retention(&policy, 2_000).unwrap();
    assert_eq!(r.entries_deleted, 4);
    assert_eq!(s.recent(100).unwrap().len(), 2);
}
