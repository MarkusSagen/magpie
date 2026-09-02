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
}
