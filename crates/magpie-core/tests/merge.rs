//! Integration test for merged_text.

use magpie_core::{open_in_memory, CaptureEvent, Content, ImageStore, Store};

struct Noop;
impl ImageStore for Noop {
    fn put(&self, h: &str, _b: &[u8]) -> std::io::Result<String> {
        Ok(h.to_string())
    }
}
fn ingest(s: &Store, t: &str) -> i64 {
    s.ingest(
        &CaptureEvent { content: Content::Text(t.into()), source_app: None, copied_at_ms: 1 },
        &Noop,
    )
    .unwrap()
    .entry_id
}

#[test]
fn merge_a_realistic_selection() {
    let s = open_in_memory().unwrap();
    let a = ingest(&s, "- item one");
    let b = ingest(&s, "- item two");
    let c = ingest(&s, "- item three");
    let merged = s.merged_text(&[a, b, c], "\n").unwrap();
    assert_eq!(merged, "- item one\n- item two\n- item three");
}
