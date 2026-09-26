//! Local projects (Phase 18): pure detection + the line-level checkbox toggle
//! primitive. The filesystem scan + two-way sync live in the app; everything
//! testable without IO is here. Files stay the source of truth — Magpie writes the
//! smallest possible patch (one line) and preserves everything else byte-for-byte.

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
}
