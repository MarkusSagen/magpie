//! One-way-import + write-back sync between the notes DB and a folder of Markdown
//! files ("vault"). Newest-wins per file, with a content-equality guard so repeated
//! syncs are idempotent (no timestamp ping-pong).
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
}
