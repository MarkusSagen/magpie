//! Pure reminder scheduling math: when a task's reminder should fire, which tasks
//! are ripe, a stable per-task fingerprint, and local UTC-offset parsing. All
//! offset/time inputs are parameters, so this is fully unit-testable.
use crate::tasks::Task;

/// The epoch-ms instant a task's reminder should fire: local midnight of the due
/// day plus the time-of-day (or `default_min` when the task is date-only).
/// `local_midnight = day_epoch_utc − offset_secs*1000`.
pub fn reminder_instant_ms(
    day_epoch_utc: i64,
    due_time_min: Option<i64>,
    offset_secs: i64,
    default_min: i64,
) -> i64 {
    let local_midnight = day_epoch_utc - offset_secs * 1000;
    let mins = due_time_min.unwrap_or(default_min);
    local_midnight + mins * 60_000
}

/// Open tasks whose reminder instant is at or before `now_ms`, earliest first.
pub fn due_before(tasks: Vec<Task>, now_ms: i64, offset_secs: i64, default_min: i64) -> Vec<Task> {
    let instant = |t: &Task| {
        t.due_ms
            .map(|d| reminder_instant_ms(d, t.due_time_min, offset_secs, default_min))
    };
    let mut out: Vec<Task> = tasks
        .into_iter()
        .filter(|t| !t.done)
        .filter(|t| instant(t).map(|i| i <= now_ms).unwrap_or(false))
        .collect();
    out.sort_by_key(|t| instant(t).unwrap_or(i64::MAX));
    out
}

/// A stable dedup key: note + title + due date + due time. Survives line moves;
/// changes (re-arms) when the due date or time changes.
pub fn fingerprint(t: &Task) -> String {
    format!(
        "{}|{}|{}|{}",
        t.note_id,
        t.title,
        t.due_ms.unwrap_or(0),
        t.due_time_min.unwrap_or(-1)
    )
}

/// Parse a `date +%z` offset ("+HHMM" / "-HHMM") into seconds east of UTC.
pub fn parse_offset(s: &str) -> Option<i64> {
    let s = s.trim();
    let b = s.as_bytes();
    if b.len() < 5 {
        return None;
    }
    let sign = match b[0] {
        b'+' => 1,
        b'-' => -1,
        _ => return None,
    };
    let hh: i64 = s.get(1..3)?.parse().ok()?;
    let mm: i64 = s.get(3..5)?.parse().ok()?;
    Some(sign * (hh * 3600 + mm * 60))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::Priority;

    const DAY: i64 = 86_400_000;

    fn task(due: Option<i64>, time: Option<i64>, done: bool, title: &str) -> Task {
        Task {
            note_id: 1,
            note_name: "n".into(),
            line_index: 0,
            done,
            title: title.into(),
            priority: Priority::None,
            due_ms: due,
            due_time_min: time,
            recur: None,
            project: None,
            source_app_id: None,
            source_entry_id: None,
        }
    }

    #[test]
    fn instant_applies_offset_and_default() {
        let day = 20_000 * DAY;
        assert_eq!(
            reminder_instant_ms(day, Some(840), 7200, 540),
            day - 7200 * 1000 + 840 * 60_000
        );
        assert_eq!(reminder_instant_ms(day, None, 0, 540), day + 540 * 60_000);
    }

    #[test]
    fn due_before_filters_and_sorts() {
        let day = 20_000 * DAY;
        let now = day + 12 * 3_600_000;
        let ripe = due_before(
            vec![
                task(Some(day), Some(600), false, "ten"),
                task(Some(day), Some(540), false, "nine"),
                task(Some(day), Some(23 * 60), false, "late"),
                task(Some(day), Some(600), true, "done"),
                task(None, None, false, "nodate"),
            ],
            now,
            0,
            540,
        );
        let titles: Vec<&str> = ripe.iter().map(|t| t.title.as_str()).collect();
        assert_eq!(titles, vec!["nine", "ten"]);
    }

    #[test]
    fn fingerprint_is_stable_and_distinct() {
        let a = task(Some(100), Some(540), false, "x");
        let mut b = a.clone();
        b.line_index = 9;
        assert_eq!(fingerprint(&a), fingerprint(&b));
        let mut c = a.clone();
        c.due_time_min = Some(600);
        assert_ne!(fingerprint(&a), fingerprint(&c));
    }

    #[test]
    fn parse_offset_variants() {
        assert_eq!(parse_offset("+0200"), Some(7200));
        assert_eq!(parse_offset("-0500"), Some(-18000));
        assert_eq!(parse_offset("+0530"), Some(19800));
        assert_eq!(parse_offset("junk"), None);
        assert_eq!(parse_offset(""), None);
    }
}
