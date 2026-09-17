//! Saved bookmarks — a curated, local, cross-browser link store.
use crate::store::{Result, Store};
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bookmark {
    pub id: i64,
    pub url: String,
    pub title: String,
    pub domain: String,
    pub added_ms: i64,
    pub tags: Vec<String>,
}

const COLS: &str = "id, url, title, domain, added_ms, tags";

fn row(r: &rusqlite::Row) -> rusqlite::Result<Bookmark> {
    let tags: String = r.get(5)?;
    Ok(Bookmark {
        id: r.get(0)?,
        url: r.get(1)?,
        title: r.get(2)?,
        domain: r.get(3)?,
        added_ms: r.get(4)?,
        tags: tags
            .split(',')
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect(),
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

    /// Bookmarks matching `query` (case-insensitive over title/url/domain/tags), newest first.
    pub fn list_bookmarks(&self, query: &str, limit: i64) -> Result<Vec<Bookmark>> {
        let like = format!("%{}%", query.trim());
        let mut stmt = self.conn().prepare(&format!(
            "SELECT {COLS} FROM bookmarks
             WHERE ?1 = '%%' OR title LIKE ?1 COLLATE NOCASE OR url LIKE ?1 COLLATE NOCASE OR domain LIKE ?1 COLLATE NOCASE OR tags LIKE ?1 COLLATE NOCASE
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

    /// Replace a bookmark's tags. Normalizes: trim, lowercase, drop empties, dedupe
    /// (order preserved), store comma-joined.
    pub fn set_bookmark_tags(&self, id: i64, tags: &[String]) -> Result<()> {
        let mut seen = std::collections::HashSet::new();
        let norm: Vec<String> = tags
            .iter()
            .map(|t| t.trim().to_lowercase())
            .filter(|t| !t.is_empty() && seen.insert(t.clone()))
            .collect();
        self.conn().execute(
            "UPDATE bookmarks SET tags = ?2 WHERE id = ?1",
            rusqlite::params![id, norm.join(",")],
        )?;
        Ok(())
    }
}

/// Read bookmarks from a Firefox `places.sqlite` (title, url). Copies the file to a
/// temp path first (Firefox may hold a lock), opens it read-only, and reads
/// http(s) bookmark rows. Returns (title, url) pairs.
pub fn read_firefox_bookmarks(places_path: &Path) -> Result<Vec<(String, String)>> {
    let tmp = std::env::temp_dir().join(format!("magpie-ff-import-{}.sqlite", std::process::id()));
    std::fs::copy(places_path, &tmp)
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
    let result = (|| {
        let conn = rusqlite::Connection::open_with_flags(
            &tmp,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;
        let mut stmt = conn.prepare(
            "SELECT COALESCE(b.title, ''), p.url
             FROM moz_bookmarks b JOIN moz_places p ON b.fk = p.id
             WHERE b.type = 1 AND p.url LIKE 'http%'
             ORDER BY b.id",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        rows.collect::<Result<Vec<_>>>()
    })();
    let _ = std::fs::remove_file(&tmp);
    result
}
