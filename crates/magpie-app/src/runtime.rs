use magpie_app::app_state::{current_results, ingest_event, AppState};
use magpie_app::config::Config;
use magpie_app::paste_action::{perform_paste, resolve_quick_paste, PasteKind};
use crate::{EntryRow, LauncherWindow};
use magpie_core::{open, Entry};
use magpie_platform::os::hotkeys::Hotkeys;
use magpie_platform::os::paste::EnigoPaster;
use magpie_platform::os::source_app::ActiveWinSource;
use magpie_platform::{
    default_app_denylist, default_ignore_regexes, parse_hotkey, CapturePolicy, Watcher,
};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub fn data_dir() -> std::path::PathBuf {
    dirs::data_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("magpie")
}

pub fn build_state() -> Arc<AppState> {
    let dir = data_dir();
    std::fs::create_dir_all(&dir).ok();
    let store = open(&dir.join("magpie.sqlite3")).expect("open db");
    Arc::new(AppState {
        store: std::sync::Mutex::new(store),
        images: magpie_app::image_cache::FsImageStore {
            dir: dir.join("images"),
        },
        ui: std::sync::Mutex::new(magpie_app::viewmodel::UiState::new()),
    })
}

fn preview_title(e: &Entry) -> String {
    let line = e.full_text.lines().next().unwrap_or("").trim();
    if line.is_empty() {
        e.kind.as_str().to_string()
    } else {
        line.chars().take(80).collect()
    }
}

fn to_rows(entries: &[Entry]) -> Vec<EntryRow> {
    entries
        .iter()
        .map(|e| EntryRow {
            title: SharedString::from(preview_title(e)),
            subtitle: SharedString::from(format!("{} · copied {}×", e.kind.as_str(), e.copy_count)),
            kind: SharedString::from(e.kind.as_str()),
        })
        .collect()
}

/// Recompute results from the current UI state and push them into the window.
fn refresh(ui: &LauncherWindow, state: &AppState) {
    let results = current_results(state, now_ms());
    let detail = results
        .first()
        .map(|e| e.full_text.clone())
        .unwrap_or_default();
    ui.set_entries(ModelRc::new(VecModel::from(to_rows(&results))));
    ui.set_detail_text(SharedString::from(detail));
}

/// Poll the clipboard on a background thread; refresh the window on each capture.
fn spawn_watcher(state: Arc<AppState>, denylist: Vec<String>, weak: slint::Weak<LauncherWindow>) {
    std::thread::spawn(move || {
        let clip = match magpie_platform::platform_clipboard() {
            Ok(c) => c,
            Err(_) => return,
        };
        let mut policy = CapturePolicy::new();
        policy.ignore_regexes = default_ignore_regexes();
        policy.app_denylist = denylist;
        let mut watcher = Watcher::new(clip, ActiveWinSource, policy);
        loop {
            if let Some(ev) = watcher.poll_once(now_ms()) {
                if ingest_event(&state, &ev).is_ok() {
                    let w = weak.clone();
                    let s = state.clone();
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(ui) = w.upgrade() {
                            refresh(&ui, &s);
                        }
                    });
                }
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    });
}

enum HotAction {
    Launcher,
    QuickPaste(usize),
}

/// Register launcher + quick-paste hotkeys and drain their events on a thread.
fn spawn_hotkeys(
    cfg: &Config,
    state: Arc<AppState>,
    weak: slint::Weak<LauncherWindow>,
) -> Result<Hotkeys, String> {
    let hk = Hotkeys::new()?;
    let mut actions: HashMap<u32, HotAction> = HashMap::new();

    if let Ok(spec) = parse_hotkey(&cfg.launcher_hotkey) {
        if let Ok(id) = hk.register(&spec) {
            actions.insert(id, HotAction::Launcher);
        }
    }
    for (i, combo) in cfg.quick_paste_hotkeys.iter().enumerate() {
        if let Ok(spec) = parse_hotkey(combo) {
            if let Ok(id) = hk.register(&spec) {
                actions.insert(id, HotAction::QuickPaste(i + 1));
            }
        }
    }

    let auto = cfg.paste_on_select;
    std::thread::spawn(move || {
        let rx = global_hotkey::GlobalHotKeyEvent::receiver();
        loop {
            if let Ok(ev) = rx.recv() {
                if ev.state != global_hotkey::HotKeyState::Pressed {
                    continue;
                }
                match actions.get(&ev.id) {
                    Some(HotAction::Launcher) => {
                        let w = weak.clone();
                        let s = state.clone();
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(ui) = w.upgrade() {
                                refresh(&ui, &s);
                                let _ = ui.show();
                            }
                        });
                    }
                    Some(HotAction::QuickPaste(slot)) => {
                        let recent = current_results(&state, now_ms());
                        if let Some(entry) = resolve_quick_paste(&recent, *slot) {
                            if let Ok(mut clip) = magpie_platform::platform_clipboard() {
                                let _ = perform_paste(
                                    &mut clip,
                                    &EnigoPaster,
                                    entry,
                                    PasteKind::Formatted,
                                    auto,
                                );
                            }
                        }
                    }
                    None => {}
                }
            }
        }
    });

    Ok(hk)
}

pub fn start() {
    let cfg = magpie_app::config::load_or_default(&data_dir().join("config.toml"));
    let mut denylist = default_app_denylist();
    denylist.extend(cfg.app_denylist.clone());

    let state = build_state();
    let ui = LauncherWindow::new().expect("create window");
    let weak = ui.as_weak();

    // Callbacks: search updates the UI state and refreshes; activate/copy-only paste.
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_search_changed(move |text| {
            if let Ok(mut u) = s.ui.lock() {
                u.text = text.to_string();
            }
            if let Some(ui) = w.upgrade() {
                refresh(&ui, &s);
            }
        });
    }
    {
        let s = state.clone();
        let auto = cfg.paste_on_select;
        ui.on_activate(move |idx| {
            let recent = current_results(&s, now_ms());
            if let Some(entry) = recent.get(idx as usize) {
                if let Ok(mut clip) = magpie_platform::platform_clipboard() {
                    let _ =
                        perform_paste(&mut clip, &EnigoPaster, entry, PasteKind::Formatted, auto);
                }
            }
        });
    }
    {
        let s = state.clone();
        ui.on_copy_only(move |idx| {
            let recent = current_results(&s, now_ms());
            if let Some(entry) = recent.get(idx as usize) {
                if let Ok(mut clip) = magpie_platform::platform_clipboard() {
                    let _ =
                        perform_paste(&mut clip, &EnigoPaster, entry, PasteKind::PlainText, false);
                }
            }
        });
    }

    refresh(&ui, &state);
    spawn_watcher(state.clone(), denylist, weak.clone());
    // Keep the hotkey manager alive for the whole run.
    let _hotkeys = spawn_hotkeys(&cfg, state.clone(), weak.clone());
    // Keep the tray icon alive for the whole run.
    let _tray = build_tray();

    ui.run().expect("run event loop");
}

/// Build a minimal tray icon with a Quit item, and drain its menu events.
fn build_tray() -> Option<tray_icon::TrayIcon> {
    use tray_icon::menu::{Menu, MenuEvent, MenuItem};
    use tray_icon::TrayIconBuilder;

    let menu = Menu::new();
    let quit = MenuItem::new("Quit Magpie", true, None);
    menu.append(&quit).ok()?;
    let quit_id = quit.id().clone();

    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip("Magpie")
        .with_title("🐦")
        .build()
        .ok()?;

    std::thread::spawn(move || {
        let rx = MenuEvent::receiver();
        while let Ok(ev) = rx.recv() {
            if ev.id == quit_id {
                let _ = slint::invoke_from_event_loop(|| {
                    let _ = slint::quit_event_loop();
                });
            }
        }
    });

    Some(tray)
}
