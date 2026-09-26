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
    /// Session-only capture pause (the watcher skips polling while set). Not persisted.
    pub paused: std::sync::atomic::AtomicBool,
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
    /// Selected colour-theme id at startup (from `Config::theme`); mapped to a
    /// palette index and pushed into the window on build/show. After that the
    /// in-session source of truth is the window's `theme-index`; the picker
    /// callback persists the chosen id back out.
    pub theme: String,
    /// Region for locale-dependent holiday answers (from `Config::holiday_region`);
    /// parsed to `magpie_core::holidays::Region` when building the date card.
    pub holiday_region: String,
    /// World clock (screen 16): home zone (session-immutable), the persisted city
    /// list, the current query's target zones (shown on top of the list), and the
    /// chosen 24-hour-strip hour (`-1` = follow the current home hour).
    pub world_clock_home: String,
    pub world_clock_cities: Mutex<Vec<String>>,
    pub wc_session_targets: Mutex<Vec<String>>,
    pub wc_chosen_hour: Mutex<i32>,
    /// File search (screens 19/21/22/23): active locations, match/case mode, tab,
    /// which files are marked "viewed", and the last result's file paths (in shown
    /// order, for index→action). `search_gen` cancels a stale background run.
    pub search: Mutex<SearchState>,
    pub search_gen: std::sync::atomic::AtomicU64,
    /// Keyboard-nav history: screens (`(section, view)`) visited before the
    /// current one (`⌘[` pops here), most-recent last.
    pub nav_back: Mutex<Vec<(String, String)>>,
    /// Screens undone via `⌘[` (`⌘]` pops here), most-recent last. Cleared on
    /// any *new* navigation so it only ever replays an undone `⌘[`.
    pub nav_fwd: Mutex<Vec<(String, String)>>,
    /// Data view (screen 13/14): the raw text of the currently-open dataset, the
    /// active "Read as" override, its display name, and the session's recent-data
    /// rail. Not persisted — durable recent-data storage waits on the Phase 7
    /// migration slot (see the redesign plan), so this is intentionally in-memory.
    pub data_input: Mutex<String>,
    pub data_read_as: Mutex<magpie_core::table::ReadAs>,
    pub data_name: Mutex<String>,
    pub data_recent: Mutex<Vec<DataSource>>,
    pub data_chart: Mutex<ChartState>,
    /// Calculator Graphs view (screen 12): the two functions currently plotted.
    /// `graph_g` empty = a single-function plot. Set when a graph is opened.
    pub calc_graph_f: Mutex<String>,
    pub calc_graph_g: Mutex<String>,
}

/// File-search session state (screens 19/21/22/23).
#[derive(Debug, Clone, Default)]
pub struct SearchState {
    /// Active location chips (display form, e.g. `~/code/hobby/magpie`).
    pub locations: Vec<String>,
    /// 0 Text, 1 Regex, 2 Sed.
    pub mode: i32,
    /// 0 Smart, 1 Sensitive, 2 Insensitive.
    pub case: i32,
    /// 0 All, 1 Names, 2 Contents, 3 Documents.
    pub tab: i32,
    pub hide_viewed: bool,
    /// Absolute paths marked "viewed".
    pub viewed: std::collections::HashSet<String>,
    /// The last result's file paths, in shown order (for index → action).
    pub result_paths: Vec<String>,
}

/// One entry in the Data view's session recent-data rail.
#[derive(Debug, Clone)]
pub struct DataSource {
    pub name: String,
    pub meta: String,
    pub input: String,
    pub read_as: magpie_core::table::ReadAs,
}

/// Chart-panel state for the Data view (screen 14). `x`/`y` index into the parsed
/// table's numeric columns; `kind` is 0 Line, 1 Bar, 2 Scatter, 3 Hist.
#[derive(Debug, Clone)]
pub struct ChartState {
    pub open: bool,
    pub kind: i32,
    pub x: usize,
    pub y: usize,
    pub log: bool,
}

impl Default for ChartState {
    fn default() -> Self {
        Self {
            open: false,
            kind: 2,
            x: 0,
            y: 0,
            log: false,
        }
    }
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
            paused: std::sync::atomic::AtomicBool::new(false),
            mask_apps: Vec::new(),
            mask_patterns: Vec::new(),
            mask_visible_chars: 3,
            open_to_today: false,
            fetch_link_favicons: true,
            fetch_link_previews: true,
            log_clock_entries: true,
            theme: "dark".into(),
            holiday_region: "SE".into(),
            world_clock_home: "Europe/Stockholm".into(),
            world_clock_cities: std::sync::Mutex::new(Vec::new()),
            wc_session_targets: std::sync::Mutex::new(Vec::new()),
            wc_chosen_hour: std::sync::Mutex::new(-1),
            search: std::sync::Mutex::new(crate::app_state::SearchState::default()),
            search_gen: std::sync::atomic::AtomicU64::new(0),
            nav_back: std::sync::Mutex::new(Vec::new()),
            nav_fwd: std::sync::Mutex::new(Vec::new()),
            data_input: std::sync::Mutex::new(String::new()),
            data_read_as: std::sync::Mutex::new(magpie_core::table::ReadAs::Auto),
            data_name: std::sync::Mutex::new(String::new()),
            data_recent: std::sync::Mutex::new(Vec::new()),
            data_chart: std::sync::Mutex::new(crate::app_state::ChartState::default()),
            calc_graph_f: std::sync::Mutex::new(String::new()),
            calc_graph_g: std::sync::Mutex::new(String::new()),
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
