//! Integration tests for tags.

use magpie_core::{default_query, open_in_memory, CaptureEvent, Content, ImageStore, Store};

struct Noop;
impl ImageStore for Noop {
    fn put(&self, h: &str, _b: &[u8]) -> std::io::Result<String> {
        Ok(h.to_string())
    }
}
fn ingest(s: &Store, t: &str) -> i64 {
    s.ingest(
        &CaptureEvent {
            content: Content::Text(t.into()),
            source_app: None,
            copied_at_ms: 1,
        },
        &Noop,
    )
    .unwrap()
    .entry_id
}

#[test]
fn tag_filter_combines_with_text_search() {
    let s = open_in_memory().unwrap();
    let a = ingest(&s, "alpha note");
    let b = ingest(&s, "alpha memo");
    ingest(&s, "beta note");
    s.add_tag(a, "work").unwrap();
    s.add_tag(b, "home").unwrap();

    let mut q = default_query();
    q.text = "alpha".into();
    q.tag = Some("work".into());
    let rows = s.search(&q).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, a);
}

#[test]
fn all_tags_reflects_adds_and_removes() {
    let s = open_in_memory().unwrap();
    let id = ingest(&s, "x");
    s.add_tag(id, "one").unwrap();
    s.add_tag(id, "two").unwrap();
    assert_eq!(
        s.all_tags().unwrap(),
        vec!["one".to_string(), "two".to_string()]
    );
    s.remove_tag(id, "one").unwrap();
    assert_eq!(s.all_tags().unwrap(), vec!["two".to_string()]);
}
