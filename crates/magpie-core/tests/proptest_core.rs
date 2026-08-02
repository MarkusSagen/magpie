//! Property-based / fuzz tests for magpie-core. These hammer the store and
//! search with generated inputs to check invariants and robustness.

use magpie_core::detect::detect_text_kind;
use magpie_core::metrics::text_metrics;
use magpie_core::{
    default_query, open_in_memory, CaptureEvent, Content, ImageStore, Kind, SearchMode,
};
use proptest::prelude::*;
use std::collections::HashSet;

struct Noop;
impl ImageStore for Noop {
    fn put(&self, h: &str, _b: &[u8]) -> std::io::Result<String> {
        Ok(h.to_string())
    }
}
fn ev(s: &str, ms: i64) -> CaptureEvent {
    CaptureEvent {
        content: Content::Text(s.into()),
        source_app: None,
        copied_at_ms: ms,
    }
}

// Simple ascii words -> phrases (keeps FTS tokenization predictable).
fn word() -> impl Strategy<Value = String> {
    "[a-z]{1,6}"
}
fn phrase() -> impl Strategy<Value = String> {
    prop::collection::vec(word(), 1..4).prop_map(|w| w.join(" "))
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    /// Metrics: char_count is always the Unicode scalar count of the text.
    #[test]
    fn metrics_char_count_matches_scalars(s in ".{0,200}") {
        let m = text_metrics(&s);
        prop_assert_eq!(m.char_count as usize, s.chars().count());
        prop_assert!(m.word_count >= 0 && m.line_count >= 0);
    }

    /// Detection never panics and always yields a text-ish kind.
    #[test]
    fn detect_never_panics(s in ".{0,200}") {
        let k = detect_text_kind(&s);
        prop_assert!(matches!(k, Kind::Text | Kind::Link | Kind::Email | Kind::Color));
        // as_str/from_str round-trips.
        prop_assert_eq!(Kind::from_str(k.as_str()), Some(k));
    }

    /// Dedup invariant: entries == unique inputs; sum(copy_count) == total ingests.
    #[test]
    fn dedup_invariant(inputs in prop::collection::vec(phrase(), 1..30)) {
        let s = open_in_memory().unwrap();
        for (i, txt) in inputs.iter().enumerate() {
            s.ingest(&ev(txt, i as i64 + 1), &Noop).unwrap();
        }
        let unique: HashSet<&String> = inputs.iter().collect();
        let rows = s.search(&default_query()).unwrap();
        prop_assert_eq!(rows.len(), unique.len());
        let total: i64 = rows.iter().map(|e| e.copy_count).sum();
        prop_assert_eq!(total, inputs.len() as i64);
    }

    /// Preview is always a char-prefix of full_text and at most 200 chars.
    #[test]
    fn preview_is_bounded_char_prefix(s in ".{0,600}") {
        let store = open_in_memory().unwrap();
        // skip empty (empty clipboard isn't a capture we care about here)
        prop_assume!(!s.is_empty());
        store.ingest(&ev(&s, 1), &Noop).unwrap();
        let e = &store.recent(1).unwrap()[0];
        prop_assert!(e.preview_text.chars().count() <= 200);
        let expected: String = s.chars().take(200).collect();
        prop_assert_eq!(&e.preview_text, &expected);
    }

    /// Word search finds an entry that contains the searched token.
    #[test]
    fn word_search_finds_containing_entry(entries in prop::collection::vec(phrase(), 1..12)) {
        let s = open_in_memory().unwrap();
        for (i, txt) in entries.iter().enumerate() {
            s.ingest(&ev(txt, i as i64 + 1), &Noop).unwrap();
        }
        let target = &entries[0];
        let token = target.split_whitespace().next().unwrap().to_string();
        let mut q = default_query();
        q.text = token.clone();
        let rows = s.search(&q).unwrap();
        prop_assert!(rows.iter().any(|e| e.full_text == *target));
        prop_assert!(rows.iter().all(|e| e.full_text.contains(&token)));
    }

    /// Search never errors for ANY query text in ANY mode (robustness).
    #[test]
    fn search_never_errors(query in ".{0,80}", mode in 0u8..4) {
        let s = open_in_memory().unwrap();
        s.ingest(&ev("some seed content here", 1), &Noop).unwrap();
        s.ingest(&ev("https://example.com/path", 2), &Noop).unwrap();
        let mut q = default_query();
        q.text = query;
        q.mode = match mode {
            0 => SearchMode::Word,
            1 => SearchMode::Exact,
            2 => SearchMode::Fuzzy,
            _ => SearchMode::Regex,
        };
        prop_assert!(s.search(&q).is_ok());
    }

    /// Exact-substring treats LIKE metacharacters literally.
    #[test]
    fn exact_treats_wildcards_literally(needle in "[a-z%_]{1,6}") {
        let s = open_in_memory().unwrap();
        let content = format!("literal[{needle}]end");
        s.ingest(&ev(&content, 1), &Noop).unwrap();
        s.ingest(&ev("zzz totally different zzz", 2), &Noop).unwrap();
        let mut q = default_query();
        q.mode = SearchMode::Exact;
        q.text = needle.clone();
        let rows = s.search(&q).unwrap();
        // every hit genuinely contains the literal needle
        prop_assert!(rows.iter().all(|e| e.full_text.contains(&needle)));
        prop_assert!(rows.iter().any(|e| e.full_text == content));
    }
}
