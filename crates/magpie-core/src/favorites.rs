//! Favorites & Quicklinks (Phase 10): a small ordered list the user pins with
//! `⌘D` — a clipboard entry, note, task, bookmark, file, snippet, a saved search,
//! or a **quicklink** (a URL/path, optionally with a `{query}` template and a
//! keyword that activates it from any search bar). Stored in the `favorites`
//! table (migration v4); pure CRUD lives here so it is table-driven testable.

use crate::store::{Result, Store};
use rusqlite::params;

/// One favorite / quicklink row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FavoriteItem {
    pub id: i64,
    /// `clip` | `note` | `task` | `bookmark` | `file` | `snippet` | `search` | `quicklink`.
    pub kind: String,
    /// Kind-specific target: a URL / path / query, or JSON for structured kinds.
    pub payload: String,
    pub label: String,
    /// Activation keyword for a quicklink (`gh`), or "" for a plain favorite.
    pub keyword: String,
    pub position: i64,
    pub created_at_ms: i64,
}

/// Expand a quicklink `payload` template with the text after its keyword:
/// `https://github.com/search?q={query}` + `rust` → the URL with `{query}`
/// percent-ish-substituted (spaces → `+`). Templates without `{query}` return the
/// payload unchanged (the query is ignored).
pub fn expand_quicklink(payload: &str, query: &str) -> String {
    if payload.contains("{query}") {
        payload.replace("{query}", &query.trim().replace(' ', "+"))
    } else {
        payload.to_string()
    }
}

impl Store {
    /// Add a favorite (append to the end). Re-favoriting the same `(kind, payload)`
    /// is a no-op that returns the existing id, so `⌘D` is idempotent.
    pub fn add_favorite(
        &self,
        kind: &str,
        payload: &str,
        label: &str,
        keyword: &str,
        now_ms: i64,
    ) -> Result<i64> {
        let conn = self.conn();
        if let Ok(id) = conn.query_row(
            "SELECT id FROM favorites WHERE kind = ?1 AND payload = ?2",
            params![kind, payload],
            |r| r.get::<_, i64>(0),
        ) {
            return Ok(id);
        }
        let next_pos: i64 = conn
            .query_row(
                "SELECT COALESCE(MAX(position), -1) + 1 FROM favorites",
                [],
                |r| r.get(0),
            )
            .unwrap_or(0);
        conn.execute(
            "INSERT INTO favorites (kind, payload, label, keyword, position, created_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![kind, payload, label, keyword, next_pos, now_ms],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// All favorites in sidebar order (position, then insertion order).
    pub fn list_favorites(&self) -> Result<Vec<FavoriteItem>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, kind, payload, label, keyword, position, created_at_ms
             FROM favorites ORDER BY position, id",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(FavoriteItem {
                id: r.get(0)?,
                kind: r.get(1)?,
                payload: r.get(2)?,
                label: r.get(3)?,
                keyword: r.get(4)?,
                position: r.get(5)?,
                created_at_ms: r.get(6)?,
            })
        })?;
        rows.collect()
    }

    pub fn is_favorited(&self, kind: &str, payload: &str) -> Result<bool> {
        Ok(self
            .conn()
            .query_row(
                "SELECT 1 FROM favorites WHERE kind = ?1 AND payload = ?2",
                params![kind, payload],
                |_| Ok(()),
            )
            .is_ok())
    }

    /// Toggle a favorite (`⌘D`): removes it if present, else adds it. Returns the
    /// new state (`true` = now favorited).
    pub fn toggle_favorite(
        &self,
        kind: &str,
        payload: &str,
        label: &str,
        now_ms: i64,
    ) -> Result<bool> {
        if self.is_favorited(kind, payload)? {
            self.conn().execute(
                "DELETE FROM favorites WHERE kind = ?1 AND payload = ?2",
                params![kind, payload],
            )?;
            Ok(false)
        } else {
            self.add_favorite(kind, payload, label, "", now_ms)?;
            Ok(true)
        }
    }

    pub fn delete_favorite(&self, id: i64) -> Result<()> {
        self.conn()
            .execute("DELETE FROM favorites WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// The quicklink whose keyword matches (case-insensitive), if any.
    pub fn favorite_by_keyword(&self, keyword: &str) -> Result<Option<FavoriteItem>> {
        let kw = keyword.trim().to_lowercase();
        if kw.is_empty() {
            return Ok(None);
        }
        Ok(self
            .list_favorites()?
            .into_iter()
            .find(|f| !f.keyword.is_empty() && f.keyword.to_lowercase() == kw))
    }

    /// Move a favorite up (`delta < 0`) or down (`delta > 0`) by swapping its
    /// `position` with the neighbour in that direction. No-op at the ends.
    pub fn move_favorite(&self, id: i64, delta: i64) -> Result<()> {
        let items = self.list_favorites()?;
        let Some(idx) = items.iter().position(|f| f.id == id) else {
            return Ok(());
        };
        let target = idx as i64 + delta.signum();
        if target < 0 || target as usize >= items.len() {
            return Ok(());
        }
        let a = &items[idx];
        let b = &items[target as usize];
        let conn = self.conn();
        conn.execute(
            "UPDATE favorites SET position = ?1 WHERE id = ?2",
            params![b.position, a.id],
        )?;
        conn.execute(
            "UPDATE favorites SET position = ?1 WHERE id = ?2",
            params![a.position, b.id],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::open_in_memory;

    #[test]
    fn add_list_delete_roundtrip() {
        let s = open_in_memory().unwrap();
        let a = s.add_favorite("clip", "hello", "hello", "", 100).unwrap();
        s.add_favorite("bookmark", "https://x.com", "X", "", 200)
            .unwrap();
        let items = s.list_favorites().unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].payload, "hello"); // position order
        assert_eq!(items[1].label, "X");
        s.delete_favorite(a).unwrap();
        assert_eq!(s.list_favorites().unwrap().len(), 1);
    }

    #[test]
    fn favoriting_is_idempotent() {
        let s = open_in_memory().unwrap();
        let a = s.add_favorite("clip", "dup", "dup", "", 1).unwrap();
        let b = s.add_favorite("clip", "dup", "dup again", "", 2).unwrap();
        assert_eq!(a, b);
        assert_eq!(s.list_favorites().unwrap().len(), 1);
    }

    #[test]
    fn toggle_adds_then_removes() {
        let s = open_in_memory().unwrap();
        assert!(s.toggle_favorite("note", "n1", "Note 1", 1).unwrap());
        assert!(s.is_favorited("note", "n1").unwrap());
        assert!(!s.toggle_favorite("note", "n1", "Note 1", 2).unwrap());
        assert!(!s.is_favorited("note", "n1").unwrap());
    }

    #[test]
    fn keyword_lookup_and_expansion() {
        let s = open_in_memory().unwrap();
        s.add_favorite(
            "quicklink",
            "https://github.com/search?q={query}",
            "GitHub search",
            "gh",
            1,
        )
        .unwrap();
        let f = s.favorite_by_keyword("GH").unwrap().unwrap();
        assert_eq!(f.keyword, "gh");
        assert_eq!(
            expand_quicklink(&f.payload, "rust lang"),
            "https://github.com/search?q=rust+lang"
        );
        assert!(s.favorite_by_keyword("nope").unwrap().is_none());
        // A quicklink without {query} ignores the query.
        assert_eq!(
            expand_quicklink("https://x.com", "ignored"),
            "https://x.com"
        );
    }

    #[test]
    fn reorder_swaps_neighbours() {
        let s = open_in_memory().unwrap();
        let a = s.add_favorite("clip", "a", "a", "", 1).unwrap();
        let b = s.add_favorite("clip", "b", "b", "", 2).unwrap();
        s.move_favorite(b, -1).unwrap(); // b moves up above a
        let items = s.list_favorites().unwrap();
        assert_eq!(items[0].id, b);
        assert_eq!(items[1].id, a);
        // No-op at the top.
        s.move_favorite(b, -1).unwrap();
        assert_eq!(s.list_favorites().unwrap()[0].id, b);
    }
}
