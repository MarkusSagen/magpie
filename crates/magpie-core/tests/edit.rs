//! Integration tests for editable items + snippets.

use magpie_core::{
    default_query, open_in_memory, CaptureEvent, Content, ImageStore, RetentionPolicy, Store,
};

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
fn edited_entry_is_searchable_by_new_text_and_slottable() {
    let s = open_in_memory().unwrap();
    let id = ingest(&s, "typpo here", 1);
    assert!(s.update_entry_text(id, "typo fixed here", 2).unwrap());

    let mut q = default_query();
    q.text = "fixed".into();
    let hits = s.search(&q).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].id, id);

    s.assign_slot(4, id).unwrap();
    assert_eq!(s.slot_entry(4).unwrap().unwrap().full_text, "typo fixed here");
}

#[test]
fn snippet_is_pinned_and_survives_retention() {
    let s = open_in_memory().unwrap();
    let snip = s.create_snippet("my reusable template", 1).unwrap();
    ingest(&s, "junk", 2);

    let policy = RetentionPolicy { max_entries: None, max_age_ms: Some(0), max_image_bytes: None };
    s.enforce_retention(&policy, 1_000_000).unwrap();

    let remaining: Vec<i64> = s.recent(100).unwrap().into_iter().map(|e| e.id).collect();
    assert!(remaining.contains(&snip));
    let mut q = default_query();
    q.text = "reusable".into();
    assert_eq!(s.search(&q).unwrap().len(), 1);
}
