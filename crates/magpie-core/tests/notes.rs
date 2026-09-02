use magpie_core::{open_in_memory, Store};

fn store() -> Store {
    open_in_memory().unwrap()
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
