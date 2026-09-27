//! Local-project scanner (Phase 18): walks the configured roots with ripgrep's
//! `ignore` crate, detects project markers, reads tasks from `TODO.md` /
//! `.magpie/tasks.md`, and reads the git branch from `.git/HEAD` (no git binary).
//! Detection is efficient — it attributes marker files to their parent directory.

use ignore::WalkBuilder;
use magpie_core::projects::{markers_for, Marker};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

/// One task read from a project file.
#[derive(Debug, Clone)]
pub struct ProjTask {
    pub title: String,
    pub done: bool,
    /// File shown in the UI, relative to the project root (`TODO.md`,
    /// `.magpie/tasks.md`).
    pub file_rel: String,
    pub file_abs: PathBuf,
    /// 0-based line index within the file (for `file:line` + line-level writes).
    pub line: usize,
}

/// A detected project.
#[derive(Debug, Clone)]
pub struct ProjectHit {
    pub path: PathBuf,
    pub name: String,
    pub markers: Vec<Marker>,
    pub branch: String,
    pub tasks: Vec<ProjTask>,
    /// Tags from `.magpie/project.toml` (Phase 18b), e.g. `["hobby"]`.
    pub tags: Vec<String>,
}

impl ProjectHit {
    pub fn open_tasks(&self) -> usize {
        self.tasks.iter().filter(|t| !t.done).count()
    }
    pub fn syncable(&self) -> bool {
        Marker::is_syncable(&self.markers)
    }
}

/// Read the current branch from `.git/HEAD` without shelling out to git.
fn git_branch(dir: &Path) -> String {
    let head = dir.join(".git/HEAD");
    if let Ok(s) = std::fs::read_to_string(&head) {
        if let Some(r) = s.trim().strip_prefix("ref: refs/heads/") {
            return r.to_string();
        }
        // Detached HEAD → short hash.
        return s.trim().chars().take(7).collect();
    }
    String::new()
}

/// Parse `- [ ]`/`- [x]` task lines out of a file's `content` (line-indexed). Pure so
/// the exact bytes the caller keeps as a sync snapshot match the parsed line indices.
fn parse_tasks(content: &str, file_abs: &Path, file_rel: &str) -> Vec<ProjTask> {
    let mut out = Vec::new();
    for (i, raw) in content.lines().enumerate() {
        let t = raw.trim_start();
        let (done, rest) = if let Some(r) = t.strip_prefix("- [ ] ") {
            (false, r)
        } else if let Some(r) = t.strip_prefix("- [/] ") {
            (false, r)
        } else if let Some(r) = t
            .strip_prefix("- [x] ")
            .or_else(|| t.strip_prefix("- [X] "))
        {
            (true, r)
        } else {
            continue;
        };
        out.push(ProjTask {
            title: rest.trim().to_string(),
            done,
            file_rel: file_rel.to_string(),
            file_abs: file_abs.to_path_buf(),
            line: i,
        });
    }
    out
}

fn is_marker_name(name: &str) -> bool {
    matches!(
        name,
        ".magpie"
            | "TODO.md"
            | "todo.md"
            | "TODO.txt"
            | ".git"
            | ".jj"
            | ".hg"
            | ".vscode"
            | ".idea"
            | ".obsidian"
            | "logseq"
            | ".projectile"
    ) || name.ends_with(".code-workspace")
}

/// Scan `roots` (to `depth`) and return detected projects, sorted by name.
/// `include_repos` keeps bare git/editor-only folders (off by default they'd bury
/// the real projects); `ignored` paths (display form) are excluded.
///
/// Also returns a snapshot map (`file_abs` → exact bytes read) for each syncable
/// task file, so the two-way sync can use the bytes the UI's line indices refer to
/// as the merge pivot (Phase 18b).
pub fn scan(
    roots: &[String],
    depth: usize,
    include_repos: bool,
    ignored: &[String],
) -> (Vec<ProjectHit>, HashMap<PathBuf, String>) {
    // dir → set of marker child names.
    let mut per_dir: BTreeMap<PathBuf, Vec<String>> = BTreeMap::new();
    for root in roots {
        let root = crate::file_search::expand_tilde(root);
        if !root.exists() {
            continue;
        }
        let walk = WalkBuilder::new(&root)
            .max_depth(Some(depth))
            .hidden(false) // marker files/dirs are dotfiles
            .git_ignore(true)
            .git_global(true)
            .build();
        for dent in walk.flatten() {
            let name = dent.file_name().to_string_lossy().to_string();
            if is_marker_name(&name) {
                if let Some(parent) = dent.path().parent() {
                    per_dir.entry(parent.to_path_buf()).or_default().push(name);
                }
            }
        }
    }

    let mut hits: Vec<ProjectHit> = Vec::new();
    let mut snapshots: HashMap<PathBuf, String> = HashMap::new();
    for (dir, names) in per_dir {
        let markers = markers_for(&names);
        if markers.is_empty() {
            continue;
        }
        let syncable = Marker::is_syncable(&markers);
        if !syncable && !include_repos {
            continue;
        }
        let display = display_path(&dir);
        if ignored.iter().any(|ig| ig == &display) {
            continue;
        }
        let folder_name = dir
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| display.clone());
        // `.magpie/project.toml` may override the display name + carry tags.
        let meta = std::fs::read_to_string(dir.join(".magpie/project.toml"))
            .ok()
            .map(|c| magpie_core::projects::parse_project_toml(&c))
            .unwrap_or_default();
        let name = meta.name.clone().unwrap_or(folder_name);
        let mut tasks = Vec::new();
        if syncable {
            // Dedupe by canonical path so a case-insensitive filesystem (macOS/Windows)
            // doesn't read the same `TODO.md` twice as `todo.md`.
            let mut seen: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
            for (rel, abs) in [
                ("TODO.md", dir.join("TODO.md")),
                ("todo.md", dir.join("todo.md")),
                ("TODO.txt", dir.join("TODO.txt")),
                (".magpie/tasks.md", dir.join(".magpie/tasks.md")),
            ] {
                if let Ok(canon) = std::fs::canonicalize(&abs) {
                    if !seen.insert(canon) {
                        continue; // already read under another case
                    }
                }
                if let Ok(content) = std::fs::read_to_string(&abs) {
                    tasks.extend(parse_tasks(&content, &abs, rel));
                    snapshots.insert(abs.clone(), content);
                }
            }
        }
        hits.push(ProjectHit {
            path: dir.clone(),
            name,
            markers,
            branch: git_branch(&dir),
            tasks,
            tags: meta.tags,
        });
    }
    hits.sort_by_key(|h| h.name.to_lowercase());
    (hits, snapshots)
}

/// Display a path with `~` for the home directory.
pub fn display_path(p: &Path) -> String {
    if let Some(home) = dirs::home_dir() {
        if let Ok(rel) = p.strip_prefix(&home) {
            return format!("~/{}", rel.display());
        }
    }
    p.display().to_string()
}

/// Keep `.magpie/` out of git for a project by appending it to `.git/info/exclude`
/// (a private, per-clone ignore — never touches a tracked `.gitignore`). No-op unless
/// the folder has both a `.git` dir and a `.magpie/` dir and doesn't already exclude
/// it. Returns true if the entry was added. Magpie never runs git itself.
pub fn ensure_git_excluded(project_dir: &Path) -> std::io::Result<bool> {
    let git = project_dir.join(".git");
    if !git.is_dir() || !project_dir.join(".magpie").is_dir() {
        return Ok(false);
    }
    let info = git.join("info");
    std::fs::create_dir_all(&info)?;
    let exclude = info.join("exclude");
    let existing = std::fs::read_to_string(&exclude).unwrap_or_default();
    if existing
        .lines()
        .any(|l| matches!(l.trim(), ".magpie/" | ".magpie"))
    {
        return Ok(false);
    }
    let mut out = existing;
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(".magpie/\n");
    std::fs::write(&exclude, out)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use magpie_core::projects::{reconcile_file, toggle_task_line, ConflictPolicy};

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("magpie-proj-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn ensure_git_excluded_appends_once() {
        let root = tmp("git-exclude");
        let proj = root.join("p");
        std::fs::create_dir_all(proj.join(".magpie")).unwrap();
        std::fs::create_dir_all(proj.join(".git/info")).unwrap();
        std::fs::write(proj.join(".git/info/exclude"), "# existing\n*.log\n").unwrap();

        assert!(ensure_git_excluded(&proj).unwrap(), "adds the entry");
        let body = std::fs::read_to_string(proj.join(".git/info/exclude")).unwrap();
        assert!(body.contains("\n.magpie/\n"));
        assert!(
            body.starts_with("# existing\n*.log\n"),
            "preserves existing"
        );
        // Idempotent.
        assert!(!ensure_git_excluded(&proj).unwrap(), "no double-add");
        assert_eq!(
            std::fs::read_to_string(proj.join(".git/info/exclude"))
                .unwrap()
                .matches(".magpie/")
                .count(),
            1
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn ensure_git_excluded_noop_without_git_or_magpie() {
        let root = tmp("git-exclude-none");
        let proj = root.join("p");
        std::fs::create_dir_all(proj.join(".magpie")).unwrap(); // no .git
        assert!(!ensure_git_excluded(&proj).unwrap());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn scan_reads_project_toml_name_and_tags() {
        let root = tmp("meta");
        let proj = root.join("folder-slug");
        std::fs::create_dir_all(proj.join(".magpie")).unwrap();
        std::fs::write(
            proj.join(".magpie/project.toml"),
            "name = \"Pretty Name\"\ntags = [\"hobby\"]\n",
        )
        .unwrap();
        std::fs::write(proj.join(".magpie/tasks.md"), "- [ ] a\n").unwrap();
        let (hits, _snap) = scan(&[root.to_string_lossy().to_string()], 4, true, &[]);
        let hit = hits
            .iter()
            .find(|h| h.name == "Pretty Name")
            .expect("name from project.toml");
        assert_eq!(hit.tags, vec!["hobby".to_string()]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn scan_captures_snapshot_and_tasks() {
        let root = tmp("scan-snap");
        let proj = root.join("acme");
        std::fs::create_dir_all(&proj).unwrap();
        let todo = proj.join("TODO.md");
        std::fs::write(&todo, "# Tasks\n- [ ] a\n- [x] b\n").unwrap();

        let (hits, snapshots) = scan(&[root.to_string_lossy().to_string()], 4, true, &[]);
        let _ = &todo;
        let hit = hits.iter().find(|h| h.name == "acme").expect("acme found");
        assert!(hit.syncable());
        assert_eq!(hit.tasks.len(), 2);
        // The snapshot is the exact bytes whose line indices the tasks reference. Look
        // it up by the scan's own path (temp dirs are symlinked on macOS).
        let a = hit.tasks.iter().find(|t| t.title == "a").unwrap();
        assert_eq!(a.line, 1); // "a" is on line 1 (0-based), matching the snapshot
        assert_eq!(
            snapshots.get(&a.file_abs).map(String::as_str),
            Some("# Tasks\n- [ ] a\n- [x] b\n")
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn snapshot_pivot_toggle_writes_disk() {
        // Mirrors toggle_project_task: pivot = snapshot, reconcile against fresh disk.
        let root = tmp("toggle-clean");
        let proj = root.join("p");
        std::fs::create_dir_all(&proj).unwrap();
        let todo = proj.join("TODO.md");
        std::fs::write(&todo, "- [ ] a\n- [ ] b\n").unwrap();

        let _ = &todo;
        let (_hits, snapshots) = scan(&[root.to_string_lossy().to_string()], 4, true, &[]);
        let (key, baseline) = snapshots.iter().next().unwrap();
        let baseline = baseline.clone();
        let disk = std::fs::read_to_string(key).unwrap();
        let mine = toggle_task_line(&baseline, 1).unwrap(); // toggle "b"
        let r = reconcile_file(&baseline, &mine, &disk, ConflictPolicy::Ask);
        assert!(r.conflicts.is_empty());
        std::fs::write(key, r.write.as_ref().unwrap()).unwrap();
        assert_eq!(std::fs::read_to_string(key).unwrap(), "- [ ] a\n- [x] b\n");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn stale_snapshot_conflicts_when_same_line_edited_on_disk() {
        // The UI showed the snapshot; disk edited the SAME line before the toggle.
        let root = tmp("toggle-conflict");
        let proj = root.join("p");
        std::fs::create_dir_all(&proj).unwrap();
        let todo = proj.join("TODO.md");
        std::fs::write(&todo, "- [ ] task\n").unwrap();

        let _ = &todo;
        let (_hits, snapshots) = scan(&[root.to_string_lossy().to_string()], 4, true, &[]);
        let (key, baseline) = snapshots.iter().next().unwrap();
        let (key, baseline) = (key.clone(), baseline.clone());
        // Disk edited the task's text after the scan.
        std::fs::write(&key, "- [ ] task renamed\n").unwrap();
        let disk = std::fs::read_to_string(&key).unwrap();
        let mine = toggle_task_line(&baseline, 0).unwrap();
        let r = reconcile_file(&baseline, &mine, &disk, ConflictPolicy::Ask);
        assert_eq!(r.conflicts.len(), 1, "same-line edit + toggle conflicts");
        assert!(r.write.is_none(), "Ask holds the file (no clobber)");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn disk_insert_above_toggle_merges_silently() {
        // Disk added a task at the top after the scan; Magpie toggled a lower task.
        let root = tmp("toggle-insert");
        let proj = root.join("p");
        std::fs::create_dir_all(&proj).unwrap();
        let todo = proj.join("TODO.md");
        std::fs::write(&todo, "- [ ] a\n- [ ] b\n").unwrap();

        let _ = &todo;
        let (_hits, snapshots) = scan(&[root.to_string_lossy().to_string()], 4, true, &[]);
        let (key, baseline) = snapshots.iter().next().unwrap();
        let (key, baseline) = (key.clone(), baseline.clone());
        std::fs::write(&key, "- [ ] zero\n- [ ] a\n- [ ] b\n").unwrap(); // inserted on disk
        let disk = std::fs::read_to_string(&key).unwrap();
        let mine = toggle_task_line(&baseline, 0).unwrap(); // toggle "a" (line 0 in snapshot)
        let r = reconcile_file(&baseline, &mine, &disk, ConflictPolicy::Ask);
        assert!(r.conflicts.is_empty(), "insert-above is non-overlapping");
        assert_eq!(
            r.write.as_deref(),
            Some("- [ ] zero\n- [x] a\n- [ ] b\n"),
            "both the disk insert and the toggle land"
        );

        let _ = std::fs::remove_dir_all(&root);
    }
}
