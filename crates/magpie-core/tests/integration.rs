//! End-to-end integration tests for `magpie-core` exercised only through the
//! public API: capture -> dedup/store -> search/filter/sort.

use magpie_core::{
    default_query, open, open_in_memory, AppInfo, CaptureEvent, Content, ImageStore, Kind,
    SearchMode, Sort, Store,
};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

// ---- helpers ------------------------------------------------------------

/// A temp-dir-backed ImageStore that also counts writes (to prove dedup).
struct TempImages {
    dir: PathBuf,
    writes: AtomicUsize,
}
impl TempImages {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("magpie-core-it-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        TempImages {
            dir,
            writes: AtomicUsize::new(0),
        }
    }
    fn writes(&self) -> usize {
        self.writes.load(Ordering::SeqCst)
    }
}
impl Drop for TempImages {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.dir).ok();
    }
}
impl ImageStore for TempImages {
    fn put(&self, hash: &str, bytes: &[u8]) -> std::io::Result<String> {
        let path = self.dir.join(format!("{hash}.bin"));
        if !path.exists() {
            self.writes.fetch_add(1, Ordering::SeqCst);
            std::fs::write(&path, bytes)?;
        }
        Ok(path.to_string_lossy().into_owned())
    }
}

struct Noop;
impl ImageStore for Noop {
    fn put(&self, hash: &str, _b: &[u8]) -> std::io::Result<String> {
        Ok(hash.to_string())
    }
}

fn text(s: &str, ms: i64) -> CaptureEvent {
    CaptureEvent {
        content: Content::Text(s.into()),
        source_app: None,
        copied_at_ms: ms,
    }
}
fn text_from(s: &str, ms: i64, app: &str) -> CaptureEvent {
    CaptureEvent {
        content: Content::Text(s.into()),
        source_app: Some(AppInfo {
            identifier: app.into(),
            display_name: app.into(),
            icon_path: None,
        }),
        copied_at_ms: ms,
    }
}
fn ingest_all(store: &Store, evs: &[CaptureEvent]) {
    for ev in evs {
        store.ingest(ev, &Noop).unwrap();
    }
}

// ---- lifecycle / dedup / events ----------------------------------------

#[test]
fn full_lifecycle_dedup_and_event_log() {
    let s = open_in_memory().unwrap();
    // Same content copied three times from two apps; a distinct second entry once.
    s.ingest(&text_from("hello", 100, "com.ghostty"), &Noop)
        .unwrap();
    s.ingest(&text_from("hello", 200, "com.safari"), &Noop)
        .unwrap();
    s.ingest(&text_from("hello", 300, "com.ghostty"), &Noop)
        .unwrap();
    s.ingest(&text("world", 400), &Noop).unwrap();

    let rows = s.recent(10).unwrap();
    assert_eq!(rows.len(), 2, "identical content dedups to one entry");

    let hello = rows.iter().find(|e| e.full_text == "hello").unwrap();
    assert_eq!(hello.copy_count, 3, "three copies of the same content");
    assert_eq!(hello.first_copied_at_ms, 100, "first-copied is preserved");
    assert_eq!(hello.last_copied_at_ms, 300, "last-copied advances");
    // source app of the most recent copy is retained on the entry
    assert!(hello.source_app_id.is_some());
}

#[test]
fn most_copied_sort_orders_by_frequency() {
    let s = open_in_memory().unwrap();
    ingest_all(
        &s,
        &[
            text("once", 1),
            text("thrice", 2),
            text("thrice", 3),
            text("thrice", 4),
            text("twice", 5),
            text("twice", 6),
        ],
    );
    let mut q = default_query();
    q.sort = Sort::MostCopied;
    let rows = s.search(&q).unwrap();
    assert_eq!(rows[0].full_text, "thrice");
    assert_eq!(rows[0].copy_count, 3);
    assert_eq!(rows[1].full_text, "twice");
    assert_eq!(rows[1].copy_count, 2);
}

// ---- filters ------------------------------------------------------------

#[test]
fn filter_by_source_app() {
    let s = open_in_memory().unwrap();
    ingest_all(
        &s,
        &[
            text_from("from ghostty a", 1, "com.ghostty"),
            text_from("from ghostty b", 2, "com.ghostty"),
            text_from("from safari", 3, "com.safari"),
        ],
    );
    let ghostty: i64 = s
        .search(&default_query())
        .unwrap()
        .iter()
        .find(|e| e.full_text.contains("ghostty"))
        .and_then(|e| e.source_app_id)
        .unwrap();
    let mut q = default_query();
    q.source_app_id = Some(ghostty);
    let rows = s.search(&q).unwrap();
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|e| e.full_text.contains("ghostty")));
}

#[test]
fn filter_by_time_range_since_and_until() {
    let s = open_in_memory().unwrap();
    ingest_all(&s, &[text("t1", 1000), text("t2", 2000), text("t3", 3000)]);

    let mut since = default_query();
    since.time.since_ms = Some(2000);
    let r = since.clone();
    assert_eq!(s.search(&r).unwrap().len(), 2);

    let mut window = default_query();
    window.time.since_ms = Some(1500);
    window.time.until_ms = Some(2500);
    let rows = s.search(&window).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].full_text, "t2");
}

#[test]
fn content_types_detected_persisted_and_filterable() {
    let s = open_in_memory().unwrap();
    ingest_all(
        &s,
        &[
            text("plain words here", 1),
            text("https://example.com/x", 2),
            text("me@example.io", 3),
            text("#1a2b3c", 4),
        ],
    );
    s.ingest(
        &CaptureEvent {
            content: Content::Files(vec!["/a".into(), "/b".into()]),
            source_app: None,
            copied_at_ms: 5,
        },
        &Noop,
    )
    .unwrap();

    let check = |kind: Kind, expect: usize| {
        let mut q = default_query();
        q.kind = Some(kind);
        assert_eq!(
            s.search(&q).unwrap().len(),
            expect,
            "kind {:?}",
            kind.as_str()
        );
    };
    check(Kind::Text, 1);
    check(Kind::Link, 1);
    check(Kind::Email, 1);
    check(Kind::Color, 1);
    check(Kind::File, 1);
}

// ---- images -------------------------------------------------------------

#[test]
fn image_ingestion_dedups_and_stores_path() {
    let images = TempImages::new("img");
    let s = open_in_memory().unwrap();
    let png = Content::Image {
        bytes: vec![9, 8, 7, 6, 5],
    };

    let a = s
        .ingest(
            &CaptureEvent {
                content: png.clone(),
                source_app: None,
                copied_at_ms: 1,
            },
            &images,
        )
        .unwrap();
    let b = s
        .ingest(
            &CaptureEvent {
                content: png.clone(),
                source_app: None,
                copied_at_ms: 2,
            },
            &images,
        )
        .unwrap();
    assert_eq!(a.entry_id, b.entry_id, "identical image dedups");
    assert!(a.is_new && !b.is_new);
    assert_eq!(images.writes(), 1, "content-addressed store writes once");

    let mut q = default_query();
    q.kind = Some(Kind::Image);
    let rows = s.search(&q).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].copy_count, 2);
    assert_eq!(rows[0].byte_size, 5);
    assert!(rows[0].image_path.as_deref().unwrap().ends_with(".bin"));
}

// ---- search modes -------------------------------------------------------

#[test]
fn search_modes_end_to_end() {
    let s = open_in_memory().unwrap();
    ingest_all(
        &s,
        &[
            text("alpha beta gamma", 1),
            text("alpha only", 2),
            text("fbb exact match", 3),
            text("foo bar baz", 4),
            text("order-12345", 5),
            text("order-abcde", 6),
        ],
    );

    // word: AND of terms, order-independent
    let mut w = default_query();
    w.text = "gamma alpha".into();
    let r = s.search(&w).unwrap();
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].full_text, "alpha beta gamma");

    // exact: case-insensitive substring
    let mut ex = default_query();
    ex.mode = SearchMode::Exact;
    ex.text = "BETA GAM".into();
    assert_eq!(s.search(&ex).unwrap().len(), 1);

    // fuzzy: non-contiguous, substring boosted first
    let mut fz = default_query();
    fz.mode = SearchMode::Fuzzy;
    fz.text = "fbb".into();
    let fr = s.search(&fz).unwrap();
    assert_eq!(fr[0].full_text, "fbb exact match");
    assert!(fr.iter().any(|e| e.full_text == "foo bar baz"));

    // regex: digits
    let mut rx = default_query();
    rx.mode = SearchMode::Regex;
    rx.text = r"order-\d+".into();
    assert_eq!(s.search(&rx).unwrap().len(), 1);

    // invalid regex: empty, no panic
    rx.text = "order-(".into();
    assert_eq!(s.search(&rx).unwrap().len(), 0);
}

#[test]
fn fts_handles_punctuation_and_quotes() {
    let s = open_in_memory().unwrap();
    ingest_all(
        &s,
        &[
            text(r#"he said "hello, world!" loudly"#, 1),
            text("unrelated content", 2),
        ],
    );
    let mut q = default_query();
    q.text = "hello world".into();
    let rows = s.search(&q).unwrap();
    assert_eq!(rows.len(), 1);
    assert!(rows[0].full_text.contains("hello"));
}

// ---- unicode / metrics --------------------------------------------------

#[test]
fn unicode_content_metrics_and_search() {
    let s = open_in_memory().unwrap();
    s.ingest(&text("café ☕ naïve", 1), &Noop).unwrap();
    let rows = s.recent(1).unwrap();
    // "café ☕ naïve": c,a,f,é,SPACE,☕,SPACE,n,a,ï,v,e = 12 Unicode scalar values
    assert_eq!(rows[0].char_count, 12);
    assert_eq!(rows[0].word_count, 3);
}

// ---- scale --------------------------------------------------------------

#[test]
fn large_history_search_and_limit() {
    let s = open_in_memory().unwrap();
    for i in 0..800 {
        s.ingest(
            &text(&format!("entry number {i} needle{}", i % 7), i as i64),
            &Noop,
        )
        .unwrap();
    }
    // limit respected
    let mut q = default_query();
    q.limit = 50;
    assert_eq!(s.search(&q).unwrap().len(), 50);

    // targeted word search across the whole set
    let mut n = default_query();
    n.text = "needle3".into();
    n.limit = 1000;
    let hits = s.search(&n).unwrap();
    // "needle3" was appended when i % 7 == 3, across i in 0..800.
    let expected = (0..800).filter(|i| i % 7 == 3).count();
    assert_eq!(hits.len(), expected);
    assert!(hits.iter().all(|e| e.full_text.contains("needle3")));
}

// ---- pinning ------------------------------------------------------------

#[test]
fn pin_persists_and_is_visible() {
    let s = open_in_memory().unwrap();
    let out = s.ingest(&text("keep me", 1), &Noop).unwrap();
    s.set_pinned(out.entry_id, true).unwrap();
    let rows = s.recent(1).unwrap();
    assert!(rows[0].pinned);
    s.set_pinned(out.entry_id, false).unwrap();
    assert!(!s.recent(1).unwrap()[0].pinned);
}

// ---- persistence --------------------------------------------------------

#[test]
fn data_persists_across_reopen() {
    let dir = std::env::temp_dir().join(format!("magpie-core-persist-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let db = dir.join("magpie.sqlite3");

    {
        let s = open(&db).unwrap();
        s.ingest(&text("persisted", 1), &Noop).unwrap();
        s.ingest(&text("persisted", 2), &Noop).unwrap(); // dedup -> count 2
    }
    {
        let s = open(&db).unwrap();
        let rows = s.recent(10).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].full_text, "persisted");
        assert_eq!(rows[0].copy_count, 2);

        // search still works against the reopened FTS index
        let mut q = default_query();
        q.text = "persisted".into();
        assert_eq!(s.search(&q).unwrap().len(), 1);
    }
    std::fs::remove_dir_all(&dir).ok();
}

// ---- rich text dedup ----------------------------------------------------

#[test]
fn rich_text_dedups_with_plain_text_of_same_body() {
    let s = open_in_memory().unwrap();
    s.ingest(
        &CaptureEvent {
            content: Content::Rich {
                text: "shared body".into(),
                html: Some("<b>shared body</b>".into()),
                rtf: None,
            },
            source_app: None,
            copied_at_ms: 1,
        },
        &Noop,
    )
    .unwrap();
    s.ingest(&text("shared body", 2), &Noop).unwrap();

    let rows = s.recent(10).unwrap();
    assert_eq!(rows.len(), 1, "rich + plain of same text collapse");
    assert_eq!(rows[0].copy_count, 2);
}
