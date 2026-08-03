use crate::{ActionItem, AppItem, Bar, EntryRow, LauncherWindow, SlotItem};
use magpie_app::app_state::{current_results, ingest_event, AppState};
use magpie_app::config::Config;
use magpie_app::favicon;
use magpie_app::format_time::{abs_date, relative_time};
use magpie_app::grouping;
use magpie_app::mask_view::{mask_render, should_mask, MaskRules};
use magpie_app::merge_view::separator_str;
use magpie_app::paste_action::{perform_paste, resolve_slot_or_recent, PasteKind};
use magpie_app::retention::policy_from_config;
use magpie_app::stats_view::{range_from_index, to_bars};
use magpie_core::{open, Content, Entry, Kind, RetentionPolicy, Stats, StatsRange, Totals};
use magpie_platform::os::hotkeys::Hotkeys;
use magpie_platform::os::paste::EnigoPaster;
use magpie_platform::os::source_app::ActiveWinSource;
use magpie_platform::{
    default_app_denylist, default_ignore_regexes, parse_hotkey, CapturePolicy, Clipboard, Paster,
    Watcher,
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

pub fn build_state(cfg: &Config) -> Arc<AppState> {
    let dir = data_dir();
    std::fs::create_dir_all(&dir).ok();
    let store = open(&dir.join("magpie.sqlite3")).expect("open db");
    Arc::new(AppState {
        store: std::sync::Mutex::new(store),
        images: magpie_app::image_cache::FsImageStore {
            dir: dir.join("images"),
        },
        ui: std::sync::Mutex::new(magpie_app::viewmodel::UiState::new()),
        merge_set: std::sync::Mutex::new(Vec::new()),
        screenshare: std::sync::Mutex::new(false),
        mask_apps: cfg.mask_apps.clone(),
        mask_patterns: cfg.mask_patterns.clone(),
        mask_visible_chars: cfg.mask_visible_chars.max(0),
    })
}

/// The glyph shown at the left of a list row for each content kind.
fn type_glyph(kind: &Kind) -> &'static str {
    match kind {
        Kind::Link => "🔗",
        Kind::Color => "🎨",
        Kind::Email => "✉️",
        Kind::Image => "🖼️",
        Kind::File => "📁",
        Kind::Text | Kind::Rtf | Kind::Html => "📄",
    }
}

/// A row badge like "3 lines" when the text has more than one non-empty line.
fn line_badge(full_text: &str) -> String {
    let n = full_text.lines().filter(|l| !l.trim().is_empty()).count();
    if n > 1 {
        format!("{n} lines")
    } else {
        String::new()
    }
}

fn preview_title(e: &Entry) -> String {
    let line = e.full_text.lines().next().unwrap_or("").trim();
    if line.is_empty() {
        e.kind.as_str().to_string()
    } else {
        line.chars().take(80).collect()
    }
}

#[allow(clippy::too_many_arguments)]
fn to_rows(
    entries: &[Entry],
    slots: &HashMap<i64, i64>,
    tags: &HashMap<i64, Vec<String>>,
    merge_set: &[i32],
    app_names: &HashMap<i64, String>,
    app_icons: &HashMap<i64, String>,
    favicon_dir: &std::path::Path,
    now: i64,
    rules: &MaskRules,
    visible: usize,
) -> Vec<EntryRow> {
    entries
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let source = app_names
                .get(&e.id)
                .cloned()
                .unwrap_or_else(|| "—".to_string());
            let when = relative_time(e.last_copied_at_ms, now);
            // Pick the per-entry icon: a link's cached site favicon, else the
            // source app icon; then load it for Slint.
            let icon_path: Option<String> = if matches!(e.kind, Kind::Link) {
                favicon::domain_of(&e.full_text)
                    .map(|d| favicon::favicon_cache_path(favicon_dir, &d))
                    .filter(|p| p.exists())
                    .and_then(|p| p.to_str().map(|s| s.to_string()))
                    .or_else(|| app_icons.get(&e.id).cloned())
            } else {
                app_icons.get(&e.id).cloned()
            };
            let (icon_img, has_icon) = match icon_path.as_deref().map(std::path::Path::new) {
                // Guard on existence so a stale DB icon_path doesn't spam load errors.
                Some(p) if p.exists() => match slint::Image::load_from_path(p) {
                    Ok(img) => (img, true),
                    Err(_) => (slint::Image::default(), false),
                },
                _ => (slint::Image::default(), false),
            };
            let masked = should_mask(
                rules,
                app_names.get(&e.id).map(|s| s.as_str()),
                &e.full_text,
            );
            let base_title = preview_title(e);
            let title = if masked {
                mask_render(&base_title, visible)
            } else {
                base_title
            };
            let tagline = tags
                .get(&e.id)
                .map(|ts| {
                    ts.iter()
                        .map(|t| format!("#{t}"))
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .unwrap_or_default();
            let subtitle = if tagline.is_empty() {
                format!("{source} · {when}")
            } else {
                format!("{source} · {when} · {tagline}")
            };
            let size = format!("{} chars · {} lines", e.char_count, e.line_count);
            EntryRow {
                title: SharedString::from(title),
                subtitle: SharedString::from(subtitle),
                kind: SharedString::from(e.kind.as_str()),
                slot: *slots.get(&e.id).unwrap_or(&0) as i32,
                merged: merge_set.contains(&(i as i32)),
                badge: SharedString::from(line_badge(&e.full_text)),
                glyph: SharedString::from(type_glyph(&e.kind)),
                source: SharedString::from(source),
                when: SharedString::from(when),
                copied: e.copy_count as i32,
                size: SharedString::from(size),
                masked,
                icon: icon_img,
                has_icon,
                words: e.word_count as i32,
                section: SharedString::from(
                    grouping::section_for(e.last_copied_at_ms, now).label(),
                ),
                copied_date: SharedString::from(abs_date(e.last_copied_at_ms)),
                pinned: e.pinned,
            }
        })
        .collect()
}

/// Recompute results from the current UI state and push them into the window.
fn refresh(ui: &LauncherWindow, state: &AppState) {
    // Push the window's tag-filter into UiState before querying.
    {
        let t = ui.get_tag_filter().to_string();
        if let Ok(mut u) = state.ui.lock() {
            u.tag = if t.is_empty() { None } else { Some(t) };
        }
    }
    let results = current_results(state, now_ms());

    let screenshare = state.screenshare.lock().map(|g| *g).unwrap_or(false);
    let rules = MaskRules::build(&state.mask_apps, &state.mask_patterns, screenshare);
    let visible = state.mask_visible_chars.max(0) as usize;
    ui.set_screenshare(screenshare);
    // Advanced-search "filtered" indicator.
    if let Ok(u) = state.ui.lock() {
        let filtered = u.app_filter.is_some()
            || u.pinned_only
            || !matches!(u.time_filter, magpie_app::viewmodel::TimeFilter::All);
        ui.set_filtered(filtered);
    }

    let (slots, tag_map, all_tags, app_names, app_icons, slot_items) = match state.store.lock() {
        Ok(store) => {
            let slot_pairs = store.slot_map().unwrap_or_default(); // (slot, entry_id)
            let slots: HashMap<i64, i64> =
                slot_pairs.iter().map(|&(slot, eid)| (eid, slot)).collect();
            // Speed-dial: (slot, short title) for each assigned slot, ascending.
            let mut slot_items: Vec<(i64, String)> = Vec::new();
            for &(slot, _eid) in &slot_pairs {
                if let Ok(Some(e)) = store.slot_entry(slot) {
                    slot_items.push((slot, preview_title(&e)));
                }
            }
            slot_items.sort_by_key(|(n, _)| *n);
            let mut tag_map: HashMap<i64, Vec<String>> = HashMap::new();
            for (eid, tag) in store.tag_pairs().unwrap_or_default() {
                tag_map.entry(eid).or_default().push(tag);
            }
            let all_tags: Vec<String> = store.all_tags().unwrap_or_default();
            let app_names: HashMap<i64, String> = store
                .app_name_pairs()
                .unwrap_or_default()
                .into_iter()
                .collect();
            let app_icons: HashMap<i64, String> = store
                .app_icon_pairs()
                .unwrap_or_default()
                .into_iter()
                .collect();
            (slots, tag_map, all_tags, app_names, app_icons, slot_items)
        }
        Err(_) => (
            HashMap::new(),
            HashMap::new(),
            Vec::new(),
            HashMap::new(),
            HashMap::new(),
            Vec::new(),
        ),
    };
    ui.set_slots(ModelRc::new(VecModel::from(
        slot_items
            .into_iter()
            .map(|(n, title)| SlotItem {
                n: n as i32,
                title: SharedString::from(title),
                filled: true,
            })
            .collect::<Vec<_>>(),
    )));

    let sel = ui.get_selected() as usize;
    let selected_tags: Vec<SharedString> = results
        .get(sel)
        .and_then(|e| tag_map.get(&e.id))
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(SharedString::from)
        .collect();
    ui.set_selected_tags(ModelRc::new(VecModel::from(selected_tags)));
    ui.set_all_tags(ModelRc::new(VecModel::from(
        all_tags
            .into_iter()
            .map(SharedString::from)
            .collect::<Vec<_>>(),
    )));

    let merge_set = state
        .merge_set
        .lock()
        .map(|m| m.clone())
        .unwrap_or_default();
    ui.set_merge_count(merge_set.len() as i32);

    // The preview binds directly to the selected row's `full` text in the UI,
    // so there is no separate detail string to push here.
    ui.set_entries(ModelRc::new(VecModel::from(to_rows(
        &results,
        &slots,
        &tag_map,
        &merge_set,
        &app_names,
        &app_icons,
        &data_dir().join("favicons"),
        now_ms(),
        &rules,
        visible,
    ))));

    // Keep the selection where it was across refresh/reopen; only reset when it
    // would point past the end.
    let cur = ui.get_selected();
    if cur < 0 || cur as usize >= results.len() {
        ui.set_selected(0);
    }
    // Rebuild the virtualized preview for the (possibly new) selection.
    ui.invoke_rebuild_preview();
}

/// Refresh results and show the launcher window. Shared by the launcher hotkey
/// and the tray (left-click + "Show Magpie").
fn show_window(ui: &LauncherWindow, state: &AppState) {
    refresh(ui, state);
    // Nudge the scroll-follow so the retained selection is back in view.
    let sel = ui.get_selected();
    ui.set_selected(-1);
    ui.set_selected(sel);
    // Capture the paste target: whatever app is frontmost right before we show.
    match magpie_platform::SourceApp::frontmost(&ActiveWinSource {
        cache_dir: data_dir().join("app_icons"),
    }) {
        Some(app) => {
            ui.set_target_app(SharedString::from(app.display_name.clone()));
            let (img, has) = app
                .icon_path
                .as_deref()
                .map(std::path::Path::new)
                .filter(|p| p.exists())
                .and_then(|p| slint::Image::load_from_path(p).ok())
                .map(|i| (i, true))
                .unwrap_or((slint::Image::default(), false));
            ui.set_target_icon(img);
            ui.set_target_has_icon(has);
        }
        None => {
            ui.set_target_app(SharedString::from(""));
            ui.set_target_has_icon(false);
        }
    }
    let _ = ui.show();
}

/// The ⌘K action set: (id, icon, label, shortcut). Dispatch by id in Slint's
/// `run-action`.
const ACTIONS: &[(&str, &str, &str, &str)] = &[
    ("paste", "📋", "Paste", "⏎"),
    ("copy", "📄", "Copy", "⌘C"),
    ("keep", "📎", "Paste & keep open", "⌥⏎"),
    ("edit", "✏️", "Edit", "⌘E"),
    ("snippet", "🧩", "New snippet", "⌘N"),
    ("pin", "📌", "Pin / Unpin", "⌘P"),
    ("merge", "➕", "Add to merge", "⌘M"),
    ("delete", "🗑", "Delete", "⌃X"),
];

/// Push the ⌘K action list filtered by `query` (case-insensitive label match)
/// and keep `action-selected` in range.
fn set_actions_filtered(ui: &LauncherWindow, query: &str) {
    let q = query.to_lowercase();
    let items: Vec<ActionItem> = ACTIONS
        .iter()
        .filter(|(_, _, label, _)| q.is_empty() || label.to_lowercase().contains(&q))
        .map(|(id, icon, label, key)| ActionItem {
            icon: SharedString::from(*icon),
            label: SharedString::from(*label),
            key: SharedString::from(*key),
            id: SharedString::from(*id),
        })
        .collect();
    let len = items.len() as i32;
    ui.set_actions(ModelRc::new(VecModel::from(items)));
    if ui.get_action_selected() >= len {
        ui.set_action_selected((len - 1).max(0));
    }
}

/// After hiding, wait for focus to return to the previous app, send the paste
/// keystroke, and optionally re-show the window. Runs off the UI thread.
fn spawn_paste(weak: slint::Weak<LauncherWindow>, keep_open: bool) {
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(120));
        let _ = magpie_platform::Paster::paste(&EnigoPaster);
        if keep_open {
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = weak.upgrade() {
                    let _ = ui.show();
                }
            });
        }
    });
}

/// The Enter / ⌘Enter flow: copy the entry at `idx`, hide Magpie so the previous
/// app regains focus, then paste into it. `keep_open` re-shows Magpie afterward.
fn paste_and_close(
    state: &AppState,
    weak: &slint::Weak<LauncherWindow>,
    idx: usize,
    keep_open: bool,
) {
    let recent = current_results(state, now_ms());
    if let Some(entry) = recent.get(idx) {
        if let Ok(mut clip) = magpie_platform::platform_clipboard() {
            // Copy only here; the keystroke is sent after the window hides.
            let _ = perform_paste(&mut clip, &EnigoPaster, entry, PasteKind::Formatted, false);
        }
    }
    if let Some(ui) = weak.upgrade() {
        let _ = ui.hide();
    }
    spawn_paste(weak.clone(), keep_open);
}

fn to_slint_bars(items: &[(String, String, i64)]) -> ModelRc<Bar> {
    let bars: Vec<Bar> = to_bars(items)
        .into_iter()
        .map(|b| Bar {
            label: b.label.into(),
            display: b.display.into(),
            value_norm: b.value_norm,
        })
        .collect();
    ModelRc::new(VecModel::from(bars))
}

fn truncate(s: &str, n: usize) -> String {
    let one_line = s.lines().next().unwrap_or("");
    one_line.chars().take(n).collect()
}

fn empty_stats() -> Stats {
    Stats {
        totals: Totals {
            copies: 0,
            unique_entries: 0,
            distinct_apps: 0,
        },
        most_copied: Vec::new(),
        over_time: Vec::new(),
        per_app: Vec::new(),
        by_type: Vec::new(),
    }
}

/// Query stats for the selected range and push all series into the window.
fn refresh_stats(ui: &LauncherWindow, state: &AppState, range_index: i32) {
    let range: StatsRange = range_from_index(range_index, now_ms());
    let stats: Stats = match state.store.lock() {
        Ok(store) => store.stats(&range, 10).unwrap_or_else(|_| empty_stats()),
        Err(_) => empty_stats(),
    };

    let ot: Vec<(String, String, i64)> = stats
        .over_time
        .iter()
        .map(|b| (String::new(), String::new(), b.count))
        .collect();
    let mc: Vec<(String, String, i64)> = stats
        .most_copied
        .iter()
        .map(|m| (truncate(&m.preview, 40), m.count.to_string(), m.count))
        .collect();
    let pa: Vec<(String, String, i64)> = stats
        .per_app
        .iter()
        .map(|a| (a.name.clone(), a.count.to_string(), a.count))
        .collect();
    let bt: Vec<(String, String, i64)> = stats
        .by_type
        .iter()
        .map(|k| (k.kind.as_str().to_string(), k.count.to_string(), k.count))
        .collect();

    ui.set_over_time_bars(to_slint_bars(&ot));
    ui.set_most_copied_bars(to_slint_bars(&mc));
    ui.set_per_app_bars(to_slint_bars(&pa));
    ui.set_by_type_bars(to_slint_bars(&bt));
    ui.set_totals_line(SharedString::from(format!(
        "{} copies · {} entries · {} apps",
        stats.totals.copies, stats.totals.unique_entries, stats.totals.distinct_apps
    )));
}

/// Apply retention caps (prune DB + delete image files). No-op when off.
fn sweep_retention(state: &AppState, policy: &RetentionPolicy) {
    if policy.is_noop() {
        return;
    }
    let removed = {
        let store = match state.store.lock() {
            Ok(s) => s,
            Err(_) => return,
        };
        match store.enforce_retention(policy, now_ms()) {
            Ok(r) => r,
            Err(_) => return,
        }
    };
    state.images.remove_paths(&removed.image_paths);
}

/// Poll the clipboard on a background thread; refresh the window on each capture.
fn spawn_watcher(
    state: Arc<AppState>,
    denylist: Vec<String>,
    retention: RetentionPolicy,
    fetch_favicons: bool,
    weak: slint::Weak<LauncherWindow>,
) {
    std::thread::spawn(move || {
        let clip = match magpie_platform::platform_clipboard() {
            Ok(c) => c,
            Err(_) => return,
        };
        let mut policy = CapturePolicy::new();
        policy.ignore_regexes = default_ignore_regexes();
        policy.app_denylist = denylist;
        let mut watcher = Watcher::new(
            clip,
            ActiveWinSource {
                cache_dir: data_dir().join("app_icons"),
            },
            policy,
        );
        let push_refresh = |weak: &slint::Weak<LauncherWindow>, state: &Arc<AppState>| {
            let w = weak.clone();
            let s = state.clone();
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = w.upgrade() {
                    refresh(&ui, &s);
                }
            });
        };
        loop {
            if let Some(ev) = watcher.poll_once(now_ms()) {
                if ingest_event(&state, &ev).is_ok() {
                    sweep_retention(&state, &retention);
                    push_refresh(&weak, &state);
                    // Best-effort site favicon for link entries; refresh again once
                    // it's cached so the icon appears without waiting on the fetch.
                    if fetch_favicons {
                        if let Content::Text(t) = &ev.content {
                            if let Some(domain) = favicon::domain_of(t) {
                                let dir = data_dir().join("favicons");
                                if favicon::ensure_favicon(&dir, &domain, favicon::fetch_favicon)
                                    .is_some()
                                {
                                    push_refresh(&weak, &state);
                                }
                            }
                        }
                    }
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
                                show_window(&ui, &s);
                            }
                        });
                    }
                    Some(HotAction::QuickPaste(slot)) => {
                        let recent = current_results(&state, now_ms());
                        let slotted = state
                            .store
                            .lock()
                            .ok()
                            .and_then(|st| st.slot_entry(*slot as i64).ok().flatten());
                        if let Some(entry) = resolve_slot_or_recent(slotted, &recent, *slot) {
                            if let Ok(mut clip) = magpie_platform::platform_clipboard() {
                                let _ = perform_paste(
                                    &mut clip,
                                    &EnigoPaster,
                                    &entry,
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

    let state = build_state(&cfg);
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
        // Build the (bounded) preview line model for the selected entry. Only the
        // visible lines are shaped by the ListView, so huge entries stay instant.
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_preview_select(move |idx, masked, revealed| {
            const PREVIEW_LINE_CAP: usize = 20_000;
            let results = current_results(&s, now_ms());
            let entry = if idx >= 0 {
                results.get(idx as usize)
            } else {
                None
            };
            let (lines, truncated) = match entry {
                Some(e) if masked && !revealed => {
                    let visible = s.mask_visible_chars.max(0) as usize;
                    (
                        vec![SharedString::from(mask_render(&e.full_text, visible))],
                        false,
                    )
                }
                Some(e) => {
                    let mut lines: Vec<SharedString> = Vec::new();
                    let mut truncated = false;
                    for (n, line) in e.full_text.split('\n').enumerate() {
                        if n >= PREVIEW_LINE_CAP {
                            truncated = true;
                            break;
                        }
                        lines.push(SharedString::from(line));
                    }
                    (lines, truncated)
                }
                None => (Vec::new(), false),
            };
            if let Some(ui) = w.upgrade() {
                ui.set_preview_lines(ModelRc::new(VecModel::from(lines)));
                ui.set_preview_truncated(truncated);
            }
        });
    }
    {
        let s = state.clone();
        ui.on_open_in_editor(move |idx| {
            let results = current_results(&s, now_ms());
            if let Some(e) = results.get(idx as usize) {
                let _ = magpie_app::external_editor::open_text(&e.full_text, &e.content_hash);
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_activate(move |idx| {
            paste_and_close(&s, &w, idx as usize, false);
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_activate_nth(move |n| {
            if n >= 1 {
                paste_and_close(&s, &w, (n - 1) as usize, false);
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_set_type_filter(move |idx| {
            if let Ok(mut u) = s.ui.lock() {
                u.type_filter = magpie_app::viewmodel::type_filter_from_index(idx);
            }
            if let Some(ui) = w.upgrade() {
                refresh(&ui, &s);
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_set_sort(move |idx| {
            if let Ok(mut u) = s.ui.lock() {
                u.sort = magpie_app::viewmodel::sort_from_index(idx);
            }
            if let Some(ui) = w.upgrade() {
                refresh(&ui, &s);
            }
        });
    }
    // ---- Speed-dial slots ----
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_paste_slot(move |n| {
            // Paste the entry assigned to slot n (or the Nth recent) via hide→paste.
            let entry = s
                .store
                .lock()
                .ok()
                .and_then(|st| st.slot_entry(n as i64).ok().flatten());
            let recent = current_results(&s, now_ms());
            if let Some(entry) = resolve_slot_or_recent(entry, &recent, n as usize) {
                if let Ok(mut clip) = magpie_platform::platform_clipboard() {
                    let _ =
                        perform_paste(&mut clip, &EnigoPaster, &entry, PasteKind::Formatted, false);
                }
            }
            if let Some(ui) = w.upgrade() {
                let _ = ui.hide();
            }
            spawn_paste(w.clone(), false);
        });
    }
    // ---- ⌘F advanced search ----
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_open_search(move || {
            if let Some(ui) = w.upgrade() {
                let apps: Vec<AppItem> = s
                    .store
                    .lock()
                    .ok()
                    .and_then(|st| st.apps_in_use().ok())
                    .unwrap_or_default()
                    .into_iter()
                    .map(|(id, name)| AppItem {
                        id: id as i32,
                        name: SharedString::from(name),
                    })
                    .collect();
                ui.set_apps(ModelRc::new(VecModel::from(apps)));
                ui.set_mode(SharedString::from("search"));
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_set_time_filter(move |idx| {
            if let Ok(mut u) = s.ui.lock() {
                u.time_filter = magpie_app::viewmodel::time_filter_from_index(idx);
            }
            if let Some(ui) = w.upgrade() {
                refresh(&ui, &s);
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_set_app_filter(move |id| {
            if let Ok(mut u) = s.ui.lock() {
                u.app_filter = if id >= 0 { Some(id as i64) } else { None };
            }
            if let Some(ui) = w.upgrade() {
                ui.set_mode(SharedString::from("list"));
                ui.set_selected(0);
                refresh(&ui, &s);
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_clear_filters(move || {
            if let Ok(mut u) = s.ui.lock() {
                u.app_filter = None;
                u.time_filter = magpie_app::viewmodel::TimeFilter::All;
                u.pinned_only = false;
            }
            if let Some(ui) = w.upgrade() {
                ui.set_time_index(0);
                ui.set_pinned_only(false);
                ui.set_mode(SharedString::from("list"));
                refresh(&ui, &s);
            }
        });
    }
    {
        let w = ui.as_weak();
        ui.on_hide_window(move || {
            if let Some(ui) = w.upgrade() {
                let _ = ui.hide();
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_toggle_pin(move |idx| {
            let results = current_results(&s, now_ms());
            if let Some(e) = results.get(idx as usize) {
                if let Ok(store) = s.store.lock() {
                    let _ = store.set_pinned(e.id, !e.pinned);
                }
            }
            if let Some(ui) = w.upgrade() {
                refresh(&ui, &s);
            }
        });
    }
    {
        let w = ui.as_weak();
        ui.on_open_actions(move || {
            if let Some(ui) = w.upgrade() {
                ui.set_actions_query(SharedString::from(""));
                ui.set_action_selected(0);
                set_actions_filtered(&ui, "");
                ui.set_mode(SharedString::from("actions"));
            }
        });
    }
    {
        let w = ui.as_weak();
        ui.on_actions_key_char(move |c| {
            if let Some(ui) = w.upgrade() {
                let q = format!("{}{}", ui.get_actions_query(), c);
                ui.set_actions_query(SharedString::from(q.clone()));
                ui.set_action_selected(0);
                set_actions_filtered(&ui, &q);
            }
        });
    }
    {
        let w = ui.as_weak();
        ui.on_actions_backspace(move || {
            if let Some(ui) = w.upgrade() {
                let mut q = ui.get_actions_query().to_string();
                q.pop();
                ui.set_actions_query(SharedString::from(q.clone()));
                ui.set_action_selected(0);
                set_actions_filtered(&ui, &q);
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_delete_entry(move |idx| {
            let results = current_results(&s, now_ms());
            if let Some(e) = results.get(idx as usize) {
                if let Ok(store) = s.store.lock() {
                    if let Ok(removed) = store.delete_entry(e.id) {
                        s.images.remove_paths(&removed.image_paths);
                    }
                }
            }
            if let Some(ui) = w.upgrade() {
                refresh(&ui, &s);
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_activate_keep(move |idx| {
            paste_and_close(&s, &w, idx as usize, true);
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_toggle_screenshare(move || {
            if let Ok(mut g) = s.screenshare.lock() {
                *g = !*g;
            }
            if let Some(ui) = w.upgrade() {
                refresh(&ui, &s);
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
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_assign_slot(move |index, n| {
            let recent = current_results(&s, now_ms());
            if let Some(entry) = recent.get(index as usize) {
                if let Ok(store) = s.store.lock() {
                    let already_here =
                        store.slot_entry(n as i64).ok().flatten().map(|e| e.id) == Some(entry.id);
                    if already_here {
                        let _ = store.clear_slot(n as i64);
                    } else {
                        let _ = store.assign_slot(n as i64, entry.id);
                    }
                }
            }
            if let Some(ui) = w.upgrade() {
                refresh(&ui, &s);
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_start_edit(move |index| {
            if let Some(ui) = w.upgrade() {
                let results = current_results(&s, now_ms());
                if let Some(e) = results.get(index as usize) {
                    ui.set_edit_text(SharedString::from(e.full_text.clone()));
                    ui.set_edit_mode(SharedString::from("entry"));
                    ui.set_mode(SharedString::from("edit"));
                }
            }
        });
    }
    {
        let w = ui.as_weak();
        ui.on_new_snippet(move || {
            if let Some(ui) = w.upgrade() {
                ui.set_edit_text(SharedString::from(""));
                ui.set_edit_mode(SharedString::from("snippet"));
                ui.set_mode(SharedString::from("edit"));
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_save_edit(move |text| {
            let text = text.to_string();
            if let Some(ui) = w.upgrade() {
                let mode = ui.get_edit_mode().to_string();
                let idx = ui.get_selected() as usize;
                // Compute results BEFORE locking the store (current_results also
                // locks it) to avoid a re-entrant Mutex deadlock.
                let results = current_results(&s, now_ms());
                if !text.trim().is_empty() {
                    if let Ok(store) = s.store.lock() {
                        if mode == "entry" {
                            if let Some(e) = results.get(idx) {
                                let _ = store.update_entry_text(e.id, &text, now_ms());
                            }
                        } else if mode == "snippet" {
                            let _ = store.create_snippet(&text, now_ms());
                        }
                    }
                }
                ui.set_edit_mode(SharedString::from("none"));
                ui.set_mode(SharedString::from("list"));
                refresh(&ui, &s);
            }
        });
    }
    {
        let w = ui.as_weak();
        ui.on_cancel_edit(move || {
            if let Some(ui) = w.upgrade() {
                ui.set_edit_mode(SharedString::from("none"));
                ui.set_mode(SharedString::from("list"));
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_add_tag(move |index, tag| {
            let recent = current_results(&s, now_ms());
            if let Some(e) = recent.get(index as usize) {
                if let Ok(store) = s.store.lock() {
                    let _ = store.add_tag(e.id, &tag);
                }
            }
            if let Some(ui) = w.upgrade() {
                ui.set_add_tag_text(SharedString::from(""));
                refresh(&ui, &s);
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_remove_tag(move |index, tag| {
            let recent = current_results(&s, now_ms());
            if let Some(e) = recent.get(index as usize) {
                if let Ok(store) = s.store.lock() {
                    let _ = store.remove_tag(e.id, &tag);
                }
            }
            if let Some(ui) = w.upgrade() {
                refresh(&ui, &s);
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_set_tag_filter(move |tag| {
            if let Some(ui) = w.upgrade() {
                let current = ui.get_tag_filter().to_string();
                let next = if current == tag.as_str() {
                    SharedString::from("")
                } else {
                    tag
                };
                ui.set_tag_filter(next);
                refresh(&ui, &s);
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_toggle_merge(move |index| {
            if let Ok(mut m) = s.merge_set.lock() {
                if let Some(pos) = m.iter().position(|&x| x == index) {
                    m.remove(pos);
                } else {
                    m.push(index);
                }
            }
            if let Some(ui) = w.upgrade() {
                refresh(&ui, &s);
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_clear_merge(move || {
            if let Ok(mut m) = s.merge_set.lock() {
                m.clear();
            }
            if let Some(ui) = w.upgrade() {
                refresh(&ui, &s);
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_merge_paste(move || {
            let indices: Vec<i32> = s.merge_set.lock().map(|m| m.clone()).unwrap_or_default();
            let results = current_results(&s, now_ms());
            let ids: Vec<i64> = indices
                .iter()
                .filter_map(|&i| results.get(i as usize).map(|e| e.id))
                .collect();
            let sep = w
                .upgrade()
                .map(|ui| separator_str(ui.get_merge_sep()))
                .unwrap_or("\n");
            let merged = s
                .store
                .lock()
                .ok()
                .and_then(|st| st.merged_text(&ids, sep).ok())
                .unwrap_or_default();
            if !merged.is_empty() {
                if let Ok(mut clip) = magpie_platform::platform_clipboard() {
                    if clip.set_content(&Content::Text(merged)).is_ok() {
                        let _ = EnigoPaster.paste();
                    }
                }
            }
            if let Ok(mut m) = s.merge_set.lock() {
                m.clear();
            }
            if let Some(ui) = w.upgrade() {
                refresh(&ui, &s);
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_toggle_view(move || {
            if let Some(ui) = w.upgrade() {
                let to_stats = ui.get_view() != "stats";
                ui.set_view(SharedString::from(if to_stats { "stats" } else { "list" }));
                if to_stats {
                    refresh_stats(&ui, &s, ui.get_range_index());
                }
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_stats_range_changed(move |idx| {
            if let Some(ui) = w.upgrade() {
                refresh_stats(&ui, &s, idx);
            }
        });
    }

    // Retention: sweep once at startup, then after each capture (in the watcher).
    let retention = policy_from_config(&cfg);
    sweep_retention(&state, &retention);

    refresh(&ui, &state);
    spawn_watcher(
        state.clone(),
        denylist,
        retention,
        cfg.fetch_link_favicons,
        weak.clone(),
    );
    // Keep the hotkey manager alive for the whole run.
    let _hotkeys = spawn_hotkeys(&cfg, state.clone(), weak.clone());
    // Keep the tray icon alive for the whole run.
    let _tray = build_tray(weak.clone(), state.clone());

    // The red close button hides the window (Magpie keeps running as a tray daemon);
    // "Quit Magpie" in the tray is the real exit.
    ui.window()
        .on_close_requested(|| slint::CloseRequestResponse::HideWindow);

    // Start hidden (background tray daemon); the launcher hotkey shows the window.
    // We deliberately do NOT call `ui.run()` (which would show the window on
    // launch) — `run_event_loop` keeps us alive with the tray icon only.
    slint::run_event_loop().expect("run event loop");
}

/// Build the tray icon: right-click shows a menu (Show / Quit); left-click opens
/// the window. On Linux, tray click events are not emitted, so "Show Magpie" in
/// the menu is the portable way to open the window.
fn build_tray(
    weak: slint::Weak<LauncherWindow>,
    state: Arc<AppState>,
) -> Option<tray_icon::TrayIcon> {
    use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
    use tray_icon::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

    let menu = Menu::new();
    let show = MenuItem::new("Show Magpie", true, None);
    let quit = MenuItem::new("Quit Magpie", true, None);
    menu.append(&show).ok()?;
    menu.append(&PredefinedMenuItem::separator()).ok()?;
    menu.append(&quit).ok()?;
    let show_id = show.id().clone();
    let quit_id = quit.id().clone();

    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .with_tooltip("Magpie")
        .with_title("🐦")
        .build()
        .ok()?;

    // Menu clicks (right-click menu): Show / Quit.
    {
        let w = weak.clone();
        let s = state.clone();
        std::thread::spawn(move || {
            let rx = MenuEvent::receiver();
            while let Ok(ev) = rx.recv() {
                if ev.id == quit_id {
                    let _ = slint::invoke_from_event_loop(|| {
                        let _ = slint::quit_event_loop();
                    });
                } else if ev.id == show_id {
                    let w = w.clone();
                    let s = s.clone();
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(ui) = w.upgrade() {
                            show_window(&ui, &s);
                        }
                    });
                }
            }
        });
    }

    // Left-click on the icon opens the window (macOS/Windows; Linux no-op).
    {
        let w = weak.clone();
        let s = state.clone();
        std::thread::spawn(move || {
            let rx = TrayIconEvent::receiver();
            while let Ok(ev) = rx.recv() {
                if let TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } = ev
                {
                    let w = w.clone();
                    let s = s.clone();
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(ui) = w.upgrade() {
                            show_window(&ui, &s);
                        }
                    });
                }
            }
        });
    }

    Some(tray)
}

#[cfg(test)]
mod glyph_tests {
    use super::type_glyph;
    use magpie_core::Kind;
    #[test]
    fn every_kind_has_a_glyph() {
        assert_eq!(type_glyph(&Kind::Link), "🔗");
        assert_eq!(type_glyph(&Kind::Color), "🎨");
        assert_eq!(type_glyph(&Kind::Email), "✉️");
        assert_eq!(type_glyph(&Kind::Image), "🖼️");
        assert_eq!(type_glyph(&Kind::File), "📁");
        assert_eq!(type_glyph(&Kind::Text), "📄");
        assert_eq!(type_glyph(&Kind::Rtf), "📄");
        assert_eq!(type_glyph(&Kind::Html), "📄");
    }
}

#[cfg(test)]
mod round1_tests {
    use super::line_badge;

    #[test]
    fn badge_counts_nonempty_lines() {
        assert_eq!(line_badge("a\nb\nc"), "3 lines");
        assert_eq!(line_badge("a\n\nb"), "2 lines"); // blank interior line ignored
    }

    #[test]
    fn badge_empty_for_single_or_no_line() {
        assert_eq!(line_badge("one line"), "");
        assert_eq!(line_badge(""), "");
        assert_eq!(line_badge("   \n  "), ""); // only blank lines
    }
}
