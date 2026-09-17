//! App-level end-to-end: build the real AppState (in-memory store + temp image
//! cache + UI state), ingest a realistic capture session, filter/search as the
//! launcher would, then quick-paste a result into a fake clipboard.

use magpie_app::app_state::{current_results, ingest_event, AppState};
use magpie_app::image_cache::FsImageStore;
use magpie_app::paste_action::{perform_paste, resolve_quick_paste, PasteKind};
use magpie_app::viewmodel::{TypeFilter, UiState};
use magpie_core::{open_in_memory, AppInfo, CaptureEvent, Content, Kind};
use magpie_platform::{Clipboard, ClipboardSnapshot, Paster};
use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

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

fn state() -> AppState {
    AppState {
        store: Mutex::new(open_in_memory().unwrap()),
        images: FsImageStore {
            dir: std::env::temp_dir().join(format!("magpie-e2e-{}", std::process::id())),
        },
        ui: Mutex::new(UiState::new()),
        merge_set: Mutex::new(Vec::new()),
        screenshare: Mutex::new(false),
        mask_apps: Vec::new(),
        mask_patterns: Vec::new(),
        mask_visible_chars: 3,
        open_to_today: false,
        fetch_link_favicons: true,
    }
}

fn ev(text: &str, ms: i64, app: &str) -> CaptureEvent {
    CaptureEvent {
        content: Content::Text(text.into()),
        source_app: Some(AppInfo {
            identifier: app.into(),
            display_name: app.into(),
            icon_path: None,
        }),
        copied_at_ms: ms,
    }
}

#[test]
fn full_session_capture_filter_and_quick_paste() {
    let st = state();

    // A realistic capture session.
    for e in [
        ev("docs/superpowers/specs/design.md", 1_000, "Ghostty"),
        ev("https://app.asana.com/1/1210421", 2_000, "Vivaldi"),
        ev("Vi har source_service på alla tabeller", 3_000, "Slack"),
        ev("docs/superpowers/specs/design.md", 4_000, "Ghostty"), // re-copy
        ev("#1a2b3c", 5_000, "Figma"),
    ] {
        ingest_event(&st, &e).unwrap();
    }

    // Default view: newest first, unique entries (doc path deduped).
    let all = current_results(&st, 10_000);
    assert_eq!(all.len(), 4);

    // Search "asana" -> the link.
    st.ui.lock().unwrap().text = "asana".into();
    let link = current_results(&st, 10_000);
    assert_eq!(link.len(), 1);
    assert_eq!(link[0].kind, Kind::Link);

    // Type filter = Color (clear text first).
    {
        let mut ui = st.ui.lock().unwrap();
        ui.text.clear();
        ui.type_filter = TypeFilter::Color;
    }
    let colors = current_results(&st, 10_000);
    assert_eq!(colors.len(), 1);
    assert_eq!(colors[0].full_text, "#1a2b3c");

    // Reset filters; quick-paste slot 1 (most recent) into a fake clipboard.
    {
        let mut ui = st.ui.lock().unwrap();
        ui.type_filter = TypeFilter::All;
    }
    let recent = current_results(&st, 10_000);
    let target = resolve_quick_paste(&recent, 1).expect("slot 1 exists");
    let expected = target.full_text.clone();

    let mut clip = FakeClip {
        last: RefCell::new(String::new()),
    };
    let paster = FakePaster {
        pasted: AtomicBool::new(false),
    };
    perform_paste(&mut clip, &paster, target, PasteKind::Formatted, true).unwrap();

    assert_eq!(
        *clip.last.borrow(),
        expected,
        "quick-paste puts the entry on the clipboard"
    );
    assert!(paster.pasted.load(Ordering::SeqCst), "auto-paste fired");

    std::fs::remove_dir_all(
        std::env::temp_dir().join(format!("magpie-e2e-{}", std::process::id())),
    )
    .ok();
}

#[test]
fn most_copied_surfaces_the_repeated_entry() {
    let st = state();
    for e in [
        ev("one-off", 1, "A"),
        ev("frequent", 2, "A"),
        ev("frequent", 3, "B"),
        ev("frequent", 4, "A"),
    ] {
        ingest_event(&st, &e).unwrap();
    }
    st.ui.lock().unwrap().sort = magpie_core::Sort::MostCopied;
    let rows = current_results(&st, 100);
    assert_eq!(rows[0].full_text, "frequent");
    assert_eq!(rows[0].copy_count, 3);
}
