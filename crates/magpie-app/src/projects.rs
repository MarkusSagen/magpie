//! Local-project scanner (Phase 18): walks the configured roots with ripgrep's
//! `ignore` crate, detects project markers, reads tasks from `TODO.md` /
//! `.magpie/tasks.md`, and reads the git branch from `.git/HEAD` (no git binary).
//! Detection is efficient — it attributes marker files to their parent directory.

use ignore::WalkBuilder;
use magpie_core::projects::{markers_for, Marker};
use std::collections::BTreeMap;
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

/// Parse `- [ ]`/`- [x]` task lines out of a file's content (line-indexed).
fn read_tasks(file_abs: &Path, file_rel: &str) -> Vec<ProjTask> {
    let content = match std::fs::read_to_string(file_abs) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
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
pub fn scan(
    roots: &[String],
    depth: usize,
    include_repos: bool,
    ignored: &[String],
) -> Vec<ProjectHit> {
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
        let name = dir
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| display.clone());
        let mut tasks = Vec::new();
        if syncable {
            for (rel, abs) in [
                ("TODO.md", dir.join("TODO.md")),
                ("todo.md", dir.join("todo.md")),
                ("TODO.txt", dir.join("TODO.txt")),
                (".magpie/tasks.md", dir.join(".magpie/tasks.md")),
            ] {
                if abs.exists() {
                    tasks.extend(read_tasks(&abs, rel));
                }
            }
        }
        hits.push(ProjectHit {
            path: dir.clone(),
            name,
            markers,
            branch: git_branch(&dir),
            tasks,
        });
    }
    hits.sort_by_key(|h| h.name.to_lowercase());
    hits
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
