use crate::detect::Kind;
use crate::model::Entry;
use crate::store::{Result, Store};
use fuzzy_matcher::skim::SkimMatcherV2;
use fuzzy_matcher::FuzzyMatcher;
use regex::RegexBuilder;
use rusqlite::types::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchMode {
    Word,
    Exact,
    Fuzzy,
    Regex,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sort {
    Recency,
    MostCopied,
    Alphabetical,
}

#[derive(Debug, Clone, Default)]
pub struct TimeRange {
    pub since_ms: Option<i64>,
    pub until_ms: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct SearchQuery {
    pub text: String,
    pub mode: SearchMode,
    pub kind: Option<Kind>,
    pub source_app_id: Option<i64>,
    pub time: TimeRange,
    pub sort: Sort,
    pub limit: i64,
}

pub fn default_query() -> SearchQuery {
    SearchQuery {
        text: String::new(),
        mode: SearchMode::Word,
        kind: None,
        source_app_id: None,
        time: TimeRange::default(),
        sort: Sort::Recency,
        limit: 200,
    }
}

fn order_clause(sort: Sort) -> &'static str {
    match sort {
        Sort::Recency => "e.last_copied_at_ms DESC",
        Sort::MostCopied => "e.copy_count DESC, e.last_copied_at_ms DESC",
        Sort::Alphabetical => "e.full_text COLLATE NOCASE ASC",
    }
}

fn fts_match(text: &str) -> String {
    // Quote each term and AND them: alpha gamma -> "alpha" AND "gamma"
    text.split_whitespace()
        .map(|t| format!("\"{}\"", t.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" AND ")
}

fn entry_cols_prefixed() -> String {
    ENTRY_COLUMNS
        .split(", ")
        .map(|c| format!("e.{c}"))
        .collect::<Vec<_>>()
        .join(", ")
}

pub(crate) const ENTRY_COLUMNS: &str =
    "id, content_hash, kind, preview_text, full_text, image_path, \
    byte_size, char_count, word_count, line_count, first_copied_at_ms, last_copied_at_ms, \
    copy_count, pinned, source_app_id";

pub(crate) fn row_to_entry(row: &rusqlite::Row) -> rusqlite::Result<Entry> {
    let kind_str: String = row.get(2)?;
    Ok(Entry {
        id: row.get(0)?,
        content_hash: row.get(1)?,
        kind: Kind::from_str(&kind_str).unwrap_or(Kind::Text),
        preview_text: row.get(3)?,
        full_text: row.get(4)?,
        image_path: row.get(5)?,
        byte_size: row.get(6)?,
        char_count: row.get(7)?,
        word_count: row.get(8)?,
        line_count: row.get(9)?,
        first_copied_at_ms: row.get(10)?,
        last_copied_at_ms: row.get(11)?,
        copy_count: row.get(12)?,
        pinned: row.get::<_, i64>(13)? != 0,
        source_app_id: row.get(14)?,
    })
}

impl Store {
    pub fn recent(&self, limit: i64) -> Result<Vec<Entry>> {
        let sql =
            format!("SELECT {ENTRY_COLUMNS} FROM entries ORDER BY last_copied_at_ms DESC LIMIT ?1");
        let mut stmt = self.conn().prepare(&sql)?;
        let rows = stmt.query_map([limit], row_to_entry)?;
        rows.collect()
    }

    fn candidates(&self, q: &SearchQuery) -> Result<Vec<Entry>> {
        let mut base = q.clone();
        base.text = String::new(); // drop text; keep kind/app/time/sort
        base.limit = 100_000; // wide net; we cap after ranking
        self.search(&base)
    }

    fn search_fuzzy(&self, q: &SearchQuery) -> Result<Vec<Entry>> {
        let matcher = SkimMatcherV2::default().ignore_case();
        let needle = q.text.trim();
        let needle_lc = needle.to_lowercase();
        let mut scored: Vec<(i64, Entry)> = Vec::new();
        for e in self.candidates(q)? {
            let hay = e.full_text.to_lowercase();
            if let Some(score) = matcher.fuzzy_match(&e.full_text, needle) {
                let boost = if hay.contains(&needle_lc) {
                    1_000_000
                } else {
                    0
                };
                scored.push((boost + score, e));
            }
        }
        scored.sort_by_key(|s| std::cmp::Reverse(s.0));
        Ok(scored
            .into_iter()
            .take(q.limit as usize)
            .map(|(_, e)| e)
            .collect())
    }

    fn search_regex(&self, q: &SearchQuery) -> Result<Vec<Entry>> {
        let re = match RegexBuilder::new(q.text.trim())
            .case_insensitive(true)
            .build()
        {
            Ok(re) => re,
            Err(_) => return Ok(Vec::new()),
        };
        Ok(self
            .candidates(q)?
            .into_iter()
            .filter(|e| re.is_match(&e.full_text))
            .take(q.limit as usize)
            .collect())
    }

    pub fn search(&self, q: &SearchQuery) -> Result<Vec<Entry>> {
        if !q.text.trim().is_empty() {
            match q.mode {
                SearchMode::Fuzzy => return self.search_fuzzy(q),
                SearchMode::Regex => return self.search_regex(q),
                _ => {}
            }
        }
        // Every clause is pushed to `clauses` and its param(s) to `params` in the
        // SAME order, then joined with AND. The word-mode text query uses an FTS
        // subquery so its MATCH param is just another positional `?` in sequence —
        // no special-case ordering. (Fuzzy/Regex are dispatched in Task 12 before
        // this SQL path runs.)
        let mut clauses: Vec<String> = Vec::new();
        let mut params: Vec<Value> = Vec::new();

        let trimmed = q.text.trim();
        if !trimmed.is_empty() {
            match q.mode {
                SearchMode::Exact => {
                    clauses.push("e.full_text LIKE ?".to_string());
                    params.push(Value::Text(format!("%{}%", trimmed)));
                }
                _ => {
                    clauses.push(
                        "e.id IN (SELECT rowid FROM entries_fts WHERE entries_fts MATCH ?)"
                            .to_string(),
                    );
                    params.push(Value::Text(fts_match(trimmed)));
                }
            }
        }
        if let Some(k) = q.kind {
            clauses.push("e.kind = ?".to_string());
            params.push(Value::Text(k.as_str().to_string()));
        }
        if let Some(app) = q.source_app_id {
            clauses.push("e.source_app_id = ?".to_string());
            params.push(Value::Integer(app));
        }
        if let Some(since) = q.time.since_ms {
            clauses.push("e.last_copied_at_ms >= ?".to_string());
            params.push(Value::Integer(since));
        }
        if let Some(until) = q.time.until_ms {
            clauses.push("e.last_copied_at_ms <= ?".to_string());
            params.push(Value::Integer(until));
        }

        let where_sql = if clauses.is_empty() {
            "1=1".to_string()
        } else {
            clauses.join(" AND ")
        };
        let sql = format!(
            "SELECT {cols} FROM entries e WHERE {where_sql} ORDER BY {order} LIMIT ?",
            cols = entry_cols_prefixed(),
            order = order_clause(q.sort),
        );
        params.push(Value::Integer(q.limit));

        let mut stmt = self.conn().prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(params), row_to_entry)?;
        rows.collect()
    }
}

#[cfg(test)]
mod tests {
    use crate::model::{CaptureEvent, Content};
    use crate::store::{open_in_memory, ImageStore};

    struct FakeImages;
    impl ImageStore for FakeImages {
        fn put(&self, hash: &str, _b: &[u8]) -> std::io::Result<String> {
            Ok(format!("/c/{hash}"))
        }
    }

    fn text_ev(t: &str, ms: i64) -> CaptureEvent {
        CaptureEvent {
            content: Content::Text(t.into()),
            source_app: None,
            copied_at_ms: ms,
        }
    }

    #[test]
    fn recent_is_newest_first_and_limited() {
        let s = open_in_memory().unwrap();
        s.ingest(&text_ev("old", 100), &FakeImages).unwrap();
        s.ingest(&text_ev("mid", 200), &FakeImages).unwrap();
        s.ingest(&text_ev("new", 300), &FakeImages).unwrap();

        let rows = s.recent(2).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].full_text, "new");
        assert_eq!(rows[1].full_text, "mid");
        assert_eq!(rows[0].kind.as_str(), "text");
    }

    use crate::detect::Kind;
    use crate::model::AppInfo;
    use crate::search::{default_query, SearchMode, Sort};

    fn app_ev(t: &str, ms: i64, ident: &str) -> CaptureEvent {
        CaptureEvent {
            content: Content::Text(t.into()),
            source_app: Some(AppInfo {
                identifier: ident.into(),
                display_name: ident.into(),
                icon_path: None,
            }),
            copied_at_ms: ms,
        }
    }

    #[test]
    fn word_mode_ands_terms() {
        let s = open_in_memory().unwrap();
        s.ingest(&text_ev("alpha beta gamma", 1), &FakeImages)
            .unwrap();
        s.ingest(&text_ev("alpha only", 2), &FakeImages).unwrap();
        let mut q = default_query();
        q.text = "alpha gamma".into();
        let rows = s.search(&q).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].full_text, "alpha beta gamma");
    }

    #[test]
    fn exact_mode_is_substring_case_insensitive() {
        let s = open_in_memory().unwrap();
        s.ingest(&text_ev("HelloWorld", 1), &FakeImages).unwrap();
        let mut q = default_query();
        q.mode = SearchMode::Exact;
        q.text = "loworl".into();
        assert_eq!(s.search(&q).unwrap().len(), 1);
    }

    #[test]
    fn filter_by_kind() {
        let s = open_in_memory().unwrap();
        s.ingest(&text_ev("plain text here", 1), &FakeImages)
            .unwrap();
        s.ingest(&text_ev("https://x.io", 2), &FakeImages).unwrap();
        let mut q = default_query();
        q.kind = Some(Kind::Link);
        let rows = s.search(&q).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].kind.as_str(), "link");
    }

    #[test]
    fn filter_by_app_and_time() {
        let s = open_in_memory().unwrap();
        s.ingest(&app_ev("from ghostty", 1000, "com.ghostty"), &FakeImages)
            .unwrap();
        s.ingest(&app_ev("from safari", 2000, "com.safari"), &FakeImages)
            .unwrap();
        let ghostty_id: i64 = s
            .conn()
            .query_row(
                "SELECT id FROM apps WHERE identifier='com.ghostty'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let mut q = default_query();
        q.source_app_id = Some(ghostty_id);
        assert_eq!(s.search(&q).unwrap().len(), 1);

        let mut q2 = default_query();
        q2.time.since_ms = Some(1500);
        let rows = s.search(&q2).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].full_text, "from safari");
    }

    #[test]
    fn sort_most_copied() {
        let s = open_in_memory().unwrap();
        s.ingest(&text_ev("once", 1), &FakeImages).unwrap();
        s.ingest(&text_ev("twice", 2), &FakeImages).unwrap();
        s.ingest(&text_ev("twice", 3), &FakeImages).unwrap();
        let mut q = default_query();
        q.sort = Sort::MostCopied;
        let rows = s.search(&q).unwrap();
        assert_eq!(rows[0].full_text, "twice");
        assert_eq!(rows[0].copy_count, 2);
    }

    #[test]
    fn word_text_and_kind_filter_combine() {
        // Guards param ordering: text query AND a filter must both apply.
        let s = open_in_memory().unwrap();
        s.ingest(&text_ev("https://alpha.example", 1), &FakeImages)
            .unwrap(); // link, token 'alpha'
        s.ingest(&text_ev("alpha plain note", 2), &FakeImages)
            .unwrap(); // text, token 'alpha'
        let mut q = default_query();
        q.text = "alpha".into();
        q.kind = Some(Kind::Link);
        let rows = s.search(&q).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].kind.as_str(), "link");
    }

    #[test]
    fn fuzzy_matches_noncontiguous_and_ranks_substring_first() {
        let s = open_in_memory().unwrap();
        s.ingest(&text_ev("foo bar baz", 1), &FakeImages).unwrap(); // fuzzy 'fbb'
        s.ingest(&text_ev("fbb exact", 2), &FakeImages).unwrap(); // substring 'fbb'
        s.ingest(&text_ev("nothing", 3), &FakeImages).unwrap();
        let mut q = default_query();
        q.mode = SearchMode::Fuzzy;
        q.text = "fbb".into();
        let rows = s.search(&q).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].full_text, "fbb exact"); // substring ranked above fuzzy-only
    }

    #[test]
    fn regex_mode_filters_and_bad_regex_is_empty() {
        let s = open_in_memory().unwrap();
        s.ingest(&text_ev("order-123", 1), &FakeImages).unwrap();
        s.ingest(&text_ev("order-abc", 2), &FakeImages).unwrap();
        let mut q = default_query();
        q.mode = SearchMode::Regex;
        q.text = r"order-\d+".into();
        assert_eq!(s.search(&q).unwrap().len(), 1);

        q.text = r"order-(".into(); // invalid
        assert_eq!(s.search(&q).unwrap().len(), 0);
    }
}
