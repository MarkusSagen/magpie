use crate::detect::Kind;
use crate::model::Entry;
use crate::store::{Result, Store};

pub(crate) const ENTRY_COLUMNS: &str = "id, content_hash, kind, preview_text, full_text, image_path, \
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
        let sql = format!(
            "SELECT {ENTRY_COLUMNS} FROM entries ORDER BY last_copied_at_ms DESC LIMIT ?1"
        );
        let mut stmt = self.conn().prepare(&sql)?;
        let rows = stmt.query_map([limit], row_to_entry)?;
        rows.collect()
    }
}

#[cfg(test)]
mod tests {
    use crate::model::{CaptureEvent, Content};
    use crate::store::{open_in_memory, ImageStore};

    struct FakeImages;
    impl ImageStore for FakeImages {
        fn put(&self, hash: &str, _b: &[u8]) -> std::io::Result<String> { Ok(format!("/c/{hash}")) }
    }

    fn text_ev(t: &str, ms: i64) -> CaptureEvent {
        CaptureEvent { content: Content::Text(t.into()), source_app: None, copied_at_ms: ms }
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
}
