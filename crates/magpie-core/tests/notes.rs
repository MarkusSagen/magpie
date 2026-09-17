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
fn time_aggregates() {
    let s = store();
    // two closed entries same task, one other task
    s.start_timer("k1|A", "A", Some(1), 1_000).unwrap();
    s.stop_active(4_000).unwrap(); // 3s on A
    s.start_timer("k2|B", "B", Some(1), 10_000).unwrap();
    s.stop_active(15_000).unwrap(); // 5s on B
                                    // by task: B (5s) before A (3s)
    let by_task = s.time_by_task(0, 20_000).unwrap();
    assert_eq!(by_task.len(), 2);
    assert_eq!(by_task[0].0, "k2|B");
    assert_eq!(by_task[0].2, 5_000);
    assert_eq!(by_task[1].2, 3_000);
    // total since 0
    assert_eq!(s.time_total_since(0, 20_000).unwrap(), 8_000);
    // since filter excludes the early A entry
    assert_eq!(s.time_total_since(5_000, 20_000).unwrap(), 5_000);
    // by day returns at least one bucket summing to 8s (all same epoch day)
    let by_day = s.time_by_day(0, 20_000).unwrap();
    let day_total: i64 = by_day.iter().map(|(_, ms)| ms).sum();
    assert_eq!(day_total, 8_000);
    // a running entry counts up to now
    s.start_timer("k3|C", "C", Some(1), 30_000).unwrap();
    assert_eq!(s.time_total_since(30_000, 32_000).unwrap(), 2_000);
}

#[test]
fn recent_time_entries_and_delete() {
    let s = store();
    s.start_timer("k1|A", "A", Some(1), 1_000).unwrap();
    s.stop_active(4_000).unwrap(); // 3s on A
    s.start_timer("k2|B", "B", Some(1), 10_000).unwrap();
    s.stop_active(15_000).unwrap(); // 5s on B

    let entries = s.recent_time_entries(10).unwrap();
    assert_eq!(entries.len(), 2);
    // newest (by start) first: B before A
    assert_eq!(entries[0].task_title, "B");
    assert_eq!(entries[0].start_ms, 10_000);
    assert_eq!(entries[0].end_ms, Some(15_000));
    assert_eq!(entries[1].task_title, "A");
    assert_eq!(entries[1].start_ms, 1_000);
    assert_eq!(entries[1].end_ms, Some(4_000));

    let before = s.time_total_since(0, 20_000).unwrap();
    assert!(s.delete_time_entry(entries[0].id).unwrap());
    let after_entries = s.recent_time_entries(10).unwrap();
    assert_eq!(after_entries.len(), 1);
    assert_eq!(after_entries[0].task_title, "A");
    let after = s.time_total_since(0, 20_000).unwrap();
    assert_eq!(before - after, 5_000); // B's duration removed

    // deleting a bogus id is a no-op
    assert!(!s.delete_time_entry(999_999).unwrap());
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
fn fresh_db_runs_migration_and_bookmarks_start_untagged() {
    // list_bookmarks selects the `tags` column, which only exists once
    // run_migrations() has ALTERed the table created by the baseline SCHEMA —
    // so a working query here already proves the migration ran.
    let s = store();
    s.add_bookmark("https://a.com", "A", "a.com", 10).unwrap();
    let all = s.list_bookmarks("", 10).unwrap();
    assert_eq!(all.len(), 1);
    assert!(all[0].tags.is_empty());
}

#[test]
fn set_bookmark_tags_normalizes() {
    let s = store();
    let id = s.add_bookmark("https://a.com", "A", "a.com", 10).unwrap();
    s.set_bookmark_tags(
        id,
        &[
            "Work".to_string(),
            " Reading ".to_string(),
            "work".to_string(),
            "".to_string(),
        ],
    )
    .unwrap();
    let all = s.list_bookmarks("", 10).unwrap();
    assert_eq!(all[0].tags, vec!["work".to_string(), "reading".to_string()]);
}

#[test]
fn list_bookmarks_matches_tags() {
    let s = store();
    let id = s.add_bookmark("https://a.com", "A", "a.com", 10).unwrap();
    s.set_bookmark_tags(id, &["reading".to_string()]).unwrap();
    let hit = s.list_bookmarks("reading", 10).unwrap();
    assert_eq!(hit.len(), 1);
    assert_eq!(hit[0].id, id);
    let miss = s.list_bookmarks("nope", 10).unwrap();
    assert!(miss.is_empty());
}

#[test]
fn legacy_db_migrates_tags_column_in_place_and_is_idempotent() {
    let path =
        std::env::temp_dir().join(format!("magpie-legacy-bmk-{}.sqlite", std::process::id()));
    let _ = std::fs::remove_file(&path);
    {
        // Pre-migration schema: the frozen baseline CREATE from schema.sql, minus tags.
        let c = rusqlite::Connection::open(&path).unwrap();
        c.execute_batch(
            "CREATE TABLE bookmarks (
               id       INTEGER PRIMARY KEY,
               url      TEXT NOT NULL,
               title    TEXT NOT NULL DEFAULT '',
               domain   TEXT NOT NULL DEFAULT '',
               added_ms INTEGER NOT NULL
             );
             CREATE UNIQUE INDEX idx_bookmarks_url ON bookmarks(url);
             CREATE INDEX idx_bookmarks_added ON bookmarks(added_ms);
             INSERT INTO bookmarks (url, title, domain, added_ms)
               VALUES ('https://legacy.com', 'Legacy', 'legacy.com', 5);
             PRAGMA user_version = 0;",
        )
        .unwrap();
    }

    // First open: init() runs the baseline SCHEMA (no-op on existing tables via
    // IF NOT EXISTS) then run_migrations() ALTERs the pre-existing bookmarks table.
    let s = magpie_core::open(&path).unwrap();
    let all = s.list_bookmarks("", 10).unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].url, "https://legacy.com");
    assert!(all[0].tags.is_empty());
    drop(s);

    // Second open: migration already applied (user_version >= 1) — must not error
    // trying to ALTER a column that already exists.
    let s2 = magpie_core::open(&path).unwrap();
    assert_eq!(s2.list_bookmarks("", 10).unwrap().len(), 1);
    drop(s2);

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(format!("{}-wal", path.display()));
    let _ = std::fs::remove_file(format!("{}-shm", path.display()));
}

#[test]
fn reads_firefox_bookmarks() {
    // Build a minimal Firefox places.sqlite in a temp file.
    let dir = std::env::temp_dir().join(format!("magpie-fftest-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let places = dir.join("places.sqlite");
    {
        let c = rusqlite::Connection::open(&places).unwrap();
        c.execute_batch(
            "CREATE TABLE moz_places(id INTEGER PRIMARY KEY, url TEXT);
             CREATE TABLE moz_bookmarks(id INTEGER PRIMARY KEY, type INTEGER, fk INTEGER, title TEXT);
             INSERT INTO moz_places(id,url) VALUES (1,'https://rust-lang.org'),(2,'https://example.com'),(3,'ftp://skip');
             INSERT INTO moz_bookmarks(id,type,fk,title) VALUES
               (10,1,1,'Rust'),(11,2,NULL,'A folder'),(12,1,2,'Example'),(13,1,3,'FTP skip');",
        )
        .unwrap();
    }
    let got = magpie_core::read_firefox_bookmarks(&places).unwrap();
    // only type=1 http(s) rows, in id order
    assert_eq!(
        got,
        vec![
            ("Rust".to_string(), "https://rust-lang.org".to_string()),
            ("Example".to_string(), "https://example.com".to_string()),
        ]
    );
    std::fs::remove_dir_all(&dir).ok();
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
