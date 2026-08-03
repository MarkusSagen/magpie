//! Cross-module integration tests for the magpie_app library.

use magpie_app::config::{load_or_default, save, Config};
use magpie_app::grouping::{group, section_for, Section};
use magpie_app::image_cache::FsImageStore;
use magpie_app::paste_action::{perform_paste, resolve_quick_paste, PasteKind};
use magpie_app::viewmodel::{to_query, TimeFilter, TypeFilter, UiState};
use magpie_core::{open_in_memory, CaptureEvent, Content, Entry, ImageStore, Kind};
use magpie_platform::{Clipboard, ClipboardSnapshot, Paster};
use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};

const DAY: i64 = 86_400_000;

fn tmp(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("magpie-app-it-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}
struct Noop;
impl ImageStore for Noop {
    fn put(&self, h: &str, _b: &[u8]) -> std::io::Result<String> {
        Ok(h.to_string())
    }
}
fn text(s: &str, ms: i64) -> CaptureEvent {
    CaptureEvent {
        content: Content::Text(s.into()),
        source_app: None,
        copied_at_ms: ms,
    }
}

// ---- config -------------------------------------------------------------

#[test]
fn config_roundtrips_and_bad_toml_falls_back() {
    let dir = tmp("cfg");
    let path = dir.join("config.toml");

    let c = Config {
        paste_on_select: false,
        app_denylist: vec!["Nope".into()],
        ..Config::default()
    };
    save(&c, &path).unwrap();
    let loaded = load_or_default(&path);
    assert!(!loaded.paste_on_select);
    assert_eq!(loaded.app_denylist, vec!["Nope".to_string()]);
    assert_eq!(loaded.quick_paste_hotkeys.len(), 9);

    // malformed toml -> defaults, not a panic
    std::fs::write(&path, "this is : not = valid ][ toml").unwrap();
    let d = load_or_default(&path);
    assert_eq!(d.launcher_hotkey, "super+shift+space");

    std::fs::remove_dir_all(&dir).ok();
}

// ---- image cache --------------------------------------------------------

#[test]
fn image_cache_put_and_thumbnail() {
    let dir = tmp("img");
    let store = FsImageStore { dir: dir.clone() };

    let p1 = store.put("hashA", &[1, 2, 3]).unwrap();
    let p2 = store.put("hashA", &[1, 2, 3]).unwrap();
    let p3 = store.put("hashB", &[9]).unwrap();
    assert_eq!(p1, p2);
    assert_ne!(p1, p3);
    assert!(std::path::Path::new(&p1).exists());

    // 2x1 RGBA -> thumbnail PNG exists
    let rgba = vec![255, 0, 0, 255, 0, 255, 0, 255];
    let thumb = store.write_thumbnail("hashA", 2, 1, &rgba, 32).unwrap();
    assert!(thumb.ends_with("hashA.thumb.png"));
    assert!(std::path::Path::new(&thumb).exists());

    std::fs::remove_dir_all(&dir).ok();
}

// ---- grouping -----------------------------------------------------------

#[test]
fn grouping_sections_today_yesterday_older() {
    assert!(matches!(
        section_for(100 * DAY + 1, 100 * DAY + 9),
        Section::Today
    ));
    assert!(matches!(
        section_for(99 * DAY, 100 * DAY + 9),
        Section::Yesterday
    ));
    assert!(matches!(section_for(1, 100 * DAY), Section::Older));

    let mk = |ms: i64| Entry {
        id: ms,
        content_hash: format!("h{ms}"),
        kind: Kind::Text,
        preview_text: String::new(),
        full_text: String::new(),
        image_path: None,
        byte_size: 0,
        char_count: 0,
        word_count: 0,
        line_count: 0,
        first_copied_at_ms: ms,
        last_copied_at_ms: ms,
        copy_count: 1,
        pinned: false,
        source_app_id: None,
    };
    let now = 100 * DAY + 5;
    let entries = vec![mk(100 * DAY), mk(99 * DAY), mk(50 * DAY), mk(100 * DAY + 1)];
    let g = group(&entries, now);
    // Today (2), Yesterday (1), Older (1) in that order
    assert_eq!(g.len(), 3);
    assert_eq!(g[0].0.label(), "Today");
    assert_eq!(g[0].1, vec![0, 3]);
    assert_eq!(g[1].0.label(), "Yesterday");
    assert_eq!(g[2].0.label(), "Older");
}

// ---- view-model -> query -> store ---------------------------------------

#[test]
fn ui_state_maps_to_query_and_filters_the_store() {
    let s = open_in_memory().unwrap();
    let now = 100 * DAY + 50_000; // mid-day, so "just before now" is still today
    s.ingest(&text("old plain note", now - 30 * DAY), &Noop)
        .unwrap();
    s.ingest(&text("https://today.example", now - 100), &Noop)
        .unwrap(); // link, today
    s.ingest(&text("today plain note", now - 200), &Noop)
        .unwrap();

    // Type=Link + Today
    let ui = UiState {
        type_filter: TypeFilter::Link,
        time_filter: TimeFilter::Today,
        ..UiState::new()
    };
    let q = to_query(&ui, now);
    let rows = s.search(&q).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].kind, Kind::Link);

    // Last7Days text search
    let ui2 = UiState {
        text: "note".into(),
        time_filter: TimeFilter::Last7Days,
        ..UiState::new()
    };
    let q2 = to_query(&ui2, now);
    let rows2 = s.search(&q2).unwrap();
    // "old plain note" is 30 days ago -> excluded; "today plain note" included
    assert_eq!(rows2.len(), 1);
    assert_eq!(rows2[0].full_text, "today plain note");
}

// ---- paste orchestration ------------------------------------------------

struct FakeClip {
    last: RefCell<String>,
}
impl Clipboard for FakeClip {
    fn snapshot(&mut self) -> ClipboardSnapshot {
        ClipboardSnapshot {
            content: None,
            change_token: 0,
            concealed: false,
        }
    }
    fn set_text(&mut self, t: &str) -> Result<(), String> {
        *self.last.borrow_mut() = t.into();
        Ok(())
    }
    fn set_content(&mut self, c: &Content) -> Result<(), String> {
        if let Content::Text(t) = c {
            *self.last.borrow_mut() = t.clone();
        }
        Ok(())
    }
}
struct FakePaster {
    pasted: AtomicBool,
}
impl Paster for FakePaster {
    fn paste(&self) -> Result<(), String> {
        self.pasted.store(true, Ordering::SeqCst);
        Ok(())
    }
}

fn entry(text: &str) -> Entry {
    Entry {
        id: 1,
        content_hash: "h".into(),
        kind: Kind::Text,
        preview_text: text.into(),
        full_text: text.into(),
        image_path: None,
        byte_size: 0,
        char_count: 0,
        word_count: 0,
        line_count: 0,
        first_copied_at_ms: 0,
        last_copied_at_ms: 0,
        copy_count: 1,
        pinned: false,
        source_app_id: None,
    }
}

#[test]
fn quick_paste_resolution_and_perform_paste_modes() {
    let recent = vec![entry("first"), entry("second"), entry("third")];
    assert_eq!(resolve_quick_paste(&recent, 1).unwrap().full_text, "first");
    assert_eq!(resolve_quick_paste(&recent, 3).unwrap().full_text, "third");
    assert!(resolve_quick_paste(&recent, 0).is_none());
    assert!(resolve_quick_paste(&recent, 4).is_none());

    // auto-paste: clipboard set AND paste fired
    let mut clip = FakeClip {
        last: RefCell::new(String::new()),
    };
    let paster = FakePaster {
        pasted: AtomicBool::new(false),
    };
    perform_paste(
        &mut clip,
        &paster,
        &entry("payload"),
        PasteKind::Formatted,
        true,
    )
    .unwrap();
    assert_eq!(*clip.last.borrow(), "payload");
    assert!(paster.pasted.load(Ordering::SeqCst));

    // clipboard-only: no paste
    let mut clip2 = FakeClip {
        last: RefCell::new(String::new()),
    };
    let paster2 = FakePaster {
        pasted: AtomicBool::new(false),
    };
    perform_paste(
        &mut clip2,
        &paster2,
        &entry("copyonly"),
        PasteKind::PlainText,
        false,
    )
    .unwrap();
    assert_eq!(*clip2.last.borrow(), "copyonly");
    assert!(!paster2.pasted.load(Ordering::SeqCst));
}
