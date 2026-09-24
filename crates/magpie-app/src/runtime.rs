use crate::{
    ActionItem, AppItem, Bar, BoardColumn, BookmarkRow, ClipRow, EntryRow, GraphEdge, GraphNode,
    JournalRow, LauncherWindow, NoteRow, Popover, RefRow, SearchBookmarkRow, SearchNoteRow,
    SearchTaskRow, SlotItem, TaskRow, TimeEntryRow,
};
use magpie_app::app_state::{current_results, ingest_event, AppState};
use magpie_app::color_view;
use magpie_app::config::Config;
use magpie_app::favicon;
use magpie_app::format_time::{abs_date, relative_time};
use magpie_app::graph;
use magpie_app::grouping;
use magpie_app::image_cache::FsImageStore;
use magpie_app::mask_view::{mask_render, should_mask, MaskRules};
use magpie_app::merge_view::separator_str;
use magpie_app::paste_action::{perform_paste, resolve_slot_or_recent, PasteKind};
use magpie_app::retention::policy_from_config;
use magpie_app::stats_view::{range_from_index, to_bars};
use magpie_app::wiki_complete;
use magpie_core::{Content, Entry, Kind, RetentionPolicy, Stats, StatsRange, Totals};
use magpie_platform::os::hotkeys::Hotkeys;
use magpie_platform::os::paste::EnigoPaster;
use magpie_platform::os::source_app::ActiveWinSource;
use magpie_platform::{
    default_app_denylist, default_ignore_regexes, parse_hotkey, CapturePolicy, Clipboard, Watcher,
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
    if let Ok(d) = std::env::var("MAGPIE_DATA_DIR") {
        if !d.is_empty() {
            return std::path::PathBuf::from(d);
        }
    }
    dirs::data_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("magpie")
}

pub fn build_state(cfg: &Config) -> Arc<AppState> {
    let dir = data_dir();
    std::fs::create_dir_all(&dir).ok();
    let db = dir.join("magpie.sqlite3");
    let key = magpie_platform::db_key(&dir).expect("obtain DB key");
    let store = magpie_core::open_or_migrate_encrypted(&db, &key).expect("open db");
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
        open_to_today: cfg.open_to_today,
        fetch_link_favicons: cfg.fetch_link_favicons,
        fetch_link_previews: cfg.fetch_link_previews,
        log_clock_entries: cfg.log_clock_entries,
        theme_dark: cfg.theme_dark,
        nav_back: std::sync::Mutex::new(Vec::new()),
        nav_fwd: std::sync::Mutex::new(Vec::new()),
    })
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

/// The row's one-line title: the first line that actually has content. Using
/// only `lines().next()` made every entry that starts with a blank line render
/// as a useless "text" row.
fn preview_title(images: &FsImageStore, e: &Entry) -> String {
    match e.full_text.lines().map(str::trim).find(|l| !l.is_empty()) {
        Some(line) => line.chars().take(80).collect(),
        // Image entries carry no text at all, so this is their only title. Say
        // "Image" with its dimensions rather than the bare kind string "image",
        // which made every image in the list look identical.
        None if matches!(e.kind, Kind::Image) => format!("Image · {}", size_line(images, e)),
        None => e.kind.as_str().to_string(),
    }
}

/// Longest edge of the cached display thumbnail. Big enough to fill the preview
/// pane of a 900px window, small enough that the row list stays cheap — and it's
/// written once, then reloaded from disk.
const THUMB_MAX_EDGE: u32 = 640;

/// Load an image entry's own bitmap for Slint, via the cached `.thumb.png`
/// (see `FsImageStore::ensure_thumbnail` for why the original can't be used).
fn load_entry_image(images: &FsImageStore, e: &Entry) -> (slint::Image, bool) {
    let none = (slint::Image::default(), false);
    let Some(bin) = e.image_path.as_deref() else {
        return none;
    };
    let Some(thumb) = images.ensure_thumbnail(bin, &e.content_hash, THUMB_MAX_EDGE) else {
        return none;
    };
    match slint::Image::load_from_path(std::path::Path::new(&thumb)) {
        Ok(img) => (img, true),
        Err(_) => none,
    }
}

/// The `Size` metadata line, per kind. Chars-and-lines is meaningless for an
/// image (it read "0 chars · 0 lines") and barely better for a file list.
fn size_line(images: &FsImageStore, e: &Entry) -> String {
    match e.kind {
        Kind::Image => {
            let dims = e
                .image_path
                .as_deref()
                .and_then(|p| images.dimensions(p))
                .map(|(w, h)| format!("{w} × {h} · "))
                .unwrap_or_default();
            format!("{dims}{}", color_view::human_bytes(e.byte_size))
        }
        Kind::File => plural(e.line_count, "file", "files"),
        _ => format!(
            "{} · {}",
            plural(e.char_count, "char", "chars"),
            plural(e.line_count, "line", "lines")
        ),
    }
}

/// "1 line" / "3 lines" — a naive `{n} lines` printed "1 lines".
fn plural(n: i64, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
}

#[allow(clippy::too_many_arguments)]
fn to_rows(
    entries: &[Entry],
    slots: &HashMap<i64, i64>,
    tags: &HashMap<i64, Vec<String>>,
    merge_set: &[i64],
    app_names: &HashMap<i64, String>,
    app_icons: &HashMap<i64, String>,
    favicon_dir: &std::path::Path,
    images: &FsImageStore,
    now: i64,
    rules: &MaskRules,
    visible: usize,
) -> Vec<EntryRow> {
    entries
        .iter()
        .map(|e| {
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
            let base_title = preview_title(images, e);
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
            let size = size_line(images, e);
            // The entry's own visual: a thumbnail for images, a swatch for
            // colours. Masked entries get neither — a screenshare must not leak
            // the picture just because the text is dotted out.
            let (thumb, has_thumb) = if masked {
                (slint::Image::default(), false)
            } else {
                load_entry_image(images, e)
            };
            let swatch_rgb = if masked || !matches!(e.kind, Kind::Color) {
                None
            } else {
                color_view::parse_color(&e.full_text)
            };
            EntryRow {
                title: SharedString::from(title),
                subtitle: SharedString::from(subtitle),
                kind: SharedString::from(e.kind.as_str()),
                slot: *slots.get(&e.id).unwrap_or(&0) as i32,
                merged: merge_set.contains(&e.id),
                badge: SharedString::from(line_badge(&e.full_text)),
                source: SharedString::from(source),
                when: SharedString::from(when),
                copied: e.copy_count as i32,
                size: SharedString::from(size),
                masked,
                icon: icon_img,
                has_icon,
                words: e.word_count as i32,
                section: SharedString::from(grouping::section_label(
                    e.pinned,
                    e.last_copied_at_ms,
                    now,
                )),
                copied_date: SharedString::from(abs_date(e.last_copied_at_ms)),
                pinned: e.pinned,
                thumb,
                has_thumb,
                swatch: match swatch_rgb {
                    Some((r, g, b)) => slint::Color::from_rgb_u8(r, g, b),
                    None => slint::Color::default(),
                },
                has_swatch: swatch_rgb.is_some(),
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
    // "Is anything narrowing the list?" — drives the empty state's wording and its
    // "Clear filters" escape hatch. The type chip and tag count too: filtering to
    // Image with no images used to say "No clipboard history yet".
    if let Ok(u) = state.ui.lock() {
        let filtered = u.app_filter.is_some()
            || u.pinned_only
            || u.tag.is_some()
            || !matches!(u.type_filter, magpie_app::viewmodel::TypeFilter::All)
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
                    slot_items.push((slot, preview_title(&state.images, &e)));
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
            .iter()
            .map(|(n, title)| SlotItem {
                n: *n as i32,
                title: SharedString::from(title.clone()),
                filled: true,
            })
            .collect::<Vec<_>>(),
    )));
    // The ⌘S picker needs all nine rows, empty ones included — `slots` above is
    // filled-only because it drives the speed-dial strip.
    ui.set_all_slots(ModelRc::new(VecModel::from(
        (1..=9)
            .map(|n| match slot_items.iter().find(|(s, _)| *s == n) {
                Some((_, title)) => SlotItem {
                    n: n as i32,
                    title: SharedString::from(title.clone()),
                    filled: true,
                },
                None => SlotItem {
                    n: n as i32,
                    title: SharedString::from("Empty"),
                    filled: false,
                },
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
        &state.images,
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

/// The view (if any) the given section is currently showing, for history
/// bookkeeping. Mirrors the `(section, view)` pairs `nav_apply` accepts — ""
/// for single-view sections (clipboard, bookmarks, stats).
fn current_view_for(ui: &LauncherWindow, section: &str) -> String {
    match section {
        "tasks" => ui.get_tasks_view().to_string(),
        "notes" => ui.get_notes_view().to_string(),
        _ => String::new(),
    }
}

/// Record-and-navigate: the entry point every keyboard/mouse nav action
/// should call (the sidebar, the per-section view-switchers, `on_set_mode_*`,
/// "jump to note" links, the dev UI tour, the `g`-leader, ⌘⌥↑/↓ view-cycling).
/// Pushes the CURRENT screen onto `nav_back` (so `⌘[` can return to it) and
/// clears `nav_fwd` (a fresh navigation invalidates any undone `⌘[`) — unless
/// we're already on that exact screen — then switches via `nav_apply`.
/// `⌘[`/`⌘]` (back/forward) call `nav_apply` directly so they don't record
/// themselves into their own history.
fn nav_to(ui: &LauncherWindow, state: &Arc<AppState>, section: &str, view: &str) {
    let cur_section = ui.get_section().to_string();
    let cur_view = current_view_for(ui, &cur_section);
    if cur_section != section || cur_view != view {
        state
            .nav_back
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push((cur_section, cur_view));
        state
            .nav_fwd
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
    }
    nav_apply(ui, state, section, view);
}

/// Jump to `section`, remembering its last-shown view (same lookup
/// `on_set_section` used) — shared by the sidebar click and the `g`-leader.
fn nav_section(ui: &LauncherWindow, state: &Arc<AppState>, section: &str) {
    let view = current_view_for(ui, section);
    nav_to(ui, state, section, &view);
}

/// Pop `nav_back`, push the current screen onto `nav_fwd`, and switch to it.
/// Also used for the `⌘⌥←` "jump to previous screen" toggle — since the
/// screen we're leaving lands on `nav_fwd`, pressing it again returns.
fn on_nav_back(ui: &LauncherWindow, state: &Arc<AppState>) {
    let popped = state
        .nav_back
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .pop();
    if let Some((section, view)) = popped {
        let cur_section = ui.get_section().to_string();
        let cur_view = current_view_for(ui, &cur_section);
        state
            .nav_fwd
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push((cur_section, cur_view));
        nav_apply(ui, state, &section, &view);
    }
}

/// Pop `nav_fwd`, push the current screen onto `nav_back`, and switch to it.
fn on_nav_forward(ui: &LauncherWindow, state: &Arc<AppState>) {
    let popped = state
        .nav_fwd
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .pop();
    if let Some((section, view)) = popped {
        let cur_section = ui.get_section().to_string();
        let cur_view = current_view_for(ui, &cur_section);
        state
            .nav_back
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push((cur_section, cur_view));
        nav_apply(ui, state, &section, &view);
    }
}

/// Cycle the current section's views by `dir` (+1/-1), wrapping. No-op for
/// single-view sections (clipboard, bookmarks, stats).
/// Step through `views` from `cur` by `dir` (+1/-1), wrapping around. An unknown
/// `cur` is treated as index 0. Pure so the wrap-around math is unit-testable.
fn cycle_view<'a>(views: &[&'a str], cur: &str, dir: i32) -> &'a str {
    let n = views.len() as i32;
    let i = views.iter().position(|v| *v == cur).unwrap_or(0) as i32;
    views[(i + dir).rem_euclid(n) as usize]
}

fn on_nav_cycle_view(ui: &LauncherWindow, state: &Arc<AppState>, dir: i32) {
    let section = ui.get_section().to_string();
    let (views, cur): (&[&str], String) = match section.as_str() {
        "tasks" => (&["today", "list", "board"], ui.get_tasks_view().to_string()),
        "notes" => (
            &["notes", "journal", "graph"],
            ui.get_notes_view().to_string(),
        ),
        _ => return,
    };
    let next = cycle_view(views, &cur, dir);
    nav_to(ui, state, &section, next);
}

/// Central navigation: set the section + view, drive the (existing) mode
/// booleans so exactly one section's overlay body shows, and refresh its data.
/// `view` is "" (or any value not recognized for that section) for
/// single-view sections (clipboard, bookmarks, stats).
///
/// This is the ONLY place that clears the mode booleans; `nav_to` (history-
/// recording) and back/forward/cycle (history-preserving) are the only
/// callers. `tasks-view`/`notes-view` are left as-is by Slint's `in-out`
/// bindings whenever we (re)enter that same section, which is what gives
/// "remember the last view shown" for free.
fn nav_apply(ui: &LauncherWindow, state: &Arc<AppState>, section: &str, view: &str) {
    ui.set_today_mode(false);
    ui.set_journal_mode(false);
    ui.set_notes_mode(false);
    ui.set_tasks_mode(false);
    ui.set_bookmarks_mode(false);
    ui.set_board_mode(false);
    ui.set_graph_mode(false);
    ui.set_search_mode(false);
    ui.set_view(SharedString::from("list"));
    ui.set_section(SharedString::from(section));
    match (section, view) {
        ("tasks", "today") => {
            ui.set_tasks_view(SharedString::from("today"));
            ui.set_today_mode(true);
            refresh_today(ui, state);
        }
        ("tasks", "board") => {
            ui.set_tasks_view(SharedString::from("board"));
            ui.set_board_mode(true);
            refresh_board(ui, state);
        }
        ("tasks", _) => {
            // "list", or any unrecognized view — the flat Tasks list.
            ui.set_tasks_view(SharedString::from("list"));
            ui.set_tasks_mode(true);
            refresh_tasks(ui, state);
        }
        ("notes", "journal") => {
            ui.set_notes_view(SharedString::from("journal"));
            ui.set_journal_mode(true);
            refresh_journal(ui, state);
        }
        ("notes", "graph") => {
            ui.set_notes_view(SharedString::from("graph"));
            ui.set_graph_mode(true);
            refresh_graph(ui, state);
        }
        ("notes", _) => {
            // "notes", or any unrecognized view — the notes list + editor.
            ui.set_notes_view(SharedString::from("notes"));
            ui.set_notes_mode(true);
            refresh_notes(ui, state);
        }
        ("bookmarks", _) => {
            ui.set_bookmarks_mode(true);
            refresh_bookmarks(ui, state);
            spawn_bookmark_favicons(ui, state);
        }
        ("stats", _) => {
            ui.set_view(SharedString::from("stats"));
            refresh_stats(ui, state, ui.get_range_index());
        }
        _ => {
            // clipboard: everything above is already cleared.
        }
    }
}

/// Refresh results and show the launcher window. Shared by the launcher hotkey
/// and the tray (left-click + "Show Magpie").
fn show_window(ui: &LauncherWindow, state: &Arc<AppState>) {
    // Deliberately NOT `ui.set_theme_dark(state.theme_dark)` here: `ui` is a
    // single long-lived window (hidden via `NSApp.hide`, never recreated — see
    // `hide_launcher`), so its `theme-dark` property already holds whatever the
    // user last toggled it to. Re-pushing the config-load-time snapshot on every
    // summon would silently revert an in-session toggle back to the stale value.
    // The snapshot is applied exactly once, right after window creation, in `start()`.
    refresh(ui, state);
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
    // Optional "open to Today" setting: land on the Today dashboard each summon.
    if state.open_to_today {
        nav_to(ui, state, "tasks", "today");
    }
    let _ = ui.show();
    // Background/agent apps don't steal focus just by showing a window — activate
    // the process and raise + key the window so the search box is ready to type.
    magpie_platform::raise_to_front();
    // Focus the search field and highlight the top item, ready to type.
    ui.invoke_summon();
}

/// Rebuild the notes list and push it (plus the currently-open note's body,
/// provenance, and links) into the window. When no note is open (`note-id < 0`)
/// or the open note vanished, it lands on today's daily note.
fn refresh_notes(ui: &LauncherWindow, state: &AppState) {
    let now = now_ms();
    let day = abs_date(now); // "YYYY-MM-DD"
    let (rows, all_tags, open_id, body, prov, links, refs) = {
        let store = match state.store.lock() {
            Ok(s) => s,
            Err(e) => e.into_inner(),
        };
        // Ensure today's daily note exists.
        let daily = store.daily_note(&day, now).ok();
        let recent = store.recent_notes(200).unwrap_or_default();
        // Per-note tags (computed once, reused for the filter and the row display).
        let tags_by_note: Vec<Vec<String>> = recent
            .iter()
            .map(|n| magpie_app::notes_view::note_tags(&n.body))
            .collect();
        // Global distinct tag set across ALL notes, sorted for stable chip order.
        let mut all_tags: Vec<String> = tags_by_note.iter().flatten().cloned().collect();
        all_tags.sort();
        all_tags.dedup();
        let filter = ui.get_note_tag_filter().to_string();
        let rows: Vec<NoteRow> = recent
            .iter()
            .zip(tags_by_note.iter())
            .filter(|(_, tags)| filter.is_empty() || tags.iter().any(|t| t == &filter))
            .map(|(n, tags)| NoteRow {
                id: n.id as i32,
                title: SharedString::from(magpie_app::notes_view::note_list_title(
                    &n.name, &n.body,
                )),
                when: SharedString::from(relative_time(n.updated_at_ms, now)),
                is_daily: n.is_daily,
                tags: SharedString::from(
                    tags.iter()
                        .map(|t| format!("#{t}"))
                        .collect::<Vec<_>>()
                        .join(" "),
                ),
            })
            .collect();
        // Which note is open? Keep the current one if still present, else the most
        // recently edited note (falling back to today's daily note). This is based
        // on the FULL `recent` set (not the tag-filtered rows) so applying a tag
        // filter never closes/replaces the note currently being edited.
        let cur = ui.get_note_id();
        let open = if cur >= 0 && recent.iter().any(|n| n.id as i32 == cur) {
            cur
        } else {
            recent
                .first()
                .map(|n| n.id as i32)
                .or_else(|| daily.as_ref().map(|d| d.id as i32))
                .unwrap_or(-1)
        };
        let opened = recent.iter().find(|n| n.id as i32 == open);
        let (body, prov, links) = match opened {
            Some(n) => (
                n.body.clone(),
                note_provenance(n),
                magpie_app::notes_view::wiki_links(&n.body),
            ),
            None => (String::new(), String::new(), Vec::new()),
        };
        let open_name = opened.map(|n| n.name.clone()).unwrap_or_default();
        let all = store.all_notes().unwrap_or_default();
        let refs = magpie_app::backlinks::find_references(&open_name, &all);
        (rows, all_tags, open, body, prov, links, refs)
    };
    ui.set_notes(ModelRc::new(VecModel::from(rows)));
    ui.set_note_all_tags(ModelRc::new(VecModel::from(
        all_tags
            .into_iter()
            .map(SharedString::from)
            .collect::<Vec<_>>(),
    )));
    ui.set_note_id(open_id);
    ui.set_note_body(SharedString::from(body));
    ui.set_note_provenance(SharedString::from(prov));
    // A stale `[[…` popup from the previously-open note shouldn't linger.
    ui.set_note_suggestions(ModelRc::new(VecModel::from(Vec::<SharedString>::new())));
    ui.set_note_links(ModelRc::new(VecModel::from(
        links
            .into_iter()
            .map(SharedString::from)
            .collect::<Vec<_>>(),
    )));
    let to_ref_row = |r: magpie_app::backlinks::Reference| RefRow {
        note_id: r.note_id as i32,
        line_index: r.line_index as i32,
        source: SharedString::from(r.note_name),
        line: SharedString::from(r.line),
    };
    ui.set_backlinks(ModelRc::new(VecModel::from(
        refs.linked.into_iter().map(to_ref_row).collect::<Vec<_>>(),
    )));
    ui.set_unlinked(ModelRc::new(VecModel::from(
        refs.unlinked
            .into_iter()
            .map(to_ref_row)
            .collect::<Vec<_>>(),
    )));
}

/// Rebuild the bookmarks list from the store, filtered by the current search query.
fn refresh_bookmarks(ui: &LauncherWindow, state: &AppState) {
    let q = ui.get_bookmark_query();
    let favicon_dir = data_dir().join("favicons");
    let preview_dir = data_dir().join("previews");
    let rows: Vec<BookmarkRow> = {
        let store = match state.store.lock() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };
        store
            .list_bookmarks(q.as_str(), 500)
            .unwrap_or_default()
            .into_iter()
            .map(|b| {
                let (icon, has_icon) = {
                    let p = favicon::favicon_cache_path(&favicon_dir, &b.domain);
                    match p.exists().then(|| slint::Image::load_from_path(&p)) {
                        Some(Ok(img)) => (img, true),
                        _ => (slint::Image::default(), false),
                    }
                };
                // Only LOAD an already-cached preview here — never fetch (privacy:
                // previews are fetched exclusively on explicit bookmark save).
                let (thumb, has_thumb) = {
                    let p = magpie_app::link_meta::preview_cache_path(&preview_dir, &b.url);
                    match p.exists().then(|| slint::Image::load_from_path(&p)) {
                        Some(Ok(img)) => (img, true),
                        _ => (slint::Image::default(), false),
                    }
                };
                let tags = SharedString::from(
                    b.tags
                        .iter()
                        .map(|t| format!("#{t}"))
                        .collect::<Vec<_>>()
                        .join(" "),
                );
                BookmarkRow {
                    id: b.id as i32,
                    title: SharedString::from(b.title),
                    domain: SharedString::from(b.domain),
                    url: SharedString::from(b.url),
                    icon,
                    has_icon,
                    thumb,
                    has_thumb,
                    tags,
                }
            })
            .collect()
    };
    ui.set_bookmarks(ModelRc::new(VecModel::from(rows)));
}

/// Best-effort background preview-thumbnail fetch for one bookmark URL, gated by
/// `state.fetch_link_previews` (privacy — called ONLY from the two explicit-save
/// paths: `on_add_bookmark` and `on_bookmark_selected`, never from
/// `refresh_bookmarks` or for merely-copied URLs). Refreshes the bookmarks list
/// once done so the thumbnail appears.
fn enrich_bookmark_preview(state: &Arc<AppState>, weak: slint::Weak<LauncherWindow>, url: String) {
    if !state.fetch_link_previews {
        return;
    }
    let state = state.clone();
    std::thread::spawn(move || {
        let dir = data_dir().join("previews");
        magpie_app::link_meta::ensure_preview(
            &dir,
            &url,
            magpie_app::link_meta::fetch_preview_image,
        );
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = weak.upgrade() {
                refresh_bookmarks(&ui, &state);
            }
        });
    });
}

/// Best-effort background favicon fetch for the domains currently in the
/// bookmarks list, gated by `state.fetch_link_favicons` (privacy). Fetches at
/// most once per domain (cache-checked in `favicon::ensure_favicon`), then
/// refreshes the bookmarks list once at the end so newly-fetched icons appear.
fn spawn_bookmark_favicons(ui: &LauncherWindow, state: &Arc<AppState>) {
    if !state.fetch_link_favicons {
        return;
    }
    let domains: Vec<String> = {
        let store = match state.store.lock() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };
        let mut seen = std::collections::HashSet::new();
        store
            .list_bookmarks("", 500)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|b| {
                if b.domain.is_empty() || !seen.insert(b.domain.clone()) {
                    None
                } else {
                    Some(b.domain)
                }
            })
            .collect()
    };
    if domains.is_empty() {
        return;
    }
    let weak = ui.as_weak();
    let state = state.clone();
    std::thread::spawn(move || {
        let dir = data_dir().join("favicons");
        let mut any_fetched = false;
        for domain in &domains {
            let path = favicon::favicon_cache_path(&dir, domain);
            let already_cached = path.exists();
            if favicon::ensure_favicon(&dir, domain, favicon::fetch_favicon).is_some()
                && !already_cached
            {
                any_fetched = true;
            }
        }
        if any_fetched {
            let w = weak.clone();
            let s = state.clone();
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = w.upgrade() {
                    refresh_bookmarks(&ui, &s);
                }
            });
        }
    });
}

/// Rebuild the "Find everywhere" palette's three result lists from the store
/// for the given query. Empty/whitespace query yields empty lists (mirrors
/// `unified_search`'s own behavior, so opening the palette with "" just clears it).
fn refresh_unified_search(ui: &LauncherWindow, state: &AppState, query: &str) {
    let now = now_ms();
    let results = {
        let store = match state.store.lock() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };
        magpie_app::unified_search::unified_search(&store, query, now, 12)
    };
    ui.set_search_notes(ModelRc::new(VecModel::from(
        results
            .notes
            .into_iter()
            .map(|(id, title)| SearchNoteRow {
                id: id as i32,
                title: SharedString::from(title),
            })
            .collect::<Vec<_>>(),
    )));
    ui.set_search_tasks(ModelRc::new(VecModel::from(
        results
            .tasks
            .into_iter()
            .map(|(id, title)| SearchTaskRow {
                id: id as i32,
                title: SharedString::from(title),
            })
            .collect::<Vec<_>>(),
    )));
    ui.set_search_bookmarks(ModelRc::new(VecModel::from(
        results
            .bookmarks
            .into_iter()
            .map(|(url, title)| SearchBookmarkRow {
                url: SharedString::from(url),
                title: SharedString::from(title),
            })
            .collect::<Vec<_>>(),
    )));
}

/// Chronological timeline over the daily notes (newest first), each rendered
/// as an editable card in Journal mode.
fn refresh_journal(ui: &LauncherWindow, state: &AppState) {
    let now = now_ms();
    let rows: Vec<JournalRow> = {
        let store = match state.store.lock() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };
        let _ = store.daily_note(&abs_date(now), now); // ensure today's entry exists
        store
            .daily_notes(200)
            .unwrap_or_default()
            .into_iter()
            .map(|n| JournalRow {
                note_id: n.id as i32,
                date: SharedString::from(n.name),
                body: SharedString::from(n.body),
            })
            .collect()
    };
    ui.set_journal(ModelRc::new(VecModel::from(rows)));
}

/// Provenance line. Phase 1 keeps it simple: whether the note was captured from a
/// clip or authored in Magpie, plus the creation date. (Enriching "captured" with
/// the exact source app name — via an app-id→name lookup — is a later refinement.)
fn note_provenance(n: &magpie_core::Note) -> String {
    let when = abs_date(n.created_at_ms);
    if n.source_entry_id.is_some() {
        format!("captured · {when}")
    } else {
        format!("created in Magpie · {when}")
    }
}

/// Compact recurrence badge, e.g. "↻1w" (empty when the task doesn't recur).
fn recur_badge(recur: Option<magpie_app::tasks::Recur>) -> String {
    match recur {
        Some(r) => {
            let u = match r.unit {
                magpie_app::tasks::RecurUnit::Day => "d",
                magpie_app::tasks::RecurUnit::Week => "w",
                magpie_app::tasks::RecurUnit::Month => "mo",
                magpie_app::tasks::RecurUnit::Year => "y",
            };
            format!("↻{}{}", r.n, u)
        }
        None => String::new(),
    }
}

/// Map a task to a `TaskRow`, including time-tracking fields (whether its timer
/// is the currently-running one, plus its accumulated duration). Shared by
/// `refresh_tasks` and `refresh_today` — the two views that surface tracking;
/// `refresh_popover` uses a lighter mapping since the popover doesn't show it.
fn task_row_with_tracking(
    t: magpie_app::tasks::Task,
    today_ms: i64,
    active: &Option<magpie_core::notes::ActiveTimer>,
    store: &magpie_core::Store,
    now: i64,
) -> TaskRow {
    let key = format!("{}|{}", t.note_id, t.title);
    let tracking = active.as_ref().map(|a| a.task_key == key).unwrap_or(false);
    let total_ms = store.total_ms_for(&key, now).unwrap_or(0);
    TaskRow {
        note_id: t.note_id as i32,
        line_index: t.line_index as i32,
        title: SharedString::from(t.title),
        done: t.done,
        doing: t.status == magpie_app::tasks::Status::Doing,
        priority: match t.priority {
            magpie_app::tasks::Priority::High => 0,
            magpie_app::tasks::Priority::Medium => 1,
            magpie_app::tasks::Priority::Low => 2,
            magpie_app::tasks::Priority::None => 3,
        },
        due: SharedString::from(t.due_ms.map(abs_date).unwrap_or_default()),
        project: SharedString::from(t.project.unwrap_or_default()),
        source: SharedString::from(t.note_name),
        recur: SharedString::from(recur_badge(t.recur)),
        overdue: t.due_ms.map(|d| d < today_ms).unwrap_or(false) && !t.done,
        bookmarked: t.bookmarked,
        tracking,
        time_total: SharedString::from(magpie_app::format_time::fmt_duration(total_ms)),
    }
}

/// Rebuild the Tasks-mode list: every task across all notes, grouped/sorted, then
/// flattened into a flat model. Done tasks are skipped unless "Show done" is on;
/// `due_ms` is formatted as an absolute date and `Priority` mapped to 0..3
/// (High=0, Medium=1, Low=2, None=3). `source` is the owning note's name.
fn refresh_tasks(ui: &LauncherWindow, state: &AppState) {
    let now = now_ms();
    let today_ms = magpie_app::format_time::parse_due("today", now).unwrap_or(0);
    let show_done = ui.get_show_done();
    let filter = ui.get_task_filter();
    // The running timer (set inside the lock, reused for the header below).
    let active;
    let rows: Vec<TaskRow> = {
        let store = match state.store.lock() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };
        active = store.active_timer().ok().flatten();
        let tasks = magpie_app::tasks::all_tasks(&store, now);
        magpie_app::tasks::group_sort(tasks)
            .into_iter()
            .flat_map(|g| g.tasks)
            .filter(|t| show_done || !t.done)
            .filter(|t| match filter.as_str() {
                "today" => t.due_ms.map(|d| d <= today_ms).unwrap_or(false),
                "recurring" => t.recur.is_some(),
                "star" => t.bookmarked,
                _ => true,
            })
            .map(|t| task_row_with_tracking(t, today_ms, &active, &store, now))
            .collect()
    };
    // Keep the keyboard selection in range after the list changes (e.g. a task
    // was completed and filtered out).
    let len = rows.len() as i32;
    if ui.get_task_selected() >= len {
        ui.set_task_selected((len - 1).max(0));
    }
    ui.set_tasks(ModelRc::new(VecModel::from(rows)));
    // Running-timer header indicator (live-ticked in Slint from these seeds).
    match &active {
        Some(a) => {
            ui.set_active_title(SharedString::from(a.task_title.clone()));
            ui.set_active_elapsed_sec(((now - a.start_ms).max(0) / 1000) as i32);
        }
        None => {
            ui.set_active_title(SharedString::from(""));
            ui.set_active_elapsed_sec(0);
        }
    }
}

/// Rebuild the Board-mode task board: read the group/layout/sort/filter
/// controls off the UI, run every task through the pure `tasks::board_columns`
/// pipeline, and map each column's tasks to `TaskRow`s with the lighter
/// (no time-tracking) mapping `refresh_popover` uses — cards don't show live
/// tracking. Also recomputes the distinct project list for the filter chips.
/// One store lock for both; models are set after it's dropped.
fn refresh_board(ui: &LauncherWindow, state: &AppState) {
    let now = now_ms();
    let today_ms = magpie_app::format_time::parse_due("today", now).unwrap_or(0);
    let group_by = match ui.get_board_group() {
        1 => magpie_app::tasks::GroupBy::Priority,
        2 => magpie_app::tasks::GroupBy::Project,
        _ => magpie_app::tasks::GroupBy::Status,
    };
    let sort_by = match ui.get_board_sort() {
        1 => magpie_app::tasks::SortBy::Priority,
        2 => magpie_app::tasks::SortBy::Title,
        3 => magpie_app::tasks::SortBy::Status,
        _ => magpie_app::tasks::SortBy::Due,
    };
    let project_raw = ui.get_board_project_filter().to_string();
    let filter = magpie_app::tasks::BoardFilter {
        project: if project_raw.is_empty() || project_raw == "All" {
            None
        } else {
            Some(project_raw)
        },
        query: ui.get_board_query().to_string(),
        hide_done: ui.get_board_hide_done(),
    };
    let to_row = |t: magpie_app::tasks::Task| TaskRow {
        note_id: t.note_id as i32,
        line_index: t.line_index as i32,
        title: SharedString::from(t.title),
        done: t.done,
        doing: t.status == magpie_app::tasks::Status::Doing,
        priority: match t.priority {
            magpie_app::tasks::Priority::High => 0,
            magpie_app::tasks::Priority::Medium => 1,
            magpie_app::tasks::Priority::Low => 2,
            magpie_app::tasks::Priority::None => 3,
        },
        due: SharedString::from(t.due_ms.map(abs_date).unwrap_or_default()),
        project: SharedString::from(t.project.unwrap_or_default()),
        source: SharedString::from(t.note_name),
        recur: SharedString::from(recur_badge(t.recur)),
        overdue: t.due_ms.map(|d| d < today_ms).unwrap_or(false) && !t.done,
        bookmarked: t.bookmarked,
        // Time tracking isn't surfaced on board cards.
        tracking: false,
        time_total: SharedString::from(""),
    };
    let (columns, projects): (Vec<BoardColumn>, Vec<SharedString>) = {
        let store = match state.store.lock() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };
        let all = magpie_app::tasks::all_tasks(&store, now);
        // Distinct projects across ALL tasks (unfiltered), "No project" last.
        let mut projects: Vec<String> = all
            .iter()
            .map(|t| {
                t.project
                    .clone()
                    .unwrap_or_else(|| "No project".to_string())
            })
            .collect();
        projects.sort();
        projects.dedup();
        let had_no_project = projects.iter().any(|p| p == "No project");
        projects.retain(|p| p != "No project");
        if had_no_project {
            projects.push("No project".to_string());
        }
        let columns = magpie_app::tasks::board_columns(all, group_by, sort_by, &filter)
            .into_iter()
            .map(|c| BoardColumn {
                title: SharedString::from(c.title),
                count: c.tasks.len() as i32,
                tasks: ModelRc::new(VecModel::from(
                    c.tasks.into_iter().map(to_row).collect::<Vec<_>>(),
                )),
            })
            .collect::<Vec<_>>();
        (
            columns,
            projects.into_iter().map(SharedString::from).collect(),
        )
    };
    ui.set_board(ModelRc::new(VecModel::from(columns)));
    ui.set_board_projects(ModelRc::new(VecModel::from(projects)));
}

/// Rebuild Graph mode: every note, laid out by `magpie_app::graph::build_graph`
/// into force-directed positions from their `[[wiki-links]]`, mapped to Slint's
/// `GraphNode`/`GraphEdge` structs (edges carry resolved endpoint coordinates —
/// no index lookups on the Slint side).
fn refresh_graph(ui: &LauncherWindow, state: &AppState) {
    let triples: Vec<(i64, String, String)> = {
        let store = match state.store.lock() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };
        store
            .all_notes()
            .unwrap_or_default()
            .into_iter()
            .map(|n| (n.id, n.name, n.body))
            .collect()
    };
    let (nodes, edges) = graph::build_graph(&triples, 500);
    let slint_edges: Vec<GraphEdge> = edges
        .iter()
        .map(|e| {
            let a = &nodes[e.a];
            let b = &nodes[e.b];
            GraphEdge {
                x1: a.x,
                y1: a.y,
                x2: b.x,
                y2: b.y,
            }
        })
        .collect();
    let total = nodes.len() as i32;
    let slint_nodes: Vec<GraphNode> = nodes
        .into_iter()
        .map(|n| GraphNode {
            id: n.id as i32,
            title: SharedString::from(n.title),
            x: n.x,
            y: n.y,
            degree: n.degree as i32,
        })
        .collect();
    ui.set_graph_nodes(ModelRc::new(VecModel::from(slint_nodes)));
    ui.set_graph_edges(ModelRc::new(VecModel::from(slint_edges)));
    ui.set_graph_total(total);
}

/// Rebuild the popover's Tasks tab: every **open** (`!done`) task across all
/// notes, grouped/sorted, mapped to `TaskRow`s exactly as `refresh_tasks` does.
/// Poison-tolerant lock so a panic elsewhere can't take the popover down.
fn refresh_popover(popover: &Popover, state: &AppState) {
    let now = now_ms();
    {
        let store = match state.store.lock() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };
        let today_ms = magpie_app::format_time::parse_due("today", now).unwrap_or(0);
        let to_row = |t: magpie_app::tasks::Task| TaskRow {
            note_id: t.note_id as i32,
            line_index: t.line_index as i32,
            title: SharedString::from(t.title),
            done: t.done,
            doing: t.status == magpie_app::tasks::Status::Doing,
            priority: match t.priority {
                magpie_app::tasks::Priority::High => 0,
                magpie_app::tasks::Priority::Medium => 1,
                magpie_app::tasks::Priority::Low => 2,
                magpie_app::tasks::Priority::None => 3,
            },
            due: SharedString::from(t.due_ms.map(abs_date).unwrap_or_default()),
            project: SharedString::from(t.project.unwrap_or_default()),
            source: SharedString::from(t.note_name),
            recur: SharedString::from(recur_badge(t.recur)),
            overdue: t.due_ms.map(|d| d < today_ms).unwrap_or(false) && !t.done,
            bookmarked: t.bookmarked,
            // Time tracking is surfaced in the full Tasks view, not the popover.
            tracking: false,
            time_total: SharedString::from(""),
        };

        let all = magpie_app::tasks::all_tasks(&store, now);

        // Tasks tab: open tasks in group_sort order (unchanged Slice A behavior).
        let open_rows: Vec<TaskRow> = magpie_app::tasks::group_sort(all.clone())
            .into_iter()
            .flat_map(|g| g.tasks)
            .filter(|t| !t.done)
            .map(to_row)
            .collect();
        popover.set_ptasks(ModelRc::new(VecModel::from(open_rows)));

        // Today tab: overdue + due-today.
        let buckets = magpie_app::tasks::partition_due(all, today_ms);
        let overdue: Vec<TaskRow> = buckets.overdue.into_iter().map(to_row).collect();
        let today: Vec<TaskRow> = buckets.today.into_iter().map(to_row).collect();
        popover.set_overdue_tasks(ModelRc::new(VecModel::from(overdue)));
        popover.set_today_tasks(ModelRc::new(VecModel::from(today)));

        // Daily tab: today's daily-note body.
        if let Ok(n) = store.daily_note(&abs_date(now), now) {
            popover.set_daily_body(SharedString::from(n.body));
        }
    }

    // Clipboard tab: recent clips (uses the same list paste_and_close indexes into,
    // so the popover clip index aligns with the paste target). Outside the store lock.
    let clips: Vec<ClipRow> = current_results(state, now)
        .iter()
        .take(12)
        .map(|e| ClipRow {
            title: SharedString::from(
                e.full_text
                    .lines()
                    .map(str::trim)
                    .find(|l| !l.is_empty())
                    .map(|l| l.chars().take(80).collect::<String>())
                    .unwrap_or_else(|| e.kind.as_str().to_string()),
            ),
            kind: SharedString::from(e.kind.as_str()),
        })
        .collect();
    popover.set_clips(ModelRc::new(VecModel::from(clips)));
}

/// Rebuild the launcher's Today dashboard: the popover's Today + Daily +
/// Clipboard tabs merged into one full-window overlay. Overdue + due-today
/// tasks (via `partition_due`, mapped with tracking like `refresh_tasks`), the
/// running-timer header seed, today's daily-note body, and the most recent
/// clips (mirrors `refresh_popover`'s clip mapping).
fn refresh_today(ui: &LauncherWindow, state: &AppState) {
    let now = now_ms();
    let today_ms = magpie_app::format_time::parse_due("today", now).unwrap_or(0);
    let active;
    {
        let store = match state.store.lock() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };
        active = store.active_timer().ok().flatten();
        let all = magpie_app::tasks::all_tasks(&store, now);
        let buckets = magpie_app::tasks::partition_due(all, today_ms);
        let due: Vec<TaskRow> = buckets
            .overdue
            .into_iter()
            .chain(buckets.today)
            .map(|t| task_row_with_tracking(t, today_ms, &active, &store, now))
            .collect();
        ui.set_today_due(ModelRc::new(VecModel::from(due)));

        if let Ok(n) = store.daily_note(&abs_date(now), now) {
            ui.set_today_journal(SharedString::from(n.body));
        }
    }

    match &active {
        Some(a) => {
            ui.set_active_title(SharedString::from(a.task_title.clone()));
            ui.set_active_elapsed_sec(((now - a.start_ms).max(0) / 1000) as i32);
        }
        None => {
            ui.set_active_title(SharedString::from(""));
            ui.set_active_elapsed_sec(0);
        }
    }

    // Clipboard: recent clips, outside the store lock (same source list
    // `paste_and_close` indexes into, so the clip index aligns with the paste
    // target).
    let clips: Vec<ClipRow> = current_results(state, now)
        .iter()
        .take(8)
        .map(|e| ClipRow {
            title: SharedString::from(
                e.full_text
                    .lines()
                    .map(str::trim)
                    .find(|l| !l.is_empty())
                    .map(|l| l.chars().take(80).collect::<String>())
                    .unwrap_or_else(|| e.kind.as_str().to_string()),
            ),
            kind: SharedString::from(e.kind.as_str()),
        })
        .collect();
    ui.set_today_clips(ModelRc::new(VecModel::from(clips)));
}

/// The ⌘K action set: (id, icon, label, shortcut). Dispatch by id in Slint's
/// `run-action`.
/// Shortcut labels must match the real bindings in `launcher.slint` — a wrong
/// hint is worse than none. The icon field is unused (Slint's `ActionIcon`
/// renders a monochrome SVG by `id` instead) — left empty, not emoji.
const ACTIONS: &[(&str, &str, &str, &str)] = &[
    ("paste", "", "Paste", "⏎"),
    ("copy", "", "Copy", "⌘C"),
    ("keep", "", "Paste & keep open", "⌘⏎"),
    ("edit", "", "Edit", "⌘E"),
    ("snippet", "", "New snippet", "⌘N"),
    ("pin", "", "Pin / Unpin", "⌘P"),
    ("slot", "", "Assign to slot…", "⌘S"),
    ("merge", "", "Add to merge", "⌘G"),
    ("note", "", "New note from this entry", "⌘J"),
    ("delete", "", "Delete", "⌘⌫"),
    ("export", "", "Export data…", ""),
    ("bookmark", "", "Bookmark this link", ""),
    ("task-from-link", "", "Create task from link", ""),
];

/// The ⌘K rows whose label matches `query` (case-insensitive substring; "" =
/// all), in declaration order. Pure, so the match logic is unit-testable.
#[allow(clippy::type_complexity)]
fn filter_actions(
    query: &str,
) -> Vec<&'static (&'static str, &'static str, &'static str, &'static str)> {
    let q = query.to_lowercase();
    ACTIONS
        .iter()
        .filter(|(_, _, label, _)| q.is_empty() || label.to_lowercase().contains(&q))
        .collect()
}

/// Keep a list selection index in range: clamp to the last row when it would
/// point past the end, and to 0 when the list is empty.
fn clamp_selection(sel: i32, len: i32) -> i32 {
    if sel >= len {
        (len - 1).max(0)
    } else {
        sel
    }
}

/// Push the ⌘K action list filtered by `query` (case-insensitive label match)
/// and keep `action-selected` in range.
fn set_actions_filtered(ui: &LauncherWindow, query: &str) {
    let rows = filter_actions(query);
    let len = rows.len() as i32;
    let items: Vec<ActionItem> = rows
        .into_iter()
        .map(|(id, icon, label, key)| ActionItem {
            icon: SharedString::from(*icon),
            label: SharedString::from(*label),
            key: SharedString::from(*key),
            id: SharedString::from(*id),
        })
        .collect();
    ui.set_actions(ModelRc::new(VecModel::from(items)));
    if ui.get_action_selected() >= len {
        ui.set_action_selected(clamp_selection(ui.get_action_selected(), len));
    }
}

/// After hiding, wait for focus to return to the previous app, send the paste
/// keystroke on the MAIN thread, and optionally re-show the window.
///
/// A background thread does the delay, then `invoke_from_event_loop` runs the
/// keystroke on the main/event-loop thread. Two reasons this beats a `Timer`:
/// (1) enigo's macOS TIS calls assert they run on the main thread
/// (`dispatch_assert_queue` → SIGTRAP from a background thread); (2)
/// `invoke_from_event_loop` posts a wakeup, so it fires reliably even while the
/// app is hidden via `NSApp.hide` (a plain `Timer` may not tick while hidden,
/// which left the clipboard set but the ⌘V never sent).
fn spawn_paste(weak: slint::Weak<LauncherWindow>, keep_open: bool) {
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(120));
        let _ = slint::invoke_from_event_loop(move || {
            let _ = magpie_platform::Paster::paste(&EnigoPaster);
            if keep_open {
                // ⌘Enter: after pasting into the app underneath, bring Magpie back
                // to the front, focused, ready to type (same as a fresh summon).
                if let Some(ui) = weak.upgrade() {
                    let _ = ui.show();
                    magpie_platform::raise_to_front();
                    ui.invoke_summon();
                }
            }
        });
    });
}

/// Set once the user has acted on the "Enable auto-paste" modal (either button),
/// so we never re-show it this session — prevents an unsigned-`.app` trap where
/// the grant is on but `AXIsProcessTrusted()` still reports false.
static A11Y_DISMISSED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// True when running as a real macOS `.app` bundle (path
/// `…/Magpie.app/Contents/MacOS/magpie`) rather than a bare dev binary from
/// `cargo run`. We only gate/prompt for Accessibility as a bundle: a bare
/// terminal-launched binary is attributed by TCC to the *responsible process*
/// (the terminal, e.g. Ghostty), so `AXIsProcessTrusted()` is always false for us
/// yet the paste is delivered via the terminal's own grant — prompting there just
/// nags for the terminal forever. Always `true` off macOS (no such gate).
fn running_as_app_bundle() -> bool {
    #[cfg(target_os = "macos")]
    {
        std::env::current_exe()
            .ok()
            .map(|p| p.to_string_lossy().contains("/Contents/MacOS/"))
            .unwrap_or(false)
    }
    #[cfg(not(target_os = "macos"))]
    {
        true
    }
}

/// Hide the launcher **without quitting the event loop**. On macOS Slint's
/// `Window::hide()` terminates `run_event_loop` once the window has been shown
/// (verified), so we hide at the AppKit level via `NSApp.hide`, which also returns
/// focus to the app underneath. Elsewhere fall back to the Slint hide (the WM
/// handles focus).
fn hide_launcher(ui: &LauncherWindow) {
    #[cfg(target_os = "macos")]
    {
        let _ = ui;
        magpie_platform::hide_and_yield_focus();
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = ui.hide();
    }
}

/// Open note `id` into Notes mode, exiting whatever other full-screen mode is
/// currently showing. Shared by "jump to the owning note" from Tasks-mode rows
/// and from the "Find everywhere" palette (⌘⇧F), so both paths behave identically.
fn open_note_into_notes_mode(ui: &LauncherWindow, state: &Arc<AppState>, note_id: i32) {
    ui.set_note_id(note_id);
    nav_to(ui, state, "notes", "notes");
}

/// Open `url` in the default browser and hide the launcher. Shared by the
/// Bookmarks-mode row click and the "Find everywhere" palette's bookmark rows.
fn open_url_and_hide(ui: &LauncherWindow, url: &str) {
    let _ = std::process::Command::new("open").arg(url).spawn();
    hide_launcher(ui);
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
    let Some(entry) = recent.get(idx) else {
        return;
    };
    if let Ok(mut clip) = magpie_platform::platform_clipboard() {
        // Copy only here; the keystroke is sent after the window hides.
        let _ = perform_paste(&mut clip, &EnigoPaster, entry, PasteKind::Formatted, false);
    }
    // Auto-paste synthesizes ⌘V, which needs Accessibility on macOS. Only gate the
    // *installed app* on it: as a bare dev binary the grant belongs to the launching
    // terminal (Ghostty), so prompting for Magpie would nag forever while the paste
    // already works via the terminal — in dev we just proceed best-effort.
    //
    // Show the informational modal AT MOST ONCE per session: once the user has acted
    // on it (either button), `A11Y_DISMISSED` is set and we always proceed
    // best-effort. This matters because for an unsigned .app macOS TCC is
    // unreliable — the toggle can be on while `AXIsProcessTrusted()` still reports
    // false (grants often need an app relaunch), which would otherwise trap the user
    // in a modal that never clears.
    if running_as_app_bundle()
        && !magpie_platform::accessibility_trusted()
        && !A11Y_DISMISSED.load(std::sync::atomic::Ordering::Relaxed)
    {
        if let Some(ui) = weak.upgrade() {
            ui.set_needs_accessibility(true);
        }
        return;
    }
    // Hide (macOS: NSApp.hide → keeps the loop alive AND returns focus to the app
    // underneath, so the paste lands there).
    if let Some(ui) = weak.upgrade() {
        hide_launcher(&ui);
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

/// A one-line chart label. Falls back to the first line WITH content, then to a
/// placeholder — blank labels left anonymous bars in the "Most copied" chart.
fn truncate(s: &str, n: usize) -> String {
    match s.lines().map(str::trim).find(|l| !l.is_empty()) {
        Some(line) => line.chars().take(n).collect(),
        None => "(whitespace)".to_string(),
    }
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
    let now = now_ms();
    let stats: Stats = match state.store.lock() {
        Ok(store) => store.stats(&range, 10).unwrap_or_else(|_| empty_stats()),
        Err(_) => empty_stats(),
    };

    if let Ok(store) = state.store.lock() {
        let week_ago = now - 7 * 86_400_000;
        let month_ago = now - 30 * 86_400_000;

        // This-week total.
        let week_ms = store.time_total_since(week_ago, now).unwrap_or(0);
        ui.set_time_week_total(SharedString::from(magpie_app::format_time::fmt_duration(
            week_ms,
        )));

        // Per-day (last 7 days).
        let day_items: Vec<(String, String, i64)> = store
            .time_by_day(week_ago, now)
            .unwrap_or_default()
            .into_iter()
            .map(|(d, ms)| (d.clone(), magpie_app::format_time::fmt_duration(ms), ms))
            .collect();
        ui.set_time_by_day_bars(to_slint_bars(&day_items));

        // Per-project (last 30 days), mapping task_key -> project via current tasks.
        let proj_of: HashMap<String, String> = magpie_app::tasks::all_tasks(&store, now)
            .into_iter()
            .map(|t| {
                (
                    format!("{}|{}", t.note_id, t.title),
                    t.project.unwrap_or_else(|| "No project".into()),
                )
            })
            .collect();
        let mut proj_totals: HashMap<String, i64> = HashMap::new();
        for (key, _title, ms) in store.time_by_task(month_ago, now).unwrap_or_default() {
            let p = proj_of
                .get(&key)
                .cloned()
                .unwrap_or_else(|| "No project".into());
            *proj_totals.entry(p).or_insert(0) += ms;
        }
        let mut proj_items: Vec<(String, String, i64)> = proj_totals
            .into_iter()
            .map(|(p, ms)| (p, magpie_app::format_time::fmt_duration(ms), ms))
            .collect();
        proj_items.sort_by_key(|b| std::cmp::Reverse(b.2));
        proj_items.truncate(8);
        ui.set_time_by_project_bars(to_slint_bars(&proj_items));

        // Recent sessions (most-recent-first), for the per-row delete list.
        let entries: Vec<TimeEntryRow> = store
            .recent_time_entries(15)
            .unwrap_or_default()
            .into_iter()
            .map(|e| {
                let dur = match e.end_ms {
                    Some(end) => (end - e.start_ms).max(0),
                    None => (now - e.start_ms).max(0),
                };
                TimeEntryRow {
                    id: e.id as i32,
                    title: SharedString::from(e.task_title),
                    when: SharedString::from(relative_time(e.start_ms, now)),
                    duration: SharedString::from(magpie_app::format_time::fmt_duration(dur)),
                    running: e.end_ms.is_none(),
                }
            })
            .collect();
        ui.set_time_entries(ModelRc::new(VecModel::from(entries)));
    }

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

    // Weekly buckets kick in for an all-time range spanning >90 days; infer it
    // from the gap between the first two buckets rather than duplicating the rule.
    let weekly = stats
        .over_time
        .windows(2)
        .next()
        .map(|w| w[1].start_ms - w[0].start_ms > 86_400_000)
        .unwrap_or(false);
    let ot_pairs: Vec<(i64, i64)> = stats
        .over_time
        .iter()
        .map(|b| (b.start_ms, b.count))
        .collect();
    ui.set_over_time_caption(SharedString::from(
        magpie_app::stats_view::over_time_caption(&ot_pairs, weekly),
    ));

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
        let log_path = data_dir().join("logs").join("magpie.log");
        loop {
            // Guard each iteration: a panic in poll/ingest is logged + surfaced by
            // the panic hook, but must NOT silently end clipboard capture — catch it
            // and keep polling so the watcher survives a bad event.
            let step = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                if let Some(ev) = watcher.poll_once(now_ms()) {
                    if ingest_event(&state, &ev).is_ok() {
                        sweep_retention(&state, &retention);
                        push_refresh(&weak, &state);
                        // Best-effort site favicon for link entries; refresh again
                        // once cached so the icon appears without waiting on it.
                        if fetch_favicons {
                            if let Content::Text(t) = &ev.content {
                                if let Some(domain) = favicon::domain_of(t) {
                                    let dir = data_dir().join("favicons");
                                    if favicon::ensure_favicon(
                                        &dir,
                                        &domain,
                                        favicon::fetch_favicon,
                                    )
                                    .is_some()
                                    {
                                        push_refresh(&weak, &state);
                                    }
                                }
                            }
                        }
                    }
                }
            }));
            if step.is_err() {
                magpie_app::diagnostics::log_line(
                    &log_path,
                    "watcher iteration panicked; continuing capture",
                );
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    });
}

/// Opt-in live vault watcher: every ~4s, run a 3-way reconcile between the notes
/// DB and `vault_path` (pulling external `.md` edits into notes, pushing note
/// edits out, and writing a keep-both conflict file when both sides diverged).
/// Refreshes the open window only when the reconcile actually changed something,
/// so an idle vault doesn't cause needless UI churn. Never spawned unless
/// `Config::vault_watch` is on (and `vault_path` is set) — see `start`.
fn spawn_vault_watcher(
    state: Arc<AppState>,
    vault_path: std::path::PathBuf,
    weak: slint::Weak<LauncherWindow>,
) {
    std::thread::spawn(move || {
        let log_path = data_dir().join("logs").join("magpie.log");
        loop {
            let step = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let report = {
                    let store = state.store.lock().unwrap_or_else(|e| e.into_inner());
                    magpie_core::reconcile_vault(&store, &vault_path, now_ms())
                    // guard dropped here, before any refresh
                };
                match report {
                    Ok(r) => {
                        let changed =
                            r.pulled + r.pushed + r.created_notes + r.created_files + r.conflicts
                                > 0;
                        if changed {
                            let w = weak.clone();
                            let s = state.clone();
                            let _ = slint::invoke_from_event_loop(move || {
                                if let Some(ui) = w.upgrade() {
                                    refresh(&ui, &s);
                                }
                            });
                        }
                    }
                    Err(e) => {
                        magpie_app::diagnostics::log_line(
                            &log_path,
                            &format!("vault watcher reconcile failed: {e}"),
                        );
                    }
                }
            }));
            if step.is_err() {
                magpie_app::diagnostics::log_line(
                    &log_path,
                    "vault watcher iteration panicked; continuing",
                );
            }
            std::thread::sleep(Duration::from_secs(4));
        }
    });
}

/// Local UTC offset in seconds, via `date +%z` (no dependency). 0 on failure/non-unix.
fn local_offset_seconds() -> i64 {
    #[cfg(unix)]
    {
        std::process::Command::new("date")
            .arg("+%z")
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .and_then(|s| magpie_app::reminders::parse_offset(s.trim()))
            .unwrap_or(0)
    }
    #[cfg(not(unix))]
    {
        0
    }
}

/// Reminder scheduler: first tick ~4 s after launch, then every 60 s. Fires a
/// native notification for each newly-ripe open task (deduped via task_reminders);
/// the first tick coalesces a >1 backlog into a single summary.
fn spawn_reminders(state: Arc<AppState>) {
    const DEFAULT_MIN: i64 = 540; // 09:00 local for date-only tasks
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(4));
        let mut first = true;
        loop {
            let s = state.clone();
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                reminder_tick(&s, first, DEFAULT_MIN);
            }));
            first = false;
            std::thread::sleep(Duration::from_secs(60));
        }
    });
}

fn reminder_tick(state: &AppState, first: bool, default_min: i64) {
    let now = now_ms();
    let offset = local_offset_seconds();
    let mut to_fire: Vec<(magpie_app::tasks::Task, String)> = Vec::new();
    {
        let store = match state.store.lock() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };
        let tasks = magpie_app::tasks::all_tasks(&store, now);
        for t in magpie_app::reminders::due_before(tasks, now, offset, default_min) {
            let fp = magpie_app::reminders::fingerprint(&t);
            if !store.reminder_fired(&fp).unwrap_or(false) {
                to_fire.push((t, fp));
            }
        }
    }
    if !to_fire.is_empty() {
        if first && to_fire.len() > 1 {
            magpie_app::diagnostics::notify("Magpie", &format!("{} tasks due", to_fire.len()));
        } else {
            for (t, _) in &to_fire {
                let msg = if t.note_name.is_empty() {
                    t.title.clone()
                } else {
                    format!("{} · {}", t.title, t.note_name)
                };
                magpie_app::diagnostics::notify("Task due", &msg);
            }
        }
        let store = match state.store.lock() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };
        for (_, fp) in &to_fire {
            let _ = store.mark_reminder(fp, now);
        }
    }
    reschedule_os_reminders(state, default_min);
}

/// Re-schedule OS-level notifications for all open, future-dated tasks so they fire
/// even if Magpie is quit before then. Bundle-only under the hood; cheap no-op in dev.
fn reschedule_os_reminders(state: &AppState, default_min: i64) {
    let store = state.store.lock().unwrap_or_else(|e| e.into_inner());
    let now = now_ms();
    let offset = local_offset_seconds();
    let tasks = magpie_app::tasks::all_tasks(&store, now);
    drop(store);
    magpie_platform::cancel_scheduled_reminders();
    for (t, inst) in magpie_app::reminders::future_reminders(tasks, now, offset, default_min) {
        let secs = ((inst - now) as f64 / 1000.0).max(1.0);
        // Embed the note id right after the prefix so the notification-click
        // delegate can recover it (`magpie-task-<note_id>-<fingerprint>`); the
        // fingerprint stays as the stable dedup/replace key.
        let ident = format!(
            "magpie-task-{}-{}",
            t.note_id,
            magpie_app::reminders::fingerprint(&t)
        );
        let body = if t.note_name.is_empty() {
            t.title.clone()
        } else {
            format!("{}  ·  {}", t.title, t.note_name)
        };
        magpie_platform::schedule_notification(&ident, "Task due", &body, secs);
    }
}

/// The type-filter chips, in the order `launcher.slint` renders them — the index
/// into this list IS the argument `set-type-filter` takes (see
/// `viewmodel::type_filter_from_index`). Used by the UI tour to select a chip by
/// name; keep in sync with the Slint chip row.
const TYPE_CHIPS: &[&str] = &["all", "text", "link", "email", "color", "image", "file"];

/// The overlays a UI tour walks through, in order. Each is a `mode`/`view` the
/// launcher can be driven into without a real keystroke.
const TOUR_STEPS: &[&str] = &[
    "list", "help", "actions", "filters", "slots", "stats", "edit", "merge", "mask", "empty",
];

/// Dev/QA visibility hooks, both opt-in via the environment and no-ops otherwise:
///
/// * `MAGPIE_SHOW_ON_LAUNCH=1` — summon the launcher right after startup instead
///   of waiting for the global hotkey. Magpie normally boots as a hidden tray
///   daemon, so without this there is nothing on screen to look at (or to
///   `screencapture`) unless you can press the hotkey.
/// * `MAGPIE_UI_TOUR=1` — additionally step through every overlay
///   (help → actions → filters → slots → stats → edit), pausing
///   `MAGPIE_UI_TOUR_MS` (default 3000) on each, so one run yields a screenshot
///   of every screen. Set it to a comma-separated subset instead
///   (`MAGPIE_UI_TOUR=slots,edit`) to go straight to the screens you care about —
///   a short run is far less likely to be cut off by the display locking.
///
/// Both drive the window through `invoke_from_event_loop`, i.e. on the UI thread
/// as a user event — never synchronously from another thread.
fn spawn_dev_ui_hooks(weak: slint::Weak<LauncherWindow>, state: Arc<AppState>) {
    if std::env::var_os("MAGPIE_SHOW_ON_LAUNCH").is_none() {
        return;
    }
    let steps: Vec<String> = match std::env::var("MAGPIE_UI_TOUR") {
        Err(_) => Vec::new(),
        // "1" (or any truthy-looking single value) means "the whole tour".
        Ok(v) if v.trim().is_empty() || v == "1" => TOUR_STEPS
            .iter()
            .skip(1)
            .map(|s| (*s).to_string())
            .collect(),
        Ok(v) => v
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
    };
    let step_ms: u64 = std::env::var("MAGPIE_UI_TOUR_MS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(3000);

    std::thread::spawn(move || {
        // Let the event loop and the tray settle before summoning.
        std::thread::sleep(Duration::from_millis(900));
        {
            let (w, s) = (weak.clone(), state.clone());
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = w.upgrade() {
                    show_window(&ui, &s);
                    // A fresh window for screenshots: no leftover query.
                    ui.set_query(SharedString::from(""));
                }
            });
        }
        for step in steps {
            std::thread::sleep(Duration::from_millis(step_ms));
            let w = weak.clone();
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = w.upgrade() {
                    // Reset to the list first so each step starts from a known state.
                    ui.set_mode(SharedString::from("list"));
                    ui.set_view(SharedString::from("list"));
                    match step.as_str() {
                        "actions" => ui.invoke_open_actions(),
                        "filters" => ui.invoke_open_search(),
                        "slots" => ui.invoke_open_slots(),
                        "stats" => ui.invoke_toggle_view(),
                        "edit" => ui.invoke_start_edit(ui.get_selected()),
                        // Queue a few entries so the merge bar is populated.
                        "merge" => {
                            for i in 0..3 {
                                ui.invoke_toggle_merge(i);
                            }
                        }
                        "mask" => ui.invoke_toggle_screenshare(),
                        "notes" => ui.invoke_set_mode_notes(true),
                        "tasks" => ui.invoke_set_mode_tasks(true),
                        "bookmarks" => ui.invoke_set_mode_bookmarks(true),
                        "today" => ui.invoke_set_mode_today(true),
                        "journal" => ui.invoke_set_mode_journal(true),
                        "graph" => ui.invoke_set_mode_graph(true),
                        // First-class router targets, so a screenshot pass can
                        // reach EVERY surface by name without temporary hooks:
                        //   section:<clipboard|tasks|notes|bookmarks|stats>
                        //   taskview:<today|list|board>  noteview:<notes|journal|graph>
                        //   board  (shorthand for the tasks section's Kanban)
                        "board" => {
                            ui.invoke_set_section(SharedString::from("tasks"));
                            ui.invoke_set_task_view(SharedString::from("board"));
                        }
                        s if s.starts_with("section:") => {
                            ui.invoke_set_section(SharedString::from(&s["section:".len()..]));
                        }
                        s if s.starts_with("taskview:") => {
                            ui.invoke_set_section(SharedString::from("tasks"));
                            ui.invoke_set_task_view(SharedString::from(&s["taskview:".len()..]));
                        }
                        s if s.starts_with("noteview:") => {
                            ui.invoke_set_section(SharedString::from("notes"));
                            ui.invoke_set_note_view(SharedString::from(&s["noteview:".len()..]));
                        }
                        // Toggle slot 1 on the selection, to see the speed-dial
                        // strip populated. Running it twice clears it again.
                        "slot1" => ui.invoke_assign_slot(ui.get_selected(), 1),
                        "empty" => {
                            let q = SharedString::from("zzqqxnomatch");
                            ui.set_query(q.clone());
                            ui.invoke_search_changed(q);
                        }
                        // "text"/"link"/"color"/"image"/"file" select that type chip.
                        kind if TYPE_CHIPS.contains(&kind) => {
                            let idx =
                                TYPE_CHIPS.iter().position(|c| *c == kind).unwrap_or(0) as i32;
                            ui.set_type_index(idx);
                            ui.invoke_set_type_filter(idx);
                        }
                        other => ui.set_mode(SharedString::from(other)),
                    }
                }
            });
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
                                // Set the clipboard here (this runs on the hotkey
                                // thread); the ⌘V keystroke must fire on the main
                                // thread (enigo/TIS asserts main-thread), so defer it.
                                let _ = perform_paste(
                                    &mut clip,
                                    &EnigoPaster,
                                    &entry,
                                    PasteKind::Formatted,
                                    false,
                                );
                            }
                            if auto {
                                let _ = slint::invoke_from_event_loop(|| {
                                    let _ = magpie_platform::Paster::paste(&EnigoPaster);
                                });
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
    // Diagnostics first: log everything, surface panics, detect a previous silent
    // crash. Paths live under <data_dir>/logs.
    let log_dir = data_dir().join("logs");
    let log_path = log_dir.join("magpie.log");
    let session_path = log_dir.join("session");
    let crashes_path = log_dir.join("crashes");
    use magpie_app::diagnostics as diag;
    diag::install_panic_hook(log_path.clone());
    let prev_exit = diag::read_prev_exit(&session_path);
    diag::mark_running(&session_path);
    diag::log_line(&log_path, "startup");

    let cfg = magpie_app::config::load_or_default(&data_dir().join("config.toml"));
    let mut denylist = default_app_denylist();
    denylist.extend(cfg.app_denylist.clone());

    let state = build_state(&cfg);
    let ui = LauncherWindow::new().expect("create window");
    let weak = ui.as_weak();
    // Apply the persisted theme once, at window creation — see the comment in
    // `show_window` for why this isn't repeated on every summon.
    ui.set_theme_dark(state.theme_dark);

    // The menubar popover: a second, chromeless, always-on-top window shown from
    // the tray left-click. Kept alive for the whole run alongside `ui` (dropping it
    // early would tear the window down). It's a separate top-level component with its
    // OWN `Theme` global, so its theme must be set + toggled independently of `ui`.
    // Created here (before the theme-toggle handler) so the toggle can update both.
    let popover = Popover::new().expect("create popover");
    popover.set_theme_dark(state.theme_dark);

    // Match the native window chrome (titlebar / traffic-light strip) to the
    // theme, so light mode isn't undercut by a dark OS titlebar.
    magpie_platform::set_appearance(state.theme_dark);

    {
        let w = weak.clone();
        ui.on_dismiss_welcome(move || {
            if let Some(ui) = w.upgrade() {
                ui.set_show_welcome(false);
            }
        });
    }
    {
        let w = weak.clone();
        let pw = popover.as_weak();
        ui.on_toggle_theme(move || {
            if let Some(ui) = w.upgrade() {
                let new_dark = !ui.get_theme_dark();
                ui.set_theme_dark(new_dark);
                if let Some(p) = pw.upgrade() {
                    p.set_theme_dark(new_dark);
                }
                magpie_platform::set_appearance(new_dark);
                let cfg_path = data_dir().join("config.toml");
                let mut cfg = magpie_app::config::load_or_default(&cfg_path);
                cfg.theme_dark = new_dark;
                let _ = magpie_app::config::save(&cfg, &cfg_path);
            }
        });
    }
    {
        let p = popover.as_weak();
        popover.on_dismiss(move || {
            if let Some(p) = p.upgrade() {
                let _ = p.hide();
            }
        });
    }
    {
        let w = ui.as_weak();
        let s = state.clone();
        let pw = popover.as_weak();
        popover.on_open_full(move || {
            if let Some(p) = pw.upgrade() {
                let _ = p.hide();
            }
            if let Some(ui) = w.upgrade() {
                show_window(&ui, &s);
            }
        });
    }
    // Tasks-tab actions: toggle a task's checkbox, quick-add to today's daily
    // note, and jump to a task's owning note in the full window.
    {
        let s = state.clone();
        let pw = popover.as_weak();
        popover.on_toggle_ptask(move |note_id, line_index| {
            {
                let store = match s.store.lock() {
                    Ok(g) => g,
                    Err(e) => e.into_inner(),
                };
                let _ = magpie_app::tasks::toggle_task(
                    &store,
                    note_id as i64,
                    line_index as usize,
                    now_ms(),
                );
            }
            if let Some(p) = pw.upgrade() {
                refresh_popover(&p, &s);
            }
        });
    }
    {
        let s = state.clone();
        let pw = popover.as_weak();
        popover.on_add_task(move |text| {
            {
                let store = match s.store.lock() {
                    Ok(g) => g,
                    Err(e) => e.into_inner(),
                };
                let day = abs_date(now_ms());
                if let Ok(n) = store.daily_note(&day, now_ms()) {
                    let body = magpie_app::tasks::append_task_line(&n.body, text.as_str());
                    let _ = store.update_note_body(n.id, &body, now_ms());
                }
            }
            if let Some(p) = pw.upgrade() {
                refresh_popover(&p, &s);
            }
        });
    }
    {
        let s = state.clone();
        popover.on_edit_daily(move |text| {
            let store = match s.store.lock() {
                Ok(g) => g,
                Err(e) => e.into_inner(),
            };
            let day = abs_date(now_ms());
            if let Ok(n) = store.daily_note(&day, now_ms()) {
                let _ = store.update_note_body(n.id, text.as_str(), now_ms());
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        let pw = popover.as_weak();
        popover.on_paste_clip(move |idx| {
            if let Some(p) = pw.upgrade() {
                let _ = p.hide();
            }
            paste_and_close(&s, &w, idx.max(0) as usize, false);
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        let pw = popover.as_weak();
        popover.on_open_task_note(move |note_id| {
            if let Some(p) = pw.upgrade() {
                let _ = p.hide();
            }
            if let Some(ui) = w.upgrade() {
                show_window(&ui, &s);
                open_note_into_notes_mode(&ui, &s, note_id);
            }
        });
    }

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
            // For normal-size, unmasked entries, also expose the joined text so the
            // UI can show a selectable/copyable TextEdit. Huge entries keep
            // preview-text empty and fall back to the fast virtualized line list.
            const SELECTABLE_MAX_LINES: usize = 800;
            const SELECTABLE_MAX_CHARS: usize = 40_000;
            let selectable = match entry {
                Some(e)
                    if (!masked || revealed)
                        && !truncated
                        && lines.len() <= SELECTABLE_MAX_LINES
                        && e.full_text.len() <= SELECTABLE_MAX_CHARS =>
                {
                    e.full_text.clone()
                }
                _ => String::new(),
            };
            // Image entries have no text at all, so without this the preview pane
            // was blank. A masked entry stays hidden until Reveal.
            let (preview_image, has_preview_image) = match entry {
                Some(e) if !masked || revealed => load_entry_image(&s.images, e),
                _ => (slint::Image::default(), false),
            };
            if let Some(ui) = w.upgrade() {
                ui.set_preview_lines(ModelRc::new(VecModel::from(lines)));
                ui.set_preview_truncated(truncated);
                ui.set_preview_text(SharedString::from(selectable));
                ui.set_preview_image(preview_image);
                ui.set_preview_has_image(has_preview_image);
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
        // Preview font zoom: ⌘+ bigger, ⌘- smaller, ⌘0 reset. Clamped 9–28px.
        let w = ui.as_weak();
        ui.on_zoom_preview(move |dir| {
            if let Some(ui) = w.upgrade() {
                let next = if dir == 0 {
                    13.0
                } else {
                    (ui.get_preview_font_size() + dir as f32).clamp(9.0, 28.0)
                };
                ui.set_preview_font_size(next);
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_activate(move |idx| {
            // Defer out of the synchronous key-event dispatch: hiding/deactivating
            // the window from inside its own keyDown handler aborts on macOS.
            let (s, w) = (s.clone(), w.clone());
            let _ = slint::invoke_from_event_loop(move || {
                paste_and_close(&s, &w, idx as usize, false);
            });
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_activate_nth(move |n| {
            if n >= 1 {
                let (s, w) = (s.clone(), w.clone());
                let _ = slint::invoke_from_event_loop(move || {
                    paste_and_close(&s, &w, (n - 1) as usize, false);
                });
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
            // Deferred so the window hide happens outside the click-event dispatch.
            let (s, w) = (s.clone(), w.clone());
            let _ = slint::invoke_from_event_loop(move || {
                let entry = s
                    .store
                    .lock()
                    .ok()
                    .and_then(|st| st.slot_entry(n as i64).ok().flatten());
                let recent = current_results(&s, now_ms());
                if let Some(entry) = resolve_slot_or_recent(entry, &recent, n as usize) {
                    if let Ok(mut clip) = magpie_platform::platform_clipboard() {
                        let _ = perform_paste(
                            &mut clip,
                            &EnigoPaster,
                            &entry,
                            PasteKind::Formatted,
                            false,
                        );
                    }
                }
                if let Some(ui) = w.upgrade() {
                    hide_launcher(&ui);
                }
                spawn_paste(w.clone(), false);
            });
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
                // Mirrored into the window so the Filters overlay can tick the
                // active source app.
                ui.set_app_index(id);
                ui.set_mode(SharedString::from("list"));
                ui.set_selected(0);
                refresh(&ui, &s);
            }
        });
    }
    {
        // "Pinned only" — the query layer always supported it, but nothing in the
        // UI could turn it on.
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_set_pinned_only(move |on| {
            if let Ok(mut u) = s.ui.lock() {
                u.pinned_only = on;
            }
            if let Some(ui) = w.upgrade() {
                ui.set_selected(0);
                refresh(&ui, &s);
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_clear_filters(move || {
            // Clears *every* narrowing control, type chip and tag included — a
            // partial reset would leave the user still looking at an empty list.
            if let Ok(mut u) = s.ui.lock() {
                u.app_filter = None;
                u.time_filter = magpie_app::viewmodel::TimeFilter::All;
                u.type_filter = magpie_app::viewmodel::TypeFilter::All;
                u.tag = None;
                u.pinned_only = false;
            }
            if let Some(ui) = w.upgrade() {
                ui.set_time_index(0);
                ui.set_app_index(-1);
                ui.set_type_index(0);
                ui.set_tag_filter(SharedString::from(""));
                ui.set_pinned_only(false);
                ui.set_selected(0);
                ui.set_mode(SharedString::from("list"));
                refresh(&ui, &s);
            }
        });
    }
    {
        ui.on_open_accessibility_settings(|| {
            // Prompt first so macOS registers Magpie in the Accessibility list
            // (a bare trust check never adds it), then deep-link to the pane.
            let _ = magpie_platform::prompt_accessibility();
            magpie_platform::open_accessibility_settings();
        });
    }
    {
        let w = ui.as_weak();
        ui.on_recheck_accessibility(move || {
            // The user says they've enabled it. Close the modal and never gate again
            // this session — even if AXIsProcessTrusted() still reports false (common
            // for an unsigned .app until relaunch). Subsequent pastes proceed
            // best-effort; if the grant is live the keystroke lands, and if it needs
            // a relaunch the user isn't trapped in an un-clearable modal.
            A11Y_DISMISSED.store(true, std::sync::atomic::Ordering::Relaxed);
            if let Some(ui) = w.upgrade() {
                ui.set_needs_accessibility(false);
            }
        });
    }
    {
        let w = ui.as_weak();
        ui.on_dismiss_accessibility(move || {
            // "Not now" — stop gating for the session; paste best-effort afterward.
            A11Y_DISMISSED.store(true, std::sync::atomic::Ordering::Relaxed);
            if let Some(ui) = w.upgrade() {
                ui.set_needs_accessibility(false);
            }
        });
    }
    {
        let w = ui.as_weak();
        ui.on_hide_window(move || {
            // Deferred: Esc fires this inside keyDown; hiding the window there aborts.
            let w = w.clone();
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = w.upgrade() {
                    // Return focus to the app the user came from (keeps loop alive).
                    hide_launcher(&ui);
                }
            });
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
        ui.on_open_slots(move || {
            if let Some(ui) = w.upgrade() {
                ui.set_mode(SharedString::from("slots"));
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
            let (s, w) = (s.clone(), w.clone());
            let _ = slint::invoke_from_event_loop(move || {
                paste_and_close(&s, &w, idx as usize, true);
            });
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
            // Resolve the row to its entry id up front — the queue is keyed by id
            // so it stays correct across searches and filter changes.
            let results = current_results(&s, now_ms());
            let Some(id) = results.get(index as usize).map(|e| e.id) else {
                return;
            };
            if let Ok(mut m) = s.merge_set.lock() {
                if let Some(pos) = m.iter().position(|&x| x == id) {
                    m.remove(pos);
                } else {
                    m.push(id);
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
            // Deferred, then hide → paste, exactly like `paste_and_close`. Pasting
            // straight from here sent ⌘V to the *launcher* (it still had focus), so
            // the merged text landed in Magpie's own search field instead of the
            // app the user came from.
            let (s, w) = (s.clone(), w.clone());
            let _ = slint::invoke_from_event_loop(move || {
                let ids: Vec<i64> = s.merge_set.lock().map(|m| m.clone()).unwrap_or_default();
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
                if merged.is_empty() {
                    return;
                }
                if let Ok(mut clip) = magpie_platform::platform_clipboard() {
                    if clip.set_content(&Content::Text(merged)).is_err() {
                        return;
                    }
                }
                if let Ok(mut m) = s.merge_set.lock() {
                    m.clear();
                }
                if let Some(ui) = w.upgrade() {
                    refresh(&ui, &s);
                    hide_launcher(&ui);
                }
                spawn_paste(w.clone(), false);
            });
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
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_delete_time_entry(move |id| {
            {
                let store = s.store.lock().unwrap_or_else(|e| e.into_inner());
                let _ = store.delete_time_entry(id as i64);
            }
            if let Some(ui) = w.upgrade() {
                refresh_stats(&ui, &s, ui.get_range_index());
            }
        });
    }
    // ---- Sidebar navigation router ----
    // `set-section` is what the sidebar's 5 icons call; reading back the
    // retained `tasks-view`/`notes-view` before routing is what gives
    // "remember the last view shown" for a re-entered multi-view section.
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_set_section(move |section| {
            if let Some(ui) = w.upgrade() {
                nav_section(&ui, &s, section.as_str());
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_set_task_view(move |v| {
            if let Some(ui) = w.upgrade() {
                nav_to(&ui, &s, "tasks", v.as_str());
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_set_note_view(move |v| {
            if let Some(ui) = w.upgrade() {
                nav_to(&ui, &s, "notes", v.as_str());
            }
        });
    }
    // ---- Keyboard nav layer: g-leader section jump, ⌘[/⌘] history,
    // ⌘⌥← jump-to-previous, ⌘⌥↑/↓ view cycling (Slint side: launcher.slint). ----
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_nav_section(move |section| {
            if let Some(ui) = w.upgrade() {
                nav_section(&ui, &s, section.as_str());
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_nav_back(move || {
            if let Some(ui) = w.upgrade() {
                on_nav_back(&ui, &s);
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_nav_forward(move || {
            if let Some(ui) = w.upgrade() {
                on_nav_forward(&ui, &s);
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        // Jump-to-previous is one `nav-back` — the screen we leave lands on
        // `nav_fwd`, so pressing ⌘⌥← again toggles back to it.
        ui.on_nav_previous(move || {
            if let Some(ui) = w.upgrade() {
                on_nav_back(&ui, &s);
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_nav_cycle_view(move |dir| {
            if let Some(ui) = w.upgrade() {
                on_nav_cycle_view(&ui, &s, dir);
            }
        });
    }

    // ---- Notes mode ----
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_set_mode_notes(move |on| {
            if let Some(ui) = w.upgrade() {
                if on {
                    nav_to(&ui, &s, "notes", "notes");
                } else {
                    nav_to(&ui, &s, "clipboard", "");
                }
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_open_note(move |id| {
            if let Some(ui) = w.upgrade() {
                ui.set_note_id(id);
                refresh_notes(&ui, &s);
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_filter_notes_by_tag(move |tag| {
            if let Some(ui) = w.upgrade() {
                ui.set_note_tag_filter(tag);
                refresh_notes(&ui, &s);
            }
        });
    }
    {
        // Keyboard note switching: move to the adjacent note in the displayed list
        // (recent_notes order) and open it.
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_nav_note(move |delta| {
            if let Some(ui) = w.upgrade() {
                let cur = ui.get_note_id();
                let notes = {
                    let store = match s.store.lock() {
                        Ok(g) => g,
                        Err(e) => e.into_inner(),
                    };
                    store.recent_notes(200).unwrap_or_default()
                };
                if notes.is_empty() {
                    return;
                }
                let idx = notes.iter().position(|n| n.id as i32 == cur).unwrap_or(0) as i64;
                let new = (idx + delta as i64).clamp(0, notes.len() as i64 - 1) as usize;
                ui.set_note_id(notes[new].id as i32);
                refresh_notes(&ui, &s);
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_edit_note_body_at(move |body, caret| {
            if let Some(ui) = w.upgrade() {
                let id = ui.get_note_id();
                let (names, own_name) = {
                    let store = match s.store.lock() {
                        Ok(g) => g,
                        Err(e) => e.into_inner(),
                    };
                    if id >= 0 {
                        let _ = store.update_note_body(id as i64, body.as_str(), now_ms());
                    }
                    let own_name = if id >= 0 {
                        store.get_note(id as i64).ok().flatten().map(|n| n.name)
                    } else {
                        None
                    };
                    (store.all_note_names().unwrap_or_default(), own_name)
                };
                // Refresh only the links strip (cheap) — don't rebuild the list on
                // every keystroke.
                ui.set_note_links(ModelRc::new(VecModel::from(
                    magpie_app::notes_view::wiki_links(body.as_str())
                        .into_iter()
                        .map(SharedString::from)
                        .collect::<Vec<_>>(),
                )));
                let suggestions =
                    match wiki_complete::active_wiki_query(body.as_str(), caret.max(0) as usize) {
                        Some((_, query)) => {
                            let names: Vec<String> = names
                                .into_iter()
                                .filter(|n| Some(n) != own_name.as_ref())
                                .collect();
                            wiki_complete::rank_matches(&names, query, 8)
                        }
                        None => Vec::new(),
                    };
                ui.set_note_suggestions(ModelRc::new(VecModel::from(
                    suggestions
                        .into_iter()
                        .map(SharedString::from)
                        .collect::<Vec<_>>(),
                )));
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_accept_note_suggestion(move |name| {
            if let Some(ui) = w.upgrade() {
                let id = ui.get_note_id();
                let body = ui.get_note_body().to_string();
                let caret = ui.get_note_caret();
                let (new_text, new_caret) = wiki_complete::apply_wiki_completion(
                    &body,
                    caret.max(0) as usize,
                    name.as_str(),
                );
                if id >= 0 {
                    let store = match s.store.lock() {
                        Ok(g) => g,
                        Err(e) => e.into_inner(),
                    };
                    let _ = store.update_note_body(id as i64, &new_text, now_ms());
                }
                ui.set_note_body(SharedString::from(new_text.clone()));
                ui.set_note_caret(new_caret as i32);
                ui.set_note_links(ModelRc::new(VecModel::from(
                    magpie_app::notes_view::wiki_links(&new_text)
                        .into_iter()
                        .map(SharedString::from)
                        .collect::<Vec<_>>(),
                )));
                ui.set_note_suggestions(ModelRc::new(VecModel::from(Vec::<SharedString>::new())));
            }
        });
    }
    {
        let w = ui.as_weak();
        ui.on_dismiss_note_suggestions(move || {
            if let Some(ui) = w.upgrade() {
                ui.set_note_suggestions(ModelRc::new(VecModel::from(Vec::<SharedString>::new())));
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_open_page(move |name| {
            if let Some(ui) = w.upgrade() {
                let id = {
                    let store = match s.store.lock() {
                        Ok(g) => g,
                        Err(e) => e.into_inner(),
                    };
                    store
                        .upsert_note_by_name(name.as_str(), now_ms())
                        .ok()
                        .map(|n| n.id as i32)
                };
                if let Some(id) = id {
                    ui.set_note_id(id);
                    refresh_notes(&ui, &s);
                }
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_link_mention(move |note_id, line_index| {
            {
                let store = match s.store.lock() {
                    Ok(g) => g,
                    Err(e) => e.into_inner(),
                };
                if let Some(ui) = w.upgrade() {
                    let open_id = ui.get_note_id() as i64;
                    let target = store.get_note(open_id).ok().flatten().map(|n| n.name);
                    let src = store.get_note(note_id as i64).ok().flatten();
                    if let (Some(target), Some(src)) = (target, src) {
                        let body = magpie_app::backlinks::link_mention(
                            &src.body,
                            line_index as usize,
                            &target,
                        );
                        let _ = store.update_note_body(src.id, &body, now_ms());
                    }
                }
            }
            if let Some(ui) = w.upgrade() {
                refresh_notes(&ui, &s);
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_new_note(move || {
            if let Some(ui) = w.upgrade() {
                // Unique "Untitled" page.
                let id = {
                    let store = match s.store.lock() {
                        Ok(g) => g,
                        Err(e) => e.into_inner(),
                    };
                    let mut name = "Untitled".to_string();
                    let mut i = 2;
                    while store.note_by_name(&name).ok().flatten().is_some() {
                        name = format!("Untitled ({i})");
                        i += 1;
                    }
                    store
                        .upsert_note_by_name(&name, now_ms())
                        .ok()
                        .map(|n| n.id as i32)
                };
                if let Some(id) = id {
                    ui.set_note_id(id);
                    refresh_notes(&ui, &s);
                }
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_note_from_entry(move |idx| {
            if let Some(ui) = w.upgrade() {
                let recent = current_results(&s, now_ms());
                if let Some(entry) = recent.get(idx as usize) {
                    let id = {
                        let store = match s.store.lock() {
                            Ok(g) => g,
                            Err(e) => e.into_inner(),
                        };
                        store
                            .create_note_from_entry(entry.id, now_ms())
                            .ok()
                            .map(|n| n.id as i32)
                    };
                    if let Some(id) = id {
                        open_note_into_notes_mode(&ui, &s, id);
                    }
                }
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_promote_line(move |byte| {
            if let Some(ui) = w.upgrade() {
                let id = ui.get_note_id();
                if id >= 0 {
                    let body = ui.get_note_body().to_string();
                    let new = magpie_app::tasks::promote_line(&body, byte.max(0) as usize);
                    if new != body {
                        let store = match s.store.lock() {
                            Ok(g) => g,
                            Err(e) => e.into_inner(),
                        };
                        if store
                            .update_note_body(id as i64, &new, now_ms())
                            .unwrap_or(false)
                        {
                            ui.set_note_body(SharedString::from(new.clone()));
                            ui.set_note_links(ModelRc::new(VecModel::from(
                                magpie_app::notes_view::wiki_links(&new)
                                    .into_iter()
                                    .map(SharedString::from)
                                    .collect::<Vec<_>>(),
                            )));
                        }
                    }
                }
            }
        });
    }
    // ---- Tasks mode ----
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_set_mode_tasks(move |on| {
            if let Some(ui) = w.upgrade() {
                if on {
                    // Remembers the last tasks sub-view shown (today/list/board),
                    // same as the sidebar's Tasks icon.
                    let view = ui.get_tasks_view().to_string();
                    nav_to(&ui, &s, "tasks", &view);
                } else {
                    nav_to(&ui, &s, "clipboard", "");
                }
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_toggle_task(move |note_id, line_index| {
            {
                let store = match s.store.lock() {
                    Ok(g) => g,
                    Err(e) => e.into_inner(),
                };
                magpie_app::tasks::toggle_task(
                    &store,
                    note_id as i64,
                    line_index as usize,
                    now_ms(),
                );
            }
            if let Some(ui) = w.upgrade() {
                refresh_tasks(&ui, &s);
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_toggle_bookmark(move |note_id, line_index| {
            {
                let store = match s.store.lock() {
                    Ok(g) => g,
                    Err(e) => e.into_inner(),
                };
                let _ = magpie_app::tasks::set_bookmark(
                    &store,
                    note_id as i64,
                    line_index as usize,
                    now_ms(),
                );
            }
            if let Some(ui) = w.upgrade() {
                refresh_tasks(&ui, &s);
            }
        });
    }
    {
        // Start/stop a task's timer. Toggling the already-running task stops it;
        // otherwise start it (which stops any other running timer).
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_toggle_timer(move |note_id, title| {
            // If this stops the running timer, capture its start/end so we can log
            // an org CLOCK entry into the note after the stop is recorded.
            let mut stopped: Option<(i64, i64)> = None;
            {
                let store = match s.store.lock() {
                    Ok(g) => g,
                    Err(e) => e.into_inner(),
                };
                let key = format!("{}|{}", note_id, title);
                let active = store.active_timer().ok().flatten();
                let running_this = active.as_ref().map(|a| a.task_key == key).unwrap_or(false);
                if running_this {
                    let end = now_ms();
                    if store.stop_active(end).unwrap_or(false) {
                        stopped = active.map(|a| (a.start_ms, end));
                    }
                } else {
                    let _ = store.start_timer(&key, title.as_str(), Some(note_id as i64), now_ms());
                }
            }
            if let Some((start_ms, end_ms)) = stopped {
                if s.log_clock_entries {
                    let store = match s.store.lock() {
                        Ok(g) => g,
                        Err(e) => e.into_inner(),
                    };
                    if let Ok(Some(note)) = store.get_note(note_id as i64) {
                        let new = magpie_app::tasks::log_clock(
                            &note.body,
                            title.as_str(),
                            start_ms,
                            end_ms,
                            local_offset_seconds(),
                        );
                        if new != note.body {
                            let _ = store.update_note_body(note_id as i64, &new, now_ms());
                        }
                    }
                }
            }
            if let Some(ui) = w.upgrade() {
                refresh_tasks(&ui, &s);
            }
        });
    }
    {
        // ⌘K "Export data…": write a Markdown vault + clipboard JSONL to a dated
        // folder in Downloads and reveal it in Finder.
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_export_data(move || {
            let out = dirs::home_dir()
                .map(|h| h.join("Downloads"))
                .unwrap_or_else(|| std::path::PathBuf::from("."))
                .join(format!("Magpie Export {}", abs_date(now_ms())));
            {
                let store = match s.store.lock() {
                    Ok(g) => g,
                    Err(e) => e.into_inner(),
                };
                let _ = std::fs::create_dir_all(&out);
                let _ = magpie_core::export_markdown(&store, &out);
                let _ = magpie_core::export_clipboard_jsonl(&store, &out.join("clipboard.jsonl"));
            }
            magpie_app::diagnostics::reveal(&out);
            if let Some(ui) = w.upgrade() {
                hide_launcher(&ui);
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_open_task_note(move |note_id| {
            if let Some(ui) = w.upgrade() {
                open_note_into_notes_mode(&ui, &s, note_id);
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_refresh_tasks(move || {
            if let Some(ui) = w.upgrade() {
                refresh_tasks(&ui, &s);
            }
        });
    }
    // ---- Bookmarks mode ----
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_set_mode_bookmarks(move |on| {
            if let Some(ui) = w.upgrade() {
                if on {
                    nav_to(&ui, &s, "bookmarks", "");
                } else {
                    nav_to(&ui, &s, "clipboard", "");
                }
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_bookmark_search(move |_q| {
            if let Some(ui) = w.upgrade() {
                refresh_bookmarks(&ui, &s);
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_add_bookmark(move |input| {
            // The input may carry trailing hashtags, e.g. "https://x.com #work #reading".
            let mut url = String::new();
            let mut tags: Vec<String> = Vec::new();
            for tok in input.split_whitespace() {
                if url.is_empty() && (tok.starts_with("http://") || tok.starts_with("https://")) {
                    url = tok.to_string();
                } else if let Some(t) = tok.strip_prefix('#') {
                    if !t.is_empty() {
                        tags.push(t.to_string());
                    }
                }
            }
            if !(url.starts_with("http://") || url.starts_with("https://")) {
                return;
            }
            let domain = magpie_app::favicon::domain_of(&url).unwrap_or_default();
            {
                let store = match s.store.lock() {
                    Ok(g) => g,
                    Err(e) => e.into_inner(),
                };
                if let Ok(id) = store.add_bookmark(&url, "", &domain, now_ms()) {
                    if !tags.is_empty() {
                        let _ = store.set_bookmark_tags(id, &tags);
                    }
                }
            }
            if let Some(ui) = w.upgrade() {
                refresh_bookmarks(&ui, &s);
            }
            // Background: fetch the real title, then upsert + refresh.
            let s2 = s.clone();
            let w2 = w.clone();
            let url_for_preview = url.clone();
            std::thread::spawn(move || {
                let meta = magpie_app::link_meta::fetch_link_meta(&url);
                if let Some(title) = meta.title {
                    {
                        let store = match s2.store.lock() {
                            Ok(g) => g,
                            Err(e) => e.into_inner(),
                        };
                        let _ = store.add_bookmark(&url, &title, &domain, now_ms());
                    }
                    let s3 = s2.clone();
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(ui) = w2.upgrade() {
                            refresh_bookmarks(&ui, &s3);
                        }
                    });
                }
            });
            // Explicit save — fetch a rich preview thumbnail (privacy: NEVER done
            // for merely-copied URLs, only here and in `on_bookmark_selected`).
            enrich_bookmark_preview(&s, w.clone(), url_for_preview);
        });
    }
    {
        let w = ui.as_weak();
        ui.on_open_bookmark(move |url| {
            if let Some(ui) = w.upgrade() {
                open_url_and_hide(&ui, url.as_str());
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_delete_bookmark(move |id| {
            {
                let store = match s.store.lock() {
                    Ok(g) => g,
                    Err(e) => e.into_inner(),
                };
                let _ = store.delete_bookmark(id as i64);
            }
            if let Some(ui) = w.upgrade() {
                refresh_bookmarks(&ui, &s);
            }
        });
    }
    {
        // ⌘K "Bookmark this link": save the selected clipboard entry if it's a URL.
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_bookmark_selected(move || {
            if let Some(ui) = w.upgrade() {
                let idx = ui.get_selected();
                let recent = current_results(&s, now_ms());
                if let Some(e) = (idx >= 0).then(|| recent.get(idx as usize)).flatten() {
                    let url = e.full_text.trim().to_string();
                    if url.starts_with("http://") || url.starts_with("https://") {
                        ui.invoke_add_bookmark(SharedString::from(url));
                        nav_to(&ui, &s, "bookmarks", "");
                    }
                }
            }
        });
    }
    {
        // ⌘K "Create task from link": any selected entry (not just URLs) becomes
        // a task on today's daily note.
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_task_from_link(move || {
            if let Some(ui) = w.upgrade() {
                let idx = ui.get_selected();
                let recent = current_results(&s, now_ms());
                if let Some(e) = (idx >= 0).then(|| recent.get(idx as usize)).flatten() {
                    let text = e.full_text.trim().to_string();
                    if !text.is_empty() {
                        let store = match s.store.lock() {
                            Ok(g) => g,
                            Err(e) => e.into_inner(),
                        };
                        let day = abs_date(now_ms());
                        if let Ok(n) = store.daily_note(&day, now_ms()) {
                            let body = magpie_app::tasks::append_task_line(&n.body, &text);
                            let _ = store.update_note_body(n.id, &body, now_ms());
                        }
                    }
                }
            }
        });
    }

    // ---- Today mode ----
    // A full-window dashboard merging the popover's Today + Daily + Clipboard
    // tabs: due/overdue tasks, the running timer, today's journal, and recent
    // clips.
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_set_mode_today(move |on| {
            if let Some(ui) = w.upgrade() {
                if on {
                    nav_to(&ui, &s, "tasks", "today");
                } else {
                    nav_to(&ui, &s, "clipboard", "");
                }
            }
        });
    }
    {
        let s = state.clone();
        ui.on_edit_today_journal(move |text| {
            let store = match s.store.lock() {
                Ok(g) => g,
                Err(e) => e.into_inner(),
            };
            let day = abs_date(now_ms());
            if let Ok(n) = store.daily_note(&day, now_ms()) {
                let _ = store.update_note_body(n.id, text.as_str(), now_ms());
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_set_mode_journal(move |on| {
            if let Some(ui) = w.upgrade() {
                if on {
                    nav_to(&ui, &s, "notes", "journal");
                } else {
                    nav_to(&ui, &s, "clipboard", "");
                }
            }
        });
    }
    {
        let s = state.clone();
        ui.on_edit_journal_entry(move |note_id, text| {
            let store = match s.store.lock() {
                Ok(g) => g,
                Err(e) => e.into_inner(),
            };
            let _ = store.update_note_body(note_id as i64, text.as_str(), now_ms());
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_today_paste_clip(move |idx| {
            paste_and_close(&s, &w, idx.max(0) as usize, false);
        });
    }

    // ---- Board mode ----
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_set_mode_board(move |on| {
            if let Some(ui) = w.upgrade() {
                if on {
                    nav_to(&ui, &s, "tasks", "board");
                } else {
                    nav_to(&ui, &s, "clipboard", "");
                }
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_board_controls_changed(move || {
            if let Some(ui) = w.upgrade() {
                refresh_board(&ui, &s);
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_board_open_task(move |note_id| {
            if let Some(ui) = w.upgrade() {
                open_note_into_notes_mode(&ui, &s, note_id);
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_board_set_status(move |note_id, line_index, status| {
            {
                let store = match s.store.lock() {
                    Ok(g) => g,
                    Err(e) => e.into_inner(),
                };
                let status = magpie_app::tasks::status_from_column(status);
                magpie_app::tasks::set_task_status(
                    &store,
                    note_id as i64,
                    line_index as usize,
                    status,
                    now_ms(),
                );
            }
            if let Some(ui) = w.upgrade() {
                refresh_board(&ui, &s);
            }
        });
    }

    // ---- Graph mode ----
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_set_mode_graph(move |on| {
            if let Some(ui) = w.upgrade() {
                if on {
                    nav_to(&ui, &s, "notes", "graph");
                } else {
                    nav_to(&ui, &s, "clipboard", "");
                }
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_graph_open(move |note_id| {
            if let Some(ui) = w.upgrade() {
                open_note_into_notes_mode(&ui, &s, note_id);
            }
        });
    }

    // ---- Find everywhere (⌘⇧F): unified search over notes/tasks/bookmarks ----
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_run_unified_search(move |q| {
            if let Some(ui) = w.upgrade() {
                refresh_unified_search(&ui, &s, q.as_str());
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_open_search_note(move |note_id| {
            if let Some(ui) = w.upgrade() {
                open_note_into_notes_mode(&ui, &s, note_id);
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_open_search_task(move |note_id| {
            if let Some(ui) = w.upgrade() {
                open_note_into_notes_mode(&ui, &s, note_id);
            }
        });
    }
    {
        let w = ui.as_weak();
        ui.on_open_search_bookmark(move |url| {
            if let Some(ui) = w.upgrade() {
                open_url_and_hide(&ui, url.as_str());
            }
        });
    }
    {
        let w = ui.as_weak();
        ui.on_close_search(move || {
            if let Some(ui) = w.upgrade() {
                ui.set_search_mode(false);
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
    // Ask for notification permission once (macOS bundle only; no-op in dev), so
    // reminder banners show as "Magpie" instead of being silently dropped.
    magpie_platform::request_notification_authorization();
    // Route reminder-notification clicks: recover the task's note (encoded in the
    // notification identifier) and open Magpie to it. macOS bundle only — in dev
    // the delegate install is a no-op, so the handler is simply never called.
    {
        let w = weak.clone();
        let s = state.clone();
        magpie_platform::set_notification_click_handler(move |note_id| {
            let (w, s) = (w.clone(), s.clone());
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = w.upgrade() {
                    show_window(&ui, &s);
                    if note_id >= 0 {
                        open_note_into_notes_mode(&ui, &s, note_id as i32);
                    } else {
                        nav_to(&ui, &s, "tasks", "today");
                    }
                }
            });
        });
        magpie_platform::install_notification_delegate();
    }
    spawn_reminders(state.clone());
    // Opt-in live bidirectional vault sync: only when explicitly enabled AND a
    // vault path is configured.
    if cfg.vault_watch {
        if let Some(vault_path) = cfg.vault_path.clone() {
            spawn_vault_watcher(
                state.clone(),
                std::path::PathBuf::from(vault_path),
                weak.clone(),
            );
        }
    }
    // Keep the hotkey manager alive for the whole run.
    let _hotkeys = spawn_hotkeys(&cfg, state.clone(), weak.clone());
    // Keep the tray icon alive for the whole run. It lives in a thread-local
    // (not a local binding) so the ticker below can reach it from the event
    // loop; see the `TRAY` thread_local for why (`TrayIcon` isn't `Send`).
    TRAY.with(|t| *t.borrow_mut() = build_tray(weak.clone(), state.clone(), popover.as_weak()));

    // Live menu-bar timer: while a task timer is running, show its elapsed
    // time as the tray title ("▶ 12:34"); clear it when nothing is running.
    // A background thread (not a Slint `Timer`) because a plain Slint timer
    // may not tick while the app is hidden via `NSApp.hide`.
    {
        let state = state.clone();
        std::thread::spawn(move || {
            let mut last: Option<String> = None;
            loop {
                let title: Option<String> = {
                    let store = state.store.lock().unwrap_or_else(|e| e.into_inner());
                    match store.active_timer() {
                        Ok(Some(t)) => {
                            let secs = ((now_ms() - t.start_ms).max(0) / 1000) as u64;
                            Some(format!("▶ {}", fmt_ms_clock(secs)))
                        }
                        _ => None,
                    }
                };
                if title != last {
                    last = title.clone();
                    let _ = slint::invoke_from_event_loop(move || {
                        TRAY.with(|tray| {
                            if let Some(tray) = &*tray.borrow() {
                                tray.set_title(title.as_deref());
                            }
                        });
                    });
                }
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
        });
    }

    // The red close button hides the window (Magpie keeps running as a tray daemon);
    // "Quit Magpie" in the tray is the real exit.
    ui.window()
        .on_close_requested(|| slint::CloseRequestResponse::HideWindow);

    // If the previous run ended abnormally, tell the user (loud, not silent).
    // Only pop a dialog for the installed .app — in a dev binary, Ctrl-C'ing
    // `cargo run` is a normal "abnormal" exit and shouldn't nag; just log it.
    if prev_exit == diag::PrevExit::Crashed && running_as_app_bundle() {
        diag::log_line(&log_path, "previous run ended abnormally");
        let looping = diag::record_and_check_loop(&crashes_path);
        if looping {
            let choice = diag::alert(
                "Magpie is crashing repeatedly",
                &format!(
                    "Magpie has crashed several times in a row.\n\nA log was saved to:\n{}",
                    log_path.display()
                ),
                &["View Log", "Disable auto-start", "Quit"],
            );
            match choice.as_deref() {
                Some("View Log") => diag::reveal(&log_path),
                Some("Disable auto-start") => {
                    let exe = std::env::current_exe().unwrap_or_default();
                    let _ = magpie_platform::platform_autostart(&exe.to_string_lossy())
                        .set_enabled(false);
                    diag::log_line(&log_path, "auto-start disabled after crash loop");
                }
                Some("Quit") => {
                    diag::mark_clean(&session_path);
                    return;
                }
                _ => {}
            }
        } else {
            let choice = diag::alert(
                "Magpie recovered from a crash",
                &format!(
                    "Magpie quit unexpectedly last time and has restarted.\n\nA log was saved to:\n{}",
                    log_path.display()
                ),
                &["View Log", "Dismiss"],
            );
            if choice.as_deref() == Some("View Log") {
                diag::reveal(&log_path);
            }
        }
    } else if prev_exit == diag::PrevExit::Crashed {
        // Dev binary: record + log, but don't pop a dialog (Ctrl-C is routine).
        let _ = diag::record_and_check_loop(&crashes_path);
        diag::log_line(
            &log_path,
            "previous run ended abnormally (dev — not surfaced)",
        );
    }

    // First-run welcome overlay + sample data: seed a small starter note the very
    // first time Magpie ever launches (detected via a `.onboarded` marker file),
    // then show the launcher with the welcome card on top shortly after boot.
    let onboarded_marker = data_dir().join(".onboarded");
    let first_run = !onboarded_marker.exists();
    if first_run {
        // Seed a small sample so the Notes/Tasks/Today views aren't empty.
        {
            let store = match state.store.lock() {
                Ok(g) => g,
                Err(e) => e.into_inner(),
            };
            if let Ok(n) = store.upsert_note_by_name("Welcome to Magpie", now_ms()) {
                let body = "# Welcome to Magpie\n\nYour clipboard, notes, tasks, bookmarks and time — all local.\n\n- [ ] Try me: press Space to check me off !high @today\n- [ ] A recurring chore @mon +1w\n- [ ] A someday idea #ideas *\n\nLink things with [[wiki links]]. Press ⌘K on any clipboard entry for actions.";
                let _ = store.update_note_body(n.id, body, now_ms());
            }
            let _ = store.daily_note(&abs_date(now_ms()), now_ms());
        }
        let _ = std::fs::write(&onboarded_marker, b"1");
        // Show the launcher + the welcome overlay shortly after the loop starts.
        let w = weak.clone();
        let s = state.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(700));
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = w.upgrade() {
                    show_window(&ui, &s);
                    ui.set_show_welcome(true);
                }
            });
        });
    }

    // Opt-in dev hooks (MAGPIE_SHOW_ON_LAUNCH / MAGPIE_UI_TOUR); no-op otherwise.
    spawn_dev_ui_hooks(weak.clone(), state.clone());

    // Start hidden (background tray daemon); the launcher hotkey shows the window.
    // We deliberately do NOT call `ui.run()` (which would show the window on
    // launch) — `run_event_loop` keeps us alive with the tray icon only.
    slint::run_event_loop().expect("run event loop");

    // Reached only on a real quit (tray Quit → quit_event_loop). Mark the session
    // clean so the next launch doesn't report a false crash.
    diag::mark_clean(&session_path);
    diag::log_line(&log_path, "clean shutdown");
}

// `tray_icon::TrayIcon` wraps an `Rc<RefCell<_>>` internally, so it is not
// `Send`. It must live on (and only be touched from) the main/event-loop
// thread. Keeping it in a thread-local lets the background ticker reach it
// via `slint::invoke_from_event_loop` (whose closure must be `Send`) without
// ever capturing the `TrayIcon` itself — only a `Send` `Option<String>` title
// crosses the thread boundary.
thread_local! {
    static TRAY: std::cell::RefCell<Option<tray_icon::TrayIcon>> = const { std::cell::RefCell::new(None) };
}

/// Format elapsed seconds as a menu-bar clock: `mm:ss`, switching to `h:mm:ss`
/// past one hour.
fn fmt_ms_clock(secs: u64) -> String {
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}

/// Decode the embedded menu-bar template PNG (black magpie silhouette on
/// transparent) into a tray icon.
fn load_tray_icon() -> Option<tray_icon::Icon> {
    let bytes = include_bytes!("../icons/tray-template.png");
    let img = image::load_from_memory(bytes).ok()?.into_rgba8();
    let (w, h) = img.dimensions();
    tray_icon::Icon::from_rgba(img.into_raw(), w, h).ok()
}

/// Build the tray icon: right-click shows a menu (Show / Quit); left-click opens
/// the window. On Linux, tray click events are not emitted, so "Show Magpie" in
/// the menu is the portable way to open the window.
fn build_tray(
    weak: slint::Weak<LauncherWindow>,
    state: Arc<AppState>,
    popover: slint::Weak<Popover>,
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

    let mut builder = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .with_tooltip("Magpie");
    // Menu-bar icon: the magpie silhouette as a template image (macOS tints it for
    // the light/dark menu bar). No emoji fallback if decoding fails — an empty
    // title just leaves the tray icon blank rather than shipping a bird emoji.
    match load_tray_icon() {
        Some(icon) => {
            builder = builder.with_icon(icon).with_icon_as_template(true);
        }
        None => {
            builder = builder.with_title("");
        }
    }
    let tray = builder.build().ok()?;

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

    // Left-click on the icon toggles the popover, anchored under the tray icon
    // (macOS/Windows; Linux emits no click events, so "Show Magpie" is the door).
    {
        let popover = popover.clone();
        let state = state.clone();
        std::thread::spawn(move || {
            let rx = TrayIconEvent::receiver();
            while let Ok(ev) = rx.recv() {
                if let TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    rect,
                    ..
                } = ev
                {
                    let pw = popover.clone();
                    let s = state.clone();
                    // Centre the 360px-wide popover under the icon, clamped to the
                    // left screen edge, and drop it just below the menu bar.
                    let (px, py) = (
                        (rect.position.x + rect.size.width as f64 / 2.0 - 180.0).max(0.0) as i32,
                        (rect.position.y + rect.size.height as f64) as i32,
                    );
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(p) = pw.upgrade() {
                            if p.window().is_visible() {
                                let _ = p.hide();
                            } else {
                                refresh_popover(&p, &s);
                                p.window().set_position(slint::WindowPosition::Physical(
                                    slint::PhysicalPosition::new(px, py),
                                ));
                                let _ = p.show();
                                magpie_platform::raise_to_front();
                            }
                        }
                    });
                }
            }
        });
    }

    Some(tray)
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

#[cfg(test)]
mod nav_tests {
    use super::{clamp_selection, cycle_view, filter_actions};

    #[test]
    fn cycle_view_wraps_both_directions() {
        let views = ["today", "list", "board"];
        assert_eq!(cycle_view(&views, "list", 1), "board");
        assert_eq!(cycle_view(&views, "board", 1), "today"); // wrap forward
        assert_eq!(cycle_view(&views, "today", -1), "board"); // wrap backward
        assert_eq!(cycle_view(&views, "list", -1), "today");
    }

    #[test]
    fn cycle_view_treats_unknown_current_as_index_zero() {
        let views = ["notes", "journal", "graph"];
        assert_eq!(cycle_view(&views, "nonsense", 1), "journal");
        assert_eq!(cycle_view(&views, "nonsense", -1), "graph");
    }

    #[test]
    fn clamp_selection_bounds_index() {
        assert_eq!(clamp_selection(0, 3), 0);
        assert_eq!(clamp_selection(2, 3), 2); // in range: unchanged
        assert_eq!(clamp_selection(5, 3), 2); // past end: last row
        assert_eq!(clamp_selection(5, 0), 0); // empty list: 0, never negative
    }

    #[test]
    fn filter_actions_matches_labels_case_insensitively() {
        // No query: the full set.
        assert_eq!(filter_actions("").len(), super::ACTIONS.len());
        // "paste" matches "Paste" and "Paste & keep open".
        let ids: Vec<&str> = filter_actions("paste").iter().map(|r| r.0).collect();
        assert_eq!(ids, vec!["paste", "keep"]);
        // Case-insensitive, single match.
        let ids: Vec<&str> = filter_actions("PIN").iter().map(|r| r.0).collect();
        assert_eq!(ids, vec!["pin"]);
        // No match: empty.
        assert!(filter_actions("zzzznomatch").is_empty());
    }
}
