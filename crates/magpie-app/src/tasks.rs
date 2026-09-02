//! Tasks: `- [ ]` / `- [x]` lines inside notes, with inline `!priority @due #project`.
//! Pure parsing + line edits; notes stay the source of truth.

use crate::format_time::parse_due;
use magpie_core::Note;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Priority {
    High,
    Medium,
    Low,
    None,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Task {
    pub note_id: i64,
    pub note_name: String,
    pub line_index: usize,
    pub done: bool,
    pub title: String,
    pub priority: Priority,
    pub due_ms: Option<i64>,
    pub project: Option<String>,
    pub source_app_id: Option<i64>,
    pub source_entry_id: Option<i64>,
}

/// If `trimmed` (leading whitespace already removed) is a task line, return
/// (done, rest-after-marker).
fn task_marker(trimmed: &str) -> Option<(bool, &str)> {
    if let Some(rest) = trimmed.strip_prefix("- [ ] ") {
        return Some((false, rest));
    }
    if let Some(rest) = trimmed
        .strip_prefix("- [x] ")
        .or_else(|| trimmed.strip_prefix("- [X] "))
    {
        return Some((true, rest));
    }
    if trimmed == "- [ ]" {
        return Some((false, ""));
    }
    if trimmed.eq_ignore_ascii_case("- [x]") {
        return Some((true, ""));
    }
    None
}

fn parse_priority(tok: &str) -> Option<Priority> {
    match tok.to_ascii_lowercase().as_str() {
        "!high" | "!hi" | "!h" | "!1" => Some(Priority::High),
        "!med" | "!medium" | "!m" | "!2" => Some(Priority::Medium),
        "!low" | "!lo" | "!l" | "!3" => Some(Priority::Low),
        _ => None,
    }
}

/// Parse every task line in a note, carrying the note's provenance onto each task.
pub fn parse_tasks_in(note: &Note, now_ms: i64) -> Vec<Task> {
    let mut out = Vec::new();
    for (i, raw) in note.body.lines().enumerate() {
        let (done, rest) = match task_marker(raw.trim_start()) {
            Some(x) => x,
            None => continue,
        };
        let mut priority = Priority::None;
        let mut due_ms = None;
        let mut project = None;
        let mut title_toks: Vec<&str> = Vec::new();
        for tok in rest.split_whitespace() {
            if priority == Priority::None {
                if let Some(p) = parse_priority(tok) {
                    priority = p;
                    continue;
                }
            }
            if due_ms.is_none() {
                if let Some(s) = tok.strip_prefix('@') {
                    if let Some(d) = parse_due(s, now_ms) {
                        due_ms = Some(d);
                        continue;
                    }
                }
            }
            if project.is_none() {
                if let Some(name) = tok.strip_prefix('#') {
                    if !name.is_empty() && !name.starts_with('#') {
                        project = Some(name.to_string());
                        continue;
                    }
                }
            }
            title_toks.push(tok);
        }
        out.push(Task {
            note_id: note.id,
            note_name: note.name.clone(),
            line_index: i,
            done,
            title: title_toks.join(" "),
            priority,
            due_ms,
            project,
            source_app_id: note.source_app_id,
            source_entry_id: note.source_entry_id,
        });
    }
    out
}

use magpie_core::Store;

/// Prepend `- [ ] ` to the line containing byte offset `cursor_byte` (preserving
/// indentation). No-op if that line is already a task.
pub fn promote_line(body: &str, cursor_byte: usize) -> String {
    let cb = cursor_byte.min(body.len());
    let start = body[..cb].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let end = body[cb..].find('\n').map(|i| cb + i).unwrap_or(body.len());
    let line = &body[start..end];
    if task_marker(line.trim_start()).is_some() {
        return body.to_string();
    }
    let indent = line.len() - line.trim_start().len();
    let (ind, content) = line.split_at(indent);
    format!("{}{ind}- [ ] {content}{}", &body[..start], &body[end..])
}

/// Flip the checkbox on line `line_index`. No-op if it's not a task line.
pub fn toggle_line(body: &str, line_index: usize) -> String {
    let mut lines: Vec<String> = body.split('\n').map(|s| s.to_string()).collect();
    if line_index >= lines.len() {
        return body.to_string();
    }
    let line = &lines[line_index];
    let indent = line.len() - line.trim_start().len();
    let (ind, rest) = line.split_at(indent);
    let flipped = if let Some(r) = rest.strip_prefix("- [ ]") {
        Some(format!("{ind}- [x]{r}"))
    } else {
        rest.strip_prefix("- [x]")
            .or_else(|| rest.strip_prefix("- [X]"))
            .map(|r| format!("{ind}- [ ]{r}"))
    };
    if let Some(f) = flipped {
        lines[line_index] = f;
    }
    lines.join("\n")
}

/// Toggle a task's done state and persist it to its note. Returns whether it changed.
pub fn toggle_task(store: &Store, note_id: i64, line_index: usize, now_ms: i64) -> bool {
    let body = match store.get_note(note_id) {
        Ok(Some(n)) => n.body,
        _ => return false,
    };
    let new = toggle_line(&body, line_index);
    new != body
        && store
            .update_note_body(note_id, &new, now_ms)
            .unwrap_or(false)
}

/// Every task across all notes.
pub fn all_tasks(store: &Store, now_ms: i64) -> Vec<Task> {
    store
        .all_notes()
        .unwrap_or_default()
        .iter()
        .flat_map(|n| parse_tasks_in(n, now_ms))
        .collect()
}

pub struct TaskGroup {
    pub project: String,
    pub tasks: Vec<Task>,
}

fn cmp_due(a: Option<i64>, b: Option<i64>) -> std::cmp::Ordering {
    match (a, b) {
        (Some(x), Some(y)) => x.cmp(&y),
        (Some(_), None) => std::cmp::Ordering::Less, // scheduled before unscheduled
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    }
}

/// Group by project (alphabetical; "No project" last), sort within: open before
/// done, then due ascending (unscheduled last), then priority (High first), title.
pub fn group_sort(tasks: Vec<Task>) -> Vec<TaskGroup> {
    use std::collections::BTreeMap;
    let mut map: BTreeMap<String, Vec<Task>> = BTreeMap::new();
    for t in tasks {
        let key = t
            .project
            .clone()
            .unwrap_or_else(|| "No project".to_string());
        map.entry(key).or_default().push(t);
    }
    let mut groups = Vec::new();
    let mut no_project = None;
    for (project, mut ts) in map {
        ts.sort_by(|a, b| {
            a.done
                .cmp(&b.done)
                .then_with(|| cmp_due(a.due_ms, b.due_ms))
                .then(a.priority.cmp(&b.priority))
                .then_with(|| a.title.cmp(&b.title))
        });
        if project == "No project" {
            no_project = Some(ts);
        } else {
            groups.push(TaskGroup { project, tasks: ts });
        }
    }
    if let Some(ts) = no_project {
        groups.push(TaskGroup {
            project: "No project".to_string(),
            tasks: ts,
        });
    }
    groups
}

/// Append a new `- [ ] <text>` task line to `body`. No-op if `text` is blank.
pub fn append_task_line(body: &str, text: &str) -> String {
    let t = text.trim();
    if t.is_empty() {
        return body.to_string();
    }
    let sep = if body.is_empty() || body.ends_with('\n') {
        ""
    } else {
        "\n"
    };
    format!("{body}{sep}- [ ] {t}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use magpie_core::Note;
    fn note(body: &str) -> Note {
        Note {
            id: 1,
            name: "Daily".into(),
            is_daily: true,
            body: body.into(),
            created_at_ms: 0,
            updated_at_ms: 0,
            source_app_id: Some(7),
            source_entry_id: None,
        }
    }
    #[test]
    fn parses_checkbox_priority_due_project_and_title() {
        const DAY: i64 = 86_400_000;
        let now = 20_000 * DAY;
        let n = note("intro\n- [ ] Fix mount !high @tomorrow #prod now\n- [x] done thing\nplain");
        let ts = parse_tasks_in(&n, now);
        assert_eq!(ts.len(), 2);
        assert_eq!(ts[0].title, "Fix mount now");
        assert_eq!(ts[0].priority, Priority::High);
        assert_eq!(ts[0].due_ms, Some(20_001 * DAY));
        assert_eq!(ts[0].project.as_deref(), Some("prod"));
        assert!(!ts[0].done);
        assert_eq!(ts[0].line_index, 1);
        assert_eq!(ts[0].source_app_id, Some(7));
        assert!(ts[1].done && ts[1].title == "done thing");
    }

    #[test]
    fn promote_and_toggle_lines() {
        let body = "note title\nfix the bug\ndone already";
        // cursor somewhere in "fix the bug" (line 1)
        let cb = body.find("fix").unwrap() + 1;
        let promoted = promote_line(body, cb);
        assert!(promoted.contains("- [ ] fix the bug"));
        assert_eq!(promote_line(&promoted, cb), promoted); // no double-add
                                                           // toggle that now-task line (index 1)
        let toggled = toggle_line(&promoted, 1);
        assert!(toggled.contains("- [x] fix the bug"));
        assert!(toggle_line(&toggled, 1).contains("- [ ] fix the bug"));
        assert_eq!(toggle_line(body, 0), body); // non-task line: no-op
    }

    #[test]
    fn append_task_line_adds_checkbox() {
        assert_eq!(append_task_line("", "buy milk"), "- [ ] buy milk");
        assert_eq!(append_task_line("note", "buy milk"), "note\n- [ ] buy milk");
        assert_eq!(
            append_task_line("note\n", "  buy milk "),
            "note\n- [ ] buy milk"
        );
        assert_eq!(append_task_line("note", "   "), "note"); // empty text: no-op
    }

    #[test]
    fn group_sort_orders_open_due_priority() {
        let mk =
            |title: &str, done: bool, due: Option<i64>, p: Priority, proj: Option<&str>| Task {
                note_id: 1,
                note_name: "n".into(),
                line_index: 0,
                done,
                title: title.into(),
                priority: p,
                due_ms: due,
                project: proj.map(|s| s.into()),
                source_app_id: None,
                source_entry_id: None,
            };
        let tasks = vec![
            mk("done", true, None, Priority::High, Some("a")),
            mk("later", false, Some(200), Priority::Low, Some("a")),
            mk("soon", false, Some(100), Priority::Low, Some("a")),
            mk("noproj", false, None, Priority::High, None),
        ];
        let g = group_sort(tasks);
        // "a" group first, "No project" last
        assert_eq!(g[0].project, "a");
        assert_eq!(g.last().unwrap().project, "No project");
        // within "a": open before done, due asc
        let titles: Vec<&str> = g[0].tasks.iter().map(|t| t.title.as_str()).collect();
        assert_eq!(titles, vec!["soon", "later", "done"]);
    }
}
