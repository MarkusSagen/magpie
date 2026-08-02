use crate::store::{Result, Store};

fn norm(tag: &str) -> String {
    tag.trim().to_lowercase()
}

impl Store {
    pub fn add_tag(&self, entry_id: i64, tag: &str) -> Result<()> {
        let t = norm(tag);
        if t.is_empty() {
            return Ok(());
        }
        self.conn().execute(
            "INSERT OR IGNORE INTO entry_tags (entry_id, tag) VALUES (?1, ?2)",
            rusqlite::params![entry_id, t],
        )?;
        Ok(())
    }

    pub fn remove_tag(&self, entry_id: i64, tag: &str) -> Result<()> {
        let t = norm(tag);
        self.conn().execute(
            "DELETE FROM entry_tags WHERE entry_id = ?1 AND tag = ?2",
            rusqlite::params![entry_id, t],
        )?;
        Ok(())
    }

    pub fn tags_of(&self, entry_id: i64) -> Result<Vec<String>> {
        let mut stmt = self
            .conn()
            .prepare("SELECT tag FROM entry_tags WHERE entry_id = ?1 ORDER BY tag")?;
        let rows = stmt.query_map([entry_id], |r| r.get::<_, String>(0))?;
        rows.collect()
    }

    pub fn all_tags(&self) -> Result<Vec<String>> {
        let mut stmt = self
            .conn()
            .prepare("SELECT DISTINCT tag FROM entry_tags ORDER BY tag")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        rows.collect()
    }

    /// (entry_id, tag) pairs for annotating a list. Ordered by entry then tag.
    pub fn tag_pairs(&self) -> Result<Vec<(i64, String)>> {
        let mut stmt = self
            .conn()
            .prepare("SELECT entry_id, tag FROM entry_tags ORDER BY entry_id, tag")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
        rows.collect()
    }
}

#[cfg(test)]
mod tests {
    use crate::model::{CaptureEvent, Content};
    use crate::store::{open_in_memory, ImageStore, Store};

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
    fn add_normalizes_and_is_idempotent() {
        let s = open_in_memory().unwrap();
        let id = ingest(&s, "x");
        s.add_tag(id, "  Work ").unwrap();
        s.add_tag(id, "work").unwrap();
        s.add_tag(id, "  ").unwrap();
        assert_eq!(s.tags_of(id).unwrap(), vec!["work".to_string()]);
    }

    #[test]
    fn remove_and_listing() {
        let s = open_in_memory().unwrap();
        let a = ingest(&s, "a");
        let b = ingest(&s, "b");
        s.add_tag(a, "red").unwrap();
        s.add_tag(a, "blue").unwrap();
        s.add_tag(b, "red").unwrap();
        assert_eq!(s.tags_of(a).unwrap(), vec!["blue".to_string(), "red".to_string()]);
        assert_eq!(s.all_tags().unwrap(), vec!["blue".to_string(), "red".to_string()]);
        s.remove_tag(a, "RED").unwrap();
        assert_eq!(s.tags_of(a).unwrap(), vec!["blue".to_string()]);
    }

    #[test]
    fn tag_pairs_lists_all() {
        let s = open_in_memory().unwrap();
        let id = ingest(&s, "x");
        s.add_tag(id, "a").unwrap();
        s.add_tag(id, "b").unwrap();
        assert_eq!(s.tag_pairs().unwrap(), vec![(id, "a".to_string()), (id, "b".to_string())]);
    }
}
