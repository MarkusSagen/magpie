use magpie_core::{open_in_memory, Store};
use magpie_core::{CaptureEvent, Content, ImageStore};

fn store() -> Store {
    open_in_memory().unwrap()
}

struct NoopImg;
impl ImageStore for NoopImg {
    fn put(&self, h: &str, _b: &[u8]) -> std::io::Result<String> {
        Ok(h.to_string())
    }
}

#[test]
fn upsert_by_name_is_idempotent_and_trims() {
    let s = store();
    let a = s.upsert_note_by_name("  Prod Incidents  ", 10).unwrap();
    let b = s.upsert_note_by_name("Prod Incidents", 20).unwrap();
    assert_eq!(a.id, b.id); // same page
    assert_eq!(a.name, "Prod Incidents");
    assert!(!a.is_daily);
    assert!(s.upsert_note_by_name("   ", 30).is_err()); // empty name rejected
}

#[test]
fn daily_note_get_or_create() {
    let s = store();
    let a = s.daily_note("2026-09-02", 100).unwrap();
    let b = s.daily_note("2026-09-02", 200).unwrap();
    assert_eq!(a.id, b.id);
    assert!(a.is_daily);
    assert_eq!(a.name, "2026-09-02");
}

#[test]
fn update_body_bumps_updated_and_reports_missing() {
    let s = store();
    let n = s.upsert_note_by_name("page", 100).unwrap();
    assert!(s.update_note_body(n.id, "hello [[world]]", 500).unwrap());
    let got = s.get_note(n.id).unwrap().unwrap();
    assert_eq!(got.body, "hello [[world]]");
    assert_eq!(got.updated_at_ms, 500);
    assert!(!s.update_note_body(999_999, "x", 600).unwrap()); // unknown id
}

#[test]
fn recent_notes_orders_by_updated_desc() {
    let s = store();
    let a = s.upsert_note_by_name("a", 100).unwrap();
    let b = s.upsert_note_by_name("b", 200).unwrap();
    s.update_note_body(a.id, "later", 300).unwrap(); // a now most recent
    let names: Vec<String> = s
        .recent_notes(10)
        .unwrap()
        .into_iter()
        .map(|n| n.name)
        .collect();
    assert_eq!(names, vec!["a".to_string(), "b".to_string()]);
    let _ = b;
}

#[test]
fn all_note_names_sorted_distinct() {
    let s = store();
    s.upsert_note_by_name("zeta", 1).unwrap();
    s.upsert_note_by_name("alpha", 2).unwrap();
    assert_eq!(
        s.all_note_names().unwrap(),
        vec!["alpha".to_string(), "zeta".to_string()]
    );
}

#[test]
fn all_notes_returns_every_note() {
    let s = store();
    s.upsert_note_by_name("a", 100).unwrap();
    s.daily_note("2026-09-02", 200).unwrap();
    let names: Vec<String> = s.all_notes().unwrap().into_iter().map(|n| n.name).collect();
    assert_eq!(names.len(), 2);
    assert!(names.contains(&"a".to_string()) && names.contains(&"2026-09-02".to_string()));
}

#[test]
fn create_note_from_entry_copies_provenance() {
    let s = store();
    let ing = s
        .ingest(
            &CaptureEvent {
                content: Content::Text("Fix the mount\nsecond line".into()),
                source_app: None,
                copied_at_ms: 42,
            },
            &NoopImg,
        )
        .unwrap();
    let note = s.create_note_from_entry(ing.entry_id, 100).unwrap();
    assert_eq!(note.name, "Fix the mount"); // first line
    assert_eq!(note.body, "Fix the mount\nsecond line");
    assert_eq!(note.source_entry_id, Some(ing.entry_id));
    // A second capture of the same entry gets a suffixed unique name.
    let note2 = s.create_note_from_entry(ing.entry_id, 200).unwrap();
    assert_eq!(note2.name, "Fix the mount (2)");
}

#[test]
fn reminder_fired_roundtrip() {
    let s = store();
    assert!(!s.reminder_fired("fp1").unwrap());
    s.mark_reminder("fp1", 123).unwrap();
    assert!(s.reminder_fired("fp1").unwrap());
    assert!(!s.reminder_fired("other").unwrap());
    s.mark_reminder("fp1", 456).unwrap();
    assert!(s.reminder_fired("fp1").unwrap());
}

#[test]
fn time_tracking_start_stop_total() {
    let s = store();
    assert!(s.active_timer().unwrap().is_none());
    s.start_timer("n1|Ship", "Ship", Some(1), 1_000).unwrap();
    let a = s.active_timer().unwrap().unwrap();
    assert_eq!(a.task_key, "n1|Ship");
    assert_eq!(s.total_ms_for("n1|Ship", 5_000).unwrap(), 4_000);
    s.start_timer("n1|Review", "Review", Some(1), 5_000)
        .unwrap();
    assert_eq!(s.active_timer().unwrap().unwrap().task_key, "n1|Review");
    assert_eq!(s.total_ms_for("n1|Ship", 9_000).unwrap(), 4_000);
    assert!(s.stop_active(9_000).unwrap());
    assert!(s.active_timer().unwrap().is_none());
    assert_eq!(s.total_ms_for("n1|Review", 20_000).unwrap(), 4_000);
    assert!(!s.stop_active(9_000).unwrap());
}

#[test]
fn bookmarks_crud() {
    let s = store();
    let id = s.add_bookmark("https://a.com/x", "A", "a.com", 10).unwrap();
    assert!(id > 0);
    s.add_bookmark("https://a.com/x", "A better", "a.com", 20)
        .unwrap();
    s.add_bookmark("https://b.com", "B", "b.com", 30).unwrap();
    let all = s.list_bookmarks("", 50).unwrap();
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].url, "https://b.com");
    let hit = s.list_bookmarks("better", 50).unwrap();
    assert_eq!(hit.len(), 1);
    assert_eq!(hit[0].title, "A better");
    assert!(s.delete_bookmark(hit[0].id).unwrap());
    assert_eq!(s.list_bookmarks("", 50).unwrap().len(), 1);
}

#[test]
fn daily_notes_newest_first() {
    let s = store();
    s.daily_note("2026-09-08", 10).unwrap();
    s.daily_note("2026-09-10", 20).unwrap();
    s.daily_note("2026-09-09", 30).unwrap();
    s.upsert_note_by_name("Not a daily", 40).unwrap(); // excluded
    let days = s.daily_notes(50).unwrap();
    let names: Vec<&str> = days.iter().map(|n| n.name.as_str()).collect();
    assert_eq!(names, vec!["2026-09-10", "2026-09-09", "2026-09-08"]);
}
