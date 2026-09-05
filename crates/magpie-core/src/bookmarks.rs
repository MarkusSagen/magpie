//! Saved bookmarks — a curated, local, cross-browser link store.
use crate::store::{Result, Store};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bookmark {
    pub id: i64,
    pub url: String,
    pub title: String,
    pub domain: String,
    pub added_ms: i64,
}

const COLS: &str = "id, url, title, domain, added_ms";

fn row(r: &rusqlite::Row) -> rusqlite::Result<Bookmark> {
    Ok(Bookmark {
        id: r.get(0)?,
        url: r.get(1)?,
        title: r.get(2)?,
        domain: r.get(3)?,
        added_ms: r.get(4)?,
    })
}

impl Store {
    /// Insert or update a bookmark (unique by url). Returns the row id.
    pub fn add_bookmark(&self, url: &str, title: &str, domain: &str, now_ms: i64) -> Result<i64> {
        self.conn().execute(
            "INSERT INTO bookmarks (url, title, domain, added_ms) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(url) DO UPDATE SET
               title = CASE WHEN excluded.title <> '' THEN excluded.title ELSE bookmarks.title END,
               domain = excluded.domain",
            rusqlite::params![url, title, domain, now_ms],
        )?;
        self.conn()
            .query_row("SELECT id FROM bookmarks WHERE url = ?1", [url], |r| {
                r.get(0)
            })
    }

    /// Bookmarks matching `query` (case-insensitive over title/url/domain), newest first.
    pub fn list_bookmarks(&self, query: &str, limit: i64) -> Result<Vec<Bookmark>> {
        let like = format!("%{}%", query.trim());
        let mut stmt = self.conn().prepare(&format!(
            "SELECT {COLS} FROM bookmarks
             WHERE ?1 = '%%' OR title LIKE ?1 COLLATE NOCASE OR url LIKE ?1 COLLATE NOCASE OR domain LIKE ?1 COLLATE NOCASE
             ORDER BY added_ms DESC LIMIT ?2"
        ))?;
        let rows = stmt.query_map(rusqlite::params![like, limit], row)?;
        rows.collect()
    }

    /// Delete a bookmark. Returns whether a row was removed.
    pub fn delete_bookmark(&self, id: i64) -> Result<bool> {
        Ok(self
            .conn()
            .execute("DELETE FROM bookmarks WHERE id = ?1", [id])?
            > 0)
    }
}
