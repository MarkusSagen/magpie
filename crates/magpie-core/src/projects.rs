//! Local projects (Phase 18): pure detection + the line-level checkbox toggle
//! primitive. The filesystem scan + two-way sync live in the app; everything
//! testable without IO is here. Files stay the source of truth — Magpie writes the
//! smallest possible patch (one line) and preserves everything else byte-for-byte.

use crate::merge3::{merge3, Conflict};
use crate::store::{Result, Store};
use rusqlite::OptionalExtension;

/// A marker that makes a folder a project.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Marker {
    /// A full `.magpie/` project directory.
    Magpie,
    /// A root `TODO.md` / `todo.md` / `TODO.txt`.
    Todo,
    /// A git/jj/hg repo (listed only, off by default).
    Repo,
    /// An editor/tool marker (`.vscode/`, `.idea/`, `.obsidian/`, …) — identifies
    /// the root + helps name it, never read as tasks.
    Editor,
}

/// Which entry names in a directory imply which markers. `names` is the set of
/// immediate child file/dir names. Order of the returned vec is stable
/// (Magpie, Todo, Repo, Editor).
pub fn markers_for(names: &[String]) -> Vec<Marker> {
    let has = |n: &str| names.iter().any(|x| x == n);
    let mut out = Vec::new();
    if has(".magpie") {
        out.push(Marker::Magpie);
    }
    if has("TODO.md") || has("todo.md") || has("TODO.txt") {
        out.push(Marker::Todo);
    }
    if has(".git") || has(".jj") || has(".hg") {
        out.push(Marker::Repo);
    }
    if has(".vscode")
        || has(".idea")
        || has(".obsidian")
        || has("logseq")
        || has(".projectile")
        || names.iter().any(|n| n.ends_with(".code-workspace"))
    {
        out.push(Marker::Editor);
    }
    out
}

impl Marker {
    /// Short badge label for the UI.
    pub fn badge(self) -> &'static str {
        match self {
            Marker::Magpie => ".magpie",
            Marker::Todo => "TODO.md",
            Marker::Repo => "git",
            Marker::Editor => "editor",
        }
    }
    /// A folder is a *synced* project (not merely "listed") if it has `.magpie/`
    /// or a root TODO file. A bare repo/editor marker only lists it.
    pub fn is_syncable(markers: &[Marker]) -> bool {
        markers
            .iter()
            .any(|m| matches!(m, Marker::Magpie | Marker::Todo))
    }
}

/// A URL/tag-safe slug for a project name (`Home renovation` → `home-renovation`).
pub fn slugify(name: &str) -> String {
    let mut out = String::new();
    let mut last_dash = true; // avoid leading dash
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

/// The `#proj/<slug>` tag for a project.
pub fn project_tag(name: &str) -> String {
    format!("proj/{}", slugify(name))
}

/// Flip a task-line body (leading indent already stripped) between open/done, or
/// `None` if it isn't a checkbox line.
fn toggled_marker(rest: &str) -> Option<String> {
    if let Some(after) = rest.strip_prefix("- [ ] ") {
        return Some(format!("- [x] {after}"));
    }
    if let Some(after) = rest.strip_prefix("- [/] ") {
        return Some(format!("- [x] {after}"));
    }
    if let Some(after) = rest
        .strip_prefix("- [x] ")
        .or_else(|| rest.strip_prefix("- [X] "))
    {
        return Some(format!("- [ ] {after}"));
    }
    None
}

/// Toggle the checkbox on line `line_index` of `content` between `- [ ]`/`- [/]`
/// and `- [x]`, rewriting ONLY that line (leading whitespace, trailing text, and
/// the file's other lines + final-newline state preserved). Returns the new
/// content, or `None` if the line isn't a checkbox task. This is the two-way
/// sync's line-level patch primitive.
pub fn toggle_task_line(content: &str, line_index: usize) -> Option<String> {
    let had_final_nl = content.ends_with('\n');
    let mut lines: Vec<String> = content.split('\n').map(|s| s.to_string()).collect();
    // `split('\n')` on a trailing-newline string yields a trailing "" — drop it so
    // indices match visible lines, and restore the newline at the end.
    if had_final_nl {
        lines.pop();
    }
    let line = lines.get_mut(line_index)?;
    let indent_len = line.len() - line.trim_start().len();
    let (indent, rest) = line.split_at(indent_len);
    *line = format!("{indent}{}", toggled_marker(rest)?);
    let mut joined = lines.join("\n");
    if had_final_nl {
        joined.push('\n');
    }
    Some(joined)
}

/// How to resolve overlapping edits when reconciling a project file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictPolicy {
    /// Hold the file: don't write, don't advance the baseline; surface the conflicts
    /// for Review (the file re-surfaces until resolved).
    Ask,
    /// The disk always wins a conflicting region; apply the merged result silently.
    FileWins,
}

/// The outcome of reconciling one file: what (if anything) to write to disk, what to
/// store as the new baseline, and any unresolved conflicts to surface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileReconcile {
    /// New content to write to disk. `None` = leave the file untouched (either
    /// nothing changed, or an unresolved `Ask` conflict is being held).
    pub write: Option<String>,
    /// New baseline content to persist. `None` = leave the stored baseline unchanged
    /// (an unresolved `Ask` conflict, so the divergence re-surfaces next run).
    pub new_baseline: Option<String>,
    /// Unresolved conflicts to surface for Review. Empty on a clean or `FileWins` run.
    pub conflicts: Vec<Conflict>,
}

/// Reconcile one project file with the line-level 3-way merge.
///
/// - `baseline`: the last content both sides agreed on (the merge pivot).
/// - `mine`: Magpie's intended content = `baseline` with its line patch applied
///   (equal to `baseline` when Magpie has no pending edit — a pure external pull).
/// - `disk`: the current bytes on disk.
///
/// A clean merge is written (only if it actually differs from `disk`, to avoid mtime
/// churn) and becomes the new baseline. On conflict, `FileWins` applies the
/// disk-preferring merge; `Ask` holds the file and returns the conflicts.
pub fn reconcile_file(
    baseline: &str,
    mine: &str,
    disk: &str,
    policy: ConflictPolicy,
) -> FileReconcile {
    let merged = merge3(baseline, mine, disk);
    if merged.conflicts.is_empty() || policy == ConflictPolicy::FileWins {
        // Clean, or File-wins (where `merged` already resolved conflicts to disk).
        let write = (merged.merged != disk).then(|| merged.merged.clone());
        FileReconcile {
            write,
            new_baseline: Some(merged.merged),
            conflicts: Vec::new(),
        }
    } else {
        // Ask + unresolved conflicts: hold the file, keep the old baseline.
        FileReconcile {
            write: None,
            new_baseline: None,
            conflicts: merged.conflicts,
        }
    }
}

impl Store {
    /// The stored sync baseline (last agreed content) for a project file, keyed by
    /// its project `root` and path relative to that root. `None` if never synced.
    pub fn project_baseline(&self, root: &str, rel_path: &str) -> Result<Option<String>> {
        self.conn()
            .query_row(
                "SELECT baseline FROM project_sync WHERE root = ?1 AND rel_path = ?2",
                rusqlite::params![root, rel_path],
                |r| r.get::<_, String>(0),
            )
            .optional()
    }

    /// Upsert the sync baseline for a project file after a successful reconcile.
    pub fn set_project_baseline(
        &self,
        root: &str,
        rel_path: &str,
        baseline: &str,
        now_ms: i64,
    ) -> Result<()> {
        let hash = blake3::hash(baseline.as_bytes()).to_hex().to_string();
        self.conn().execute(
            "INSERT INTO project_sync (root, rel_path, baseline, baseline_hash, updated_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(root, rel_path) DO UPDATE SET
                baseline = excluded.baseline,
                baseline_hash = excluded.baseline_hash,
                updated_at_ms = excluded.updated_at_ms",
            rusqlite::params![root, rel_path, baseline, hash, now_ms],
        )?;
        Ok(())
    }

    /// Forget a file's baseline (e.g. the user deleted the file and chose not to
    /// re-link). Never called as part of a normal sync — deletions are surfaced, not
    /// acted on automatically.
    pub fn forget_project_baseline(&self, root: &str, rel_path: &str) -> Result<()> {
        self.conn().execute(
            "DELETE FROM project_sync WHERE root = ?1 AND rel_path = ?2",
            rusqlite::params![root, rel_path],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_markers() {
        let names: Vec<String> = [".magpie", "TODO.md", ".git", "src"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let m = markers_for(&names);
        assert!(m.contains(&Marker::Magpie));
        assert!(m.contains(&Marker::Todo));
        assert!(m.contains(&Marker::Repo));
        assert!(Marker::is_syncable(&m));

        let repo_only: Vec<String> = [".git"].iter().map(|s| s.to_string()).collect();
        assert!(!Marker::is_syncable(&markers_for(&repo_only)));
    }

    #[test]
    fn slugs_and_tags() {
        assert_eq!(slugify("Home renovation"), "home-renovation");
        assert_eq!(slugify("magpie"), "magpie");
        assert_eq!(slugify("  A/B  c!"), "a-b-c");
        assert_eq!(project_tag("Home renovation"), "proj/home-renovation");
    }

    #[test]
    fn toggle_preserves_everything_else() {
        let content = "# Tasks\n\n- [ ] one @fri\n  - [ ] nested\n- [x] done\nplain line\n";
        // Toggle "one" (line 2).
        let out = toggle_task_line(content, 2).unwrap();
        assert_eq!(
            out,
            "# Tasks\n\n- [x] one @fri\n  - [ ] nested\n- [x] done\nplain line\n"
        );
        // Toggle the nested one (line 3) — indentation preserved.
        let out2 = toggle_task_line(&out, 3).unwrap();
        assert!(out2.contains("  - [x] nested"));
        // Toggle the done one back (line 4).
        let out3 = toggle_task_line(content, 4).unwrap();
        assert!(out3.contains("- [ ] done"));
        // A non-task line returns None.
        assert!(toggle_task_line(content, 5).is_none());
    }

    #[test]
    fn toggle_no_final_newline() {
        let content = "- [ ] a\n- [ ] b";
        let out = toggle_task_line(content, 1).unwrap();
        assert_eq!(out, "- [ ] a\n- [x] b");
        assert!(!out.ends_with('\n'));
    }

    // ---- reconcile_file (pure orchestration over merge3) ----

    #[test]
    fn reconcile_clean_pull_updates_baseline_not_disk() {
        // Magpie made no edit (mine == baseline); disk changed externally.
        let base = "- [ ] a\n- [ ] b\n";
        let disk = "- [x] a\n- [ ] b\n";
        let r = reconcile_file(base, base, disk, ConflictPolicy::Ask);
        assert_eq!(r.write, None, "disk already has the content; no write");
        assert_eq!(r.new_baseline.as_deref(), Some(disk), "baseline advances");
        assert!(r.conflicts.is_empty());
    }

    #[test]
    fn reconcile_clean_push_writes_disk() {
        // Magpie toggled; disk unchanged from baseline.
        let base = "- [ ] a\n- [ ] b\n";
        let mine = "- [ ] a\n- [x] b\n";
        let r = reconcile_file(base, mine, base, ConflictPolicy::Ask);
        assert_eq!(r.write.as_deref(), Some(mine));
        assert_eq!(r.new_baseline.as_deref(), Some(mine));
        assert!(r.conflicts.is_empty());
    }

    #[test]
    fn reconcile_clean_merge_of_disjoint_edits() {
        let base = "- [ ] a\n- [ ] b\n- [ ] c\n";
        let mine = "- [x] a\n- [ ] b\n- [ ] c\n"; // Magpie toggled line 0
        let disk = "- [ ] a\n- [ ] b\n- [ ] c edited\n"; // disk edited line 2
        let r = reconcile_file(base, mine, disk, ConflictPolicy::Ask);
        assert_eq!(
            r.write.as_deref(),
            Some("- [x] a\n- [ ] b\n- [ ] c edited\n")
        );
        assert!(r.conflicts.is_empty());
    }

    #[test]
    fn reconcile_ask_holds_on_conflict() {
        let base = "- [ ] task\n";
        let mine = "- [x] task\n"; // Magpie toggled
        let disk = "- [ ] task edited\n"; // disk edited same line
        let r = reconcile_file(base, mine, disk, ConflictPolicy::Ask);
        assert_eq!(r.write, None, "Ask holds the file");
        assert_eq!(r.new_baseline, None, "baseline is NOT advanced while held");
        assert_eq!(r.conflicts.len(), 1);
    }

    #[test]
    fn reconcile_file_wins_takes_disk_and_advances() {
        let base = "- [ ] task\n";
        let mine = "- [x] task\n";
        let disk = "- [ ] task edited\n";
        let r = reconcile_file(base, mine, disk, ConflictPolicy::FileWins);
        assert_eq!(r.write, None, "merged == disk here, so no write needed");
        assert_eq!(
            r.new_baseline.as_deref(),
            Some(disk),
            "baseline follows disk"
        );
        assert!(r.conflicts.is_empty(), "File-wins resolves the conflict");
    }

    #[test]
    fn reconcile_idempotent_when_all_agree() {
        let s = "- [ ] a\n";
        let r = reconcile_file(s, s, s, ConflictPolicy::Ask);
        assert_eq!(r.write, None);
        assert_eq!(r.new_baseline.as_deref(), Some(s));
        assert!(r.conflicts.is_empty());
    }

    #[test]
    fn project_baseline_roundtrips() {
        let store = crate::open_in_memory().unwrap();
        assert_eq!(store.project_baseline("/p", "TODO.md").unwrap(), None);
        store
            .set_project_baseline("/p", "TODO.md", "- [ ] a\n", 100)
            .unwrap();
        assert_eq!(
            store.project_baseline("/p", "TODO.md").unwrap().as_deref(),
            Some("- [ ] a\n")
        );
        // Upsert overwrites.
        store
            .set_project_baseline("/p", "TODO.md", "- [x] a\n", 200)
            .unwrap();
        assert_eq!(
            store.project_baseline("/p", "TODO.md").unwrap().as_deref(),
            Some("- [x] a\n")
        );
        // Forgetting clears it; other keys are unaffected.
        store
            .set_project_baseline("/p", "other.md", "x\n", 200)
            .unwrap();
        store.forget_project_baseline("/p", "TODO.md").unwrap();
        assert_eq!(store.project_baseline("/p", "TODO.md").unwrap(), None);
        assert_eq!(
            store.project_baseline("/p", "other.md").unwrap().as_deref(),
            Some("x\n")
        );
    }
}
