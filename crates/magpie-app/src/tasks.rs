//! Tasks: `- [ ]` / `- [x]` lines inside notes, with inline `!priority @due #project`.
//! Pure parsing + line edits; notes stay the source of truth.

use crate::format_time::{abs_date, civil_from_days, days_from_civil, parse_due, parse_time};
use magpie_core::Note;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Priority {
    High,
    Medium,
    Low,
    None,
}

/// A task's kanban-style status. `- [ ]` = Todo, `- [/]` = Doing (the
/// Obsidian/Logseq "in progress" convention), `- [x]`/`- [X]` = Done.
/// `Task::done` stays derived from this (`status == Done`) so existing
/// reminders/grouping code keeps working unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Todo,
    Doing,
    Done,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecurUnit {
    Day,
    Week,
    Month,
    Year,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Recur {
    pub n: i64,
    pub unit: RecurUnit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Task {
    pub note_id: i64,
    pub note_name: String,
    pub line_index: usize,
    pub done: bool,
    pub status: Status,
    pub title: String,
    pub priority: Priority,
    pub due_ms: Option<i64>,
    pub due_time_min: Option<i64>,
    pub recur: Option<Recur>,
    pub bookmarked: bool,
    pub project: Option<String>,
    pub source_app_id: Option<i64>,
    pub source_entry_id: Option<i64>,
}

/// If `trimmed` (leading whitespace already removed) is a task line, return
/// (status, rest-after-marker). Recognizes the Obsidian/Logseq "in progress"
/// `- [/]` convention alongside the standard `- [ ]`/`- [x]`.
fn task_marker(trimmed: &str) -> Option<(Status, &str)> {
    if let Some(rest) = trimmed.strip_prefix("- [ ] ") {
        return Some((Status::Todo, rest));
    }
    if let Some(rest) = trimmed.strip_prefix("- [/] ") {
        return Some((Status::Doing, rest));
    }
    if let Some(rest) = trimmed
        .strip_prefix("- [x] ")
        .or_else(|| trimmed.strip_prefix("- [X] "))
    {
        return Some((Status::Done, rest));
    }
    if trimmed == "- [ ]" {
        return Some((Status::Todo, ""));
    }
    if trimmed == "- [/]" {
        return Some((Status::Doing, ""));
    }
    if trimmed.eq_ignore_ascii_case("- [x]") {
        return Some((Status::Done, ""));
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
        let (status, rest) = match task_marker(raw.trim_start()) {
            Some(x) => x,
            None => continue,
        };
        let done = status == Status::Done;
        let mut priority = Priority::None;
        let mut due_ms = None;
        let mut project = None;
        let mut title_toks: Vec<&str> = Vec::new();
        let mut due_time_min: Option<i64> = None;
        let mut recur: Option<Recur> = None;
        let mut bookmarked = false;
        let mut toks = rest.split_whitespace().peekable();
        while let Some(tok) = toks.next() {
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
                        if let Some(next) = toks.peek() {
                            if let Some(tm) = parse_time(next) {
                                due_time_min = Some(tm);
                                toks.next();
                            }
                        }
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
            if recur.is_none() {
                if let Some(r) = parse_recur(tok) {
                    recur = Some(r);
                    continue;
                }
            }
            if !bookmarked && tok == "*" {
                bookmarked = true;
                continue;
            }
            title_toks.push(tok);
        }
        out.push(Task {
            note_id: note.id,
            note_name: note.name.clone(),
            line_index: i,
            done,
            status,
            title: title_toks.join(" "),
            priority,
            due_ms,
            due_time_min,
            recur,
            bookmarked,
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
/// `[ ]`/`[/]` → `[x]` (checking a to-do or in-progress task completes it);
/// `[x]`/`[X]` → `[ ]`.
pub fn toggle_line(body: &str, line_index: usize) -> String {
    let mut lines: Vec<String> = body.split('\n').map(|s| s.to_string()).collect();
    if line_index >= lines.len() {
        return body.to_string();
    }
    let line = &lines[line_index];
    let indent = line.len() - line.trim_start().len();
    let (ind, rest) = line.split_at(indent);
    let flipped = if let Some(r) = rest
        .strip_prefix("- [ ]")
        .or_else(|| rest.strip_prefix("- [/]"))
    {
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
    let new = {
        let lines: Vec<&str> = body.split('\n').collect();
        match lines
            .get(line_index)
            .and_then(|l| reschedule_line(l, now_ms))
        {
            Some(resched) => {
                let mut v: Vec<String> = body.split('\n').map(str::to_string).collect();
                v[line_index] = resched;
                v.join("\n")
            }
            None => toggle_line(&body, line_index),
        }
    };
    new != body
        && store
            .update_note_body(note_id, &new, now_ms)
            .unwrap_or(false)
}

/// Rewrite the checkbox marker on `line_index` to `status`. No-op if that line
/// isn't a task line. Preserves indentation and everything else on the line
/// (priority/due/#project/title) — only the ` `/`/`/`x` inside `[ ]` changes.
pub fn set_line_status(body: &str, line_index: usize, status: Status) -> String {
    let mut lines: Vec<String> = body.split('\n').map(|s| s.to_string()).collect();
    let Some(line) = lines.get(line_index) else {
        return body.to_string();
    };
    let indent = line.len() - line.trim_start().len();
    let (ind, rest) = line.split_at(indent);
    let glyph = match status {
        Status::Todo => ' ',
        Status::Doing => '/',
        Status::Done => 'x',
    };
    let new_rest = rest
        .strip_prefix("- [ ]")
        .or_else(|| rest.strip_prefix("- [/]"))
        .or_else(|| rest.strip_prefix("- [x]"))
        .or_else(|| rest.strip_prefix("- [X]"))
        .map(|r| format!("- [{glyph}]{r}"));
    if let Some(r) = new_rest {
        lines[line_index] = format!("{ind}{r}");
    }
    lines.join("\n")
}

/// Persist a status change on a task line. Returns whether the note changed.
pub fn set_task_status(
    store: &Store,
    note_id: i64,
    line_index: usize,
    status: Status,
    now_ms: i64,
) -> bool {
    let body = match store.get_note(note_id) {
        Ok(Some(n)) => n.body,
        _ => return false,
    };
    let new = set_line_status(&body, line_index, status);
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

/// How to cluster tasks into board columns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupBy {
    Status,
    Priority,
    Project,
}

/// How to order tasks within a board column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortBy {
    Due,
    Priority,
    Title,
    Status,
}

/// Filters applied before grouping. `project` matches a task's exact `#project`
/// (or "No project" for tasks with none); `query` is a case-insensitive
/// substring match on `title` ("" = no query); `hide_done` drops completed tasks.
#[derive(Debug, Clone, Default)]
pub struct BoardFilter {
    pub project: Option<String>,
    pub query: String,
    pub hide_done: bool,
}

/// One column of the task board: a header title plus its ordered tasks.
pub struct BoardColumn {
    pub title: String,
    pub tasks: Vec<Task>,
}

/// Todo < Doing < Done, for `SortBy::Status`.
fn status_rank(s: Status) -> u8 {
    match s {
        Status::Todo => 0,
        Status::Doing => 1,
        Status::Done => 2,
    }
}

fn title_ci(t: &Task) -> String {
    t.title.to_lowercase()
}

fn sort_tasks(tasks: &mut [Task], sort_by: SortBy) {
    match sort_by {
        SortBy::Due => tasks.sort_by(|a, b| {
            cmp_due(a.due_ms, b.due_ms)
                .then_with(|| a.priority.cmp(&b.priority))
                .then_with(|| title_ci(a).cmp(&title_ci(b)))
        }),
        SortBy::Priority => tasks.sort_by(|a, b| {
            a.priority
                .cmp(&b.priority)
                .then_with(|| cmp_due(a.due_ms, b.due_ms))
                .then_with(|| title_ci(a).cmp(&title_ci(b)))
        }),
        SortBy::Title => tasks.sort_by(|a, b| {
            title_ci(a)
                .cmp(&title_ci(b))
                .then_with(|| cmp_due(a.due_ms, b.due_ms))
        }),
        SortBy::Status => tasks.sort_by(|a, b| {
            status_rank(a.status)
                .cmp(&status_rank(b.status))
                .then_with(|| cmp_due(a.due_ms, b.due_ms))
                .then_with(|| title_ci(a).cmp(&title_ci(b)))
        }),
    }
}

fn apply_board_filter(tasks: Vec<Task>, filter: &BoardFilter) -> Vec<Task> {
    let query = filter.query.to_lowercase();
    tasks
        .into_iter()
        .filter(|t| !filter.hide_done || t.status != Status::Done)
        .filter(|t| match &filter.project {
            None => true,
            Some(p) => t.project.as_deref().unwrap_or("No project") == p.as_str(),
        })
        .filter(|t| query.is_empty() || t.title.to_lowercase().contains(&query))
        .collect()
}

/// Filter → group → sort tasks into ordered board columns. `Status` and
/// `Priority` groupings always yield their full fixed set of columns (empty
/// ones included, since Kanban needs somewhere to drop a card); `Project`
/// yields one column per distinct project actually present, alphabetical,
/// with "No project" last (mirrors `group_sort`'s project ordering).
pub fn board_columns(
    tasks: Vec<Task>,
    group_by: GroupBy,
    sort_by: SortBy,
    filter: &BoardFilter,
) -> Vec<BoardColumn> {
    let filtered = apply_board_filter(tasks, filter);
    let mut columns = match group_by {
        GroupBy::Status => {
            let mut cols: Vec<BoardColumn> = ["To do", "Doing", "Done"]
                .into_iter()
                .map(|title| BoardColumn {
                    title: title.to_string(),
                    tasks: Vec::new(),
                })
                .collect();
            for t in filtered {
                cols[status_rank(t.status) as usize].tasks.push(t);
            }
            cols
        }
        GroupBy::Priority => {
            let mut cols: Vec<BoardColumn> = ["High", "Medium", "Low", "No priority"]
                .into_iter()
                .map(|title| BoardColumn {
                    title: title.to_string(),
                    tasks: Vec::new(),
                })
                .collect();
            for t in filtered {
                let idx = match t.priority {
                    Priority::High => 0,
                    Priority::Medium => 1,
                    Priority::Low => 2,
                    Priority::None => 3,
                };
                cols[idx].tasks.push(t);
            }
            cols
        }
        GroupBy::Project => {
            use std::collections::BTreeMap;
            let mut map: BTreeMap<String, Vec<Task>> = BTreeMap::new();
            for t in filtered {
                let key = t
                    .project
                    .clone()
                    .unwrap_or_else(|| "No project".to_string());
                map.entry(key).or_default().push(t);
            }
            let mut cols = Vec::new();
            let mut no_project = None;
            for (project, ts) in map {
                if project == "No project" {
                    no_project = Some(ts);
                } else {
                    cols.push(BoardColumn {
                        title: project,
                        tasks: ts,
                    });
                }
            }
            if let Some(ts) = no_project {
                cols.push(BoardColumn {
                    title: "No project".to_string(),
                    tasks: ts,
                });
            }
            cols
        }
    };
    for col in &mut columns {
        sort_tasks(&mut col.tasks, sort_by);
    }
    columns
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

/// Open tasks that have a due date, split relative to `today_ms` (a day-epoch,
/// midnight-UTC ms). `overdue` is strictly before today; `today` is exactly today.
/// Tasks due later, with no due date, or already done are excluded. Each bucket is
/// sorted by (due date, priority).
pub struct DueBuckets {
    pub overdue: Vec<Task>,
    pub today: Vec<Task>,
}

pub fn partition_due(tasks: Vec<Task>, today_ms: i64) -> DueBuckets {
    let mut overdue = Vec::new();
    let mut today = Vec::new();
    for t in tasks {
        if t.done {
            continue;
        }
        match t.due_ms {
            Some(d) if d < today_ms => overdue.push(t),
            Some(d) if d == today_ms => today.push(t),
            _ => {}
        }
    }
    let by = |a: &Task, b: &Task| a.due_ms.cmp(&b.due_ms).then(a.priority.cmp(&b.priority));
    overdue.sort_by(by);
    today.sort_by(by);
    DueBuckets { overdue, today }
}

/// Parse a `+<n><unit>` recurrence token (unit d|w|mo|y). `None` if not a valid rule
/// (a bare `+1`, `+0d`, or non-numeric stays in the task title).
pub fn parse_recur(tok: &str) -> Option<Recur> {
    let s = tok.strip_prefix('+')?;
    // Try the two-char unit ("mo") before the single-char ones so "1mo" isn't read
    // as "1m" + leftover.
    let (num, unit) = [
        ("mo", RecurUnit::Month),
        ("d", RecurUnit::Day),
        ("w", RecurUnit::Week),
        ("y", RecurUnit::Year),
    ]
    .into_iter()
    .find_map(|(suffix, unit)| s.strip_suffix(suffix).map(|n| (n, unit)))?;
    let n: i64 = num.parse().ok()?;
    if n <= 0 {
        return None;
    }
    Some(Recur { n, unit })
}

fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

fn days_in_month(y: i64, m: i64) -> i64 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if is_leap(y) {
                29
            } else {
                28
            }
        }
        _ => 30,
    }
}

/// One recurrence step forward from a day-epoch (days since 1970-01-01). Month/year
/// clamp the day to the target month length (Jan 31 +1mo → Feb 28/29).
pub fn add_recur(day_epoch: i64, r: Recur) -> i64 {
    match r.unit {
        RecurUnit::Day => day_epoch + r.n,
        RecurUnit::Week => day_epoch + r.n * 7,
        RecurUnit::Month => {
            let (y, m, d) = civil_from_days(day_epoch);
            let total = (m as i64 - 1) + r.n;
            let ny = y + total.div_euclid(12);
            let nm = total.rem_euclid(12) + 1;
            let nd = (d as i64).min(days_in_month(ny, nm));
            days_from_civil(ny, nm, nd)
        }
        RecurUnit::Year => {
            let (y, m, d) = civil_from_days(day_epoch);
            let ny = y + r.n;
            let nd = (d as i64).min(days_in_month(ny, m as i64));
            days_from_civil(ny, m as i64, nd)
        }
    }
}

/// Next occurrence strictly after `today_days`, stepping by the rule from `due_days`.
pub fn next_occurrence(due_days: i64, r: Recur, today_days: i64) -> i64 {
    let mut next = add_recur(due_days, r);
    while next <= today_days {
        next = add_recur(next, r);
    }
    next
}

/// If `line` is an OPEN recurring task with a due date, return it rescheduled to the
/// next future occurrence (still `- [ ]`, `@due` rewritten as absolute YYYY-MM-DD).
/// `None` otherwise (not a task, already done, no due, or no recurrence rule).
pub fn reschedule_line(line: &str, now_ms: i64) -> Option<String> {
    let (status, rest) = task_marker(line.trim_start())?;
    if status == Status::Done {
        return None;
    }
    let mut due_ms: Option<i64> = None;
    let mut due_token: Option<String> = None;
    let mut recur: Option<Recur> = None;
    let mut toks = rest.split_whitespace().peekable();
    while let Some(t) = toks.next() {
        if due_ms.is_none() {
            if let Some(s) = t.strip_prefix('@') {
                if let Some(d) = parse_due(s, now_ms) {
                    due_ms = Some(d);
                    due_token = Some(s.to_string());
                    if let Some(nx) = toks.peek() {
                        if parse_time(nx).is_some() {
                            toks.next();
                        }
                    }
                    continue;
                }
            }
        }
        if recur.is_none() {
            if let Some(r) = parse_recur(t) {
                recur = Some(r);
                continue;
            }
        }
    }
    let (due_ms, due_token, recur) = (due_ms?, due_token?, recur?);
    const DAY_MS: i64 = 86_400_000;
    let today = now_ms.div_euclid(DAY_MS);
    let next_days = next_occurrence(due_ms.div_euclid(DAY_MS), recur, today);
    let new = format!("@{}", abs_date(next_days * DAY_MS));
    Some(line.replacen(&format!("@{due_token}"), &new, 1))
}

/// Add or remove a standalone `*` bookmark marker on the task line at `line_index`.
/// No-op if the line isn't a task line.
pub fn toggle_bookmark_line(body: &str, line_index: usize) -> String {
    let mut lines: Vec<String> = body.split('\n').map(str::to_string).collect();
    let Some(line) = lines.get(line_index) else {
        return body.to_string();
    };
    if task_marker(line.trim_start()).is_none() {
        return body.to_string();
    }
    let has = line.split_whitespace().any(|t| t == "*");
    let new = if has {
        let kept: Vec<&str> = line.split(' ').filter(|t| *t != "*").collect();
        kept.join(" ").trim_end().to_string()
    } else {
        format!("{} *", line.trim_end())
    };
    lines[line_index] = new;
    lines.join("\n")
}

/// Toggle a task's bookmark and persist it to its note. Returns whether it changed.
pub fn set_bookmark(store: &Store, note_id: i64, line_index: usize, now_ms: i64) -> bool {
    let body = match store.get_note(note_id) {
        Ok(Some(n)) => n.body,
        _ => return false,
    };
    let new = toggle_bookmark_line(&body, line_index);
    new != body
        && store
            .update_note_body(note_id, &new, now_ms)
            .unwrap_or(false)
}

/// Format epoch-ms (shifted by `offset_secs`) as local `YYYY-MM-DD HH:MM`.
fn local_stamp(ms: i64, offset_secs: i64) -> String {
    const DAY_MS: i64 = 86_400_000;
    let local_ms = ms + offset_secs * 1000;
    let secs_of_day = local_ms.div_euclid(1000).rem_euclid(86_400);
    let (y, m, d) = civil_from_days(local_ms.div_euclid(DAY_MS));
    let h = secs_of_day / 3600;
    let mi = (secs_of_day % 3600) / 60;
    format!("{y:04}-{m:02}-{d:02} {h:02}:{mi:02}")
}

/// Format a duration in ms as org's `H:MM` (hours unbounded, minutes zero-padded).
/// Negative durations clamp to `0:00`.
fn duration_hm(ms: i64) -> String {
    let mins = (ms.max(0)) / 60_000;
    format!("{}:{:02}", mins / 60, mins % 60)
}

/// Insert an org-mode CLOCK entry for a completed timer under the task line whose
/// title matches `title`, into its `:LOGBOOK:` drawer (created if absent), indented
/// two spaces past the task line's own indent. Newest CLOCK line goes first (right
/// after `:LOGBOOK:`). Returns the new body; returns `body` unchanged if no matching
/// task line is found.
pub fn log_clock(body: &str, title: &str, start_ms: i64, end_ms: i64, offset_secs: i64) -> String {
    // Reuse the real task parser (any `now_ms` works for title-matching purposes:
    // due-token stripping doesn't depend on it — relative tokens like `@today`
    // always resolve, and absolute `@YYYY-MM-DD` tokens don't consult it either).
    let dummy = Note {
        id: 0,
        name: String::new(),
        is_daily: false,
        body: body.to_string(),
        created_at_ms: 0,
        updated_at_ms: 0,
        source_app_id: None,
        source_entry_id: None,
    };
    let Some(task_idx) = parse_tasks_in(&dummy, end_ms)
        .into_iter()
        .find(|t| t.title == title)
        .map(|t| t.line_index)
    else {
        return body.to_string();
    };
    let lines: Vec<&str> = body.split('\n').collect();
    let task_line = lines[task_idx];
    let indent_len = task_line.len() - task_line.trim_start().len();
    let indent = &task_line[..indent_len];
    let drawer_indent = format!("{indent}  ");

    let clock_line = format!(
        "{drawer_indent}CLOCK: [{}]--[{}] => {}",
        local_stamp(start_ms, offset_secs),
        local_stamp(end_ms, offset_secs),
        duration_hm(end_ms - start_ms)
    );

    let mut out: Vec<String> = lines.iter().map(|s| s.to_string()).collect();
    let has_drawer = out
        .get(task_idx + 1)
        .map(|l| l.trim() == ":LOGBOOK:")
        .unwrap_or(false);
    if has_drawer {
        out.insert(task_idx + 2, clock_line);
    } else {
        out.insert(task_idx + 1, format!("{drawer_indent}:LOGBOOK:"));
        out.insert(task_idx + 2, clock_line);
        out.insert(task_idx + 3, format!("{drawer_indent}:END:"));
    }
    out.join("\n")
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
    fn parses_doing_status_alongside_todo_and_done() {
        let n = note("- [ ] a\n- [/] b\n- [x] c");
        let ts = parse_tasks_in(&n, 0);
        assert_eq!(ts.len(), 3);
        assert_eq!(
            ts.iter().map(|t| t.status).collect::<Vec<_>>(),
            vec![Status::Todo, Status::Doing, Status::Done]
        );
        assert_eq!(
            ts.iter().map(|t| t.done).collect::<Vec<_>>(),
            vec![false, false, true]
        );
        assert_eq!(ts[1].title, "b");
    }

    #[test]
    fn doing_marker_title_strips_priority_and_project() {
        let n = note("- [/] Fix mount !high #prod");
        let ts = parse_tasks_in(&n, 0);
        assert_eq!(ts[0].status, Status::Doing);
        assert_eq!(ts[0].title, "Fix mount");
        assert_eq!(ts[0].priority, Priority::High);
        assert_eq!(ts[0].project.as_deref(), Some("prod"));
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
    fn toggle_line_completes_a_doing_task() {
        let body = "- [/] in progress";
        assert_eq!(toggle_line(body, 0), "- [x] in progress");
    }

    #[test]
    fn set_line_status_swaps_marker_and_preserves_rest() {
        assert_eq!(set_line_status("- [ ] a", 0, Status::Doing), "- [/] a");
        assert_eq!(set_line_status("  - [ ] x", 0, Status::Doing), "  - [/] x");
        assert_eq!(
            set_line_status("- [ ] a !high #proj", 0, Status::Doing),
            "- [/] a !high #proj"
        );
        assert_eq!(set_line_status("- [/] a", 0, Status::Done), "- [x] a");
        assert_eq!(set_line_status("- [x] a", 0, Status::Todo), "- [ ] a");
        assert_eq!(
            set_line_status("plain line", 0, Status::Doing),
            "plain line"
        );
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
                status: if done { Status::Done } else { Status::Todo },
                title: title.into(),
                priority: p,
                due_ms: due,
                due_time_min: None,
                recur: None,
                bookmarked: false,
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

    /// Full-field `Task` builder for board-column tests (avoids repeating every
    /// field for scenarios that only care about a couple of them).
    fn bt(
        title: &str,
        status: Status,
        priority: Priority,
        due: Option<i64>,
        project: Option<&str>,
    ) -> Task {
        Task {
            note_id: 1,
            note_name: "n".into(),
            line_index: 0,
            done: status == Status::Done,
            status,
            title: title.into(),
            priority,
            due_ms: due,
            due_time_min: None,
            recur: None,
            bookmarked: false,
            project: project.map(String::from),
            source_app_id: None,
            source_entry_id: None,
        }
    }

    #[test]
    fn board_columns_group_by_status_three_columns_in_order_incl_empty() {
        let tasks = vec![
            bt("a", Status::Todo, Priority::None, None, None),
            bt("b", Status::Done, Priority::None, None, None),
        ];
        let cols = board_columns(tasks, GroupBy::Status, SortBy::Due, &BoardFilter::default());
        assert_eq!(cols.len(), 3);
        assert_eq!(cols[0].title, "To do");
        assert_eq!(cols[1].title, "Doing");
        assert_eq!(cols[2].title, "Done");
        assert_eq!(cols[0].tasks.len(), 1);
        assert!(cols[1].tasks.is_empty()); // empty column retained
        assert_eq!(cols[2].tasks.len(), 1);
    }

    #[test]
    fn board_columns_hide_done_removes_done_tasks() {
        let tasks = vec![
            bt("a", Status::Todo, Priority::None, None, None),
            bt("b", Status::Done, Priority::None, None, None),
        ];
        let filter = BoardFilter {
            hide_done: true,
            ..Default::default()
        };
        let cols = board_columns(tasks, GroupBy::Status, SortBy::Due, &filter);
        let total: usize = cols.iter().map(|c| c.tasks.len()).sum();
        assert_eq!(total, 1);
        assert!(cols[2].tasks.is_empty());
    }

    #[test]
    fn board_columns_project_filter_keeps_only_that_project() {
        let tasks = vec![
            bt("a", Status::Todo, Priority::None, None, Some("work")),
            bt("b", Status::Todo, Priority::None, None, Some("home")),
            bt("c", Status::Todo, Priority::None, None, None),
        ];
        let filter = BoardFilter {
            project: Some("work".to_string()),
            ..Default::default()
        };
        let cols = board_columns(tasks, GroupBy::Project, SortBy::Due, &filter);
        let titles: Vec<&str> = cols
            .iter()
            .flat_map(|c| c.tasks.iter())
            .map(|t| t.title.as_str())
            .collect();
        assert_eq!(titles, vec!["a"]);
    }

    #[test]
    fn board_columns_project_filter_no_project_matches_projectless_tasks() {
        let tasks = vec![
            bt("a", Status::Todo, Priority::None, None, Some("work")),
            bt("b", Status::Todo, Priority::None, None, None),
        ];
        let filter = BoardFilter {
            project: Some("No project".to_string()),
            ..Default::default()
        };
        let cols = board_columns(tasks, GroupBy::Project, SortBy::Due, &filter);
        let titles: Vec<&str> = cols
            .iter()
            .flat_map(|c| c.tasks.iter())
            .map(|t| t.title.as_str())
            .collect();
        assert_eq!(titles, vec!["b"]);
    }

    #[test]
    fn board_columns_query_filters_title_case_insensitively() {
        let tasks = vec![
            bt("Fix mount", Status::Todo, Priority::None, None, None),
            bt("Buy milk", Status::Todo, Priority::None, None, None),
        ];
        let filter = BoardFilter {
            query: "FIX".to_string(),
            ..Default::default()
        };
        let cols = board_columns(tasks, GroupBy::Status, SortBy::Due, &filter);
        let titles: Vec<&str> = cols
            .iter()
            .flat_map(|c| c.tasks.iter())
            .map(|t| t.title.as_str())
            .collect();
        assert_eq!(titles, vec!["Fix mount"]);
    }

    #[test]
    fn board_columns_sort_by_due_orders_scheduled_before_unscheduled() {
        let tasks = vec![
            bt("no due", Status::Todo, Priority::None, None, None),
            bt("due", Status::Todo, Priority::None, Some(100), None),
        ];
        let cols = board_columns(tasks, GroupBy::Status, SortBy::Due, &BoardFilter::default());
        let titles: Vec<&str> = cols[0].tasks.iter().map(|t| t.title.as_str()).collect();
        assert_eq!(titles, vec!["due", "no due"]);
    }

    #[test]
    fn board_columns_group_by_priority_four_named_columns() {
        let tasks = vec![
            bt("h", Status::Todo, Priority::High, None, None),
            bt("m", Status::Todo, Priority::Medium, None, None),
            bt("l", Status::Todo, Priority::Low, None, None),
            bt("n", Status::Todo, Priority::None, None, None),
        ];
        let cols = board_columns(
            tasks,
            GroupBy::Priority,
            SortBy::Due,
            &BoardFilter::default(),
        );
        let titles: Vec<&str> = cols.iter().map(|c| c.title.as_str()).collect();
        assert_eq!(titles, vec!["High", "Medium", "Low", "No priority"]);
        assert_eq!(cols[0].tasks[0].title, "h");
        assert_eq!(cols[1].tasks[0].title, "m");
        assert_eq!(cols[2].tasks[0].title, "l");
        assert_eq!(cols[3].tasks[0].title, "n");
    }

    #[test]
    fn partition_due_splits_overdue_and_today() {
        const DAY: i64 = 86_400_000;
        let today = 20_000 * DAY;
        let mk = |due: Option<i64>, done: bool, pri: Priority| Task {
            note_id: 1,
            note_name: "n".into(),
            line_index: 0,
            done,
            status: if done { Status::Done } else { Status::Todo },
            title: "t".into(),
            priority: pri,
            due_ms: due,
            due_time_min: None,
            recur: None,
            bookmarked: false,
            project: None,
            source_app_id: None,
            source_entry_id: None,
        };
        let tasks = vec![
            mk(Some(today - DAY), false, Priority::None), // overdue
            mk(Some(today), false, Priority::None),       // today
            mk(Some(today + DAY), false, Priority::None), // later → dropped
            mk(None, false, Priority::None),              // no due → dropped
            mk(Some(today), true, Priority::None),        // done → dropped
        ];
        let b = partition_due(tasks, today);
        assert_eq!(b.overdue.len(), 1);
        assert_eq!(b.overdue[0].due_ms, Some(today - DAY));
        assert_eq!(b.today.len(), 1);
        assert_eq!(b.today[0].due_ms, Some(today));
    }

    #[test]
    fn parses_due_with_time_and_leaves_bare_number() {
        let n = Note {
            id: 1,
            name: "n".into(),
            is_daily: false,
            body: "- [ ] ship it @today 14:00\n- [ ] count to @today 5".into(),
            created_at_ms: 0,
            updated_at_ms: 0,
            source_app_id: None,
            source_entry_id: None,
        };
        let ts = parse_tasks_in(&n, 0);
        assert_eq!(ts[0].title, "ship it");
        assert_eq!(ts[0].due_time_min, Some(840));
        assert!(ts[0].due_ms.is_some());
        assert_eq!(ts[1].title, "count to 5");
        assert_eq!(ts[1].due_time_min, None);
    }

    #[test]
    fn parse_recur_units_and_rejects() {
        assert_eq!(
            parse_recur("+1d"),
            Some(Recur {
                n: 1,
                unit: RecurUnit::Day
            })
        );
        assert_eq!(
            parse_recur("+2w"),
            Some(Recur {
                n: 2,
                unit: RecurUnit::Week
            })
        );
        assert_eq!(
            parse_recur("+1mo"),
            Some(Recur {
                n: 1,
                unit: RecurUnit::Month
            })
        );
        assert_eq!(
            parse_recur("+3y"),
            Some(Recur {
                n: 3,
                unit: RecurUnit::Year
            })
        );
        assert_eq!(parse_recur("+1"), None);
        assert_eq!(parse_recur("+0d"), None);
        assert_eq!(parse_recur("+xw"), None);
        assert_eq!(parse_recur("1w"), None);
    }

    #[test]
    fn add_recur_clamps_month_and_year() {
        let jan31 = super::days_from_civil(2026, 1, 31);
        assert_eq!(
            add_recur(
                jan31,
                Recur {
                    n: 1,
                    unit: RecurUnit::Month
                }
            ),
            super::days_from_civil(2026, 2, 28)
        );
        let feb29 = super::days_from_civil(2024, 2, 29);
        assert_eq!(
            add_recur(
                feb29,
                Recur {
                    n: 1,
                    unit: RecurUnit::Year
                }
            ),
            super::days_from_civil(2025, 2, 28)
        );
        let d = super::days_from_civil(2026, 3, 1);
        assert_eq!(
            add_recur(
                d,
                Recur {
                    n: 2,
                    unit: RecurUnit::Week
                }
            ),
            d + 14
        );
    }

    #[test]
    fn next_occurrence_skips_past_today() {
        let due = super::days_from_civil(2026, 1, 1);
        let today = super::days_from_civil(2026, 1, 20);
        assert_eq!(
            next_occurrence(
                due,
                Recur {
                    n: 1,
                    unit: RecurUnit::Week
                },
                today
            ),
            super::days_from_civil(2026, 1, 22)
        );
    }

    #[test]
    fn parse_tasks_in_reads_recur_and_cleans_title() {
        let n = Note {
            id: 1,
            name: "n".into(),
            is_daily: false,
            body: "- [ ] water plants @today +1w".into(),
            created_at_ms: 0,
            updated_at_ms: 0,
            source_app_id: None,
            source_entry_id: None,
        };
        let t = &parse_tasks_in(&n, 0)[0];
        assert_eq!(t.title, "water plants");
        assert_eq!(
            t.recur,
            Some(Recur {
                n: 1,
                unit: RecurUnit::Week
            })
        );
    }

    #[test]
    fn reschedule_bumps_due_and_keeps_open() {
        const DAY_MS: i64 = 86_400_000;
        let now = super::days_from_civil(2026, 1, 20) * DAY_MS;
        let out = reschedule_line("- [ ] water plants @2026-01-01 +1w", now).unwrap();
        assert_eq!(out, "- [ ] water plants @2026-01-22 +1w");
        assert!(reschedule_line("- [ ] plain @2026-01-01", now).is_none());
        assert!(reschedule_line("- [x] done @2026-01-01 +1w", now).is_none());
        assert!(reschedule_line("- [ ] no due +1w", now).is_none());
    }

    #[test]
    fn parses_and_toggles_bookmark() {
        let n = Note {
            id: 1,
            name: "n".into(),
            is_daily: false,
            body: "- [ ] buy milk *\n- [ ] plain".into(),
            created_at_ms: 0,
            updated_at_ms: 0,
            source_app_id: None,
            source_entry_id: None,
        };
        let ts = parse_tasks_in(&n, 0);
        assert!(ts[0].bookmarked);
        assert_eq!(ts[0].title, "buy milk");
        assert!(!ts[1].bookmarked);
        let off = toggle_bookmark_line(&n.body, 0);
        assert_eq!(off.lines().next().unwrap(), "- [ ] buy milk");
        let on = toggle_bookmark_line(&off, 0);
        assert_eq!(on.lines().next().unwrap(), "- [ ] buy milk *");
        assert_eq!(toggle_bookmark_line("hello", 0), "hello");
    }

    #[test]
    fn log_clock_creates_new_drawer() {
        const DAY_MS: i64 = 86_400_000;
        let day = super::days_from_civil(2026, 1, 20) * DAY_MS;
        let start = day + 9 * 3_600_000; // 09:00
        let end = start + 90 * 60_000; // +90m -> 10:30
        let body = "- [ ] Ship it";
        let new = log_clock(body, "Ship it", start, end, 0);
        assert_eq!(
            new,
            "- [ ] Ship it\n  :LOGBOOK:\n  CLOCK: [2026-01-20 09:00]--[2026-01-20 10:30] => 1:30\n  :END:"
        );
    }

    #[test]
    fn log_clock_appends_to_existing_drawer() {
        const DAY_MS: i64 = 86_400_000;
        let day = super::days_from_civil(2026, 1, 20) * DAY_MS;
        let start = day + 9 * 3_600_000;
        let end = start + 30 * 60_000; // 0:30
        let body = "- [ ] Ship it\n  :LOGBOOK:\n  CLOCK: [2026-01-19 09:00]--[2026-01-19 10:00] => 1:00\n  :END:";
        let new = log_clock(body, "Ship it", start, end, 0);
        let lines: Vec<&str> = new.lines().collect();
        assert_eq!(lines[0], "- [ ] Ship it");
        assert_eq!(lines[1], "  :LOGBOOK:");
        assert_eq!(
            lines[2],
            "  CLOCK: [2026-01-20 09:00]--[2026-01-20 09:30] => 0:30"
        );
        assert_eq!(
            lines[3],
            "  CLOCK: [2026-01-19 09:00]--[2026-01-19 10:00] => 1:00"
        );
        assert_eq!(lines[4], "  :END:");
        assert_eq!(lines.len(), 5);
        // no duplicate :END:
        assert_eq!(new.matches(":END:").count(), 1);
    }

    #[test]
    fn log_clock_no_matching_title_is_noop() {
        let body = "- [ ] Ship it";
        let new = log_clock(body, "Nope", 0, 1, 0);
        assert_eq!(new, body);
    }

    #[test]
    fn duration_hm_formats() {
        assert_eq!(duration_hm(90 * 60_000), "1:30");
        assert_eq!(duration_hm(5 * 60_000), "0:05");
        assert_eq!(duration_hm(125 * 60_000), "2:05");
        assert_eq!(duration_hm(-1000), "0:00");
    }

    #[test]
    fn log_clock_preserves_indentation() {
        const DAY_MS: i64 = 86_400_000;
        let day = super::days_from_civil(2026, 1, 20) * DAY_MS;
        let start = day;
        let end = start + 5 * 60_000;
        let body = "- [ ] Parent\n  - [ ] Nested";
        let new = log_clock(body, "Nested", start, end, 0);
        let lines: Vec<&str> = new.lines().collect();
        assert_eq!(lines[0], "- [ ] Parent");
        assert_eq!(lines[1], "  - [ ] Nested");
        assert_eq!(lines[2], "    :LOGBOOK:");
        assert_eq!(
            lines[3],
            "    CLOCK: [2026-01-20 00:00]--[2026-01-20 00:05] => 0:05"
        );
        assert_eq!(lines[4], "    :END:");
    }
}
