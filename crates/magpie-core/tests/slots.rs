//! Integration tests for pinned quick-paste slots.

use magpie_core::{open_in_memory, CaptureEvent, Content, ImageStore, RetentionPolicy, Store};

struct Noop;
impl ImageStore for Noop {
    fn put(&self, h: &str, _b: &[u8]) -> std::io::Result<String> {
        Ok(h.to_string())
    }
}
fn ingest(s: &Store, t: &str, ms: i64) -> i64 {
    s.ingest(
        &CaptureEvent { content: Content::Text(t.into()), source_app: None, copied_at_ms: ms },
        &Noop,
    )
    .unwrap()
    .entry_id
}

#[test]
fn slot_entry_is_stable_as_newer_things_are_copied() {
    let s = open_in_memory().unwrap();
    let fav = ingest(&s, "my snippet", 1);
    s.assign_slot(2, fav).unwrap();
    for i in 0..10 {
        ingest(&s, &format!("noise {i}"), 100 + i);
    }
    let e = s.slot_entry(2).unwrap().unwrap();
    assert_eq!(e.full_text, "my snippet");
}

#[test]
fn slotted_entry_survives_retention_because_it_is_pinned() {
    let s = open_in_memory().unwrap();
    let fav = ingest(&s, "keep me", 1);
    s.assign_slot(1, fav).unwrap(); // pins it
    ingest(&s, "throwaway", 2);

    let policy = RetentionPolicy { max_entries: None, max_age_ms: Some(0), max_image_bytes: None };
    s.enforce_retention(&policy, 1_000_000).unwrap();

    assert!(s.slot_entry(1).unwrap().is_some());
    assert_eq!(s.slot_entry(1).unwrap().unwrap().full_text, "keep me");
}
