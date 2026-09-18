//! One-way-import + write-back sync, and a proper 3-way reconcile, between the
//! notes DB and a folder of Markdown files ("vault").
//!
//! `sync_vault` is newest-wins per file, with a content-equality guard so repeated
//! syncs are idempotent (no timestamp ping-pong). `reconcile_vault` is the live
//! bidirectional path: it keeps a per-note baseline hash of the last agreed
//! content so it can tell which side actually changed, and writes a keep-both
//! conflict file when both sides diverged instead of silently picking a winner.
use crate::export::sanitize_filename;
use crate::store::{Result, Store};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::UNIX_EPOCH;

/// Map an io error into the crate's rusqlite-based `Result`.
fn io_err(e: std::io::Error) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(Box::new(e))
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SyncReport {
    pub imported: usize, // new notes created from files
    pub updated: usize,  // existing notes overwritten from a newer file
    pub exported: usize, // files written from a newer/absent note
    pub skipped: usize,  // already in sync
}

/// What a single file<->note pair needs. Pure so it is unit-testable with arbitrary
/// timestamps (real file mtimes are hard to control in tests).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VaultAction {
    Create,
    ImportUpdate,
    ExportUpdate,
    Skip,
}

/// `note`: `Some((body, updated_at_ms))` if a note with this (sanitized) name exists.
pub fn resolve_action(
    note: Option<(&str, i64)>,
    file_body: &str,
    file_mtime_ms: i64,
) -> VaultAction {
    match note {
        None => VaultAction::Create,
        Some((body, _updated)) if body == file_body => VaultAction::Skip,
        Some((_, updated)) if file_mtime_ms > updated => VaultAction::ImportUpdate,
        Some(_) => VaultAction::ExportUpdate,
    }
}

/// Sync `dir` (a folder of `.md` files) with the notes DB. Newest-wins per file;
/// idempotent — a second consecutive call with no changes returns an all-`skipped`
/// report.
pub fn sync_vault(store: &Store, dir: &Path, now_ms: i64) -> Result<SyncReport> {
    std::fs::create_dir_all(dir).map_err(io_err)?;

    let notes = store.all_notes()?;
    let mut by_name: HashMap<String, crate::notes::Note> = HashMap::new();
    for n in notes {
        by_name.insert(sanitize_filename(&n.name), n);
    }
    let mut matched: HashSet<String> = HashSet::new();

    let mut report = SyncReport::default();

    for entry in std::fs::read_dir(dir).map_err(io_err)? {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let path = entry.path();
        if path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
            != Some("md".to_string())
        {
            continue;
        }
        let stem = match path.file_stem().and_then(|s| s.to_str()) {
            Some(s) => s.to_string(),
            None => continue,
        };
        let file_body = match std::fs::read_to_string(&path) {
            Ok(b) => b,
            Err(_) => continue,
        };
        let file_mtime_ms = std::fs::metadata(&path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as i64)
            .unwrap_or(now_ms);

        let note = by_name.get(&stem);
        let action = resolve_action(
            note.map(|n| (n.body.as_str(), n.updated_at_ms)),
            &file_body,
            file_mtime_ms,
        );
        match action {
            VaultAction::Create => {
                let n = store.upsert_note_by_name(&stem, file_mtime_ms)?;
                store.update_note_body(n.id, &file_body, file_mtime_ms)?;
                report.imported += 1;
            }
            VaultAction::ImportUpdate => {
                let n = note.expect("ImportUpdate implies a note");
                store.update_note_body(n.id, &file_body, file_mtime_ms)?;
                report.updated += 1;
            }
            VaultAction::ExportUpdate => {
                let n = note.expect("ExportUpdate implies a note");
                std::fs::write(&path, &n.body).map_err(io_err)?;
                report.exported += 1;
            }
            VaultAction::Skip => {
                report.skipped += 1;
            }
        }
        matched.insert(stem);
    }

    for (stem, note) in &by_name {
        if matched.contains(stem) {
            continue;
        }
        let path = dir.join(format!("{stem}.md"));
        std::fs::write(&path, &note.body).map_err(io_err)?;
        report.exported += 1;
    }

    Ok(report)
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReconcileReport {
    pub pulled: usize,        // file → note (external edit imported)
    pub pushed: usize,        // note → file (note edit written out)
    pub created_notes: usize, // new note from a new file
    pub created_files: usize, // new file from a note with no file
    pub conflicts: usize,     // both changed → conflict file written, file taken into note
    pub unchanged: usize,
}

fn hash_hex(s: &str) -> String {
    blake3::hash(s.as_bytes()).to_hex().to_string()
}

/// 3-way reconcile between the notes DB and `dir` (a folder of `.md` files), using
/// each note's stored `vault_synced_hash` as the baseline (the content last agreed
/// by both sides):
///  - file == note: in sync (heal baseline if drifted) -> unchanged
///  - file changed, note == baseline: pull file into note -> pulled
///  - note changed, file == baseline: push note to file -> pushed
///  - both diverged from baseline (or the baseline matches neither side): CONFLICT
///    -- write the note's current body to `<stem>.conflict-<now_ms>.md`, then take
///    the file's content into the note -> conflicts
///  - file with no note: create a note from the file -> created_notes
///  - note with no file (non-empty body): create the file -> created_files
///
/// After every case the note's baseline hash is set to the now-agreed content, so
/// a second call with no external changes reports all-`unchanged`. Conflict files
/// are never re-imported (they are not a note's canonical file). Never deletes
/// notes or files.
pub fn reconcile_vault(store: &Store, dir: &Path, now_ms: i64) -> Result<ReconcileReport> {
    std::fs::create_dir_all(dir).map_err(io_err)?;

    let notes = store.notes_for_sync()?;
    let mut by_name: HashMap<String, (crate::notes::Note, String)> = HashMap::new();
    for (n, hash) in notes {
        by_name.insert(sanitize_filename(&n.name), (n, hash));
    }
    let mut matched: HashSet<String> = HashSet::new();
    let mut report = ReconcileReport::default();

    for entry in std::fs::read_dir(dir).map_err(io_err)? {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let path = entry.path();
        if path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
            != Some("md".to_string())
        {
            continue;
        }
        let stem = match path.file_stem().and_then(|s| s.to_str()) {
            Some(s) => s.to_string(),
            None => continue,
        };
        // Our own keep-both conflict files are artifacts, not a note's canonical
        // file — never reconcile them back in (that would resurrect the losing
        // side as a brand-new note on every subsequent run).
        if stem.contains(".conflict-") {
            continue;
        }
        let file_body = match std::fs::read_to_string(&path) {
            Ok(b) => b,
            Err(_) => continue,
        };
        let file_hash = hash_hex(&file_body);

        match by_name.get(&stem) {
            None => {
                let n = store.upsert_note_by_name(&stem, now_ms)?;
                store.update_note_body(n.id, &file_body, now_ms)?;
                store.set_vault_hash(n.id, &file_hash)?;
                report.created_notes += 1;
            }
            Some((note, baseline)) => {
                let note_hash = hash_hex(&note.body);
                if note_hash == file_hash {
                    // In sync. Heal the baseline if it had drifted (e.g. an
                    // interrupted previous run), so future comparisons stay sound.
                    if baseline != &note_hash {
                        store.set_vault_hash(note.id, &note_hash)?;
                    }
                    report.unchanged += 1;
                } else {
                    let baseline_matches_note = baseline == &note_hash;
                    let baseline_matches_file = baseline == &file_hash;
                    if baseline_matches_note && !baseline_matches_file {
                        // Only the file changed since the last agreed content.
                        store.update_note_body(note.id, &file_body, now_ms)?;
                        store.set_vault_hash(note.id, &file_hash)?;
                        report.pulled += 1;
                    } else if baseline_matches_file && !baseline_matches_note {
                        // Only the note changed since the last agreed content.
                        std::fs::write(&path, &note.body).map_err(io_err)?;
                        store.set_vault_hash(note.id, &note_hash)?;
                        report.pushed += 1;
                    } else {
                        // Both sides diverged from the baseline (or there was no
                        // baseline and the two sides simply disagree): keep both.
                        // Preserve the note's version in a conflict file, then
                        // take the file's version into the note.
                        let conflict_path = dir.join(format!("{stem}.conflict-{now_ms}.md"));
                        std::fs::write(&conflict_path, &note.body).map_err(io_err)?;
                        store.update_note_body(note.id, &file_body, now_ms)?;
                        store.set_vault_hash(note.id, &file_hash)?;
                        report.conflicts += 1;
                    }
                }
            }
        }
        matched.insert(stem);
    }

    for (stem, (note, _baseline)) in &by_name {
        if matched.contains(stem) || note.body.is_empty() {
            // No file, and either already handled above or an empty body that was
            // never synced — skip so a fresh blank note doesn't spam an empty file.
            continue;
        }
        let path = dir.join(format!("{stem}.md"));
        std::fs::write(&path, &note.body).map_err(io_err)?;
        let note_hash = hash_hex(&note.body);
        store.set_vault_hash(note.id, &note_hash)?;
        report.created_files += 1;
    }

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_action_all_branches() {
        // No note yet -> Create.
        assert_eq!(resolve_action(None, "hello", 100), VaultAction::Create);

        // Equal bodies -> Skip, even if the mtime differs wildly.
        assert_eq!(
            resolve_action(Some(("same", 50)), "same", 999_999),
            VaultAction::Skip
        );

        // Differing bodies, file newer than the note -> ImportUpdate.
        assert_eq!(
            resolve_action(Some(("old", 50)), "new", 100),
            VaultAction::ImportUpdate
        );

        // Differing bodies, file not newer than the note -> ExportUpdate.
        assert_eq!(
            resolve_action(Some(("old", 100)), "new", 100),
            VaultAction::ExportUpdate
        );
        assert_eq!(
            resolve_action(Some(("old", 100)), "new", 50),
            VaultAction::ExportUpdate
        );
    }

    #[test]
    fn sync_vault_is_idempotent() {
        let store = crate::open_in_memory().unwrap();
        let dir = std::env::temp_dir().join(format!("magpie-vault-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // A file with no matching note -> imported.
        std::fs::write(dir.join("Alpha.md"), "hello").unwrap();

        // A note with no matching file -> exported.
        let n = store.upsert_note_by_name("Beta", 10).unwrap();
        store.update_note_body(n.id, "world", 20).unwrap();

        let now = 1_000_000_000_000i64;
        let report = sync_vault(&store, &dir, now).unwrap();
        assert_eq!(report.imported, 1);
        assert!(report.exported >= 1);

        let alpha = store.note_by_name("Alpha").unwrap().unwrap();
        assert_eq!(alpha.body, "hello");
        let beta_body = std::fs::read_to_string(dir.join("Beta.md")).unwrap();
        assert_eq!(beta_body, "world");

        // Second consecutive sync must be a full no-op.
        let report2 = sync_vault(&store, &dir, now + 1).unwrap();
        assert_eq!(report2.imported, 0);
        assert_eq!(report2.updated, 0);
        assert_eq!(report2.exported, 0);

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- reconcile_vault: the safety-critical 3-way sync ----

    /// Fresh, isolated temp dir for one reconcile test.
    fn reconcile_tmp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "magpie-reconcile-test-{}-{tag}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn reconcile_new_file_creates_note() {
        let store = crate::open_in_memory().unwrap();
        let dir = reconcile_tmp_dir("new-file");
        std::fs::write(dir.join("A.md"), "x").unwrap();

        let now = 1_000_000_000_000i64;
        let r = reconcile_vault(&store, &dir, now).unwrap();
        assert_eq!(r.created_notes, 1);
        let a = store.note_by_name("A").unwrap().unwrap();
        assert_eq!(a.body, "x");

        // Second reconcile with no external change: full no-op.
        let r2 = reconcile_vault(&store, &dir, now + 1).unwrap();
        assert_eq!(
            r2,
            ReconcileReport {
                unchanged: 1,
                ..Default::default()
            }
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reconcile_new_note_creates_file() {
        let store = crate::open_in_memory().unwrap();
        let dir = reconcile_tmp_dir("new-note");
        let n = store.upsert_note_by_name("B", 10).unwrap();
        store.update_note_body(n.id, "y", 20).unwrap();

        let now = 1_000_000_000_000i64;
        let r = reconcile_vault(&store, &dir, now).unwrap();
        assert!(r.created_files >= 1);
        let body = std::fs::read_to_string(dir.join("B.md")).unwrap();
        assert_eq!(body, "y");

        // Second reconcile with no external change: full no-op.
        let r2 = reconcile_vault(&store, &dir, now + 1).unwrap();
        assert_eq!(
            r2,
            ReconcileReport {
                unchanged: 1,
                ..Default::default()
            }
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reconcile_pulls_external_file_edit() {
        let store = crate::open_in_memory().unwrap();
        let dir = reconcile_tmp_dir("pull");
        std::fs::write(dir.join("A.md"), "x").unwrap();

        let now = 1_000_000_000_000i64;
        reconcile_vault(&store, &dir, now).unwrap(); // establishes the baseline

        // External edit only; note is untouched.
        std::fs::write(dir.join("A.md"), "x2").unwrap();
        let r = reconcile_vault(&store, &dir, now + 1).unwrap();
        assert_eq!(r.pulled, 1);
        let a = store.note_by_name("A").unwrap().unwrap();
        assert_eq!(a.body, "x2");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reconcile_pushes_note_edit_to_file() {
        let store = crate::open_in_memory().unwrap();
        let dir = reconcile_tmp_dir("push");
        let n = store.upsert_note_by_name("B", 10).unwrap();
        store.update_note_body(n.id, "y", 20).unwrap();

        let now = 1_000_000_000_000i64;
        reconcile_vault(&store, &dir, now).unwrap(); // establishes the baseline (creates B.md)

        // Note edit only; file is untouched.
        store.update_note_body(n.id, "y2", now + 1).unwrap();
        let r = reconcile_vault(&store, &dir, now + 2).unwrap();
        assert_eq!(r.pushed, 1);
        let body = std::fs::read_to_string(dir.join("B.md")).unwrap();
        assert_eq!(body, "y2");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reconcile_conflict_keeps_both_sides() {
        let store = crate::open_in_memory().unwrap();
        let dir = reconcile_tmp_dir("conflict");
        let n = store.upsert_note_by_name("C", 10).unwrap();
        store.update_note_body(n.id, "sync-me", 20).unwrap();

        let now = 1_000_000_000_000i64;
        reconcile_vault(&store, &dir, now).unwrap(); // establishes the baseline (creates C.md)

        // BOTH sides diverge from the agreed baseline.
        store.update_note_body(n.id, "db-version", now + 1).unwrap();
        std::fs::write(dir.join("C.md"), "file-version").unwrap();

        let r = reconcile_vault(&store, &dir, now + 2).unwrap();
        assert_eq!(r.conflicts, 1, "expected exactly one conflict");

        // The note takes the file's version...
        let c = store.note_by_name("C").unwrap().unwrap();
        assert_eq!(c.body, "file-version");

        // ...and the note's own version is preserved in a keep-both conflict file.
        let conflict_entry = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .find(|e| e.file_name().to_string_lossy().starts_with("C.conflict-"))
            .expect("a C.conflict-*.md file must exist");
        let conflict_body = std::fs::read_to_string(conflict_entry.path()).unwrap();
        assert_eq!(conflict_body, "db-version");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reconcile_is_idempotent_after_a_conflict() {
        let store = crate::open_in_memory().unwrap();
        let dir = reconcile_tmp_dir("idempotent");
        let n = store.upsert_note_by_name("D", 10).unwrap();
        store.update_note_body(n.id, "sync-me", 20).unwrap();

        let now = 1_000_000_000_000i64;
        reconcile_vault(&store, &dir, now).unwrap();

        store.update_note_body(n.id, "db-version", now + 1).unwrap();
        std::fs::write(dir.join("D.md"), "file-version").unwrap();
        let conflict_report = reconcile_vault(&store, &dir, now + 2).unwrap();
        assert_eq!(conflict_report.conflicts, 1);

        let files_before = std::fs::read_dir(&dir).unwrap().count();

        // A reconcile right after, with no further external change, must be a
        // full no-op and must NOT write a second conflict file.
        let r = reconcile_vault(&store, &dir, now + 3).unwrap();
        assert_eq!(
            r,
            ReconcileReport {
                unchanged: 1,
                ..Default::default()
            }
        );

        let files_after = std::fs::read_dir(&dir).unwrap().count();
        assert_eq!(
            files_before, files_after,
            "idempotent reconcile must not create a new conflict file"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
