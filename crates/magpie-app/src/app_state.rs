use crate::image_cache::FsImageStore;
use crate::viewmodel::{to_query, UiState};
use magpie_core::{CaptureEvent, Entry, Store};
use std::sync::Mutex;

pub struct AppState {
    pub store: Mutex<Store>,
    pub images: FsImageStore,
    pub ui: Mutex<UiState>,
}

pub fn ingest_event(state: &AppState, ev: &CaptureEvent) -> Result<(), String> {
    let store = state.store.lock().map_err(|e| e.to_string())?;
    store
        .ingest(ev, &state.images)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

pub fn current_results(state: &AppState, now_ms: i64) -> Vec<Entry> {
    let ui = state.ui.lock().expect("ui lock");
    let q = to_query(&ui, now_ms);
    drop(ui);
    let store = state.store.lock().expect("store lock");
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
