//! Every content kind is classified and stored correctly, and is findable via a
//! kind-filtered search. Guards the "copied X shows up as X" behavior.

use magpie_core::{default_query, open_in_memory, CaptureEvent, Content, ImageStore, Kind, Store};

struct FakeImages;
impl ImageStore for FakeImages {
    fn put(&self, hash: &str, _b: &[u8]) -> std::io::Result<String> {
        Ok(format!("/img/{hash}.png"))
    }
}

fn ingest(store: &Store, content: Content, ms: i64) -> i64 {
    store
        .ingest(
            &CaptureEvent {
                content,
                source_app: None,
                copied_at_ms: ms,
            },
            &FakeImages,
        )
        .unwrap()
        .entry_id
}

fn kind_of(store: &Store, id: i64) -> Kind {
    store
        .recent(1000)
        .unwrap()
        .into_iter()
        .find(|e| e.id == id)
        .unwrap()
        .kind
}

#[test]
fn each_content_kind_is_classified() {
    let s = open_in_memory().unwrap();
    let text = ingest(&s, Content::Text("just some words".into()), 1);
    let link = ingest(&s, Content::Text("https://example.com/a".into()), 2);
    let email = ingest(&s, Content::Text("me@example.io".into()), 3);
    let color = ingest(&s, Content::Text("#1a2b3c".into()), 4);
    let rgb = ingest(&s, Content::Text("rgb(10, 20, 30)".into()), 5);
    let image = ingest(
        &s,
        Content::Image {
            bytes: vec![1, 2, 3, 4],
        },
        6,
    );
    let files = ingest(
        &s,
        Content::Files(vec!["/a/one.png".into(), "/b/two.pdf".into()]),
        7,
    );

    assert_eq!(kind_of(&s, text), Kind::Text);
    assert_eq!(kind_of(&s, link), Kind::Link);
    assert_eq!(kind_of(&s, email), Kind::Email);
    assert_eq!(kind_of(&s, color), Kind::Color);
    assert_eq!(kind_of(&s, rgb), Kind::Color);
    assert_eq!(kind_of(&s, image), Kind::Image);
    assert_eq!(kind_of(&s, files), Kind::File);
}

#[test]
fn image_entry_has_image_path_and_no_text() {
    let s = open_in_memory().unwrap();
    let id = ingest(
        &s,
        Content::Image {
            bytes: vec![9, 9, 9],
        },
        1,
    );
    let e = s
        .recent(10)
        .unwrap()
        .into_iter()
        .find(|e| e.id == id)
        .unwrap();
    assert!(e.image_path.is_some(), "image entry keeps an image_path");
    assert!(e.full_text.is_empty(), "image entry has no text body");
}

#[test]
fn file_entry_stores_the_paths() {
    let s = open_in_memory().unwrap();
    let id = ingest(
        &s,
        Content::Files(vec!["/x/a.txt".into(), "/y/b.txt".into()]),
        1,
    );
    let e = s
        .recent(10)
        .unwrap()
        .into_iter()
        .find(|e| e.id == id)
        .unwrap();
    assert_eq!(e.full_text, "/x/a.txt\n/y/b.txt");
}

#[test]
fn kind_filtered_search_returns_only_that_kind() {
    let s = open_in_memory().unwrap();
    ingest(&s, Content::Text("plain".into()), 1);
    ingest(&s, Content::Text("https://x.io".into()), 2);
    let files = ingest(&s, Content::Files(vec!["/f/one".into()]), 3);
    ingest(&s, Content::Image { bytes: vec![1] }, 4);

    let mut q = default_query();
    q.kind = Some(Kind::File);
    let hits = s.search(&q).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].id, files);
    assert_eq!(hits[0].kind, Kind::File);
}
