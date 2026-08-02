use crate::detect::detect_text_kind;
use crate::metrics::text_metrics;
use crate::model::{content_hash, Content};
use crate::store::{Result, Store};
use rusqlite::{params, OptionalExtension};

impl Store {
    pub fn update_entry_text(&self, id: i64, new_text: &str, now_ms: i64) -> Result<bool> {
        let hash = content_hash(&Content::Text(new_text.to_string()));
        let collision: Option<i64> = self
            .conn()
            .query_row(
                "SELECT id FROM entries WHERE content_hash = ?1 AND id != ?2",
                params![hash, id],
                |r| r.get(0),
            )
            .optional()?;
        if collision.is_some() {
            return Ok(false);
        }
        let kind = detect_text_kind(new_text);
        let m = text_metrics(new_text);
        let preview: String = new_text.chars().take(200).collect();
        self.conn().execute(
            "UPDATE entries SET
               content_hash = ?1, kind = ?2, preview_text = ?3, full_text = ?4,
               byte_size = ?5, char_count = ?6, word_count = ?7, line_count = ?8,
               last_copied_at_ms = ?9
             WHERE id = ?10",
            params![
                hash,
                kind.as_str(),
                preview,
                new_text,
                new_text.len() as i64,
                m.char_count,
                m.word_count,
                m.line_count,
                now_ms,
                id
            ],
        )?;
        Ok(true)
    }

    pub fn create_snippet(&self, text: &str, now_ms: i64) -> Result<i64> {
        let hash = content_hash(&Content::Text(text.to_string()));
        if let Some(id) = self
            .conn()
            .query_row(
                "SELECT id FROM entries WHERE content_hash = ?1",
                [&hash],
                |r| r.get::<_, i64>(0),
            )
            .optional()?
        {
            self.set_pinned(id, true)?;
            return Ok(id);
        }
        let kind = detect_text_kind(text);
        let m = text_metrics(text);
        let preview: String = text.chars().take(200).collect();
        self.conn().execute(
            "INSERT INTO entries
               (content_hash, kind, preview_text, full_text, image_path,
                byte_size, char_count, word_count, line_count,
                first_copied_at_ms, last_copied_at_ms, copy_count, pinned, source_app_id)
             VALUES (?1,?2,?3,?4,NULL,?5,?6,?7,?8,?9,?9,1,1,NULL)",
            params![
                hash,
                kind.as_str(),
                preview,
                text,
                text.len() as i64,
                m.char_count,
                m.word_count,
                m.line_count,
                now_ms
            ],
        )?;
        Ok(self.conn().last_insert_rowid())
    }
}

#[cfg(test)]
mod tests {
    use crate::model::{CaptureEvent, Content};
    use crate::search::default_query;
    use crate::store::{open_in_memory, ImageStore, Store};

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
    fn update_changes_text_metrics_and_reindexes_fts() {
        let s = open_in_memory().unwrap();
        let id = ingest(&s, "hello world", 1);
        let ok = s.update_entry_text(id, "https://example.com/new", 5_000).unwrap();
        assert!(ok);

        let e = s.recent(1).unwrap().into_iter().next().unwrap();
        assert_eq!(e.id, id);
        assert_eq!(e.full_text, "https://example.com/new");
        assert_eq!(e.kind.as_str(), "link");
        assert_eq!(e.last_copied_at_ms, 5_000);

        let mut q = default_query();
        q.text = "hello".into();
        assert_eq!(s.search(&q).unwrap().len(), 0);
        q.text = "example".into();
        assert_eq!(s.search(&q).unwrap().len(), 1);
    }

    #[test]
    fn update_to_duplicate_is_rejected_noop() {
        let s = open_in_memory().unwrap();
        let a = ingest(&s, "alpha", 1);
        let _b = ingest(&s, "beta", 2);
        let ok = s.update_entry_text(a, "beta", 3).unwrap();
        assert!(!ok);
        let e = s.recent(100).unwrap().into_iter().find(|e| e.id == a).unwrap();
        assert_eq!(e.full_text, "alpha");
    }

    #[test]
    fn create_snippet_inserts_pinned_text_entry() {
        let s = open_in_memory().unwrap();
        let id = s.create_snippet("Dear team, thanks!", 100).unwrap();
        let e = s.recent(1).unwrap().into_iter().next().unwrap();
        assert_eq!(e.id, id);
        assert_eq!(e.full_text, "Dear team, thanks!");
        assert!(e.pinned);
        assert_eq!(e.kind.as_str(), "text");
        assert!(e.source_app_id.is_none());
    }

    #[test]
    fn create_snippet_of_existing_text_pins_and_returns_existing() {
        let s = open_in_memory().unwrap();
        let existing = ingest(&s, "https://reuse.me", 1);
        let got = s.create_snippet("https://reuse.me", 2).unwrap();
        assert_eq!(got, existing);
        assert_eq!(s.recent(100).unwrap().len(), 1);
        let e = s.recent(1).unwrap().into_iter().next().unwrap();
        assert!(e.pinned);
    }
}
