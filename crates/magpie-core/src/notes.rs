use crate::store::{Result, Store};
use rusqlite::OptionalExtension;

const NOTE_COLS: &str =
    "id, name, is_daily, body, created_at_ms, updated_at_ms, source_app_id, source_entry_id";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note {
    pub id: i64,
    pub name: String,
    pub is_daily: bool,
    pub body: String,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub source_app_id: Option<i64>,
    pub source_entry_id: Option<i64>,
}

fn row_to_note(r: &rusqlite::Row) -> rusqlite::Result<Note> {
    Ok(Note {
        id: r.get(0)?,
        name: r.get(1)?,
        is_daily: r.get::<_, i64>(2)? != 0,
        body: r.get(3)?,
        created_at_ms: r.get(4)?,
        updated_at_ms: r.get(5)?,
        source_app_id: r.get(6)?,
        source_entry_id: r.get(7)?,
    })
}

impl Store {
    pub fn note_by_name(&self, name: &str) -> Result<Option<Note>> {
        self.conn()
            .query_row(
                &format!("SELECT {NOTE_COLS} FROM notes WHERE name = ?1"),
                [name],
                row_to_note,
            )
            .optional()
    }

    pub fn get_note(&self, id: i64) -> Result<Option<Note>> {
        self.conn()
            .query_row(
                &format!("SELECT {NOTE_COLS} FROM notes WHERE id = ?1"),
                [id],
                row_to_note,
            )
            .optional()
    }

    // ---- Scratch capture (Phase 20): a scratch is a note with `scratch = 1`. ----

    /// The current ("Now") scratch — the newest scratch note — creating an empty one
    /// if none exists.
    pub fn scratch_current(&self, now_ms: i64) -> Result<Note> {
        if let Some(n) = self
            .conn()
            .query_row(
                &format!(
                    "SELECT {NOTE_COLS} FROM notes WHERE scratch = 1 \
                     ORDER BY updated_at_ms DESC, id DESC LIMIT 1"
                ),
                [],
                row_to_note,
            )
            .optional()?
        {
            return Ok(n);
        }
        self.new_scratch(now_ms)
    }

    /// Start a fresh empty scratch and return it (becomes the new "Now").
    pub fn new_scratch(&self, now_ms: i64) -> Result<Note> {
        self.conn().execute(
            "INSERT INTO notes (name, is_daily, body, created_at_ms, updated_at_ms, scratch)
             VALUES (?1, 0, '', ?2, ?2, 1)",
            rusqlite::params![format!("scratch-{now_ms}"), now_ms],
        )?;
        let id = self.conn().last_insert_rowid();
        Ok(self.get_note(id)?.expect("row exists after insert"))
    }

    /// Previous scratches ("Earlier") — all but the newest, newest first.
    pub fn scratch_earlier(&self, limit: i64) -> Result<Vec<Note>> {
        let mut stmt = self.conn().prepare(&format!(
            "SELECT {NOTE_COLS} FROM notes WHERE scratch = 1 \
             ORDER BY updated_at_ms DESC, id DESC LIMIT ?1 OFFSET 1"
        ))?;
        let rows = stmt.query_map([limit], row_to_note)?;
        rows.collect()
    }

    pub fn set_scratch_pinned(&self, id: i64, pinned: bool) -> Result<()> {
        self.conn().execute(
            "UPDATE notes SET scratch_pinned = ?2 WHERE id = ?1",
            rusqlite::params![id, pinned as i64],
        )?;
        Ok(())
    }

    pub fn scratch_is_pinned(&self, id: i64) -> Result<bool> {
        Ok(self
            .conn()
            .query_row(
                "SELECT scratch_pinned FROM notes WHERE id = ?1",
                [id],
                |r| r.get::<_, i64>(0),
            )
            .optional()?
            .unwrap_or(0)
            == 1)
    }

    /// Delete unpinned scratch notes not updated since `older_than_ms` (30-day sweep).
    /// Returns how many were removed.
    pub fn prune_scratches(&self, older_than_ms: i64) -> Result<usize> {
        self.conn().execute(
            "DELETE FROM notes WHERE scratch = 1 AND scratch_pinned = 0 AND updated_at_ms < ?1",
            [older_than_ms],
        )
    }

    // ---- Note version history (backlog #4). ----

    /// Snapshot a note's body as a restore point. Coalesces rapid saves: if the newest
    /// version for this note is younger than `coalesce_ms`, it's updated in place rather
    /// than adding a row; a snapshot identical to the newest is skipped. Keeps the
    /// newest `keep` versions per note.
    pub fn snapshot_note_version(
        &self,
        note_id: i64,
        body: &str,
        now_ms: i64,
        coalesce_ms: i64,
        keep: i64,
    ) -> Result<()> {
        let latest: Option<(i64, String, i64)> = self
            .conn()
            .query_row(
                "SELECT id, body, created_at_ms FROM note_versions
                 WHERE note_id = ?1 ORDER BY created_at_ms DESC, id DESC LIMIT 1",
                [note_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        if let Some((id, last_body, last_ms)) = latest {
            if last_body == body {
                return Ok(()); // no change → no new restore point
            }
            if now_ms - last_ms < coalesce_ms {
                self.conn().execute(
                    "UPDATE note_versions SET body = ?2, created_at_ms = ?3 WHERE id = ?1",
                    rusqlite::params![id, body, now_ms],
                )?;
                return Ok(());
            }
        }
        self.conn().execute(
            "INSERT INTO note_versions (note_id, body, created_at_ms) VALUES (?1, ?2, ?3)",
            rusqlite::params![note_id, body, now_ms],
        )?;
        self.conn().execute(
            "DELETE FROM note_versions WHERE note_id = ?1 AND id NOT IN
             (SELECT id FROM note_versions WHERE note_id = ?1 ORDER BY created_at_ms DESC LIMIT ?2)",
            rusqlite::params![note_id, keep],
        )?;
        Ok(())
    }

    /// A note's version history, newest first: `(version_id, created_at_ms, body)`.
    pub fn note_versions(&self, note_id: i64, limit: i64) -> Result<Vec<(i64, i64, String)>> {
        let mut stmt = self.conn().prepare(
            "SELECT id, created_at_ms, body FROM note_versions
             WHERE note_id = ?1 ORDER BY created_at_ms DESC, id DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(rusqlite::params![note_id, limit], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })?;
        rows.collect()
    }

    /// The body of a specific version (for restore/preview).
    pub fn note_version_body(&self, version_id: i64) -> Result<Option<String>> {
        self.conn()
            .query_row(
                "SELECT body FROM note_versions WHERE id = ?1",
                [version_id],
                |r| r.get::<_, String>(0),
            )
            .optional()
    }

    pub fn upsert_note_by_name(&self, name: &str, now_ms: i64) -> Result<Note> {
        let n = name.trim();
        if n.is_empty() {
            return Err(rusqlite::Error::InvalidParameterName(
                "empty note name".into(),
            ));
        }
        self.conn().execute(
            "INSERT OR IGNORE INTO notes (name, is_daily, body, created_at_ms, updated_at_ms)
             VALUES (?1, 0, '', ?2, ?2)",
            rusqlite::params![n, now_ms],
        )?;
        Ok(self
            .note_by_name(n)?
            .expect("row exists after INSERT OR IGNORE"))
    }

    pub fn daily_note(&self, day: &str, now_ms: i64) -> Result<Note> {
        self.conn().execute(
            "INSERT OR IGNORE INTO notes (name, is_daily, body, created_at_ms, updated_at_ms)
             VALUES (?1, 1, '', ?2, ?2)",
            rusqlite::params![day, now_ms],
        )?;
        Ok(self
            .note_by_name(day)?
            .expect("row exists after INSERT OR IGNORE"))
    }

    pub fn update_note_body(&self, id: i64, body: &str, now_ms: i64) -> Result<bool> {
        let n = self.conn().execute(
            "UPDATE notes SET body = ?2, updated_at_ms = ?3 WHERE id = ?1",
            rusqlite::params![id, body, now_ms],
        )?;
        Ok(n > 0)
    }

    pub fn recent_notes(&self, limit: i64) -> Result<Vec<Note>> {
        let mut stmt = self.conn().prepare(&format!(
            "SELECT {NOTE_COLS} FROM notes ORDER BY updated_at_ms DESC, id DESC LIMIT ?1"
        ))?;
        let rows = stmt.query_map([limit], row_to_note)?;
        rows.collect()
    }

    pub fn all_notes(&self) -> Result<Vec<Note>> {
        let mut stmt = self.conn().prepare(&format!(
            "SELECT {NOTE_COLS} FROM notes ORDER BY updated_at_ms DESC, id DESC"
        ))?;
        let rows = stmt.query_map([], row_to_note)?;
        rows.collect()
    }

    /// (note, its stored vault_synced_hash) for every note — for 3-way reconcile.
    pub fn notes_for_sync(&self) -> Result<Vec<(Note, String)>> {
        let mut stmt = self
            .conn()
            .prepare(&format!("SELECT {NOTE_COLS}, vault_synced_hash FROM notes"))?;
        let rows = stmt.query_map([], |r| Ok((row_to_note(r)?, r.get::<_, String>(8)?)))?;
        rows.collect()
    }

    /// Record the baseline hash after a successful sync of this note.
    pub fn set_vault_hash(&self, id: i64, hash: &str) -> Result<()> {
        self.conn().execute(
            "UPDATE notes SET vault_synced_hash = ?2 WHERE id = ?1",
            rusqlite::params![id, hash],
        )?;
        Ok(())
    }

    /// Daily journal notes (is_daily = 1), newest date first (names are YYYY-MM-DD,
    /// so lexical DESC = chronological DESC).
    pub fn daily_notes(&self, limit: i64) -> Result<Vec<Note>> {
        let mut stmt = self.conn().prepare(&format!(
            "SELECT {NOTE_COLS} FROM notes WHERE is_daily = 1 ORDER BY name DESC LIMIT ?1"
        ))?;
        let rows = stmt.query_map([limit], row_to_note)?;
        rows.collect()
    }

    pub fn all_note_names(&self) -> Result<Vec<String>> {
        let mut stmt = self
            .conn()
            .prepare("SELECT name FROM notes ORDER BY name")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        rows.collect()
    }

    fn unique_note_name(&self, base: &str) -> Result<String> {
        if self.note_by_name(base)?.is_none() {
            return Ok(base.to_string());
        }
        for i in 2..10_000 {
            let cand = format!("{base} ({i})");
            if self.note_by_name(&cand)?.is_none() {
                return Ok(cand);
            }
        }
        Ok(format!("{base} (dup)"))
    }

    pub fn create_note_from_entry(&self, entry_id: i64, now_ms: i64) -> Result<Note> {
        let (full_text, app_id): (String, Option<i64>) = self.conn().query_row(
            "SELECT full_text, source_app_id FROM entries WHERE id = ?1",
            [entry_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let first = full_text
            .lines()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("")
            .trim();
        let base: String = if first.is_empty() {
            format!("Note {now_ms}")
        } else {
            first.chars().take(60).collect()
        };
        let name = self.unique_note_name(&base)?;
        self.conn().execute(
            "INSERT INTO notes
               (name, is_daily, body, created_at_ms, updated_at_ms, source_app_id, source_entry_id)
             VALUES (?1, 0, ?2, ?3, ?3, ?4, ?5)",
            rusqlite::params![name, full_text, now_ms, app_id, entry_id],
        )?;
        let id = self.conn().last_insert_rowid();
        Ok(self.get_note(id)?.expect("row exists after INSERT"))
    }

    /// Whether a reminder with this fingerprint has already fired.
    pub fn reminder_fired(&self, fingerprint: &str) -> Result<bool> {
        let n: i64 = self.conn().query_row(
            "SELECT COUNT(*) FROM task_reminders WHERE fingerprint = ?1",
            [fingerprint],
            |r| r.get(0),
        )?;
        Ok(n > 0)
    }

    /// Record that a reminder with this fingerprint has fired.
    pub fn mark_reminder(&self, fingerprint: &str, now_ms: i64) -> Result<()> {
        self.conn().execute(
            "INSERT OR REPLACE INTO task_reminders (fingerprint, fired_at_ms) VALUES (?1, ?2)",
            rusqlite::params![fingerprint, now_ms],
        )?;
        Ok(())
    }
}

/// A currently-running timer (an entry with no end).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveTimer {
    pub task_key: String,
    pub task_title: String,
    pub start_ms: i64,
}

/// One tracked session for the "recent sessions" UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimeEntry {
    pub id: i64,
    pub task_title: String,
    pub start_ms: i64,
    pub end_ms: Option<i64>, // None = still running
}

impl Store {
    /// The running timer, if any (there is at most one).
    pub fn active_timer(&self) -> Result<Option<ActiveTimer>> {
        use rusqlite::OptionalExtension;
        self.conn()
            .query_row(
                "SELECT task_key, task_title, start_ms FROM time_entries
                 WHERE end_ms IS NULL ORDER BY start_ms DESC LIMIT 1",
                [],
                |r| {
                    Ok(ActiveTimer {
                        task_key: r.get(0)?,
                        task_title: r.get(1)?,
                        start_ms: r.get(2)?,
                    })
                },
            )
            .optional()
    }

    /// Stop the running timer (if any). Returns whether one was running.
    pub fn stop_active(&self, now_ms: i64) -> Result<bool> {
        let n = self.conn().execute(
            "UPDATE time_entries SET end_ms = ?1 WHERE end_ms IS NULL",
            [now_ms],
        )?;
        Ok(n > 0)
    }

    /// Start a timer on a task, stopping any other running one first.
    pub fn start_timer(
        &self,
        task_key: &str,
        task_title: &str,
        note_id: Option<i64>,
        now_ms: i64,
    ) -> Result<()> {
        self.stop_active(now_ms)?;
        self.conn().execute(
            "INSERT INTO time_entries (task_key, task_title, note_id, start_ms, end_ms)
             VALUES (?1, ?2, ?3, ?4, NULL)",
            rusqlite::params![task_key, task_title, note_id, now_ms],
        )?;
        Ok(())
    }

    /// Total tracked ms for a task: closed entries plus the running one (if it's
    /// this task), measured to `now_ms`.
    pub fn total_ms_for(&self, task_key: &str, now_ms: i64) -> Result<i64> {
        let mut total: i64 = self.conn().query_row(
            "SELECT COALESCE(SUM(end_ms - start_ms), 0) FROM time_entries
             WHERE task_key = ?1 AND end_ms IS NOT NULL",
            [task_key],
            |r| r.get(0),
        )?;
        if let Some(a) = self.active_timer()? {
            if a.task_key == task_key {
                total += (now_ms - a.start_ms).max(0);
            }
        }
        Ok(total)
    }

    /// Total tracked ms grouped by LOCAL day ("YYYY-MM-DD"), for entries starting at
    /// or after `since_ms`. The running entry counts to `now_ms`. Newest day first.
    pub fn time_by_day(&self, since_ms: i64, now_ms: i64) -> Result<Vec<(String, i64)>> {
        let mut stmt = self.conn().prepare(
            "SELECT date(start_ms / 1000, 'unixepoch', 'localtime') AS d,
                    SUM(COALESCE(end_ms, ?2) - start_ms)
             FROM time_entries WHERE start_ms >= ?1
             GROUP BY d ORDER BY d DESC",
        )?;
        let rows = stmt.query_map(rusqlite::params![since_ms, now_ms], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        })?;
        rows.collect()
    }

    /// Total tracked ms grouped by task (key + a representative title), since
    /// `since_ms`, largest first.
    pub fn time_by_task(&self, since_ms: i64, now_ms: i64) -> Result<Vec<(String, String, i64)>> {
        let mut stmt = self.conn().prepare(
            "SELECT task_key, MAX(task_title), SUM(COALESCE(end_ms, ?2) - start_ms) AS total
             FROM time_entries WHERE start_ms >= ?1
             GROUP BY task_key ORDER BY total DESC",
        )?;
        let rows = stmt.query_map(rusqlite::params![since_ms, now_ms], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })?;
        rows.collect()
    }

    /// Total tracked ms for all entries starting at or after `since_ms` (running to now).
    pub fn time_total_since(&self, since_ms: i64, now_ms: i64) -> Result<i64> {
        self.conn().query_row(
            "SELECT COALESCE(SUM(COALESCE(end_ms, ?2) - start_ms), 0)
             FROM time_entries WHERE start_ms >= ?1",
            rusqlite::params![since_ms, now_ms],
            |r| r.get(0),
        )
    }

    /// Most-recent tracked sessions first (by start), up to `limit`.
    pub fn recent_time_entries(&self, limit: i64) -> Result<Vec<TimeEntry>> {
        let mut stmt = self.conn().prepare(
            "SELECT id, task_title, start_ms, end_ms FROM time_entries
             ORDER BY start_ms DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map([limit], |r| {
            Ok(TimeEntry {
                id: r.get(0)?,
                task_title: r.get(1)?,
                start_ms: r.get(2)?,
                end_ms: r.get(3)?,
            })
        })?;
        rows.collect()
    }

    /// Delete one tracked session. Returns whether a row was removed.
    pub fn delete_time_entry(&self, id: i64) -> Result<bool> {
        Ok(self
            .conn()
            .execute("DELETE FROM time_entries WHERE id = ?1", [id])?
            > 0)
    }

    /// Record a closed session typed in by hand (Time tab quick-add). Doesn't touch
    /// the running timer.
    pub fn add_time_entry(
        &self,
        task_key: &str,
        task_title: &str,
        start_ms: i64,
        end_ms: i64,
    ) -> Result<i64> {
        self.conn().execute(
            "INSERT INTO time_entries (task_key, task_title, note_id, start_ms, end_ms)
             VALUES (?1, ?2, NULL, ?3, ?4)",
            rusqlite::params![task_key, task_title, start_ms, end_ms],
        )?;
        Ok(self.conn().last_insert_rowid())
    }

    /// Change a finished session's description and span (Time tab edit). Running
    /// sessions are left alone (returns false).
    pub fn update_time_entry(
        &self,
        id: i64,
        task_title: &str,
        start_ms: i64,
        end_ms: i64,
    ) -> Result<bool> {
        Ok(self.conn().execute(
            "UPDATE time_entries SET task_title = ?2, start_ms = ?3, end_ms = ?4
             WHERE id = ?1 AND end_ms IS NOT NULL",
            rusqlite::params![id, task_title, start_ms, end_ms],
        )? > 0)
    }

    /// Cut idle time out of the running session: end it at `idle_from_ms` and start
    /// the same task again at `resume_ms`. Returns false when nothing is running or
    /// the idle span doesn't fall inside the running session.
    pub fn split_active(&self, idle_from_ms: i64, resume_ms: i64) -> Result<bool> {
        let Some(a) = self.active_timer()? else {
            return Ok(false);
        };
        if idle_from_ms <= a.start_ms || resume_ms < idle_from_ms {
            return Ok(false);
        }
        let note_id: Option<i64> = self.conn().query_row(
            "SELECT note_id FROM time_entries WHERE end_ms IS NULL LIMIT 1",
            [],
            |r| r.get(0),
        )?;
        self.stop_active(idle_from_ms)?;
        self.start_timer(&a.task_key, &a.task_title, note_id, resume_ms)?;
        Ok(true)
    }
}

#[cfg(test)]
mod time_entry_tests {
    use crate::open_in_memory;

    #[test]
    fn add_manual_entry_keeps_running_timer() {
        let s = open_in_memory().unwrap();
        s.start_timer("k", "Running #x", None, 1_000).unwrap();
        s.add_time_entry("manual", "Typed #y", 100, 600).unwrap();
        assert!(s.active_timer().unwrap().is_some());
        let all = s.recent_time_entries(10).unwrap();
        assert_eq!(all.len(), 2);
        let typed = all.iter().find(|e| e.task_title == "Typed #y").unwrap();
        assert_eq!((typed.start_ms, typed.end_ms), (100, Some(600)));
    }

    #[test]
    fn update_edits_finished_entries_only() {
        let s = open_in_memory().unwrap();
        let id = s.add_time_entry("k", "Old", 100, 200).unwrap();
        assert!(s.update_time_entry(id, "New #x", 150, 400).unwrap());
        let e = s
            .recent_time_entries(5)
            .unwrap()
            .into_iter()
            .find(|e| e.id == id)
            .unwrap();
        assert_eq!(
            (e.task_title.as_str(), e.start_ms, e.end_ms),
            ("New #x", 150, Some(400))
        );
        s.start_timer("k2", "Running", None, 1_000).unwrap();
        let running = s
            .recent_time_entries(5)
            .unwrap()
            .into_iter()
            .find(|e| e.end_ms.is_none())
            .unwrap();
        assert!(!s.update_time_entry(running.id, "x", 1, 2).unwrap());
    }

    #[test]
    fn split_active_cuts_idle_out() {
        let s = open_in_memory().unwrap();
        assert!(!s.split_active(10, 20).unwrap()); // nothing running
        s.start_timer("k", "Task", None, 1_000).unwrap();
        assert!(!s.split_active(500, 900).unwrap()); // idle before the session
        assert!(s.split_active(2_000, 5_000).unwrap());
        let mut all = s.recent_time_entries(10).unwrap();
        all.sort_by_key(|e| e.start_ms);
        assert_eq!((all[0].start_ms, all[0].end_ms), (1_000, Some(2_000)));
        assert_eq!((all[1].start_ms, all[1].end_ms), (5_000, None));
        assert_eq!(s.active_timer().unwrap().unwrap().task_key, "k");
    }
}

#[cfg(test)]
mod scratch_tests {
    use crate::open_in_memory;

    #[test]
    fn scratch_current_creates_then_reuses() {
        let s = open_in_memory().unwrap();
        let a = s.scratch_current(100).unwrap();
        assert!(a.body.is_empty());
        // Same current until a new one starts.
        let b = s.scratch_current(200).unwrap();
        assert_eq!(a.id, b.id, "reuses the newest scratch");
        assert!(s.scratch_earlier(50).unwrap().is_empty());
    }

    #[test]
    fn new_scratch_moves_prior_to_earlier() {
        let s = open_in_memory().unwrap();
        let first = s.scratch_current(100).unwrap();
        s.update_note_body(first.id, "old jottings", 150).unwrap();
        let second = s.new_scratch(200).unwrap();
        assert_ne!(first.id, second.id);
        // Now = second (newest); Earlier = [first].
        assert_eq!(s.scratch_current(300).unwrap().id, second.id);
        let earlier = s.scratch_earlier(50).unwrap();
        assert_eq!(earlier.len(), 1);
        assert_eq!(earlier[0].id, first.id);
    }

    #[test]
    fn note_versions_coalesce_dedupe_and_cap() {
        let s = open_in_memory().unwrap();
        let n = s.upsert_note_by_name("N", 0).unwrap();
        // First snapshot.
        s.snapshot_note_version(n.id, "v1", 1_000, 60_000, 3)
            .unwrap();
        // Within the coalesce window + changed → updates in place (still 1 version).
        s.snapshot_note_version(n.id, "v1b", 5_000, 60_000, 3)
            .unwrap();
        assert_eq!(s.note_versions(n.id, 10).unwrap().len(), 1);
        assert_eq!(s.note_versions(n.id, 1).unwrap()[0].2, "v1b");
        // Identical body → skipped.
        s.snapshot_note_version(n.id, "v1b", 200_000, 60_000, 3)
            .unwrap();
        assert_eq!(s.note_versions(n.id, 10).unwrap().len(), 1);
        // Past the window + changed → new version.
        s.snapshot_note_version(n.id, "v2", 200_000, 60_000, 3)
            .unwrap();
        s.snapshot_note_version(n.id, "v3", 400_000, 60_000, 3)
            .unwrap();
        s.snapshot_note_version(n.id, "v4", 600_000, 60_000, 3)
            .unwrap();
        // Cap keeps the newest 3.
        let vers = s.note_versions(n.id, 10).unwrap();
        assert_eq!(vers.len(), 3);
        assert_eq!(vers[0].2, "v4");
        // Restore body lookup.
        let vid = vers[1].0;
        assert_eq!(s.note_version_body(vid).unwrap().as_deref(), Some("v3"));
    }

    #[test]
    fn prune_respects_pin_and_age() {
        let s = open_in_memory().unwrap();
        let old = s.new_scratch(1_000).unwrap();
        let pinned_old = s.new_scratch(1_100).unwrap();
        s.set_scratch_pinned(pinned_old.id, true).unwrap();
        assert!(s.scratch_is_pinned(pinned_old.id).unwrap());
        let _recent = s.new_scratch(9_000).unwrap();
        // Prune everything older than 5_000: removes `old`, keeps pinned + recent.
        let removed = s.prune_scratches(5_000).unwrap();
        assert_eq!(removed, 1);
        assert!(s.get_note(old.id).unwrap().is_none());
        assert!(s.get_note(pinned_old.id).unwrap().is_some());
    }
}
