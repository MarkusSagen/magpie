//! Calculator history (Phase 7): a persisted log of every expression evaluated
//! in the Calculator or the clipboard search bar. Rows carry the raw `input`, the
//! chosen `syntax` (plain/latex/typst), the primary `result` display, and a
//! `pinned` flag that exempts the row from the 30-day retention sweep.

use crate::store::{Result, Store};
use rusqlite::params;

/// One evaluated expression in the calculator history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalcEntry {
    pub id: i64,
    /// `math` | `date` | `graph` — drives the row glyph and the tab filter.
    pub kind: String,
    pub input: String,
    /// `plain` | `latex` | `typst` — how `input` should be read/rendered.
    pub syntax: String,
    pub result: String,
    pub pinned: bool,
    pub created_at_ms: i64,
}

impl Store {
    /// Record an evaluation. Re-evaluating the same `(kind, input)` moves the
    /// existing row to the top (updates its timestamp + result) rather than
    /// piling up duplicates, so the history stays a set of distinct expressions.
    pub fn add_calc_entry(
        &self,
        kind: &str,
        input: &str,
        syntax: &str,
        result: &str,
        now_ms: i64,
    ) -> Result<i64> {
        let input = input.trim();
        if input.is_empty() {
            return Ok(0);
        }
        let conn = self.conn();
        // Preserve a pin across the move-to-top.
        let existing: Option<(i64, bool)> = conn
            .query_row(
                "SELECT id, pinned FROM calc_history WHERE kind = ?1 AND input = ?2",
                params![kind, input],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)? != 0)),
            )
            .ok();
        if let Some((id, _pinned)) = existing {
            conn.execute(
                "UPDATE calc_history SET result = ?1, syntax = ?2, created_at_ms = ?3 WHERE id = ?4",
                params![result, syntax, now_ms, id],
            )?;
            return Ok(id);
        }
        conn.execute(
            "INSERT INTO calc_history (kind, input, syntax, result, pinned, created_at_ms)
             VALUES (?1, ?2, ?3, ?4, 0, ?5)",
            params![kind, input, syntax, result, now_ms],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// The history, newest first, capped at `limit`. `kind_filter` of `None`
    /// returns every kind; `Some("math")` restricts to that kind.
    pub fn list_calc_history(
        &self,
        kind_filter: Option<&str>,
        limit: i64,
    ) -> Result<Vec<CalcEntry>> {
        let conn = self.conn();
        let map = |r: &rusqlite::Row| {
            Ok(CalcEntry {
                id: r.get(0)?,
                kind: r.get(1)?,
                input: r.get(2)?,
                syntax: r.get(3)?,
                result: r.get(4)?,
                pinned: r.get::<_, i64>(5)? != 0,
                created_at_ms: r.get(6)?,
            })
        };
        let sql_all = "SELECT id, kind, input, syntax, result, pinned, created_at_ms \
             FROM calc_history ORDER BY created_at_ms DESC LIMIT ?1";
        let sql_kind = "SELECT id, kind, input, syntax, result, pinned, created_at_ms \
             FROM calc_history WHERE kind = ?2 ORDER BY created_at_ms DESC LIMIT ?1";
        let rows = match kind_filter {
            None => {
                let mut stmt = conn.prepare(sql_all)?;
                let it = stmt.query_map(params![limit], map)?;
                it.collect::<rusqlite::Result<Vec<_>>>()?
            }
            Some(k) => {
                let mut stmt = conn.prepare(sql_kind)?;
                let it = stmt.query_map(params![limit, k], map)?;
                it.collect::<rusqlite::Result<Vec<_>>>()?
            }
        };
        Ok(rows)
    }

    /// Per-kind counts for the tab labels: `(total, math, date, graph)`.
    pub fn calc_history_counts(&self) -> Result<(i64, i64, i64, i64)> {
        let conn = self.conn();
        let count = |where_kind: Option<&str>| -> Result<i64> {
            Ok(match where_kind {
                None => conn.query_row("SELECT COUNT(*) FROM calc_history", [], |r| r.get(0))?,
                Some(k) => conn.query_row(
                    "SELECT COUNT(*) FROM calc_history WHERE kind = ?1",
                    params![k],
                    |r| r.get(0),
                )?,
            })
        };
        Ok((
            count(None)?,
            count(Some("math"))?,
            count(Some("date"))?,
            count(Some("graph"))?,
        ))
    }

    pub fn delete_calc_entry(&self, id: i64) -> Result<()> {
        self.conn()
            .execute("DELETE FROM calc_history WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn set_calc_pinned(&self, id: i64, pinned: bool) -> Result<()> {
        self.conn().execute(
            "UPDATE calc_history SET pinned = ?1 WHERE id = ?2",
            params![pinned as i64, id],
        )?;
        Ok(())
    }

    /// Clear the history. Pinned rows are kept (they are the "Favorites").
    pub fn clear_calc_history(&self) -> Result<usize> {
        self.conn()
            .execute("DELETE FROM calc_history WHERE pinned = 0", [])
    }

    /// Retention sweep: drop non-pinned rows older than `cutoff_ms`. Returns the
    /// number removed. Mirrors the clipboard/note retention model.
    pub fn prune_calc_history(&self, cutoff_ms: i64) -> Result<usize> {
        self.conn().execute(
            "DELETE FROM calc_history WHERE pinned = 0 AND created_at_ms < ?1",
            params![cutoff_ms],
        )
    }
}

#[cfg(test)]
mod tests {
    use crate::open_in_memory;

    #[test]
    fn add_list_roundtrip_and_counts() {
        let s = open_in_memory().unwrap();
        s.add_calc_entry("math", "1 + 1", "plain", "2", 100)
            .unwrap();
        s.add_calc_entry("date", "next monday", "plain", "Mon 28 Sep 2026", 200)
            .unwrap();
        s.add_calc_entry("graph", "sin(x) = x/3", "plain", "3 intersections", 300)
            .unwrap();
        let all = s.list_calc_history(None, 50).unwrap();
        assert_eq!(all.len(), 3);
        // Newest first.
        assert_eq!(all[0].input, "sin(x) = x/3");
        let (total, math, date, graph) = s.calc_history_counts().unwrap();
        assert_eq!((total, math, date, graph), (3, 1, 1, 1));
        let math_only = s.list_calc_history(Some("math"), 50).unwrap();
        assert_eq!(math_only.len(), 1);
    }

    #[test]
    fn re_evaluating_moves_to_top_without_duplicating() {
        let s = open_in_memory().unwrap();
        s.add_calc_entry("math", "1 + 1", "plain", "2", 100)
            .unwrap();
        s.add_calc_entry("math", "2 + 2", "plain", "4", 200)
            .unwrap();
        // Re-evaluate the older expression → single row, moved to top.
        s.add_calc_entry("math", "1 + 1", "plain", "2", 300)
            .unwrap();
        let all = s.list_calc_history(None, 50).unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].input, "1 + 1");
    }

    #[test]
    fn empty_input_is_not_recorded() {
        let s = open_in_memory().unwrap();
        s.add_calc_entry("math", "   ", "plain", "", 100).unwrap();
        assert_eq!(s.list_calc_history(None, 50).unwrap().len(), 0);
    }

    #[test]
    fn clear_and_prune_keep_pinned() {
        let s = open_in_memory().unwrap();
        let a = s
            .add_calc_entry("math", "1 + 1", "plain", "2", 100)
            .unwrap();
        s.add_calc_entry("math", "2 + 2", "plain", "4", 200)
            .unwrap();
        s.set_calc_pinned(a, true).unwrap();
        // Prune everything before ms=1_000: the unpinned "2+2" goes, pinned stays.
        let removed = s.prune_calc_history(1_000).unwrap();
        assert_eq!(removed, 1);
        assert_eq!(s.list_calc_history(None, 50).unwrap().len(), 1);
        // Clear keeps the pinned row too.
        s.clear_calc_history().unwrap();
        assert_eq!(s.list_calc_history(None, 50).unwrap().len(), 1);
    }

    #[test]
    fn delete_removes_one() {
        let s = open_in_memory().unwrap();
        let a = s
            .add_calc_entry("math", "1 + 1", "plain", "2", 100)
            .unwrap();
        s.delete_calc_entry(a).unwrap();
        assert_eq!(s.list_calc_history(None, 50).unwrap().len(), 0);
    }
}
