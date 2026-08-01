//! Edge-case and invariant tests for magpie-core (public API only).

use magpie_core::{
    default_query, open_in_memory, CaptureEvent, Content, ImageStore, Kind, SearchMode, Sort,
};

struct Noop;
impl ImageStore for Noop {
    fn put(&self, h: &str, _b: &[u8]) -> std::io::Result<String> {
        Ok(h.to_string())
    }
}
fn text(s: &str, ms: i64) -> CaptureEvent {
    CaptureEvent {
        content: Content::Text(s.into()),
        source_app: None,
        copied_at_ms: ms,
    }
}

#[test]
fn preview_is_truncated_to_200_chars_full_text_kept() {
    let s = open_in_memory().unwrap();
    let long: String = "x".repeat(500);
    s.ingest(&text(&long, 1), &Noop).unwrap();
    let e = &s.recent(1).unwrap()[0];
    assert_eq!(e.preview_text.chars().count(), 200);
    assert_eq!(e.full_text.chars().count(), 500);
    assert_eq!(e.char_count, 500);
}

#[test]
fn preview_truncation_is_unicode_safe() {
    let s = open_in_memory().unwrap();
    let long: String = "é".repeat(300); // 2 bytes each; truncation must be by char not byte
    s.ingest(&text(&long, 1), &Noop).unwrap();
    let e = &s.recent(1).unwrap()[0];
    assert_eq!(e.preview_text.chars().count(), 200);
}

#[test]
fn empty_query_returns_all_newest_first() {
    let s = open_in_memory().unwrap();
    s.ingest(&text("a", 1), &Noop).unwrap();
    s.ingest(&text("b", 2), &Noop).unwrap();
    s.ingest(&text("c", 3), &Noop).unwrap();
    let rows = s.search(&default_query()).unwrap();
    assert_eq!(
        rows.iter()
            .map(|e| e.full_text.as_str())
            .collect::<Vec<_>>(),
        vec!["c", "b", "a"]
    );
}

#[test]
fn alphabetical_sort_is_case_insensitive() {
    let s = open_in_memory().unwrap();
    s.ingest(&text("banana", 1), &Noop).unwrap();
    s.ingest(&text("Apple", 2), &Noop).unwrap();
    s.ingest(&text("cherry", 3), &Noop).unwrap();
    let mut q = default_query();
    q.sort = Sort::Alphabetical;
    let rows = s.search(&q).unwrap();
    assert_eq!(
        rows.iter()
            .map(|e| e.full_text.as_str())
            .collect::<Vec<_>>(),
        vec!["Apple", "banana", "cherry"]
    );
}

#[test]
fn rich_html_content_gets_html_kind_and_is_searchable_by_text() {
    let s = open_in_memory().unwrap();
    s.ingest(
        &CaptureEvent {
            content: Content::Rich {
                text: "the quarterly report".into(),
                html: Some("<p>the quarterly report</p>".into()),
                rtf: None,
            },
            source_app: None,
            copied_at_ms: 1,
        },
        &Noop,
    )
    .unwrap();

    let mut byhtml = default_query();
    byhtml.kind = Some(Kind::Html);
    assert_eq!(s.search(&byhtml).unwrap().len(), 1);

    let mut byword = default_query();
    byword.text = "quarterly".into();
    let rows = s.search(&byword).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].full_text, "the quarterly report");
}

#[test]
fn files_content_dedups_and_is_searchable() {
    let s = open_in_memory().unwrap();
    let files = Content::Files(vec!["/proj/a/b.txt".into(), "/proj/c/d.rs".into()]);
    s.ingest(
        &CaptureEvent {
            content: files.clone(),
            source_app: None,
            copied_at_ms: 1,
        },
        &Noop,
    )
    .unwrap();
    s.ingest(
        &CaptureEvent {
            content: files,
            source_app: None,
            copied_at_ms: 2,
        },
        &Noop,
    )
    .unwrap();

    let mut byfile = default_query();
    byfile.kind = Some(Kind::File);
    let rows = s.search(&byfile).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].copy_count, 2);
    assert_eq!(rows[0].full_text, "/proj/a/b.txt\n/proj/c/d.rs");
    assert_eq!(rows[0].line_count, 2);

    let mut search = default_query();
    search.text = "d.rs".into();
    assert_eq!(s.search(&search).unwrap().len(), 1);
}

#[test]
fn whitespace_only_text_is_captured_with_zero_words() {
    let s = open_in_memory().unwrap();
    s.ingest(&text("   \t  ", 1), &Noop).unwrap();
    let e = &s.recent(1).unwrap()[0];
    assert_eq!(e.word_count, 0);
    assert!(e.char_count > 0);
    assert_eq!(e.kind, Kind::Text);
}

#[test]
fn dedup_invariant_unique_contents_equal_entry_count() {
    let s = open_in_memory().unwrap();
    // 3 distinct contents, ingested with duplicates interleaved.
    let script = ["red", "green", "red", "blue", "green", "red", "blue"];
    for (i, c) in script.iter().enumerate() {
        s.ingest(&text(c, i as i64 + 1), &Noop).unwrap();
    }
    let rows = s.search(&default_query()).unwrap();
    // unique entries == distinct strings
    assert_eq!(rows.len(), 3);
    // sum of copy_count == total ingests
    let total: i64 = rows.iter().map(|e| e.copy_count).sum();
    assert_eq!(total, script.len() as i64);
    // red appeared 3x, green 2x, blue 2x
    let red = rows.iter().find(|e| e.full_text == "red").unwrap();
    assert_eq!(red.copy_count, 3);
}

#[test]
fn fuzzy_on_empty_query_falls_back_to_all() {
    // Fuzzy/regex are only dispatched when text is non-empty; empty text lists all.
    let s = open_in_memory().unwrap();
    s.ingest(&text("anything", 1), &Noop).unwrap();
    let mut q = default_query();
    q.mode = SearchMode::Fuzzy;
    q.text = "".into();
    assert_eq!(s.search(&q).unwrap().len(), 1);
}
