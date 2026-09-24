use crate::image_cache::FsImageStore;
use crate::viewmodel::{to_query, UiState};
use magpie_core::{CaptureEvent, Entry, Store};
use std::sync::Mutex;

pub struct AppState {
    pub store: Mutex<Store>,
    pub images: FsImageStore,
    pub ui: Mutex<UiState>,
    /// Entry **ids** queued for merge, in the order they were added. Ids, not
    /// list indices: the queue has to survive a search or filter change, which
    /// reorders (and re-populates) the result list under it.
    pub merge_set: Mutex<Vec<i64>>,
    /// Session-only screensharing mode (masks everything). Not persisted.
    pub screenshare: Mutex<bool>,
    /// Masking config snapshot (from `Config`, immutable for the session).
    pub mask_apps: Vec<String>,
    pub mask_patterns: Vec<String>,
    pub mask_visible_chars: i64,
    /// Open the Today dashboard on each summon (from `Config`, session-immutable).
    pub open_to_today: bool,
    /// Fetch site favicons for links/bookmarks (from `Config::fetch_link_favicons`).
    pub fetch_link_favicons: bool,
    /// Fetch rich OG-image preview thumbnails for bookmarks ON EXPLICIT SAVE only
    /// (from `Config::fetch_link_previews`).
    pub fetch_link_previews: bool,
    /// Log a completed task-timer session as an org CLOCK entry in the note's
    /// `:LOGBOOK:` drawer (from `Config::log_clock_entries`).
    pub log_clock_entries: bool,
    /// UI theme at startup (from `Config::theme_dark`); pushed into the window
    /// on build/show. The in-session source of truth after that is the window's
    /// own `theme-dark` property — the toggle callback persists it back out.
    pub theme_dark: bool,
    /// Keyboard-nav history: screens (`(section, view)`) visited before the
    /// current one (`⌘[` pops here), most-recent last.
    pub nav_back: Mutex<Vec<(String, String)>>,
    /// Screens undone via `⌘[` (`⌘]` pops here), most-recent last. Cleared on
    /// any *new* navigation so it only ever replays an undone `⌘[`.
    pub nav_fwd: Mutex<Vec<(String, String)>>,
}

pub fn ingest_event(state: &AppState, ev: &CaptureEvent) -> Result<(), String> {
    let store = state.store.lock().map_err(|e| e.to_string())?;
    store
        .ingest(ev, &state.images)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

pub fn current_results(state: &AppState, now_ms: i64) -> Vec<Entry> {
    // Recover the guard even if a mutex was poisoned by a panic on another thread
    // (e.g. an ingest failure on the watcher thread). A poisoned lock must never
    // crash the UI thread — this runs inside Slint callbacks, and a panic there
    // unwinds through the event loop and kills the whole app.
    let ui = state.ui.lock().unwrap_or_else(|e| e.into_inner());
    let q = to_query(&ui, now_ms);
    drop(ui);
    let store = state.store.lock().unwrap_or_else(|e| e.into_inner());
    store.search(&q).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use magpie_core::{open_in_memory, CaptureEvent, Content};

    fn state() -> AppState {
        AppState {
            store: std::sync::Mutex::new(open_in_memory().unwrap()),
            images: crate::image_cache::FsImageStore {
                dir: std::env::temp_dir().join("magpie-appstate-test"),
            },
            ui: std::sync::Mutex::new(crate::viewmodel::UiState::new()),
            merge_set: std::sync::Mutex::new(Vec::new()),
            screenshare: std::sync::Mutex::new(false),
            mask_apps: Vec::new(),
            mask_patterns: Vec::new(),
            mask_visible_chars: 3,
            open_to_today: false,
            fetch_link_favicons: true,
            fetch_link_previews: true,
            log_clock_entries: true,
            theme_dark: true,
            nav_back: std::sync::Mutex::new(Vec::new()),
            nav_fwd: std::sync::Mutex::new(Vec::new()),
        }
    }

    #[test]
    fn ingest_then_query_returns_entry() {
        let s = state();
        ingest_event(
            &s,
            &CaptureEvent {
                content: Content::Text("hello".into()),
                source_app: None,
                copied_at_ms: 1,
            },
        )
        .unwrap();
        let rows = current_results(&s, 1_000);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].full_text, "hello");
    }

    #[test]
    fn current_results_survives_a_poisoned_lock() {
        let s = state();
        ingest_event(
            &s,
            &CaptureEvent {
                content: Content::Text("hello".into()),
                source_app: None,
                copied_at_ms: 1,
            },
        )
        .unwrap();
        // Poison the store mutex the way a panicking watcher thread would: panic
        // while holding the guard. Silence the default panic print for the test.
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _g = s.store.lock().unwrap();
            panic!("poison the store lock");
        }));
        std::panic::set_hook(prev);
        assert!(res.is_err() && s.store.is_poisoned());
        // The UI thread must NOT panic — it recovers the guard and still queries.
        let rows = current_results(&s, 1_000);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].full_text, "hello");
    }

    #[test]
    fn ui_text_filter_applies() {
        let s = state();
        ingest_event(
            &s,
            &CaptureEvent {
                content: Content::Text("alpha".into()),
                source_app: None,
                copied_at_ms: 1,
            },
        )
        .unwrap();
        ingest_event(
            &s,
            &CaptureEvent {
                content: Content::Text("beta".into()),
                source_app: None,
                copied_at_ms: 2,
            },
        )
        .unwrap();
        s.ui.lock().unwrap().text = "alpha".into();
        let rows = current_results(&s, 1_000);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].full_text, "alpha");
    }
}
