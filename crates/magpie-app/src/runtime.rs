use crate::{
    ActionItem, AppItem, Bar, BoardColumn, BookmarkRow, CalDay, CalMonth, CalMonthLabel,
    CalYearCell, CalcRow, CalcStore, ChartPoint, ClipRow, DataColumn, DataStore, DateStore,
    EntryRow, FavoriteRow, FileTarget, GraphEdge, GraphNode, JobRow, JournalRow, LauncherWindow,
    MdLineVM, MdSpan, NoteRender, NoteRow, NoteVersionRow, Popover, ProjTaskRow, ProjectListRow,
    ProjectsStore, RecentData, RefRow, ScheduleStore, ScratchRow, ScratchStore, ScratchWindow,
    SearchBookmarkRow, SearchFile, SearchLine, SearchNoteRow, SearchStore, SearchTaskRow,
    SearchTreeRow, SecretRow, SecretsStore, SlotItem, SnippetRow, SnippetsStore, SuggestRow,
    TaskRow, TimeEntryRow, TimeStore, TransformOpts, WorldClockRow, WorldClockStore,
};
use magpie_app::app_state::DataSource;
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
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};
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
        paused: std::sync::atomic::AtomicBool::new(false),
        mask_apps: cfg.mask_apps.clone(),
        mask_patterns: cfg.mask_patterns.clone(),
        mask_visible_chars: cfg.mask_visible_chars.max(0),
        open_to_today: cfg.open_to_today,
        fetch_link_favicons: cfg.fetch_link_favicons,
        fetch_link_previews: cfg.fetch_link_previews,
        log_clock_entries: cfg.log_clock_entries,
        theme: cfg.theme.clone(),
        holiday_region: cfg.holiday_region.clone(),
        world_clock_home: cfg.world_clock_home.clone(),
        world_clock_cities: std::sync::Mutex::new(cfg.world_clock_cities.clone()),
        wc_session_targets: std::sync::Mutex::new(Vec::new()),
        wc_chosen_hour: std::sync::Mutex::new(-1),
        search: std::sync::Mutex::new(magpie_app::app_state::SearchState::default()),
        search_gen: std::sync::atomic::AtomicU64::new(0),
        key_monitor: std::sync::Mutex::new(None),
        expand_buffer: std::sync::Mutex::new(String::new()),
        expand_keywords: std::sync::Mutex::new(Vec::new()),
        pomodoro: std::sync::Mutex::new(magpie_app::app_state::PomodoroState::default()),
        projects_roots: cfg.projects_roots.clone(),
        project_scan: std::sync::Mutex::new(magpie_app::app_state::ProjectScanState {
            ignored: cfg.projects_ignored.clone(),
            ..Default::default()
        }),
        project_conflict_policy: std::sync::Mutex::new(if cfg.project_sync_conflict == "file" {
            magpie_core::projects::ConflictPolicy::FileWins
        } else {
            magpie_core::projects::ConflictPolicy::Ask
        }),
        projects_keep_magpie_out_of_git: std::sync::atomic::AtomicBool::new(
            cfg.projects_keep_magpie_out_of_git,
        ),
        project_conflicts: std::sync::Mutex::new(Vec::new()),
        project_task_refs: std::sync::Mutex::new(std::collections::HashMap::new()),
        project_note_refs: std::sync::Mutex::new(std::collections::HashMap::new()),
        scratch_shown_id: std::sync::Mutex::new(-1),
        popover_default_tab: std::sync::atomic::AtomicI32::new(
            match cfg.popover_default_tab.as_str() {
                "tasks" => 0,
                "today" => 1,
                "daily" => 2,
                _ => 3, // clipboard (default)
            },
        ),
        open_with: cfg.open_with.clone(),
        popover_pinned: std::sync::atomic::AtomicBool::new(cfg.popover_pinned),
        popover_size: std::sync::Mutex::new(magpie_app::panel::clamp_size(
            cfg.popover_width as i32,
            cfg.popover_height as i32,
        )),
        popover_resize_anchor: std::sync::Mutex::new(magpie_app::panel::clamp_size(
            cfg.popover_width as i32,
            cfg.popover_height as i32,
        )),
        nav_back: std::sync::Mutex::new(Vec::new()),
        nav_fwd: std::sync::Mutex::new(Vec::new()),
        data_input: std::sync::Mutex::new(String::new()),
        data_read_as: std::sync::Mutex::new(magpie_core::table::ReadAs::Auto),
        data_name: std::sync::Mutex::new(String::new()),
        data_recent: std::sync::Mutex::new(Vec::new()),
        data_chart: std::sync::Mutex::new(magpie_app::app_state::ChartState::default()),
        calc_graph_f: std::sync::Mutex::new(String::new()),
        calc_graph_g: std::sync::Mutex::new(String::new()),
        suppress_capture: std::sync::Mutex::new(std::collections::HashSet::new()),
        last_results: std::sync::Mutex::new(Vec::new()),
        preview_gen: std::sync::atomic::AtomicU64::new(0),
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
/// If `e` is a copied *path* to an existing image file (a `File` entry or path-like
/// text with an image extension), return the absolute path so the preview can show
/// the image itself. `None` otherwise.
fn image_file_of(e: &Entry) -> Option<std::path::PathBuf> {
    let candidate = if matches!(e.kind, Kind::File) {
        e.full_text.lines().next().unwrap_or("").trim()
    } else if magpie_core::openwith::looks_like_path(&e.full_text) {
        e.full_text.trim()
    } else {
        return None;
    };
    if magpie_core::openwith::category_for_path(candidate) != "image" {
        return None;
    }
    let path = magpie_app::file_search::expand_tilde(candidate);
    path.exists().then_some(path)
}

/// The first file path a clip refers to (a copied File, or a text clip that is a path),
/// for QuickLook thumbnailing. Any type — the caller decides what to render.
fn file_path_of(e: &Entry) -> Option<std::path::PathBuf> {
    let candidate = if matches!(e.kind, Kind::File) {
        e.full_text.lines().next().unwrap_or("").trim()
    } else if magpie_core::openwith::looks_like_path(&e.full_text) {
        e.full_text.trim()
    } else {
        return None;
    };
    if candidate.is_empty() {
        return None;
    }
    let path = magpie_app::file_search::expand_tilde(candidate);
    (path.exists() && !path.is_dir()).then_some(path)
}

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
    secret_ids: &std::collections::HashSet<i64>,
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
                domain: SharedString::from(if matches!(e.kind, Kind::Link) {
                    favicon::domain_of(&e.full_text).unwrap_or_default()
                } else {
                    String::new()
                }),
                thumb,
                has_thumb,
                swatch: match swatch_rgb {
                    Some((r, g, b)) => slint::Color::from_rgb_u8(r, g, b),
                    None => slint::Color::default(),
                },
                has_swatch: swatch_rgb.is_some(),
                is_secret: secret_ids.contains(&e.id),
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
    // Cache the displayed entries so the preview/selection hot path (arrow keys) reads
    // from memory instead of re-running the whole DB search on every keystroke.
    if let Ok(mut c) = state.last_results.lock() {
        *c = results.clone();
    }

    let screenshare = state.screenshare.lock().map(|g| *g).unwrap_or(false);
    let rules = MaskRules::build(&state.mask_apps, &state.mask_patterns, screenshare);
    let visible = state.mask_visible_chars.max(0) as usize;
    ui.set_screenshare(screenshare);
    // "Is anything narrowing the list?" — drives the empty state's wording and its
    // "Clear filters" escape hatch. The type chip and tag count too: filtering to
    // Image with no images used to say "No clipboard history yet".
    let active_app_id: Option<i64>;
    if let Ok(u) = state.ui.lock() {
        use magpie_app::viewmodel::{TimeFilter, TypeFilter};
        let time_on = !matches!(u.time_filter, TimeFilter::All);
        let filtered = u.app_filter.is_some()
            || u.pinned_only
            || u.tag.is_some()
            || !matches!(u.type_filter, TypeFilter::All)
            || time_on;
        ui.set_filtered(filtered);
        // The Filter button badge counts the popover's own controls (screen 02).
        let match_on = !matches!(u.mode, magpie_core::SearchMode::Word);
        let count =
            u.app_filter.is_some() as i32 + time_on as i32 + u.pinned_only as i32 + match_on as i32;
        ui.set_filter_count(count);
        ui.set_active_time_label(SharedString::from(match u.time_filter {
            TimeFilter::Today => "Today",
            TimeFilter::Last7Days => "Last 7 days",
            TimeFilter::Last30Days => "Last 30 days",
            TimeFilter::All => "",
        }));
        // Keep the popover controls in sync with the actual filter state.
        ui.set_time_index(match u.time_filter {
            TimeFilter::All => 0,
            TimeFilter::Today => 1,
            TimeFilter::Last7Days => 2,
            TimeFilter::Last30Days => 3,
        });
        ui.set_match_index(match_mode_index(u.mode));
        active_app_id = u.app_filter;
    } else {
        active_app_id = None;
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
    // Active source-app filter chip label (screen 02): resolve the app_id → name.
    let active_app_name = active_app_id
        .and_then(|id| {
            state
                .store
                .lock()
                .ok()
                .and_then(|st| st.apps_in_use().ok())
                .and_then(|v| v.into_iter().find(|(i, _)| *i == id).map(|(_, n)| n))
        })
        .unwrap_or_default();
    ui.set_active_app_name(SharedString::from(active_app_name));
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

    // Which visible entries are promoted secrets (for the lock badge).
    let secret_ids: std::collections::HashSet<i64> = {
        let store = state.store.lock().unwrap_or_else(|e| e.into_inner());
        store
            .secret_entry_ids()
            .unwrap_or_default()
            .into_iter()
            .collect()
    };

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
        &secret_ids,
    ))));

    // Keep the selection where it was across refresh/reopen; only reset when it
    // would point past the end.
    let cur = ui.get_selected();
    if cur < 0 || cur as usize >= results.len() {
        ui.set_selected(0);
    }
    // Rebuild the virtualized preview for the (possibly new) selection.
    ui.invoke_rebuild_preview();

    // Inline answer card (dates / math / units) for the current query.
    let query = state.ui.lock().map(|u| u.text.clone()).unwrap_or_default();
    set_answer_card(ui, state, &query);
    // Full date answer + calendar (screen 15) when the query resolves to a date.
    populate_date_card(ui, state, 0);
    set_quicklink_hint(ui, state, &query);
    refresh_favorites(ui, state);
}

/// If `query` is "<keyword> …" and `<keyword>` matches a quicklink favorite, light
/// up the quicklink hint chip (↵ opens the target). Requires a space so a plain
/// search isn't hijacked.
fn set_quicklink_hint(ui: &LauncherWindow, state: &AppState, query: &str) {
    let found = query.split_once(' ').and_then(|(kw, rest)| {
        state
            .store
            .lock()
            .ok()
            .and_then(|st| st.favorite_by_keyword(kw).ok().flatten())
            .map(|f| (f, rest.to_string()))
    });
    match found {
        Some((f, rest)) => {
            let target = magpie_core::favorites::expand_quicklink(&f.payload, &rest);
            ui.set_has_quicklink(true);
            ui.set_quicklink_hint(SharedString::from(format!("{} → {target}", f.label)));
        }
        None => {
            ui.set_has_quicklink(false);
            ui.set_quicklink_hint(SharedString::from(""));
        }
    }
}

/// Push the Favorites list (Phase 10) into the sidebar model.
fn refresh_favorites(ui: &LauncherWindow, state: &AppState) {
    let rows: Vec<FavoriteRow> = {
        let st = state.store.lock().unwrap_or_else(|e| e.into_inner());
        st.list_favorites().unwrap_or_default()
    }
    .into_iter()
    .map(|f| FavoriteRow {
        id: f.id as i32,
        kind: SharedString::from(f.kind),
        label: SharedString::from(f.label),
        keyword: SharedString::from(f.keyword),
    })
    .collect();
    ui.set_favorites(ModelRc::new(VecModel::from(rows)));
}

/// Map an eval answer kind to the short label shown in the answer card header.
fn answer_kind_label(kind: &magpie_core::eval::AnswerKind) -> &'static str {
    use magpie_core::eval::AnswerKind::*;
    match kind {
        Date => "Date",
        DateTime => "Date & time",
        DateRange => "Date range",
        Duration => "Duration",
        Number => "Number",
        Rational => "Fraction",
        Percent => "Percent",
        Unit => "Unit",
        Error => "Error",
    }
}

/// Evaluate `query` and populate the inline answer-card properties, or hide the
/// card when the query is not answer-like (or evaluates to an error). The selected
/// format index is only clamped here — it is reset to 0 by `on_search_changed` when
/// the query text actually changes, so Tab cycling survives background refreshes.
fn set_answer_card(ui: &LauncherWindow, state: &AppState, query: &str) {
    use magpie_core::eval;
    let answer = if eval::is_answer_like(query) {
        eval::evaluate(
            query,
            eval::ReferenceNow {
                epoch_secs: now_ms() / 1000,
                utc_offset_secs: local_offset_seconds() as i32,
            },
        )
    } else {
        None
    };
    match answer {
        Some(a) if !matches!(a.kind, eval::AnswerKind::Error) => {
            let chips: Vec<SharedString> = a
                .formats
                .iter()
                .map(|c| SharedString::from(c.text.clone()))
                .collect();
            let n = chips.len() as i32;
            ui.set_answer_kind(SharedString::from(answer_kind_label(&a.kind)));
            ui.set_answer_display(SharedString::from(a.display));
            ui.set_answer_explanation(SharedString::from(a.explanation));
            ui.set_answer_formats(ModelRc::new(VecModel::from(chips)));
            if ui.get_answer_format_index() >= n.max(1) {
                ui.set_answer_format_index(0);
            }
            ui.set_answer_random(false);
            ui.set_has_answer(n > 0);
        }
        // Not a date/math answer — is it a random-generation request?
        _ if set_random_answer(ui, query) => {}
        // …or a world-clock query (time in X)?
        _ if set_worldclock_answer(ui, state, query) => {}
        _ => ui.set_has_answer(false),
    }
}

/// If `query` is a world-clock query (`time in tokyo`, `3pm pst to cet`), show the
/// summary line in the inline answer card (↵ pastes it) and feed the query's
/// target cities into the World clock tab. Returns `true` on a match.
fn set_worldclock_answer(ui: &LauncherWindow, state: &AppState, query: &str) -> bool {
    use magpie_core::worldclock;
    let Some(req) = worldclock::parse_worldclock(query) else {
        // Leaving a world-clock query clears its session targets (drop the lock
        // before refreshing — `refresh_worldclock` re-locks the same mutex).
        let had = {
            let mut t = match state.wc_session_targets.lock() {
                Ok(t) => t,
                Err(_) => return false,
            };
            let had = !t.is_empty();
            t.clear();
            had
        };
        if had {
            refresh_worldclock(ui, state);
        }
        return false;
    };
    // Record the query's targets so the World clock tab includes them, and set the
    // strip hour from an explicit base time if given.
    if let Ok(mut t) = state.wc_session_targets.lock() {
        *t = req.targets.clone();
    }
    if let Some((h, _)) = req.base_hm {
        if let Ok(mut c) = state.wc_chosen_hour.lock() {
            *c = h as i32;
        }
    }
    refresh_worldclock(ui, state);

    // Inline answer: the summary line for the queried cities.
    let epoch = now_ms() / 1000;
    let home = req
        .base_zone
        .clone()
        .unwrap_or_else(|| state.world_clock_home.clone());
    let chosen = state
        .wc_chosen_hour
        .lock()
        .map(|h| *h)
        .ok()
        .filter(|h| *h >= 0)
        .map(|h| h as u32)
        .unwrap_or_else(|| worldclock::home_current_hour(&home, epoch));
    let wc = worldclock::compute(&home, &req.targets, epoch, chosen);
    let summary = worldclock::summary_line(&wc);
    // Headline = the first target's time.
    let display = wc
        .rows
        .iter()
        .find(|r| !r.is_home)
        .or_else(|| wc.rows.first())
        .map(|r| format!("{} {}", r.name, r.big_time))
        .unwrap_or_default();
    ui.set_answer_kind(SharedString::from("World clock"));
    ui.set_answer_display(SharedString::from(display));
    ui.set_answer_explanation(SharedString::from(
        "Open Calculator → World clock for the full grid",
    ));
    ui.set_answer_formats(ModelRc::new(VecModel::from(vec![SharedString::from(
        summary,
    )])));
    if ui.get_answer_format_index() >= 1 {
        ui.set_answer_format_index(0);
    }
    ui.set_answer_random(false);
    ui.set_has_answer(true);
    true
}

/// If `query` is a random-generation command (`password 24`, `uuid`, `2d6`,
/// `random 1-100`, `coin`), populate the inline answer card with fresh candidates
/// (drawn from the OS CSPRNG) and return `true`. Tab cycles the candidates (a
/// re-roll) and ↵ pastes the selected one — passwords never touch history unless
/// pasted. Returns `false` when the query is not a generator command.
fn set_random_answer(ui: &LauncherWindow, query: &str) -> bool {
    use magpie_core::random::{self as rnd, RandomRequest};
    let Some(req) = rnd::parse_random_request(query) else {
        return false;
    };
    // How many candidate chips to offer (Tab re-rolls through them).
    let (kind, explanation, candidates): (String, String, Vec<String>) = match req {
        RandomRequest::Password { len, opts } => {
            let mut sets = Vec::new();
            if opts.letters {
                sets.push("letters");
            }
            if opts.digits {
                sets.push("digits");
            }
            if opts.symbols {
                sets.push("symbols");
            }
            let cands = (0..5)
                .map(|_| rnd::password(&magpie_platform::random_bytes(len.max(1)), len, &opts))
                .collect();
            (
                "Random · password".into(),
                format!("{len} chars · {} · Tab to re-roll", sets.join(" + ")),
                cands,
            )
        }
        RandomRequest::Uuid => {
            let cands = (0..5)
                .map(|_| {
                    let mut b = [0u8; 16];
                    b.copy_from_slice(&magpie_platform::random_bytes(16));
                    rnd::uuid_v4(b)
                })
                .collect();
            (
                "Random · UUID v4".into(),
                "RFC 4122 · Tab to re-roll".into(),
                cands,
            )
        }
        RandomRequest::IntRange { lo, hi } => {
            let cands = (0..6)
                .map(|_| rnd::int_in_range(magpie_platform::random_u64(), lo, hi).to_string())
                .collect();
            (
                "Random · number".into(),
                format!("integer in [{lo}, {hi}] · Tab to re-roll"),
                cands,
            )
        }
        RandomRequest::FloatRange { lo, hi } => {
            let cands = (0..6)
                .map(|_| {
                    format!(
                        "{:.4}",
                        rnd::float_in_range(magpie_platform::random_u64(), lo, hi)
                    )
                })
                .collect();
            (
                "Random · number".into(),
                format!("float in [{lo}, {hi}) · Tab to re-roll"),
                cands,
            )
        }
        RandomRequest::Dice { count, sides } => {
            let roll = || -> String {
                let dice: Vec<i64> = (0..count)
                    .map(|_| rnd::int_in_range(magpie_platform::random_u64(), 1, sides))
                    .collect();
                if count == 1 {
                    dice[0].to_string()
                } else {
                    let sum: i64 = dice.iter().sum();
                    let parts: Vec<String> = dice.iter().map(|d| d.to_string()).collect();
                    format!("{sum}  ({})", parts.join(" + "))
                }
            };
            let cands = (0..6).map(|_| roll()).collect();
            (
                format!("Random · {count}d{sides}"),
                "Tab to re-roll".into(),
                cands,
            )
        }
        RandomRequest::Coin => {
            let cands = (0..6)
                .map(|_| {
                    if magpie_platform::random_u64() & 1 == 0 {
                        "Heads".to_string()
                    } else {
                        "Tails".to_string()
                    }
                })
                .collect();
            (
                "Random · coin flip".into(),
                "Tab to flip again".into(),
                cands,
            )
        }
    };
    let display = candidates.first().cloned().unwrap_or_default();
    let chips: Vec<SharedString> = candidates.into_iter().map(SharedString::from).collect();
    let n = chips.len() as i32;
    ui.set_answer_kind(SharedString::from(kind));
    ui.set_answer_display(SharedString::from(display));
    ui.set_answer_explanation(SharedString::from(explanation));
    ui.set_answer_formats(ModelRc::new(VecModel::from(chips)));
    if ui.get_answer_format_index() >= n.max(1) {
        ui.set_answer_format_index(0);
    }
    ui.set_answer_random(true);
    ui.set_has_answer(n > 0);
    n > 0
}

/// Populate the DateStore (screen 15: date tile, countdown headline, region eve
/// switch, month grids + year heatmap) when the current query resolves to a date
/// or range. `eve_selected` retargets the calendar to the eve (Christmas 24 Dec)
/// when the region observes it. Clears `has-date` for non-date queries.
fn populate_date_card(ui: &LauncherWindow, state: &AppState, eve_selected: i32) {
    use magpie_core::{calendar, eval, holidays};
    let ds = ui.global::<DateStore>();
    let query = state.ui.lock().map(|u| u.text.clone()).unwrap_or_default();
    let now_day = (now_ms() / 1000 + local_offset_seconds()).div_euclid(86_400);
    let ans = if eval::is_answer_like(&query) {
        eval::evaluate(
            &query,
            eval::ReferenceNow {
                epoch_secs: now_ms() / 1000,
                utc_offset_secs: local_offset_seconds() as i32,
            },
        )
    } else {
        None
    };
    // Only date/range answers get the calendar.
    let (primary_target, range, is_range, range_label) = match ans.as_ref().map(|a| &a.value) {
        Some(eval::AnswerValue::Date { day_epoch }) => {
            (*day_epoch, (now_day, *day_epoch), false, String::new())
        }
        Some(eval::AnswerValue::DateRange {
            start_day_epoch,
            end_day_epoch,
            label,
        }) => (
            *end_day_epoch,
            (*start_day_epoch, *end_day_epoch),
            true,
            label.clone(),
        ),
        _ => {
            ds.set_has_date(false);
            return;
        }
    };
    let region = holidays::Region::from_id(&state.holiday_region).unwrap_or(holidays::Region::Se);
    // Eve switch is only meaningful for single-date holidays (Christmas).
    let primary_hol = if is_range {
        None
    } else {
        holidays::label_for_day(region, primary_target)
    };
    let eve = primary_hol.as_ref().and_then(|h| h.eve.clone());
    let shown_target = if eve_selected == 1 {
        eve.as_ref().map(|(d, _)| *d).unwrap_or(primary_target)
    } else {
        primary_target
    };

    // Header label.
    let kind_label = if is_range {
        format!("Date range · {range_label}")
    } else if eve_selected == 1 {
        // The eve label is "24 Dec · Julafton" → take the name after "· ".
        let name = eve
            .as_ref()
            .map(|(_, l)| l.rsplit('·').next().unwrap_or(l).trim().to_string())
            .unwrap_or_else(|| "Christmas Eve".into());
        format!("Date · {name}")
    } else if let Some(h) = &primary_hol {
        format!("Date · {}", h.label)
    } else {
        "Date".into()
    };

    // Tile + headline + sub.
    ds.set_kind_label(kind_label.into());
    ds.set_tile_month(calendar::month_abbrev_upper(shown_target).into());
    let (_, _, dom) = calendar::ymd(shown_target);
    ds.set_tile_day(dom.to_string().into());
    ds.set_tile_weekday(calendar::weekday_long(shown_target).into());

    let diff = shown_target - now_day;
    let display = ans.as_ref().map(|a| a.display.clone()).unwrap_or_default();
    let is_countdown_display = display
        .split(' ')
        .next()
        .map(|w| !w.is_empty() && w.chars().all(|c| c.is_ascii_digit()))
        .unwrap_or(false);
    let headline = if is_range {
        range_label.clone()
    } else if is_countdown_display {
        display.clone()
    } else if diff == 0 {
        "Today".into()
    } else if diff > 0 {
        format!("{diff} days")
    } else {
        format!("{} days ago", -diff)
    };
    ds.set_headline(headline.into());

    let subline = if is_range {
        format!(
            "{} \u{2013} {}",
            calendar::full_date(range.0),
            calendar::full_date(range.1)
        )
    } else {
        let weeks = (diff.abs() + 3) / 7;
        format!(
            "{weeks} week{} \u{00b7} {} \u{00b7} week {}",
            if weeks == 1 { "" } else { "s" },
            calendar::full_date(shown_target),
            calendar::iso_week(shown_target)
        )
    };
    ds.set_subline(subline.into());
    ds.set_readline(
        ans.as_ref()
            .map(|a| a.explanation.clone())
            .unwrap_or_default()
            .into(),
    );

    // Eve switch.
    ds.set_eve_has(eve.is_some());
    if let Some((_, eve_label)) = &eve {
        ds.set_primary_label(calendar::short_date(primary_target).into());
        ds.set_eve_label(eve_label.clone().into());
    }
    ds.set_eve_selected(eve_selected);

    // Calendar layout → Slint models.
    let lay = calendar::layout(
        range.0.min(range.1),
        range.0.max(range.1),
        shown_target,
        now_day,
    );
    let months: Vec<CalMonth> = lay
        .months
        .iter()
        .map(|mg| {
            let cells: Vec<CalDay> = mg
                .cells
                .iter()
                .map(|c| match c {
                    Some(d) => CalDay {
                        dom: d.dom.to_string().into(),
                        blank: false,
                        in_range: d.in_range,
                        is_target: d.is_target,
                        is_today: d.is_today,
                        is_weekend: d.is_weekend,
                    },
                    None => CalDay {
                        dom: SharedString::new(),
                        blank: true,
                        in_range: false,
                        is_target: false,
                        is_today: false,
                        is_weekend: false,
                    },
                })
                .collect();
            CalMonth {
                title: format!("{} {}", month_name_long(mg.month), mg.year).into(),
                cells: ModelRc::new(VecModel::from(cells)),
            }
        })
        .collect();
    ds.set_months(ModelRc::new(VecModel::from(months)));

    // Year strip.
    ds.set_year_title(lay.year_strip.year.to_string().into());
    ds.set_year_cols(lay.year_strip.cols as i32);
    let ycells: Vec<CalYearCell> = lay
        .year_strip
        .cells
        .iter()
        .map(|c| match c {
            Some(y) => CalYearCell {
                blank: false,
                in_range: y.in_range,
                is_target: y.is_target,
                is_today: y.is_today,
            },
            None => CalYearCell {
                blank: true,
                in_range: false,
                is_target: false,
                is_today: false,
            },
        })
        .collect();
    ds.set_year_cells(ModelRc::new(VecModel::from(ycells)));
    let labels: Vec<CalMonthLabel> = lay
        .year_strip
        .month_label_cols
        .iter()
        .map(|(m, col)| CalMonthLabel {
            label: month_name_short(*m).into(),
            col: *col as i32,
        })
        .collect();
    ds.set_year_month_labels(ModelRc::new(VecModel::from(labels)));

    ds.set_has_date(true);
}

/// Long month name for the month-grid titles (Rust-side, mirrors core's private one).
fn month_name_long(m: u32) -> &'static str {
    const N: [&str; 12] = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    N[(m.clamp(1, 12) - 1) as usize]
}

/// Short month name for the year-strip labels.
fn month_name_short(m: u32) -> &'static str {
    const N: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    N[(m.clamp(1, 12) - 1) as usize]
}

/// Recompute the World clock (screen 16) from config cities + the current query's
/// session targets, at the chosen strip hour, and push it into `WorldClockStore`.
fn refresh_worldclock(ui: &LauncherWindow, state: &AppState) {
    use magpie_core::worldclock;
    let epoch = now_ms() / 1000;
    let home = state.world_clock_home.clone();
    // Config cities + this query's session targets (deduped by compute()).
    let mut cities: Vec<String> = state
        .world_clock_cities
        .lock()
        .map(|c| c.clone())
        .unwrap_or_default();
    if let Ok(t) = state.wc_session_targets.lock() {
        for id in t.iter() {
            if !cities.contains(id) {
                cities.push(id.clone());
            }
        }
    }
    let chosen = state.wc_chosen_hour.lock().map(|h| *h).unwrap_or(-1);
    let chosen = if chosen < 0 {
        worldclock::home_current_hour(&home, epoch)
    } else {
        chosen as u32
    };
    let wc = worldclock::compute(&home, &cities, epoch, chosen);

    let rows: Vec<WorldClockRow> = wc
        .rows
        .iter()
        .map(|r| {
            let hh = r.rel_minutes / 60;
            let mm = (r.rel_minutes % 60).abs();
            let rel_label = if r.is_home {
                "you".to_string()
            } else if mm == 0 {
                format!("{hh:+}h")
            } else {
                format!("{hh:+}h{mm:02}")
            };
            let off_h = r.offset_minutes / 60;
            let off_m = (r.offset_minutes % 60).abs();
            let offset_label = if off_m == 0 {
                format!("{off_h:+}")
            } else {
                format!("{off_h:+}:{off_m:02}")
            };
            let hours: Vec<i32> = r.hours.iter().map(|h| *h as i32).collect();
            let shades: Vec<i32> = r.shades.iter().map(|s| *s as i32).collect();
            WorldClockRow {
                name: r.name.clone().into(),
                abbrev: r.abbrev.clone().into(),
                offset_label: offset_label.into(),
                rel_label: rel_label.into(),
                big_time: r.big_time.clone().into(),
                date_label: r.date_label.clone().into(),
                is_home: r.is_home,
                hours: ModelRc::new(VecModel::from(hours)),
                shades: ModelRc::new(VecModel::from(shades)),
            }
        })
        .collect();

    let store = ui.global::<WorldClockStore>();
    store.set_chosen_hour(wc.chosen_hour as i32);
    store.set_summary(worldclock::summary_line(&wc).into());
    store.set_rows(ModelRc::new(VecModel::from(rows)));
}

/// A built-in sample dataset (NASA planetary fact sheet) shown the first time the
/// Data view is opened, so the grid is never empty. Matches reference screen 13.
const SAMPLE_DATA_CSV: &str = "name,mass_1e24kg,diameter_km,distance_1e6km,period_days,moons\n\
Mercury,0.330,4879,57.9,88.0,0\n\
Venus,4.87,12104,108.2,224.7,0\n\
Earth,5.97,12756,149.6,365.2,1\n\
Mars,0.642,6792,228.0,687.0,2\n\
Jupiter,1898,142984,778.5,4331,95\n\
Saturn,568,120536,1432.0,10747,146\n\
Uranus,86.8,51118,2867.0,30589,28\n\
Neptune,102,49528,4515.0,59800,16\n";

fn read_as_from_index(i: i32) -> magpie_core::table::ReadAs {
    use magpie_core::table::ReadAs;
    match i {
        1 => ReadAs::Csv,
        2 => ReadAs::Json,
        3 => ReadAs::LaTeX,
        4 => ReadAs::Typst,
        5 => ReadAs::Lines,
        _ => ReadAs::Auto,
    }
}

fn index_from_read_as(r: magpie_core::table::ReadAs) -> i32 {
    use magpie_core::table::ReadAs;
    match r {
        ReadAs::Auto => 0,
        ReadAs::Csv | ReadAs::Tsv => 1,
        ReadAs::Json => 2,
        ReadAs::LaTeX => 3,
        ReadAs::Typst => 4,
        ReadAs::Lines | ReadAs::Markdown => 5,
    }
}

fn column_is_numeric(kind: magpie_core::table::ColumnKind) -> bool {
    use magpie_core::table::ColumnKind;
    matches!(kind, ColumnKind::Integer | ColumnKind::Float)
}

/// Parse `data_input` under the active Read-as and push the grid + recent rail
/// into the `DataStore` global, then recompute the chart. Shared by the Data
/// nav arm, the Read-as tabs, and "Open as table".
fn refresh_data(ui: &LauncherWindow, state: &AppState) {
    use magpie_core::table;
    let input = state
        .data_input
        .lock()
        .map(|g| g.clone())
        .unwrap_or_default();
    let read_as = state
        .data_read_as
        .lock()
        .map(|g| *g)
        .unwrap_or(table::ReadAs::Auto);
    let name = state
        .data_name
        .lock()
        .map(|g| g.clone())
        .unwrap_or_default();
    let ds = ui.global::<DataStore>();
    ds.set_read_as(index_from_read_as(read_as));

    // Recent rail (session-scoped).
    if let Ok(recent) = state.data_recent.lock() {
        let rows: Vec<RecentData> = recent
            .iter()
            .map(|d| RecentData {
                name: SharedString::from(d.name.clone()),
                meta: SharedString::from(d.meta.clone()),
                selected: d.name == name,
            })
            .collect();
        ds.set_recent(ModelRc::new(VecModel::from(rows)));
    }

    match table::parse(&input, read_as) {
        Ok(t) => {
            let cols: Vec<DataColumn> = t
                .columns
                .iter()
                .map(|c| DataColumn {
                    name: SharedString::from(c.name.clone()),
                    kind: SharedString::from(c.kind.as_str()),
                    numeric: column_is_numeric(c.kind),
                    cells: ModelRc::new(VecModel::from(
                        c.cells
                            .iter()
                            .map(|s| SharedString::from(s.clone()))
                            .collect::<Vec<_>>(),
                    )),
                })
                .collect();
            ds.set_columns(ModelRc::new(VecModel::from(cols)));
            ds.set_row_count(t.row_count as i32);
            ds.set_format(SharedString::from(format!("{} · header row", t.format)));
        }
        Err(e) => {
            ds.set_columns(ModelRc::new(VecModel::from(Vec::<DataColumn>::new())));
            ds.set_row_count(0);
            ds.set_format(SharedString::from(format!("error — {e}")));
        }
    }
    compute_chart(ui, state);
}

/// Recompute the chart points (normalized 0..1, origin bottom-left) + linear fit
/// from the current table and `data_chart` selection, and push them into DataStore.
fn compute_chart(ui: &LauncherWindow, state: &AppState) {
    use magpie_core::table;
    let ds = ui.global::<DataStore>();
    let mut chart = state
        .data_chart
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    ds.set_chart_open(chart.open);
    ds.set_chart_type(chart.kind);
    ds.set_chart_log(chart.log);
    if !chart.open {
        ds.set_chart_points(ModelRc::new(VecModel::from(Vec::<ChartPoint>::new())));
        return;
    }
    let input = state
        .data_input
        .lock()
        .map(|g| g.clone())
        .unwrap_or_default();
    let read_as = state
        .data_read_as
        .lock()
        .map(|g| *g)
        .unwrap_or(table::ReadAs::Auto);
    let Ok(t) = table::parse(&input, read_as) else {
        ds.set_chart_points(ModelRc::new(VecModel::from(Vec::<ChartPoint>::new())));
        return;
    };
    let numeric: Vec<usize> = t
        .columns
        .iter()
        .enumerate()
        .filter(|(_, c)| column_is_numeric(c.kind))
        .map(|(i, _)| i)
        .collect();
    if numeric.len() < 2 {
        ds.set_chart_points(ModelRc::new(VecModel::from(Vec::<ChartPoint>::new())));
        return;
    }
    // Ensure x/y point at valid, distinct numeric columns.
    if !numeric.contains(&chart.x) {
        chart.x = numeric[0];
    }
    if !numeric.contains(&chart.y) || chart.y == chart.x {
        chart.y = *numeric
            .iter()
            .find(|&&i| i != chart.x)
            .unwrap_or(&numeric[1]);
    }
    // A text/name column for point labels: first non-numeric, else none.
    let label_col = t.columns.iter().position(|c| !column_is_numeric(c.kind));
    // Save any clamping back.
    if let Ok(mut g) = state.data_chart.lock() {
        g.x = chart.x;
        g.y = chart.y;
    }

    let xcol = &t.columns[chart.x];
    let ycol = &t.columns[chart.y];
    let n = t.row_count;
    let mut pts: Vec<(f64, f64, String)> = Vec::new();
    for r in 0..n {
        let xs = xcol.cells.get(r).map(|s| s.as_str()).unwrap_or("");
        let ys = ycol.cells.get(r).map(|s| s.as_str()).unwrap_or("");
        let (Ok(mut xv), Ok(mut yv)) = (xs.parse::<f64>(), ys.parse::<f64>()) else {
            continue;
        };
        if chart.log {
            if xv <= 0.0 || yv <= 0.0 {
                continue;
            }
            xv = xv.log10();
            yv = yv.log10();
        }
        let label = label_col
            .and_then(|lc| t.columns[lc].cells.get(r))
            .cloned()
            .unwrap_or_default();
        pts.push((xv, yv, label));
    }
    if pts.is_empty() {
        ds.set_chart_points(ModelRc::new(VecModel::from(Vec::<ChartPoint>::new())));
        return;
    }
    let (xmin, xmax) = pts
        .iter()
        .fold((f64::MAX, f64::MIN), |(a, b), p| (a.min(p.0), b.max(p.0)));
    let (ymin, ymax) = pts
        .iter()
        .fold((f64::MAX, f64::MIN), |(a, b), p| (a.min(p.1), b.max(p.1)));
    let xspan = if xmax > xmin { xmax - xmin } else { 1.0 };
    let yspan = if ymax > ymin { ymax - ymin } else { 1.0 };
    // Linear fit (least squares) on the (possibly log-transformed) data.
    let m = pts.len() as f64;
    let sx: f64 = pts.iter().map(|p| p.0).sum();
    let sy: f64 = pts.iter().map(|p| p.1).sum();
    let sxx: f64 = pts.iter().map(|p| p.0 * p.0).sum();
    let sxy: f64 = pts.iter().map(|p| p.0 * p.1).sum();
    let denom = m * sxx - sx * sx;
    let (slope, fit_txt) = if denom.abs() > f64::EPSILON {
        let slope = (m * sxy - sx * sy) / denom;
        let intercept = (sy - slope * sx) / m;
        // r²
        let ybar = sy / m;
        let ss_tot: f64 = pts.iter().map(|p| (p.1 - ybar).powi(2)).sum();
        let ss_res: f64 = pts
            .iter()
            .map(|p| (p.1 - (slope * p.0 + intercept)).powi(2))
            .sum();
        let r2 = if ss_tot > 0.0 {
            1.0 - ss_res / ss_tot
        } else {
            1.0
        };
        (Some(slope), format!("Fit: slope {slope:.2} · r² {r2:.2}"))
    } else {
        (None, String::new())
    };
    let _ = slope;
    let chart_pts: Vec<ChartPoint> = pts
        .iter()
        .map(|p| ChartPoint {
            x: ((p.0 - xmin) / xspan) as f32,
            y: ((p.1 - ymin) / yspan) as f32,
            label: SharedString::from(p.2.clone()),
        })
        .collect();
    ds.set_chart_points(ModelRc::new(VecModel::from(chart_pts)));
    ds.set_chart_x_name(SharedString::from(xcol.name.clone()));
    ds.set_chart_y_name(SharedString::from(ycol.name.clone()));
    ds.set_chart_label_name(SharedString::from(
        label_col
            .map(|lc| t.columns[lc].name.clone())
            .unwrap_or_default(),
    ));
    ds.set_chart_fit(SharedString::from(fit_txt));
}

/// Open a string as the current dataset (from a clipboard entry or a recent
/// rail row), reset the Read-as to Auto, and refresh the Data view.
fn open_data(ui: &LauncherWindow, state: &AppState, name: &str, input: &str) {
    use magpie_core::table;
    if let Ok(mut g) = state.data_input.lock() {
        *g = input.to_string();
    }
    if let Ok(mut g) = state.data_read_as.lock() {
        *g = table::ReadAs::Auto;
    }
    if let Ok(mut g) = state.data_name.lock() {
        *g = name.to_string();
    }
    // Record/refresh the recent rail (most-recent first, de-duplicated by name).
    if let Ok(t) = table::parse(input, table::ReadAs::Auto) {
        let meta = format!("{} · {} × {}", t.format, t.row_count, t.column_count());
        if let Ok(mut recent) = state.data_recent.lock() {
            recent.retain(|d| d.name != name);
            recent.insert(
                0,
                DataSource {
                    name: name.to_string(),
                    meta,
                    input: input.to_string(),
                    read_as: table::ReadAs::Auto,
                },
            );
            recent.truncate(8);
        }
    }
    refresh_data(ui, state);
}

/// Advance the chart's X (or Y) selection to the next numeric column, wrapping.
fn cycle_chart_col(state: &AppState, is_x: bool) {
    use magpie_core::table;
    let input = state
        .data_input
        .lock()
        .map(|g| g.clone())
        .unwrap_or_default();
    let read_as = state
        .data_read_as
        .lock()
        .map(|g| *g)
        .unwrap_or(table::ReadAs::Auto);
    let Ok(t) = table::parse(&input, read_as) else {
        return;
    };
    let numeric: Vec<usize> = t
        .columns
        .iter()
        .enumerate()
        .filter(|(_, c)| column_is_numeric(c.kind))
        .map(|(i, _)| i)
        .collect();
    if numeric.is_empty() {
        return;
    }
    let mut g = state.data_chart.lock().unwrap_or_else(|e| e.into_inner());
    let cur = if is_x { g.x } else { g.y };
    let pos = numeric.iter().position(|&i| i == cur).unwrap_or(0);
    let next = numeric[(pos + 1) % numeric.len()];
    if is_x {
        g.x = next;
    } else {
        g.y = next;
    }
}

/// Seed the Data view with the built-in sample the first time it is opened
/// (when nothing has been loaded yet this session).
fn ensure_data_seeded(ui: &LauncherWindow, state: &AppState) {
    let empty = state
        .data_input
        .lock()
        .map(|g| g.trim().is_empty())
        .unwrap_or(true);
    if empty {
        open_data(ui, state, "planets.csv", SAMPLE_DATA_CSV);
    } else {
        refresh_data(ui, state);
    }
}

/// Local wall-clock `HH:MM` for a timestamp, for the calculator history rows.
fn hhmm(ms: i64, offset_secs: i64) -> String {
    let local = (ms + offset_secs * 1000).div_euclid(1000);
    let sod = local.rem_euclid(86_400);
    format!("{:02}:{:02}", sod / 3600, (sod % 3600) / 60)
}

/// Split a graph equation into its two curves: `sin(x) = x/3` → (`sin(x)`, `x/3`).
/// A single function (`x^3 - 4x`) returns an empty second side. Each side is left
/// intact — `Expr::parse` strips any real `f(x) =` / `y =` naming prefix itself.
fn split_fg(query: &str) -> (String, String) {
    if let Some(eq) = query.find('=') {
        let lhs = query[..eq].trim();
        let rhs = query[eq + 1..].trim();
        if !lhs.is_empty() && !rhs.is_empty() && lhs != rhs {
            return (lhs.to_string(), rhs.to_string());
        }
        return (
            if lhs.is_empty() { rhs } else { lhs }.to_string(),
            String::new(),
        );
    }
    (query.trim().to_string(), String::new())
}

/// Repopulate the calculator history list + counts + saved-graphs rail from the
/// store, honoring the active tab filter.
fn refresh_calc(ui: &LauncherWindow, state: &AppState) {
    let cs = ui.global::<CalcStore>();
    let kind_filter = match cs.get_tab() {
        1 => Some("math"),
        2 => Some("date"),
        3 => Some("graph"),
        _ => None,
    };
    let offset = local_offset_seconds();
    let now = now_ms();
    let to_row = |e: &magpie_core::calc_history::CalcEntry| CalcRow {
        id: e.id as i32,
        kind: SharedString::from(e.kind.clone()),
        input: SharedString::from(e.input.clone()),
        result: SharedString::from(e.result.clone()),
        time: SharedString::from(hhmm(e.created_at_ms, offset)),
        group: SharedString::from(grouping::section_label(e.pinned, e.created_at_ms, now)),
        pinned: e.pinned,
    };
    let (rows, counts, graphs) = {
        let st = state.store.lock().unwrap_or_else(|e| e.into_inner());
        (
            st.list_calc_history(kind_filter, 500).unwrap_or_default(),
            st.calc_history_counts().unwrap_or((0, 0, 0, 0)),
            st.list_calc_history(Some("graph"), 100).unwrap_or_default(),
        )
    };
    let history: Vec<CalcRow> = rows.iter().map(to_row).collect();
    let n = history.len() as i32;
    cs.set_history(ModelRc::new(VecModel::from(history)));
    cs.set_count_all(counts.0 as i32);
    cs.set_count_math(counts.1 as i32);
    cs.set_count_date(counts.2 as i32);
    cs.set_count_graph(counts.3 as i32);
    cs.set_saved_graphs(ModelRc::new(VecModel::from(
        graphs.iter().map(to_row).collect::<Vec<_>>(),
    )));
    if cs.get_selected() >= n {
        cs.set_selected(0);
    }
}

/// Sample the current graph functions over a fixed window, normalize to 0..1, and
/// push the polylines + intersection markers + stats into the CalcStore global.
fn compute_graph(ui: &LauncherWindow, state: &AppState) {
    use magpie_core::plot::{area_between, intersections, sample, Expr};
    let cs = ui.global::<CalcStore>();
    let f_src = state
        .calc_graph_f
        .lock()
        .map(|g| g.clone())
        .unwrap_or_default();
    let g_src = state
        .calc_graph_g
        .lock()
        .map(|g| g.clone())
        .unwrap_or_default();
    cs.set_graph_f(SharedString::from(if f_src.is_empty() {
        String::new()
    } else {
        format!("f(x) = {f_src}")
    }));
    cs.set_graph_g(SharedString::from(if g_src.is_empty() {
        String::new()
    } else {
        format!("g(x) = {g_src}")
    }));

    let (x0, x1, n) = (-5.0_f64, 5.0_f64, 240usize);
    let f = Expr::parse(&f_src);
    let g = if g_src.is_empty() {
        None
    } else {
        Expr::parse(&g_src)
    };

    // Gather finite y across both curves to fix the shared y-range.
    let f_samples = f.as_ref().map(|e| sample(e, x0, x1, n)).unwrap_or_default();
    let g_samples = g.as_ref().map(|e| sample(e, x0, x1, n)).unwrap_or_default();
    let mut ys: Vec<f64> = Vec::new();
    for s in f_samples.iter().chain(g_samples.iter()) {
        if let (_, Some(y)) = s {
            ys.push(*y);
        }
    }
    if ys.is_empty() {
        cs.set_graph_f_cmd(SharedString::from(""));
        cs.set_graph_g_cmd(SharedString::from(""));
        cs.set_graph_intersections(ModelRc::new(VecModel::from(Vec::<ChartPoint>::new())));
        cs.set_graph_stats_intersections(SharedString::from("—"));
        cs.set_graph_stats_area(SharedString::from("—"));
        cs.set_graph_stats_max(SharedString::from("—"));
        return;
    }
    let ymin = ys.iter().cloned().fold(f64::MAX, f64::min);
    let ymax = ys.iter().cloned().fold(f64::MIN, f64::max);
    let yspan = if ymax > ymin { ymax - ymin } else { 1.0 };
    // Build an SVG polyline command string in 0..1 space (y inverted so higher
    // values sit toward the top). A `None` sample lifts the pen (new `M`).
    let to_cmd = |samples: &[(f64, Option<f64>)]| -> String {
        let mut s = String::new();
        let mut pen_down = false;
        for (x, y) in samples {
            match y {
                Some(y) => {
                    let px = (x - x0) / (x1 - x0);
                    let py = 1.0 - (y - ymin) / yspan;
                    if pen_down {
                        s.push_str(&format!("L {px:.4} {py:.4} "));
                    } else {
                        s.push_str(&format!("M {px:.4} {py:.4} "));
                        pen_down = true;
                    }
                }
                None => pen_down = false,
            }
        }
        s
    };
    cs.set_graph_f_cmd(SharedString::from(to_cmd(&f_samples)));
    cs.set_graph_g_cmd(SharedString::from(to_cmd(&g_samples)));

    // Intersections + stats (only meaningful with two curves).
    if let (Some(f), Some(g)) = (f.as_ref(), g.as_ref()) {
        let roots = intersections(f, g, x0, x1, 400);
        let markers: Vec<ChartPoint> = roots
            .iter()
            .map(|(x, y)| ChartPoint {
                x: ((x - x0) / (x1 - x0)) as f32,
                y: ((y - ymin) / yspan) as f32,
                label: SharedString::from(""),
            })
            .collect();
        cs.set_graph_intersections(ModelRc::new(VecModel::from(markers)));
        let roots_txt = if roots.is_empty() {
            "none in view".to_string()
        } else {
            roots
                .iter()
                .take(4)
                .map(|(x, y)| format!("({x:.3}, {y:.3})"))
                .collect::<Vec<_>>()
                .join(", ")
        };
        cs.set_graph_stats_intersections(SharedString::from(roots_txt));
        let area = area_between(f, g, x0, x1, 400);
        cs.set_graph_stats_area(SharedString::from(format!("{area:.4}  on [−5, 5]")));
    } else {
        cs.set_graph_intersections(ModelRc::new(VecModel::from(Vec::<ChartPoint>::new())));
        cs.set_graph_stats_intersections(SharedString::from("—"));
        cs.set_graph_stats_area(SharedString::from("—"));
    }
    // Max f in view.
    let fmax = f_samples
        .iter()
        .filter_map(|(_, y)| *y)
        .fold(f64::MIN, f64::max);
    cs.set_graph_stats_max(SharedString::from(if fmax > f64::MIN {
        format!("{fmax:.3}")
    } else {
        "—".to_string()
    }));
}

/// Best-effort LaTeX → Typst conversion for the "Copy Typst" button (subset:
/// `\frac{a}{b}` → `frac(a, b)`, `\sum`/`\prod`/`\int` → their Typst names,
/// `^{…}`/`_{…}` → `^(…)`/`_(…)`, and remaining `\cmd` stripped to `cmd`).
fn latex_to_typst(src: &str) -> String {
    let mut s = src.to_string();
    for (a, b) in [
        ("\\frac", "frac"),
        ("\\sum", "sum"),
        ("\\prod", "product"),
        ("\\int", "integral"),
        ("\\cdot", " dot "),
        ("\\times", " times "),
        ("\\sqrt", "sqrt"),
        ("\\", ""),
    ] {
        s = s.replace(a, b);
    }
    s.replace("}{", ", ").replace('{', "(").replace('}', ")")
}

/// Render the current calculator query as typeset math (screen 11) into the
/// CalcStore image, honoring the Plain/LaTeX/Typst toggle. `syntax_mode` 0 = Plain
/// (hide the typeset image and show the plain result), 1 = LaTeX, 2 = Typst.
fn update_calc_math(ui: &LauncherWindow, state: &AppState) {
    use magpie_core::mathsvg::{render, Syntax};
    let _ = state;
    let cs = ui.global::<CalcStore>();
    let query = ui.get_query().to_string();
    let mode = cs.get_syntax_mode();
    let syntax = match mode {
        1 => Syntax::Latex,
        2 => Syntax::Typst,
        _ => Syntax::Plain,
    };
    if mode == 0 || query.trim().is_empty() {
        cs.set_math_has(false);
        return;
    }
    let dark = magpie_app::themes::is_dark(ui.get_theme_index() as usize);
    let color = if dark { "#ededf0" } else { "#1a1a1a" };
    match render(&query, syntax, color)
        .and_then(|svg| slint::Image::load_from_svg_data(svg.as_bytes()).ok())
    {
        Some(img) => {
            cs.set_math_svg(img);
            cs.set_math_has(true);
        }
        None => cs.set_math_has(false),
    }
}

/// Save the current search-bar expression to the calculator history. A plottable
/// expression in `x` becomes a `graph` entry and opens the Graphs view; anything
/// `eval` can resolve becomes a `math`/`date` entry and its result is copied.
fn calc_commit(ui: &LauncherWindow, state: &AppState) {
    use magpie_core::eval;
    let query = ui.get_query().to_string();
    let q = query.trim();
    if q.is_empty() {
        return;
    }
    // Graph? (contains a free `x` and parses as a function).
    if q.contains('x') {
        let (f, g) = split_fg(q);
        if magpie_core::plot::Expr::parse(&f).is_some() {
            if let Ok(mut lock) = state.calc_graph_f.lock() {
                *lock = f;
            }
            if let Ok(mut lock) = state.calc_graph_g.lock() {
                *lock = g;
            }
            if let Ok(st) = state.store.lock() {
                let _ = st.add_calc_entry("graph", q, "plain", "graph", now_ms());
            }
            ui.global::<CalcStore>().set_graphs_open(true);
            compute_graph(ui, state);
            refresh_calc(ui, state);
            return;
        }
    }
    // Otherwise a concrete math/date evaluation.
    if let Some(ans) = eval::evaluate(
        q,
        eval::ReferenceNow {
            epoch_secs: now_ms() / 1000,
            utc_offset_secs: local_offset_seconds() as i32,
        },
    ) {
        if matches!(ans.kind, eval::AnswerKind::Error) {
            return;
        }
        let kind = match ans.kind {
            eval::AnswerKind::Date | eval::AnswerKind::DateTime | eval::AnswerKind::DateRange => {
                "date"
            }
            _ => "math",
        };
        if let Ok(st) = state.store.lock() {
            let _ = st.add_calc_entry(kind, q, "plain", &ans.display, now_ms());
        }
        if let Ok(mut clip) = magpie_platform::platform_clipboard() {
            let _ = clip.set_text(&ans.display);
        }
        refresh_calc(ui, state);
    }
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
/// The placeholder for the shared top search bar on a given section.
fn section_search_placeholder(section: &str) -> &'static str {
    match section {
        "clipboard" => "Search clipboard…   app:vivaldi · is:pinned · after:2026-01-01 · >command",
        "notes" => "Search notes…",
        "tasks" => "Search tasks…",
        "bookmarks" => "Search bookmarks…",
        "snippets" => "Search snippets & commands…",
        "secrets" => "Search secrets by key or value…",
        "files" => "Search files…   in:~/code term",
        "projects" => "Search projects…",
        "calculator" => "Type math, dates, units, or a function to plot…",
        "data" => "Search data…",
        "stats" => "Stats",
        _ => "Search…",
    }
}

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
    // Context-aware top search bar: a per-section placeholder, and a fresh query so one
    // screen's filter doesn't leak into the next.
    ui.set_search_placeholder(SharedString::from(section_search_placeholder(section)));
    ui.set_query(SharedString::from(""));
    // The task tabs share one free-text filter — reset it on every nav so one
    // tab's search doesn't silently filter the next.
    ui.set_task_search(SharedString::from(""));
    if let Ok(mut u) = state.ui.lock() {
        u.text = String::new();
    }
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
        ("tasks", "time") => {
            // The Time tab lives inside the Tasks overlay (gated on tasks-view).
            ui.set_tasks_view(SharedString::from("time"));
            ui.set_tasks_mode(true);
            refresh_tasks(ui, state);
            refresh_time(ui, state);
        }
        ("tasks", "schedule") => {
            // The Schedule tab (cron jobs) lives inside the Tasks overlay.
            ui.set_tasks_view(SharedString::from("schedule"));
            ui.set_tasks_mode(true);
            refresh_tasks(ui, state);
            refresh_schedule(ui, state);
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
        ("data", _) => {
            ui.set_view(SharedString::from("data"));
            ensure_data_seeded(ui, state);
        }
        ("calculator", _) => {
            ui.set_view(SharedString::from("calculator"));
            ui.global::<CalcStore>().set_graphs_open(false);
            refresh_calc(ui, state);
            update_calc_math(ui, state);
        }
        ("snippets", _) => {
            ui.set_view(SharedString::from("snippets"));
            refresh_snippets(ui, state);
        }
        ("secrets", _) => {
            ui.set_view(SharedString::from("secrets"));
            // The shared top bar drives the vault search; start unfiltered.
            ui.global::<SecretsStore>()
                .set_query(SharedString::from(""));
            refresh_secrets(ui, state);
        }
        ("projects", _) => {
            ui.set_view(SharedString::from("projects"));
            refresh_projects(ui, state, true);
        }
        ("files", _) => {
            ui.set_view(SharedString::from("files"));
            // Default the first location to the home dir if none set yet.
            {
                let mut s = state.search.lock().unwrap_or_else(|e| e.into_inner());
                if s.locations.is_empty() {
                    s.locations.push("~".to_string());
                }
            }
            refresh_files(ui, state);
        }
        _ => {
            // clipboard: everything above is already cleared.
        }
    }
}

/// Refresh results and show the launcher window. Shared by the launcher hotkey
/// and the tray (left-click + "Show Magpie").
fn show_window(ui: &LauncherWindow, state: &Arc<AppState>) {
    // Deliberately NOT `ui.set_theme_index(...)` here: `ui` is a
    // single long-lived window (hidden via `NSApp.hide`, never recreated — see
    // `hide_launcher`), so its `theme-index` property already holds whatever the
    // user last picked. Re-pushing the config-load-time snapshot on every
    // summon would silently revert an in-session pick back to the stale value.
    // The snapshot is applied exactly once, right after window creation, in `start()`.
    refresh(ui, state);
    // Backfill site favicons for the visible web-link clips (background).
    spawn_clip_favicons(ui, state);
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
                             // "From projects": read-only rows for each syncable project's .magpie/notes/*.md,
                             // with negative ids that open the file (Phase 18b cross-view). Computed before the
                             // store lock (locks project_scan).
    let (proj_note_rows, proj_note_refs): (
        Vec<NoteRow>,
        std::collections::HashMap<i32, std::path::PathBuf>,
    ) = {
        let g = state.project_scan.lock().unwrap_or_else(|e| e.into_inner());
        let mut rows = Vec::new();
        let mut refs = std::collections::HashMap::new();
        let mut nid: i32 = -2_000_000;
        for h in g.hits.iter().filter(|h| h.syncable()) {
            for (rel, title) in magpie_app::projects::project_note_files(&h.path) {
                rows.push(NoteRow {
                    id: nid,
                    title: format!("⇄ {}/{}", h.name, title).into(),
                    when: SharedString::from("project"),
                    is_daily: false,
                    tags: SharedString::new(),
                });
                refs.insert(nid, h.path.join(&rel));
                nid -= 1;
            }
        }
        (rows, refs)
    };
    if let Ok(mut m) = state.project_note_refs.lock() {
        *m = proj_note_refs;
    }
    let (mut rows, all_tags, open_id, body, prov, links, refs) = {
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
        // Free-text filter from the Notes search box (title / body).
        let query = ui.get_note_filter().to_string().trim().to_lowercase();
        let rows: Vec<NoteRow> = recent
            .iter()
            .zip(tags_by_note.iter())
            .filter(|(_, tags)| filter.is_empty() || tags.iter().any(|t| t == &filter))
            .filter(|(n, _)| {
                query.is_empty()
                    || n.name.to_lowercase().contains(&query)
                    || n.body.to_lowercase().contains(&query)
            })
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
    // Append the read-only "From projects" rows (only when no tag filter is active, so
    // they don't masquerade as tagged notes).
    if ui.get_note_tag_filter().is_empty() {
        rows.extend(proj_note_rows);
    }
    ui.set_notes(ModelRc::new(VecModel::from(rows)));
    ui.set_note_all_tags(ModelRc::new(VecModel::from(
        all_tags
            .into_iter()
            .map(SharedString::from)
            .collect::<Vec<_>>(),
    )));
    ui.set_note_id(open_id);
    ui.set_note_body(SharedString::from(body.clone()));
    // Search-match highlighting for the open note: same as the clipboard preview — when
    // the notes search box has terms, show the body as highlighted lines scrolled to the
    // first match, so you see WHERE the note matches.
    {
        let nf = ui.get_note_filter();
        let hl_terms: Vec<String> = nf
            .as_str()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|t| t.chars().count() >= 2)
            .map(|t| t.to_lowercase())
            .collect();
        let (nhl, nhl_first) = if hl_terms.is_empty() {
            (Vec::new(), -1)
        } else {
            highlight_preview_lines(&body, &hl_terms, 4000)
        };
        ui.set_note_hl_lines(ModelRc::new(VecModel::from(nhl)));
        ui.set_note_hl_first(nhl_first);
    }
    ui.set_note_provenance(SharedString::from(prov));
    refresh_note_render(ui, &body);
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

/// Fetch site favicons (background) for the web-link clips currently in view, then
/// refresh so the rows and preview pick up the cached icons. Favicons come from
/// each site's own `/favicon.ico` — allowed by the "previews only on save" privacy
/// rule, since a favicon is a site icon, not a page preview.
fn spawn_clip_favicons(ui: &LauncherWindow, state: &Arc<AppState>) {
    if !state.fetch_link_favicons {
        return;
    }
    let dir = data_dir().join("favicons");
    let mut seen = std::collections::HashSet::new();
    let domains: Vec<String> = current_results(state, now_ms())
        .iter()
        .filter(|e| matches!(e.kind, Kind::Link))
        .filter_map(|e| favicon::domain_of(&e.full_text))
        .filter(|d| !d.is_empty() && seen.insert(d.clone()))
        .filter(|d| !favicon::favicon_cache_path(&dir, d).exists())
        .take(30)
        .collect();
    if domains.is_empty() {
        return;
    }
    let weak = ui.as_weak();
    let state = state.clone();
    std::thread::spawn(move || {
        let mut any = false;
        for d in &domains {
            if favicon::ensure_favicon(&dir, d, favicon::fetch_favicon).is_some() {
                any = true;
            }
        }
        if any {
            let w = weak.clone();
            let s = state.clone();
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = w.upgrade() {
                    refresh(&ui, &s);
                    ui.invoke_rebuild_preview();
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
/// Build the rich-Markdown line model (screen 18) from a note body and push it
/// into `NoteRender`. Pure rendering via `magpie_core::md_render`; URL favicons are
/// not loaded here (the chip falls back to a link glyph).
fn refresh_note_render(ui: &LauncherWindow, body: &str) {
    use magpie_core::md_render::{self, Block, Span};
    let lines: Vec<MdLineVM> = md_render::render_note(body)
        .into_iter()
        .map(|line| {
            let (block, done, marker) = match &line.block {
                Block::Plain => (0, false, String::new()),
                Block::H1 => (1, false, String::new()),
                Block::H2 => (2, false, String::new()),
                Block::H3 => (3, false, String::new()),
                Block::Bullet => (4, false, String::new()),
                Block::Numbered(m) => (5, false, m.clone()),
                Block::Quote => (6, false, String::new()),
                Block::Task { done } => (7, *done, String::new()),
            };
            let spans: Vec<MdSpan> = line
                .spans
                .into_iter()
                .map(|s| {
                    let mut vm = MdSpan {
                        kind: 0,
                        text: SharedString::new(),
                        href: SharedString::new(),
                        domain: SharedString::new(),
                        prio: 0,
                        icon: slint::Image::default(),
                        has_icon: false,
                    };
                    match s {
                        Span::Text(t) => {
                            vm.kind = 0;
                            vm.text = t.into();
                        }
                        Span::Bold(t) => {
                            vm.kind = 1;
                            vm.text = t.into();
                        }
                        Span::Italic(t) => {
                            vm.kind = 2;
                            vm.text = t.into();
                        }
                        Span::Code(t) => {
                            vm.kind = 3;
                            vm.text = t.into();
                        }
                        Span::Wiki(t) => {
                            vm.kind = 4;
                            vm.text = t.into();
                        }
                        Span::Url {
                            href,
                            label,
                            domain,
                        } => {
                            vm.kind = 5;
                            vm.text = label.into();
                            vm.href = href.into();
                            vm.domain = domain.into();
                        }
                        Span::Prio(p) => {
                            vm.kind = 6;
                            vm.prio = p as i32;
                        }
                        Span::DateChip(d) => {
                            vm.kind = 7;
                            vm.text = d.into();
                        }
                        Span::Tag(t) => {
                            vm.kind = 8;
                            vm.text = t.into();
                        }
                        Span::Recur(r) => {
                            vm.kind = 9;
                            vm.text = r.into();
                        }
                    }
                    vm
                })
                .collect();
            MdLineVM {
                block,
                done,
                marker: marker.into(),
                spans: ModelRc::new(VecModel::from(spans)),
            }
        })
        .collect();
    ui.global::<NoteRender>()
        .set_lines(ModelRc::new(VecModel::from(lines)));
}

/// Expand a snippet body for the preview / paste, resolving `{snippet:name}` via
/// the store and filling `{clipboard}` from the OS clipboard. `{input:…}` prompts
/// aren't collected here — they expand to empty in the preview.
fn expand_snippet(state: &AppState, body: &str) -> magpie_core::snippets::Expanded {
    use magpie_core::snippets::{expand, ExpandCtx};
    let clipboard = magpie_platform::platform_clipboard()
        .ok()
        .map(|mut c| c.snapshot())
        .and_then(|s| match s.content {
            Some(magpie_core::Content::Text(t)) => Some(t),
            _ => None,
        })
        .unwrap_or_default();
    let inputs = std::collections::HashMap::new();
    let ctx = ExpandCtx {
        epoch_secs: now_ms() / 1000,
        utc_offset_secs: local_offset_seconds() as i32,
        clipboard: &clipboard,
        inputs: &inputs,
    };
    let lookup = |name: &str| -> Option<String> {
        let store = state.store.lock().ok()?;
        store
            .snippet_by_keyword(name)
            .ok()
            .flatten()
            .or_else(|| {
                store
                    .list_snippets()
                    .ok()?
                    .into_iter()
                    .find(|s| s.name.eq_ignore_ascii_case(name))
            })
            .map(|s| s.body)
    };
    expand(body, &ctx, &lookup)
}

/// Import snippets from a conventional espanso/navi location (no file picker yet).
/// espanso: `~/.config/espanso/match/base.yml` (or the macOS Application Support
/// path); navi: every `*.cheat` under `~/.local/share/navi/cheats`. Imported rows
/// are added to the store; missing files are a no-op.
fn import_snippets(w: &slint::Weak<LauncherWindow>, state: &Arc<AppState>, espanso: bool) {
    use magpie_core::snippets::{parse_espanso_yaml, parse_navi_cheat, Snippet};
    let home = dirs::home_dir().unwrap_or_default();
    let mut found: Vec<Snippet> = Vec::new();
    if espanso {
        let candidates = [
            home.join(".config/espanso/match/base.yml"),
            home.join("Library/Application Support/espanso/match/base.yml"),
        ];
        for p in candidates {
            if let Ok(text) = std::fs::read_to_string(&p) {
                found.extend(parse_espanso_yaml(&text));
            }
        }
    } else {
        let dir = home.join(".local/share/navi/cheats");
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for e in entries.flatten() {
                if e.path().extension().and_then(|x| x.to_str()) == Some("cheat") {
                    if let Ok(text) = std::fs::read_to_string(e.path()) {
                        found.extend(parse_navi_cheat(&text));
                    }
                }
            }
        }
    }
    if found.is_empty() {
        return;
    }
    {
        let store = match state.store.lock() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };
        for s in &found {
            let _ = store.add_snippet(&s.name, &s.keyword, &s.body, &s.kind, now_ms());
        }
    }
    refresh_expand_keywords(state);
    if let Some(ui) = w.upgrade() {
        refresh_snippets(&ui, state);
    }
}

/// Rebuild the Snippets view (screen 20) from the store into `SnippetsStore`,
/// filtered by the active tab, and refresh the selected snippet's detail + preview.
fn refresh_snippets(ui: &LauncherWindow, state: &AppState) {
    let store_snips = {
        let store = match state.store.lock() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };
        store.list_snippets().unwrap_or_default()
    };
    let ss = ui.global::<SnippetsStore>();
    let count_snippets = store_snips.iter().filter(|s| !s.is_command()).count() as i32;
    let count_commands = store_snips.iter().filter(|s| s.is_command()).count() as i32;
    ss.set_count_all(store_snips.len() as i32);
    ss.set_count_snippets(count_snippets);
    ss.set_count_commands(count_commands);

    let tab = ss.get_tab();
    // Free-text filter from the shared top search bar (name / keyword / body).
    let query = ui.get_query().to_string().trim().to_lowercase();
    let filtered: Vec<&magpie_core::snippets::Snippet> = store_snips
        .iter()
        .filter(|s| match tab {
            1 => !s.is_command(),
            2 => s.is_command(),
            _ => true,
        })
        .filter(|s| {
            query.is_empty()
                || s.name.to_lowercase().contains(&query)
                || s.keyword.to_lowercase().contains(&query)
                || s.body.to_lowercase().contains(&query)
        })
        .collect();

    let rows: Vec<SnippetRow> = filtered
        .iter()
        .map(|s| SnippetRow {
            id: s.id as i32,
            name: s.name.clone().into(),
            keyword: if s.is_command() {
                s.body.chars().take(20).collect::<String>().into()
            } else {
                s.keyword.clone().into()
            },
            is_command: s.is_command(),
        })
        .collect();
    let n = rows.len() as i32;
    ss.set_rows(ModelRc::new(VecModel::from(rows)));

    let mut sel = ss.get_selected();
    if sel >= n {
        sel = 0;
        ss.set_selected(0);
    }
    if let Some(s) = filtered.get(sel.max(0) as usize) {
        ss.set_has_selection(true);
        ss.set_detail_name(s.name.clone().into());
        ss.set_detail_keyword(if s.keyword.is_empty() {
            SharedString::from("—")
        } else {
            s.keyword.clone().into()
        });
        ss.set_detail_body(s.body.clone().into());
        ss.set_detail_is_command(s.is_command());
        let preview = expand_snippet(state, &s.body).text;
        ss.set_detail_preview(preview.into());
    } else {
        ss.set_has_selection(false);
    }
}

/// Live token autocomplete for the clipboard search bar: from the token currently being
/// typed, suggest filter keys, or values for `app:`/`type:`/`cat:`. Cleared on any other
/// section. Chosen via the chip strip (`apply-suggestion`).
fn refresh_search_suggestions(ui: &LauncherWindow, state: &AppState, text: &str) {
    if ui.get_section() != "clipboard" {
        ui.set_search_suggestions(ModelRc::new(VecModel::from(Vec::<SuggestRow>::new())));
        return;
    }
    // The token being edited = text after the last whitespace; `prefix` is what precedes it.
    let split = text.rfind(char::is_whitespace).map(|i| i + 1).unwrap_or(0);
    let prefix = &text[..split];
    let last = &text[split..];
    let low = last.to_lowercase();
    let mut out: Vec<SuggestRow> = Vec::new();
    let mk = |label: String, insert: String, hint: String| SuggestRow {
        label: label.into(),
        insert: insert.into(),
        hint: hint.into(),
    };

    let value_after =
        |key: &str| -> Option<String> { low.strip_prefix(key).map(|s| s.to_string()) };

    if let Some(p) = value_after("app:") {
        let apps = {
            let store = state.store.lock().unwrap_or_else(|e| e.into_inner());
            store.apps_in_use_with_counts().unwrap_or_default()
        };
        for (_, name, count) in apps
            .into_iter()
            .filter(|(_, n, _)| n.to_lowercase().contains(&p))
        {
            out.push(mk(
                format!("app:{name}"),
                format!("{prefix}app:{} ", name.to_lowercase()),
                format!("{count}"),
            ));
            if out.len() >= 8 {
                break;
            }
        }
    } else if let Some(p) = value_after("type:").or_else(|| value_after("kind:")) {
        for k in ["text", "link", "image", "color", "email", "file", "html"] {
            if k.starts_with(&p) {
                out.push(mk(
                    format!("type:{k}"),
                    format!("{prefix}type:{k} "),
                    String::new(),
                ));
            }
        }
    } else if let Some(p) = value_after("cat:").or_else(|| value_after("category:")) {
        for c in ["browser", "editor", "terminal", "chat", "notes", "design"] {
            if c.starts_with(&p) {
                out.push(mk(
                    format!("cat:{c}"),
                    format!("{prefix}cat:{c} "),
                    String::new(),
                ));
            }
        }
    } else {
        // Suggest filter keys/tokens that start with what's being typed.
        let keys: &[(&str, &str)] = &[
            ("app:", "source app"),
            ("url:", "clip URL / source page"),
            ("site:", "source page"),
            ("type:", "content kind"),
            ("is:pinned", "pinned only"),
            ("after:", "date ≥ (YYYY-MM-DD)"),
            ("before:", "date ≤"),
            ("on:", "on a date"),
            ("browser", "source category"),
            ("editor", "source category"),
            ("terminal", "source category"),
            ("cat:", "other categories"),
        ];
        for (k, hint) in keys {
            if !low.is_empty() && k.starts_with(&low) {
                out.push(mk(
                    (*k).to_string(),
                    format!("{prefix}{k}"),
                    (*hint).to_string(),
                ));
            }
            if out.len() >= 8 {
                break;
            }
        }
    }
    ui.set_search_suggestions(ModelRc::new(VecModel::from(out)));
}

/// The DB id of the currently-selected secret row, if any.
fn selected_secret_id(ui: &LauncherWindow) -> Option<i64> {
    use slint::Model;
    let ss = ui.global::<SecretsStore>();
    let sel = ss.get_selected().max(0) as usize;
    ss.get_rows().row_data(sel).map(|r| r.id as i64)
}

fn refresh_secrets(ui: &LauncherWindow, state: &AppState) {
    let ss = ui.global::<SecretsStore>();
    let query = ss.get_query().to_string();
    let secrets = {
        let store = match state.store.lock() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };
        store.search_secrets(&query).unwrap_or_default()
    };
    // The list is always masked; only the selected detail can be revealed.
    let rows: Vec<SecretRow> = secrets
        .iter()
        .map(|s| SecretRow {
            id: s.id as i32,
            key: s.key.clone().into(),
            value_masked: SharedString::from(magpie_core::SECRET_MASK),
            notes: s.notes.clone().into(),
            revealed: false,
        })
        .collect();
    let n = rows.len() as i32;
    ss.set_rows(ModelRc::new(VecModel::from(rows)));

    let mut sel = ss.get_selected();
    if sel >= n {
        sel = 0;
        ss.set_selected(0);
    }
    let revealed = ss.get_detail_revealed();
    if let Some(s) = secrets.get(sel.max(0) as usize) {
        ss.set_has_selection(true);
        ss.set_detail_key(s.key.clone().into());
        ss.set_detail_value(if revealed {
            s.value.clone().into()
        } else {
            SharedString::from(magpie_core::SECRET_MASK)
        });
        ss.set_detail_notes(s.notes.clone().into());
    } else {
        ss.set_has_selection(false);
        ss.set_detail_revealed(false);
    }
}

/// The task title on the first line of a conflict slice, for the banner summary:
/// trims and strips a leading checkbox marker. Empty slice → "(removed)".
fn conflict_line_label(slice: &str) -> String {
    let first = slice.lines().next().unwrap_or("").trim();
    if first.is_empty() {
        return "(removed)".to_string();
    }
    for m in ["- [ ] ", "- [x] ", "- [X] ", "- [/] "] {
        if let Some(rest) = first.strip_prefix(m) {
            return rest.to_string();
        }
    }
    first.to_string()
}

/// Toggle a project task line through the 3-way merge (Phase 18b). The merge pivot is
/// the scan snapshot (the exact bytes the UI showed, so its line index aligns), and it
/// reconciles against the current disk bytes so a concurrent external edit isn't
/// clobbered. Writes the merged result, advances the stored baseline, and — under Ask
/// — records an unresolved conflict for Review. Returns true if a conflict was held.
fn toggle_project_task(
    state: &AppState,
    root: &std::path::Path,
    rel: &str,
    file_abs: &std::path::Path,
    line: usize,
) -> bool {
    let disk = match std::fs::read_to_string(file_abs) {
        Ok(d) => d,
        Err(_) => return false,
    };
    let baseline = {
        let g = state.project_scan.lock().unwrap_or_else(|e| e.into_inner());
        g.file_snapshots.get(file_abs).cloned()
    }
    .unwrap_or_else(|| disk.clone());
    let fmt = magpie_core::projects::TaskFormat::for_file(rel);
    let mine = match magpie_core::projects::toggle_task_line_fmt(&baseline, line, fmt) {
        Some(m) => m,
        None => return false, // not a task line in this format
    };
    let policy = *state
        .project_conflict_policy
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let r = magpie_core::projects::reconcile_file(&baseline, &mine, &disk, policy);
    if let Some(w) = &r.write {
        if std::fs::write(file_abs, w).is_err() {
            return false;
        }
    }
    if let Some(nb) = &r.new_baseline {
        let root_s = root.to_string_lossy().to_string();
        let store = state.store.lock().unwrap_or_else(|e| e.into_inner());
        let _ = store.set_project_baseline(&root_s, rel, nb, now_ms());
    }
    // Update the Review queue for this file: clear any prior conflict, then record the
    // new one (Ask only — FileWins/clean return no conflicts).
    let mut cs = state
        .project_conflicts
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    cs.retain(|c| c.file_abs != file_abs);
    if let Some(c0) = r.conflicts.first() {
        cs.push(magpie_app::app_state::ProjConflict {
            root: root.to_path_buf(),
            rel: rel.to_string(),
            file_abs: file_abs.to_path_buf(),
            line: c0.line,
            base: baseline,
            mine,
            disk,
            summary: format!(
                "Line {} changed in Magpie and on disk — “{}” vs “{}”",
                c0.line,
                conflict_line_label(&c0.mine),
                conflict_line_label(&c0.theirs),
            ),
        });
        return true;
    }
    false
}

/// Route a global Tasks/board toggle on a project file-task (negative synthetic
/// `note_id`) to its file via `toggle_project_task`. Returns true if a conflict held.
fn toggle_project_task_by_id(state: &AppState, note_id: i32, line: usize) -> bool {
    let r = state
        .project_task_refs
        .lock()
        .ok()
        .and_then(|m| m.get(&note_id).cloned());
    match r {
        Some(r) => toggle_project_task(state, &r.root, &r.rel, &r.file_abs, line),
        None => false,
    }
}

/// Current done-state of a project file-task's line (by negative synthetic id), read
/// fresh from disk. `None` if unknown (not a project task / line gone).
fn project_task_is_done(state: &AppState, note_id: i32, line: usize) -> Option<bool> {
    let r = state
        .project_task_refs
        .lock()
        .ok()
        .and_then(|m| m.get(&note_id).cloned())?;
    let content = std::fs::read_to_string(&r.file_abs).ok()?;
    let fmt = magpie_core::projects::TaskFormat::for_file(&r.rel);
    magpie_core::projects::parse_task_lines(&content, fmt)
        .into_iter()
        .find(|p| p.line == line)
        .map(|p| p.done)
}

/// Open the file behind a project file-task (negative synthetic `note_id`) in the
/// editor. Returns true if it was a project task (handled).
fn open_project_task_by_id(state: &AppState, note_id: i32) -> bool {
    let r = state
        .project_task_refs
        .lock()
        .ok()
        .and_then(|m| m.get(&note_id).cloned());
    if let Some(r) = r {
        let _ = std::process::Command::new("open")
            .arg("-t")
            .arg(&r.file_abs)
            .spawn();
        true
    } else {
        false
    }
}

/// Resolve the selected project's first pending conflict by writing the chosen side
/// ("keep mine" or "keep disk"), advancing the baseline, and clearing it from the
/// Review queue. Returns true if a conflict was resolved.
fn resolve_project_conflict(state: &AppState, prefer_mine: bool) -> bool {
    let root_path = {
        let g = state.project_scan.lock().unwrap_or_else(|e| e.into_inner());
        let syncable: Vec<_> = g.hits.iter().filter(|h| h.syncable()).collect();
        syncable
            .get(g.selected.max(0) as usize)
            .map(|h| h.path.clone())
    };
    let Some(root_path) = root_path else {
        return false;
    };
    let conflict = {
        let cs = state
            .project_conflicts
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        cs.iter().find(|c| c.root == root_path).cloned()
    };
    let Some(c) = conflict else { return false };
    let resolved = magpie_core::merge3::resolve(&c.base, &c.mine, &c.disk, prefer_mine);
    if std::fs::write(&c.file_abs, &resolved).is_err() {
        return false;
    }
    {
        let root_s = c.root.to_string_lossy().to_string();
        let store = state.store.lock().unwrap_or_else(|e| e.into_inner());
        let _ = store.set_project_baseline(&root_s, &c.rel, &resolved, now_ms());
    }
    let mut cs = state
        .project_conflicts
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    cs.retain(|x| x.file_abs != c.file_abs);
    true
}

/// Rescan (optionally) and populate the Projects view (screens 25/26).
fn refresh_projects(ui: &LauncherWindow, state: &Arc<AppState>, rescan: bool) {
    let ps = ui.global::<ProjectsStore>();
    ps.set_scanning(format!("Scanning {}", state.projects_roots.join(", ")).into());
    if rescan {
        let st = state.clone();
        let w = ui.as_weak();
        let roots = state.projects_roots.clone();
        let ignored = state
            .project_scan
            .lock()
            .map(|g| g.ignored.clone())
            .unwrap_or_default();
        let keep_out = state
            .projects_keep_magpie_out_of_git
            .load(std::sync::atomic::Ordering::Relaxed);
        std::thread::spawn(move || {
            // Include repos so editor/repo-only folders land in the Found inbox.
            let (hits, snapshots) = magpie_app::projects::scan(&roots, 4, true, &ignored);
            // Optionally keep each project's `.magpie/` out of git (private exclude).
            if keep_out {
                for h in hits.iter().filter(|h| h.syncable()) {
                    let _ = magpie_app::projects::ensure_git_excluded(&h.path);
                }
            }
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = w.upgrade() {
                    {
                        let mut g = st.project_scan.lock().unwrap_or_else(|e| e.into_inner());
                        g.hits = hits;
                        g.file_snapshots = snapshots;
                    }
                    populate_projects(&ui, &st);
                }
            });
        });
    } else {
        populate_projects(ui, state);
    }
}

fn marker_badges(markers: &[magpie_core::projects::Marker]) -> String {
    markers
        .iter()
        .map(|m| m.badge())
        .collect::<Vec<_>>()
        .join(" · ")
}

/// Build the Projects list + selected-project detail from the cached scan.
fn populate_projects(ui: &LauncherWindow, state: &Arc<AppState>) {
    use magpie_core::projects::Marker;
    let ps = ui.global::<ProjectsStore>();
    let hits = {
        let g = state.project_scan.lock().unwrap_or_else(|e| e.into_inner());
        g.hits.clone()
    };
    // Free-text filter from the shared top search bar (project name / path).
    let query = ui.get_query().to_string().trim().to_lowercase();
    let hits: Vec<_> = hits
        .into_iter()
        .filter(|h| {
            query.is_empty()
                || h.name.to_lowercase().contains(&query)
                || h.path.to_string_lossy().to_lowercase().contains(&query)
        })
        .collect();
    let (syncable, found): (Vec<_>, Vec<_>) = hits.iter().partition(|h| h.syncable());

    // Pending conflicts (Phase 18b): mark their projects in the list + show the banner.
    let conflicts = state
        .project_conflicts
        .lock()
        .map(|c| c.clone())
        .unwrap_or_default();
    let conflict_roots: std::collections::HashSet<std::path::PathBuf> =
        conflicts.iter().map(|c| c.root.clone()).collect();

    let list_row = |h: &magpie_app::projects::ProjectHit, is_found: bool| ProjectListRow {
        name: h.name.clone().into(),
        path: magpie_app::projects::display_path(&h.path).into(),
        badges: marker_badges(&h.markers).into(),
        open_count: format!("{} open", h.open_tasks()).into(),
        conflict: conflict_roots.contains(&h.path),
        found: is_found,
        extra: if is_found {
            format!("· {}", marker_badges(&h.markers))
        } else {
            String::new()
        }
        .into(),
    };
    ps.set_projects(ModelRc::new(VecModel::from(
        syncable
            .iter()
            .map(|h| list_row(h, false))
            .collect::<Vec<_>>(),
    )));
    ps.set_found(ModelRc::new(VecModel::from(
        found.iter().map(|h| list_row(h, true)).collect::<Vec<_>>(),
    )));

    // Detail for the selected syncable project.
    let mut sel = ps.get_selected();
    if sel < 0 || sel as usize >= syncable.len() {
        sel = 0;
        ps.set_selected(0);
    }
    if let Some(h) = syncable.get(sel as usize) {
        ps.set_has_selection(true);
        // Conflict banner for the selected project.
        let sel_conflict = conflicts.iter().find(|c| c.root == h.path);
        ps.set_detail_has_conflict(sel_conflict.is_some());
        ps.set_conflict_summary(
            sel_conflict
                .map(|c| c.summary.clone())
                .unwrap_or_default()
                .into(),
        );
        if sel_conflict.is_none() {
            ps.set_reviewing(false);
        }
        ps.set_detail_name(h.name.clone().into());
        ps.set_detail_path(magpie_app::projects::display_path(&h.path).into());
        ps.set_detail_branch(if h.branch.is_empty() {
            SharedString::new()
        } else {
            format!("branch {}", h.branch).into()
        });
        ps.set_detail_tag(format!("#{}", magpie_core::projects::project_tag(&h.name)).into());
        ps.set_detail_tags(
            h.tags
                .iter()
                .map(|t| format!("#{t}"))
                .collect::<Vec<_>>()
                .join(" ")
                .into(),
        );
        let open = h.open_tasks();
        ps.set_detail_week(format!("{open} open").into());

        // The project's actual tasks now render in the task hub (List/Board,
        // `refresh_project_tasks` below), merged from note #tags + project files.
        // `detail-tasks` keeps only the read-only EXTRAS shown beneath the hub:
        // project notes + code-comment TODO/FIXME.
        let mut rows: Vec<ProjTaskRow> = Vec::new();
        let _ = Marker::badge;

        // Project notes (.magpie/notes/*.md) — read-only listing for now; the full
        // Notes-view "From projects" integration is a follow-up (Phase 18b).
        let note_files = magpie_app::projects::project_note_files(&h.path);
        if !note_files.is_empty() {
            rows.push(ProjTaskRow {
                is_file_header: true,
                file_label: format!("Notes  {}", note_files.len()).into(),
                sync_label: "read-only".into(),
                title: SharedString::new(),
                done: false,
                source: SharedString::new(),
                idx: -1,
                read_only: true,
            });
            for (rel, title) in &note_files {
                rows.push(ProjTaskRow {
                    is_file_header: false,
                    file_label: SharedString::new(),
                    sync_label: SharedString::new(),
                    title: title.clone().into(),
                    done: false,
                    source: format!("{rel}:1").into(),
                    idx: -1,
                    read_only: true,
                });
            }
        }

        // Read-only code-comment lens (Phase 18b, PNG 25): TODO/FIXME/HACK/XXX found in
        // source, cached per selected project so re-populates don't re-walk.
        let code_todos = {
            let need = {
                let g = state.project_scan.lock().unwrap_or_else(|e| e.into_inner());
                g.code_todos.0 != h.path
            };
            if need {
                let found = magpie_app::projects::scan_code_comments(&h.path, 40);
                let mut g = state.project_scan.lock().unwrap_or_else(|e| e.into_inner());
                g.code_todos = (h.path.clone(), found);
            }
            let g = state.project_scan.lock().unwrap_or_else(|e| e.into_inner());
            g.code_todos.1.clone()
        };
        if !code_todos.is_empty() {
            rows.push(ProjTaskRow {
                is_file_header: true,
                file_label: format!("Code comments  {} · TODO / FIXME", code_todos.len()).into(),
                sync_label: "read-only".into(),
                title: SharedString::new(),
                done: false,
                source: SharedString::new(),
                idx: -1,
                read_only: true,
            });
            for c in &code_todos {
                rows.push(ProjTaskRow {
                    is_file_header: false,
                    file_label: SharedString::new(),
                    sync_label: SharedString::new(),
                    title: if c.text.is_empty() {
                        c.kind.clone().into()
                    } else {
                        format!("{}: {}", c.kind, c.text).into()
                    },
                    done: false,
                    source: format!("{}:{}", c.rel, c.line).into(),
                    idx: -1,
                    read_only: true,
                });
            }
        }
        ps.set_detail_tasks(ModelRc::new(VecModel::from(rows)));
        // The per-project task hub (merged note #tag tasks + on-disk file tasks).
        refresh_project_tasks(ui, state);
    } else {
        ps.set_has_selection(false);
    }
    ps.set_scanning(
        format!(
            "{} project{} · {} found · roots {}",
            syncable.len(),
            if syncable.len() == 1 { "" } else { "s" },
            found.len(),
            state.projects_roots.join(", ")
        )
        .into(),
    );
}

/// The lowercased set of `#tag` strings a note task may use to belong to a
/// project `name`/`tags`: the slug, the `proj/<slug>` form, the raw name, and
/// each configured tag (raw + slug).
fn project_accept_tags(name: &str, tags: &[String]) -> std::collections::HashSet<String> {
    use magpie_core::projects::{project_tag, slugify};
    let mut set = std::collections::HashSet::new();
    set.insert(slugify(name));
    set.insert(project_tag(name));
    set.insert(name.to_lowercase());
    for t in tags {
        set.insert(t.to_lowercase());
        set.insert(slugify(t));
    }
    set.retain(|s| !s.is_empty());
    set
}

/// Rebuild the selected project's task hub: merge note tasks tagged with the
/// project + its on-disk file tasks, then render the List rows and Board columns
/// scoped to that project (respecting the hub's Show-done + Board controls).
fn refresh_project_tasks(ui: &LauncherWindow, state: &Arc<AppState>) {
    let ps = ui.global::<ProjectsStore>();
    let now = now_ms();
    let today_ms = magpie_app::format_time::parse_due("today", now).unwrap_or(0);

    // Selected project identity (root path + accepted #tags).
    let (root, accept) = {
        let g = state.project_scan.lock().unwrap_or_else(|e| e.into_inner());
        let syncable: Vec<_> = g.hits.iter().filter(|h| h.syncable()).collect();
        let mut sel = ps.get_selected();
        if sel < 0 || sel as usize >= syncable.len() {
            sel = 0;
        }
        match syncable.get(sel as usize) {
            Some(h) => (h.path.clone(), project_accept_tags(&h.name, &h.tags)),
            None => {
                ps.set_detail_task_rows(ModelRc::new(VecModel::from(Vec::<TaskRow>::new())));
                ps.set_detail_board(ModelRc::new(VecModel::from(Vec::<BoardColumn>::new())));
                return;
            }
        }
    };

    // Merge sources (fills project_task_refs), then scope to this project:
    // file tasks by their ref root, note tasks by their #tag.
    let proj_tasks = project_file_tasks(state, now);
    let refs = state
        .project_task_refs
        .lock()
        .map(|m| m.clone())
        .unwrap_or_default();
    let show_done = ps.get_detail_show_done();
    let store = match state.store.lock() {
        Ok(g) => g,
        Err(e) => e.into_inner(),
    };
    let active = store.active_timer().ok().flatten();
    let mut merged = magpie_app::tasks::all_tasks(&store, now);
    merged.extend(proj_tasks);
    let scoped: Vec<magpie_app::tasks::Task> = merged
        .into_iter()
        .filter(|t| {
            if t.note_id < 0 {
                refs.get(&(t.note_id as i32))
                    .map(|r| r.root == root)
                    .unwrap_or(false)
            } else {
                t.project
                    .as_deref()
                    .map(|p| accept.contains(&p.to_lowercase()))
                    .unwrap_or(false)
            }
        })
        .collect();

    // List rows (respect the hub's Show-done).
    let list_rows: Vec<TaskRow> = magpie_app::tasks::group_sort(scoped.clone())
        .into_iter()
        .flat_map(|g| g.tasks)
        .filter(|t| show_done || !t.done)
        .map(|t| task_row_with_tracking(t, today_ms, &active, &store, now))
        .collect();
    ps.set_detail_task_rows(ModelRc::new(VecModel::from(list_rows)));

    // Board columns (respect the hub's own Board controls).
    let group_by = match ps.get_board_group() {
        1 => magpie_app::tasks::GroupBy::Priority,
        2 => magpie_app::tasks::GroupBy::Project,
        _ => magpie_app::tasks::GroupBy::Status,
    };
    let sort_by = match ps.get_board_sort() {
        1 => magpie_app::tasks::SortBy::Priority,
        2 => magpie_app::tasks::SortBy::Title,
        3 => magpie_app::tasks::SortBy::Status,
        _ => magpie_app::tasks::SortBy::Due,
    };
    let filter = magpie_app::tasks::BoardFilter {
        project: None,
        query: String::new(),
        hide_done: ps.get_board_hide_done(),
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
        tracking: false,
        time_total: SharedString::from(""),
    };
    let columns: Vec<BoardColumn> =
        magpie_app::tasks::board_columns(scoped, group_by, sort_by, &filter)
            .into_iter()
            .map(|c| BoardColumn {
                title: SharedString::from(c.title),
                count: c.tasks.len() as i32,
                tasks: ModelRc::new(VecModel::from(
                    c.tasks.into_iter().map(to_row).collect::<Vec<_>>(),
                )),
            })
            .collect();
    ps.set_detail_board(ModelRc::new(VecModel::from(columns)));
}

/// Toggle a task's done state, routing by `note_id` sign (project file vs note).
fn proj_toggle_task(state: &AppState, note_id: i32, line: usize) {
    if note_id < 0 {
        toggle_project_task_by_id(state, note_id, line);
    } else {
        let store = match state.store.lock() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };
        magpie_app::tasks::toggle_task(&store, note_id as i64, line, now_ms());
    }
}

/// Set a task's status from a board column, routing by `note_id` sign. On a
/// project file a "Doing" drop is ignored (files only carry done⇄open).
fn proj_set_status(state: &AppState, note_id: i32, line: usize, status_col: i32) {
    let target = magpie_app::tasks::status_from_column(status_col);
    if note_id < 0 {
        if !matches!(target, magpie_app::tasks::Status::Doing) {
            let want_done = matches!(target, magpie_app::tasks::Status::Done);
            if project_task_is_done(state, note_id, line) != Some(want_done) {
                toggle_project_task_by_id(state, note_id, line);
            }
        }
    } else {
        let store = match state.store.lock() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };
        magpie_app::tasks::set_task_status(&store, note_id as i64, line, target, now_ms());
    }
}

/// Bookmark a note task (project file-tasks have no DB row — skipped).
fn proj_toggle_bookmark(state: &AppState, note_id: i32, line: usize) {
    if note_id >= 0 {
        let store = match state.store.lock() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };
        magpie_app::tasks::set_bookmark(&store, note_id as i64, line, now_ms());
    }
}

/// Rename a note task's title (project file-tasks are not renamed here).
fn proj_rename_task(state: &AppState, note_id: i32, line: usize, title: &str) {
    if note_id >= 0 {
        let store = match state.store.lock() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };
        magpie_app::tasks::rename_task(&store, note_id as i64, line, title, now_ms());
    }
}

/// Local `HH:MM` for an epoch-ms instant.
fn hhmm_local(ms: i64) -> String {
    let local = ms / 1000 + local_offset_seconds();
    let sod = local.rem_euclid(86_400);
    format!("{:02}:{:02}", sod / 3600, (sod % 3600) / 60)
}

/// `H:MM:SS` duration for a millisecond span.
fn hms(ms: i64) -> String {
    let s = (ms / 1000).max(0);
    format!("{}:{:02}:{:02}", s / 3600, (s % 3600) / 60, s % 60)
}

/// Local day-number (days since epoch) for an instant.
fn local_day(ms: i64) -> i64 {
    (ms / 1000 + local_offset_seconds()).div_euclid(86_400)
}

/// Day label for the entries list: Today / Yesterday / full date.
fn day_label(day: i64, today: i64) -> String {
    if day == today {
        "Today".into()
    } else if day == today - 1 {
        "Yesterday".into()
    } else {
        magpie_core::calendar::full_date(day)
    }
}

/// A `#project`/`#tag` split from a task title (first `#word` → project, next → tag).
fn project_and_tag(title: &str) -> (String, String) {
    let tags: Vec<&str> = title
        .split_whitespace()
        .filter_map(|w| w.strip_prefix('#'))
        .collect();
    let project = tags.first().map(|s| s.to_string()).unwrap_or_default();
    let tag = tags.get(1).map(|s| format!("#{s}")).unwrap_or_default();
    (project, tag)
}

/// Rebuild the Time tab (screen 24) into `TimeStore`: the running-timer bar, entries
/// grouped by day, the This-week per-project card, and the current Pomodoro state.
fn refresh_time(ui: &LauncherWindow, state: &AppState) {
    let now = now_ms();
    let ts = ui.global::<crate::TimeStore>();
    let (entries, active, by_task, week_total) = {
        let store = match state.store.lock() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };
        let week_start = now - 7 * 86_400_000;
        (
            store.recent_time_entries(80).unwrap_or_default(),
            store.active_timer().ok().flatten(),
            store.time_by_task(week_start, now).unwrap_or_default(),
            store.time_total_since(week_start, now).unwrap_or(0),
        )
    };

    // Running-timer bar.
    if let Some(a) = &active {
        let (proj, tag) = project_and_tag(&a.task_title);
        ts.set_running(true);
        ts.set_running_desc(a.task_title.clone().into());
        ts.set_running_project(proj.into());
        ts.set_running_tag(tag.into());
        ts.set_running_elapsed(hms(now - a.start_ms).into());
    } else {
        ts.set_running(false);
    }

    // Entries grouped by local day (newest first).
    let today = local_day(now);
    let mut rows: Vec<crate::TimeRow> = Vec::new();
    let mut cur_day = i64::MIN;
    // Group totals: precompute per-day sums.
    let mut day_ms: std::collections::HashMap<i64, i64> = std::collections::HashMap::new();
    for e in &entries {
        let end = e.end_ms.unwrap_or(now);
        *day_ms.entry(local_day(e.start_ms)).or_insert(0) += (end - e.start_ms).max(0);
    }
    let blank_row = |desc: String| crate::TimeRow {
        is_day_header: false,
        day_label: SharedString::new(),
        day_total: SharedString::new(),
        desc: desc.into(),
        project: SharedString::new(),
        tag: SharedString::new(),
        span: SharedString::new(),
        duration: SharedString::new(),
        running: false,
    };
    let _ = blank_row;
    for e in &entries {
        let d = local_day(e.start_ms);
        if d != cur_day {
            cur_day = d;
            rows.push(crate::TimeRow {
                is_day_header: true,
                day_label: day_label(d, today).into(),
                day_total: hms(*day_ms.get(&d).unwrap_or(&0)).into(),
                desc: SharedString::new(),
                project: SharedString::new(),
                tag: SharedString::new(),
                span: SharedString::new(),
                duration: SharedString::new(),
                running: false,
            });
        }
        let (proj, tag) = project_and_tag(&e.task_title);
        let end = e.end_ms.unwrap_or(now);
        let span = format!(
            "{} — {}",
            hhmm_local(e.start_ms),
            if e.end_ms.is_some() {
                hhmm_local(end)
            } else {
                "now".into()
            }
        );
        rows.push(crate::TimeRow {
            is_day_header: false,
            day_label: SharedString::new(),
            day_total: SharedString::new(),
            desc: e.task_title.clone().into(),
            project: proj.into(),
            tag: tag.into(),
            span: span.into(),
            duration: hms(end - e.start_ms).into(),
            running: e.end_ms.is_none(),
        });
    }
    ts.set_rows(ModelRc::new(VecModel::from(rows)));

    // This-week per-project card (aggregate `time_by_task` by derived project).
    let mut proj_ms: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
    for (_key, title, ms) in &by_task {
        let (proj, _) = project_and_tag(title);
        let name = if proj.is_empty() { title.clone() } else { proj };
        *proj_ms.entry(name).or_insert(0) += *ms;
    }
    let mut projects: Vec<(String, i64)> = proj_ms.into_iter().collect();
    projects.sort_by_key(|p| std::cmp::Reverse(p.1));
    let max = projects.first().map(|p| p.1).unwrap_or(1).max(1) as f32;
    let active_proj = active
        .as_ref()
        .map(|a| project_and_tag(&a.task_title).0)
        .unwrap_or_default();
    let bars: Vec<crate::TimeProjectBar> = projects
        .iter()
        .take(6)
        .map(|(name, ms)| crate::TimeProjectBar {
            name: name.clone().into(),
            total: {
                let m = ms / 60000;
                format!("{}h {:02}m", m / 60, m % 60).into()
            },
            bar: (*ms as f32 / max).min(1.0),
            accent: !active_proj.is_empty() && *name == active_proj,
        })
        .collect();
    ts.set_week_projects(ModelRc::new(VecModel::from(bars)));
    let wm = week_total / 60000;
    ts.set_week_total(format!("{}:{:02}", wm / 60, wm % 60).into());

    push_pomodoro(ui, state);
}

/// Continue tracking: start a timer for the row's task (or, when `row_index` points
/// at a header/blank row — e.g. Continue-last — the most recent entry).
fn continue_time_entry(w: &slint::Weak<LauncherWindow>, state: &AppState, row_index: i32) {
    let from_row = w.upgrade().and_then(|ui| {
        ui.global::<TimeStore>()
            .get_rows()
            .row_data(row_index.max(0) as usize)
            .map(|r| r.desc.to_string())
            .filter(|t| !t.is_empty())
    });
    let title = from_row.or_else(|| {
        let store = match state.store.lock() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };
        store
            .recent_time_entries(1)
            .ok()
            .and_then(|v| v.into_iter().next())
            .map(|e| e.task_title)
    });
    if let Some(title) = title {
        {
            let store = match state.store.lock() {
                Ok(g) => g,
                Err(e) => e.into_inner(),
            };
            let _ = store.start_timer(&title, &title, None, now_ms());
        }
        if let Some(ui) = w.upgrade() {
            refresh_time(&ui, state);
        }
    }
}

/// Push the current Pomodoro state into `TimeStore`.
fn push_pomodoro(ui: &LauncherWindow, state: &AppState) {
    use magpie_app::app_state::PomodoroState;
    let p = state.pomodoro.lock().map(|g| g.clone()).unwrap_or_default();
    let ts = ui.global::<crate::TimeStore>();
    let len = PomodoroState::phase_len(p.phase).max(1);
    ts.set_pomo_running(p.running);
    ts.set_pomo_phase(p.phase);
    ts.set_pomo_remaining(
        format!("{:02}:{:02}", p.remaining_sec / 60, p.remaining_sec % 60).into(),
    );
    ts.set_pomo_progress(((len - p.remaining_sec) as f32 / len as f32).clamp(0.0, 1.0));
    ts.set_pomo_cycle(p.cycle);
    ts.set_pomo_cycles_total(4);
}

/// Refresh the cached `(keyword, snippet_id)` list the expand-as-you-type monitor
/// matches against (called on start and whenever snippets change).
fn refresh_expand_keywords(state: &AppState) {
    let kws: Vec<(String, i64)> = {
        let store = match state.store.lock() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };
        store
            .list_snippets()
            .unwrap_or_default()
            .into_iter()
            .filter(|s| !s.keyword.is_empty())
            .map(|s| (s.keyword, s.id))
            .collect()
    };
    if let Ok(mut g) = state.expand_keywords.lock() {
        *g = kws;
    }
}

/// Start the global expand-as-you-type key monitor (Phase 16b). Listen-only; keeps
/// a small in-memory rolling buffer; on a snippet-keyword suffix match it erases the
/// keyword and pastes the expansion on the main thread. Pauses during secure input.
/// Returns whether the monitor actually started (needs macOS Input Monitoring).
fn start_expand_monitor(ui: &slint::Weak<LauncherWindow>, state: &Arc<AppState>) -> bool {
    if state
        .key_monitor
        .lock()
        .map(|g| g.is_some())
        .unwrap_or(false)
    {
        return true; // already running
    }
    refresh_expand_keywords(state);
    let st = state.clone();
    let w = ui.clone();
    let monitor = magpie_platform::KeyMonitor::start(move |c| {
        // Runs on the tap thread. Pause entirely while secure input is active.
        if magpie_platform::secure_input_enabled() {
            if let Ok(mut b) = st.expand_buffer.lock() {
                b.clear();
            }
            return;
        }
        let matched: Option<(usize, i64)> = {
            let mut b = st.expand_buffer.lock().unwrap_or_else(|e| e.into_inner());
            match c {
                '\u{8}' => {
                    b.pop();
                    None
                }
                '\n' | ' ' => {
                    b.clear();
                    None
                }
                _ => {
                    b.push(c);
                    let count = b.chars().count();
                    if count > 64 {
                        *b = b.chars().skip(count - 64).collect();
                    }
                    let kws = st.expand_keywords.lock().unwrap_or_else(|e| e.into_inner());
                    kws.iter()
                        .find(|(kw, _)| b.ends_with(kw.as_str()))
                        .map(|(kw, id)| (kw.chars().count(), *id))
                }
            }
        };
        if let Some((kw_len, id)) = matched {
            if let Ok(mut b) = st.expand_buffer.lock() {
                b.clear();
            }
            let st2 = st.clone();
            let w2 = w.clone();
            let _ = slint::invoke_from_event_loop(move || {
                let body = {
                    let store = match st2.store.lock() {
                        Ok(g) => g,
                        Err(e) => e.into_inner(),
                    };
                    store.get_snippet(id).ok().flatten().map(|s| s.body)
                };
                if let Some(body) = body {
                    let expanded = expand_snippet(&st2, &body).text;
                    // Erase the typed keyword, then paste the expansion.
                    let _ = magpie_platform::send_backspaces(kw_len);
                    if let Ok(mut clip) = magpie_platform::platform_clipboard() {
                        let _ = clip.set_text(&expanded);
                    }
                    let _ = magpie_platform::Paster::paste(&EnigoPaster);
                }
                let _ = w2;
            });
        }
    });
    let started = monitor.is_some();
    if let Ok(mut g) = state.key_monitor.lock() {
        *g = monitor;
    }
    started
}

/// Stop the expand-as-you-type monitor and clear the rolling buffer.
fn stop_expand_monitor(state: &AppState) {
    if let Ok(mut g) = state.key_monitor.lock() {
        if let Some(m) = g.take() {
            m.stop();
        }
    }
    if let Ok(mut b) = state.expand_buffer.lock() {
        b.clear();
    }
}

/// Map the SearchState mode/case ints to the core enums.
fn search_mode_from(i: i32) -> magpie_core::filesearch::MatchMode {
    use magpie_core::filesearch::MatchMode::*;
    match i {
        1 => Regex,
        2 => Sed,
        _ => Text,
    }
}
fn search_case_from(i: i32) -> magpie_core::filesearch::CaseMode {
    use magpie_core::filesearch::CaseMode::*;
    match i {
        1 => Sensitive,
        2 => Insensitive,
        _ => Smart,
    }
}
fn search_tab_from(i: i32) -> magpie_app::file_search::Tab {
    use magpie_app::file_search::Tab::*;
    match i {
        1 => Names,
        2 => Contents,
        3 => Documents,
        _ => All,
    }
}

/// Run the file search for the current query on a background thread and push the
/// results (files + hierarchy tree + counts) into `SearchStore`. Stale runs are
/// cancelled via `search_gen`. The shared search box supplies the query.
fn refresh_files(ui: &LauncherWindow, state: &Arc<AppState>) {
    use magpie_app::file_search::{self, Limits};
    use std::sync::atomic::Ordering;

    let query = state.ui.lock().map(|u| u.text.clone()).unwrap_or_default();
    let (locations, mode_i, case_i, tab_i, hide_viewed) = {
        let s = state.search.lock().unwrap_or_else(|e| e.into_inner());
        (s.locations.clone(), s.mode, s.case, s.tab, s.hide_viewed)
    };
    let ss = ui.global::<SearchStore>();
    ss.set_locations(ModelRc::new(VecModel::from(
        locations
            .iter()
            .cloned()
            .map(SharedString::from)
            .collect::<Vec<_>>(),
    )));
    ss.set_mode(mode_i);
    ss.set_case_mode(case_i);
    ss.set_tab(tab_i);
    ss.set_hide_viewed(hide_viewed);
    let mode = search_mode_from(mode_i);
    ss.set_is_sed(matches!(mode, magpie_core::filesearch::MatchMode::Sed));
    let sed_cmd = magpie_core::filesearch::parse_sed(&query);
    ss.set_ignore_summary(SharedString::from(format!(
        "Ignoring .gitignore, global excludes and {} · 3 rules",
        file_search::magpie_ignore_path().display()
    )));

    // Nothing to search yet.
    let effective_query = query.trim().to_string();
    if locations.is_empty() || (effective_query.is_empty() && mode_i != 2) {
        ss.set_files(ModelRc::new(VecModel::from(Vec::<SearchFile>::new())));
        ss.set_tree(ModelRc::new(VecModel::from(Vec::<SearchTreeRow>::new())));
        ss.set_summary(SharedString::from("Type to search these locations"));
        ss.set_count_all(0);
        ss.set_count_names(0);
        ss.set_count_contents(0);
        ss.set_count_documents(0);
        return;
    }

    let gen = state.search_gen.fetch_add(1, Ordering::SeqCst) + 1;
    let st = state.clone();
    let w = ui.as_weak();
    std::thread::spawn(move || {
        let case = search_case_from(case_i);
        let tab = search_tab_from(tab_i);
        let cancelled = {
            let st = st.clone();
            move || st.search_gen.load(Ordering::SeqCst) != gen
        };
        let results = file_search::run_search(
            &locations,
            &effective_query,
            mode,
            case,
            tab,
            false,
            false,
            &Limits::default(),
            &cancelled,
        );
        if st.search_gen.load(Ordering::SeqCst) != gen {
            return;
        }
        let sed = sed_cmd.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = w.upgrade() {
                populate_search_results(&ui, &st, results, sed);
            }
        });
    });
}

/// Apply the current `s/pat/rep/flags` substitution to every matched file, writing
/// a `<path>.magpie-bak` backup first (undo). Only runs when the query parses as a
/// sed command. Reachable via the `apply-replace` callback (⌘↵).
fn apply_sed_replace(w: &slint::Weak<LauncherWindow>, state: &Arc<AppState>) {
    let query = state.ui.lock().map(|u| u.text.clone()).unwrap_or_default();
    let Some(cmd) = magpie_core::filesearch::parse_sed(&query) else {
        return;
    };
    let Ok(re) = regex::RegexBuilder::new(&cmd.pattern)
        .case_insensitive(cmd.case_insensitive)
        .build()
    else {
        return;
    };
    let paths = state
        .search
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .result_paths
        .clone();
    for p in paths {
        let Ok(content) = std::fs::read_to_string(&p) else {
            continue;
        };
        let replaced = if cmd.global {
            re.replace_all(&content, cmd.replacement.as_str())
                .to_string()
        } else {
            re.replace(&content, cmd.replacement.as_str()).to_string()
        };
        if replaced != content {
            let _ = std::fs::write(format!("{p}.magpie-bak"), &content);
            let _ = std::fs::write(&p, replaced);
        }
    }
    if let Some(ui) = w.upgrade() {
        refresh_files(&ui, state);
    }
}

/// Byte-safe split of `text` around `[start,end)` into (pre, mid, post), with pre
/// trimmed to a tail window and post to a head window so the match stays visible
/// near the left of the row (a leading/trailing `…` marks truncation).
/// First case-insensitive match of `needle_lc` (already lowercase) in `hay`, as original
/// byte offsets. Skips (None) when lowercasing would shift byte offsets (rare non-ASCII).
fn find_ci(hay: &str, needle_lc: &str) -> Option<(usize, usize)> {
    if needle_lc.is_empty() {
        return None;
    }
    let hay_lc = hay.to_lowercase();
    if hay_lc.len() != hay.len() {
        return None;
    }
    hay_lc.find(needle_lc).map(|s| (s, s + needle_lc.len()))
}

/// Build highlighted preview lines: each line split around the earliest matching query
/// term (`mid` is the match, rendered highlighted; non-matching lines have `mid` empty).
/// Returns the lines (capped) and the index of the first matching line (-1 if none), so
/// the preview can scroll to it. `terms` are lowercased word tokens.
fn highlight_preview_lines(
    full_text: &str,
    terms: &[String],
    cap: usize,
) -> (Vec<SearchLine>, i32) {
    let mut out: Vec<SearchLine> = Vec::new();
    let mut first: i32 = -1;
    for (i, line) in full_text.split('\n').enumerate() {
        if i >= cap {
            break;
        }
        let best = terms
            .iter()
            .filter_map(|t| find_ci(line, t))
            .min_by_key(|(s, _)| *s);
        let sl = match best {
            Some((s, e)) => {
                if first < 0 {
                    first = i as i32;
                }
                // Trim around the match so it stays visible on long lines.
                let (pre, mid, post) = split3(line, s, e);
                SearchLine {
                    line_no: SharedString::from(format!("{}", i + 1)),
                    pre: SharedString::from(pre),
                    mid: SharedString::from(mid),
                    post: SharedString::from(post),
                    replaced: SharedString::new(),
                }
            }
            None => SearchLine {
                line_no: SharedString::from(format!("{}", i + 1)),
                pre: SharedString::from(line),
                mid: SharedString::new(),
                post: SharedString::new(),
                replaced: SharedString::new(),
            },
        };
        out.push(sl);
    }
    (out, first)
}

fn split3(text: &str, start: usize, end: usize) -> (String, String, String) {
    const PRE: usize = 40;
    const POST: usize = 120;
    if start >= end
        || end > text.len()
        || !text.is_char_boundary(start)
        || !text.is_char_boundary(end)
    {
        let head: String = text.chars().take(160).collect();
        return (head, String::new(), String::new());
    }
    // Trim pre to the last PRE chars.
    let pre_full = &text[..start];
    let pre_chars: Vec<char> = pre_full.chars().collect();
    let pre = if pre_chars.len() > PRE {
        let tail: String = pre_chars[pre_chars.len() - PRE..].iter().collect();
        format!("…{}", tail.trim_start())
    } else {
        pre_full.to_string()
    };
    let mid = text[start..end].to_string();
    let post_full = &text[end..];
    let post_chars: Vec<char> = post_full.chars().collect();
    let post = if post_chars.len() > POST {
        let head: String = post_chars[..POST].iter().collect();
        format!("{head}…")
    } else {
        post_full.to_string()
    };
    (pre, mid, post)
}

/// Convert engine results into the Slint models (files + tree + counts + summary),
/// mark viewed, apply hide-viewed, and compute the sed replacement preview.
fn populate_search_results(
    ui: &LauncherWindow,
    state: &Arc<AppState>,
    results: magpie_app::file_search::SearchResults,
    sed: Option<magpie_core::filesearch::SedCommand>,
) {
    let (viewed, hide_viewed) = {
        let s = state.search.lock().unwrap_or_else(|e| e.into_inner());
        (s.viewed.clone(), s.hide_viewed)
    };
    // A regex to build the sed replacement preview.
    let sed_re = sed.as_ref().and_then(|c| {
        regex::RegexBuilder::new(&c.pattern)
            .case_insensitive(c.case_insensitive)
            .build()
            .ok()
            .map(|re| (re, c.replacement.clone(), c.global))
    });

    let ss = ui.global::<SearchStore>();
    let shown: Vec<&magpie_app::file_search::FileHit> = results
        .files
        .iter()
        .filter(|f| !(hide_viewed && viewed.contains(&f.path.to_string_lossy().to_string())))
        .collect();

    let max_matches = shown
        .iter()
        .map(|f| f.matches.len().max(1))
        .max()
        .unwrap_or(1) as f32;

    let files: Vec<SearchFile> = shown
        .iter()
        .map(|f| {
            let path_s = f.path.to_string_lossy().to_string();
            let is_viewed = viewed.contains(&path_s);
            let lines: Vec<SearchLine> = f
                .matches
                .iter()
                .map(|m| {
                    let (pre, mid, post) = split3(&m.text, m.match_start, m.match_end);
                    let replaced = sed_re
                        .as_ref()
                        .map(|(re, rep, global)| {
                            if *global {
                                re.replace_all(&m.text, rep.as_str()).to_string()
                            } else {
                                re.replace(&m.text, rep.as_str()).to_string()
                            }
                        })
                        .filter(|r| *r != m.text)
                        .unwrap_or_default();
                    SearchLine {
                        line_no: m.line_no.to_string().into(),
                        pre: pre.into(),
                        mid: mid.into(),
                        post: post.into(),
                        replaced: replaced.into(),
                    }
                })
                .collect();
            SearchFile {
                name: f.name.clone().into(),
                dir: f.dir_rel.clone().into(),
                path: path_s.into(),
                match_count: format!(
                    "{} match{}",
                    f.matches.len().max(1),
                    if f.matches.len() == 1 { "" } else { "es" }
                )
                .into(),
                viewed: is_viewed,
                name_only: f.name_only,
                lines: ModelRc::new(VecModel::from(lines)),
            }
        })
        .collect();

    // Hierarchy tree: one row per location, then its files (indent 1).
    use magpie_app::file_search;
    let mut tree: Vec<SearchTreeRow> = Vec::new();
    let locations = {
        let s = state.search.lock().unwrap_or_else(|e| e.into_inner());
        s.locations.clone()
    };
    for loc in &locations {
        let root = file_search::expand_tilde(loc);
        let loc_files: Vec<&&magpie_app::file_search::FileHit> =
            shown.iter().filter(|f| f.path.starts_with(&root)).collect();
        let loc_matches: usize = loc_files.iter().map(|f| f.matches.len().max(1)).sum();
        tree.push(SearchTreeRow {
            indent: 0,
            label: loc.clone().into(),
            count: loc_matches.to_string().into(),
            bar: (loc_matches as f32 / (max_matches * 3.0)).min(1.0),
            is_file: false,
            viewed: false,
        });
        for f in loc_files {
            let path_s = f.path.to_string_lossy().to_string();
            tree.push(SearchTreeRow {
                indent: 1,
                label: f.name.clone().into(),
                count: f.matches.len().max(1).to_string().into(),
                bar: (f.matches.len().max(1) as f32 / max_matches).min(1.0),
                is_file: true,
                viewed: viewed.contains(&path_s),
            });
        }
    }

    // Store the shown paths for index → action.
    {
        let mut s = state.search.lock().unwrap_or_else(|e| e.into_inner());
        s.result_paths = shown
            .iter()
            .map(|f| f.path.to_string_lossy().to_string())
            .collect();
    }

    let n_files = shown.len();
    let n_matches: usize = shown.iter().map(|f| f.matches.len()).sum();
    ss.set_summary(
        format!(
            "{} match{} · {} file{} · {} location{}",
            n_matches,
            if n_matches == 1 { "" } else { "es" },
            n_files,
            if n_files == 1 { "" } else { "s" },
            locations.len(),
            if locations.len() == 1 { "" } else { "s" },
        )
        .into(),
    );
    ss.set_count_all(n_matches as i32);
    ss.set_count_contents(shown.iter().map(|f| f.matches.len()).sum::<usize>() as i32);
    ss.set_count_names(shown.iter().filter(|f| f.name_only).count() as i32);
    ss.set_count_documents(0);
    ss.set_files(ModelRc::new(VecModel::from(files)));
    ss.set_tree(ModelRc::new(VecModel::from(tree)));
    if ss.get_selected() >= n_files as i32 {
        ss.set_selected(0);
    }
    let _ = results.truncated;
}

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

/// The config token for a popover-tab index.
fn popover_tab_token(i: i32) -> &'static str {
    match i {
        0 => "tasks",
        1 => "today",
        2 => "daily",
        _ => "clipboard",
    }
}

/// Pretty-print a hotkey spec (`super+alt+space`) with mac glyphs (`⌘⌥Space`).
fn pretty_hotkey(spec: &str) -> String {
    let mut out = String::new();
    for part in spec.split('+') {
        let p = part.trim().to_ascii_lowercase();
        let glyph = match p.as_str() {
            "super" | "cmd" | "command" => "⌘",
            "alt" | "option" | "opt" => "⌥",
            "shift" => "⇧",
            "ctrl" | "control" => "⌃",
            "space" => "Space",
            _ => "",
        };
        if glyph.is_empty() {
            out.push_str(&p.to_uppercase());
        } else {
            out.push_str(glyph);
        }
    }
    out
}

/// The scratch-hotkey presets the Settings row cycles through.
const SCRATCH_HOTKEY_PRESETS: &[&str] = &[
    "super+alt+space",
    "alt+shift+space",
    "super+shift+space",
    "super+alt+s",
];

/// Populate the Settings → Features controls from the current state/config.
fn init_settings_props(ui: &LauncherWindow, state: &AppState, cfg: &Config) {
    use std::sync::atomic::Ordering::Relaxed;
    ui.set_settings_popover_tab(state.popover_default_tab.load(Relaxed));
    ui.set_settings_sync_file_wins(matches!(
        *state
            .project_conflict_policy
            .lock()
            .unwrap_or_else(|e| e.into_inner()),
        magpie_core::projects::ConflictPolicy::FileWins
    ));
    ui.set_settings_keep_git(state.projects_keep_magpie_out_of_git.load(Relaxed));
    ui.set_settings_scratch_hotkey(pretty_hotkey(&cfg.scratch_hotkey).into());
}

/// Wire the Settings → Features callbacks: apply live + persist to config.
fn wire_settings(ui: &LauncherWindow, state: &Arc<AppState>) {
    use std::sync::atomic::Ordering::Relaxed;
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_set_popover_tab(move |i| {
            s.popover_default_tab.store(i, Relaxed);
            let path = data_dir().join("config.toml");
            let mut cfg = magpie_app::config::load_or_default(&path);
            cfg.popover_default_tab = popover_tab_token(i).to_string();
            let _ = magpie_app::config::save(&cfg, &path);
            if let Some(ui) = w.upgrade() {
                ui.set_settings_popover_tab(i);
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_set_sync_policy(move |file_wins| {
            *s.project_conflict_policy
                .lock()
                .unwrap_or_else(|e| e.into_inner()) = if file_wins {
                magpie_core::projects::ConflictPolicy::FileWins
            } else {
                magpie_core::projects::ConflictPolicy::Ask
            };
            let path = data_dir().join("config.toml");
            let mut cfg = magpie_app::config::load_or_default(&path);
            cfg.project_sync_conflict = if file_wins { "file" } else { "ask" }.into();
            let _ = magpie_app::config::save(&cfg, &path);
            if let Some(ui) = w.upgrade() {
                ui.set_settings_sync_file_wins(file_wins);
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_set_keep_git(move |b| {
            s.projects_keep_magpie_out_of_git.store(b, Relaxed);
            let path = data_dir().join("config.toml");
            let mut cfg = magpie_app::config::load_or_default(&path);
            cfg.projects_keep_magpie_out_of_git = b;
            let _ = magpie_app::config::save(&cfg, &path);
            if let Some(ui) = w.upgrade() {
                ui.set_settings_keep_git(b);
            }
        });
    }
    {
        let w = ui.as_weak();
        ui.on_cycle_scratch_hotkey(move || {
            let path = data_dir().join("config.toml");
            let mut cfg = magpie_app::config::load_or_default(&path);
            let cur = SCRATCH_HOTKEY_PRESETS
                .iter()
                .position(|p| *p == cfg.scratch_hotkey)
                .unwrap_or(0);
            let next = SCRATCH_HOTKEY_PRESETS[(cur + 1) % SCRATCH_HOTKEY_PRESETS.len()];
            cfg.scratch_hotkey = next.to_string();
            let _ = magpie_app::config::save(&cfg, &path);
            if let Some(ui) = w.upgrade() {
                ui.set_settings_scratch_hotkey(pretty_hotkey(next).into());
            }
        });
    }
}

/// Rebuild the Schedule tab's job list (Phase 21) from the job store.
fn refresh_schedule(ui: &LauncherWindow, state: &AppState) {
    let ss = ui.global::<ScheduleStore>();
    let tz = state.world_clock_home.clone();
    let now = now_ms();
    let rows: Vec<JobRow> = {
        let store = state.store.lock().unwrap_or_else(|e| e.into_inner());
        store
            .list_jobs()
            .unwrap_or_default()
            .into_iter()
            .map(|j| {
                let last = store
                    .recent_job_runs(j.id, 1)
                    .ok()
                    .and_then(|v| v.into_iter().next());
                let (last_run, failed) = match last {
                    Some(r) => {
                        let when = magpie_core::cron::fmt_local(r.start_ms / 1000, &tz);
                        match r.exit_code {
                            Some(0) => (format!("last: {when} · ok"), false),
                            Some(c) => (format!("last: {when} · failed · exit {c}"), true),
                            None => (format!("last: {when} · did not finish"), true),
                        }
                    }
                    None => ("never run".to_string(), false),
                };
                let next_run = magpie_core::cron::parse_cron(&j.cron)
                    .ok()
                    .and_then(|c| {
                        magpie_core::cron::next_runs_in_tz(&c, now / 1000, &tz, 1)
                            .first()
                            .copied()
                    })
                    .map(|ts| format!("· next {}", magpie_core::cron::fmt_local(ts, &tz)))
                    .unwrap_or_default();
                JobRow {
                    id: j.id as i32,
                    name: j.name.into(),
                    kind: j.kind.as_str().into(),
                    cron: j.cron.into(),
                    enabled: j.enabled,
                    os_managed: j.os_managed,
                    last_run: last_run.into(),
                    failed,
                    next_run: next_run.into(),
                }
            })
            .collect()
    };
    ss.set_jobs(ModelRc::new(VecModel::from(rows)));
}

/// Map the Schedule form's kind index to a `JobKind`.
fn job_kind_of(i: i32) -> magpie_core::jobs::JobKind {
    use magpie_core::jobs::JobKind;
    match i {
        1 => JobKind::Shell,
        2 => JobKind::Open,
        3 => JobKind::Url,
        _ => JobKind::Notification,
    }
}

/// The job id for a Schedule-list row index (list order matches `refresh_schedule`).
fn job_id_at(state: &AppState, row: i32) -> Option<i64> {
    let store = state.store.lock().unwrap_or_else(|e| e.into_inner());
    store
        .list_jobs()
        .ok()
        .and_then(|v| v.get(row.max(0) as usize).map(|j| j.id))
}

/// Install or remove the OS scheduler entry for a job per its `os_managed`/`enabled`.
/// Returns Err (with a message) if an OS install was requested but failed.
fn apply_os_schedule(job: &magpie_core::jobs::Job) -> Result<(), String> {
    if job.os_managed && job.enabled {
        let cron = magpie_core::cron::parse_cron(&job.cron)?;
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        magpie_platform::install_os_job(
            job.id,
            &exe,
            &cron.launchd_intervals(),
            &cron.systemd_oncalendar(),
            &cron.schtasks_args().unwrap_or_default(),
        )
    } else {
        let _ = magpie_platform::remove_os_job(job.id);
        Ok(())
    }
}

/// Wire the Schedule tab's callbacks (Phase 21).
fn wire_schedule(ui: &LauncherWindow, state: &Arc<AppState>) {
    let ss = ui.global::<ScheduleStore>();
    // New job → reset the form.
    {
        let w = ui.as_weak();
        ss.on_new_job(move || {
            if let Some(ui) = w.upgrade() {
                let ss = ui.global::<ScheduleStore>();
                ss.set_edit_id(-1);
                ss.set_form_name(SharedString::new());
                ss.set_form_kind(0);
                ss.set_form_command(SharedString::new());
                ss.set_form_when(SharedString::new());
                ss.set_form_cron(SharedString::new());
                ss.set_form_dir(SharedString::new());
                ss.set_form_timeout(300);
                ss.set_form_notify(true);
                ss.set_form_os(false);
                ss.set_next_runs(SharedString::new());
                ss.set_form_error(SharedString::new());
                ss.set_editing(true);
            }
        });
    }
    // Edit an existing job → load it into the form.
    {
        let s = state.clone();
        let w = ui.as_weak();
        ss.on_edit_job(move |row| {
            let job = job_id_at(&s, row).and_then(|id| {
                s.store
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .get_job(id)
                    .ok()
                    .flatten()
            });
            if let (Some(ui), Some(j)) = (w.upgrade(), job) {
                let ss = ui.global::<ScheduleStore>();
                ss.set_edit_id(j.id as i32);
                ss.set_form_name(j.name.into());
                ss.set_form_kind(match j.kind {
                    magpie_core::jobs::JobKind::Shell => 1,
                    magpie_core::jobs::JobKind::Open => 2,
                    magpie_core::jobs::JobKind::Url => 3,
                    _ => 0,
                });
                ss.set_form_command(j.command.into());
                ss.set_form_when(j.cron.clone().into());
                ss.set_form_cron(j.cron.into());
                ss.set_form_dir(j.working_dir.into());
                ss.set_form_timeout(j.timeout_secs as i32);
                ss.set_form_notify(j.notify_on_failure);
                ss.set_form_os(j.os_managed);
                ss.set_form_error(SharedString::new());
                ss.set_editing(true);
                schedule_preview(&ui, &s);
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ss.on_delete_job(move |row| {
            if let Some(id) = job_id_at(&s, row) {
                let _ = magpie_platform::remove_os_job(id);
                let store = s.store.lock().unwrap_or_else(|e| e.into_inner());
                let _ = store.delete_job(id);
                drop(store);
                if let Some(ui) = w.upgrade() {
                    refresh_schedule(&ui, &s);
                }
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ss.on_run_now(move |row| {
            if let Some(id) = job_id_at(&s, row) {
                // Verify + start under the lock, execute without it, finish under it.
                let vjob = {
                    let st = s.store.lock().unwrap_or_else(|e| e.into_inner());
                    st.job_for_run(id).ok().flatten()
                };
                if let Some(vjob) = vjob {
                    let run = {
                        let st = s.store.lock().unwrap_or_else(|e| e.into_inner());
                        st.start_job_run(id, now_ms()).ok()
                    };
                    let res = magpie_app::job_runner::execute_job(&vjob);
                    if let Some(run) = run {
                        let st = s.store.lock().unwrap_or_else(|e| e.into_inner());
                        let _ = st.finish_job_run(
                            run,
                            id,
                            now_ms(),
                            res.exit_code,
                            &res.stdout,
                            &res.stderr,
                            20,
                        );
                    }
                }
                if let Some(ui) = w.upgrade() {
                    refresh_schedule(&ui, &s);
                }
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ss.on_toggle_pause(move |row| {
            if let Some(id) = job_id_at(&s, row) {
                let job = {
                    let st = s.store.lock().unwrap_or_else(|e| e.into_inner());
                    let cur = st.get_job(id).ok().flatten();
                    if let Some(j) = &cur {
                        let _ = st.set_job_enabled(id, !j.enabled, now_ms());
                    }
                    st.get_job(id).ok().flatten()
                };
                if let Some(j) = job {
                    let _ = apply_os_schedule(&j);
                }
                if let Some(ui) = w.upgrade() {
                    refresh_schedule(&ui, &s);
                }
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ss.on_view_logs(move |row| {
            if let (Some(ui), Some(id)) = (w.upgrade(), job_id_at(&s, row)) {
                let tz = s.world_clock_home.clone();
                let logs = {
                    let st = s.store.lock().unwrap_or_else(|e| e.into_inner());
                    st.recent_job_runs(id, 20)
                        .unwrap_or_default()
                        .into_iter()
                        .map(|r| {
                            let when = magpie_core::cron::fmt_local(r.start_ms / 1000, &tz);
                            let code = r
                                .exit_code
                                .map(|c| format!("exit {c}"))
                                .unwrap_or_else(|| "—".into());
                            let mut s = format!("{when} · {code}");
                            if !r.stdout.trim().is_empty() {
                                s.push_str(&format!("\n{}", r.stdout.trim()));
                            }
                            if !r.stderr.trim().is_empty() {
                                s.push_str(&format!("\n[err] {}", r.stderr.trim()));
                            }
                            s
                        })
                        .collect::<Vec<_>>()
                        .join("\n\n")
                };
                let ss = ui.global::<ScheduleStore>();
                ss.set_logs(logs.into());
                ss.set_showing_logs(true);
            }
        });
    }
    {
        let w = ui.as_weak();
        ss.on_close_logs(move || {
            if let Some(ui) = w.upgrade() {
                ui.global::<ScheduleStore>().set_showing_logs(false);
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ss.on_when_changed(move |_txt| {
            if let Some(ui) = w.upgrade() {
                schedule_preview(&ui, &s);
            }
        });
    }
    {
        let w = ui.as_weak();
        ss.on_save(move || {
            if let Some(ui) = w.upgrade() {
                let ss = ui.global::<ScheduleStore>();
                let name = ss.get_form_name().to_string();
                let command = ss.get_form_command().to_string();
                let cron = resolve_form_cron(&ss);
                if name.trim().is_empty() || command.trim().is_empty() {
                    ss.set_form_error("Name and command are required.".into());
                    return;
                }
                if magpie_core::cron::parse_cron(&cron).is_err() {
                    ss.set_form_error(
                        "Couldn't understand the schedule — try a cron like `0 9 * * *`.".into(),
                    );
                    return;
                }
                ss.set_form_cron(cron.clone().into());
                let kind = ["Notification", "Shell", "Open", "URL"]
                    [ss.get_form_kind().clamp(0, 3) as usize];
                let mode = if ss.get_form_os() {
                    "Always (OS scheduler)"
                } else {
                    "While Magpie is open"
                };
                ss.set_confirm_text(
                    format!("{kind}: {command}\nWhen: {cron}\nRuns: {mode}").into(),
                );
                ss.set_form_error(SharedString::new());
                ss.set_confirming(true);
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ss.on_confirm_save(move || {
            if let Some(ui) = w.upgrade() {
                let ssg = ui.global::<ScheduleStore>();
                let id = ssg.get_edit_id();
                let name = ssg.get_form_name().to_string();
                let kind = job_kind_of(ssg.get_form_kind());
                let command = ssg.get_form_command().to_string();
                let cron = resolve_form_cron(&ssg);
                let dir = ssg.get_form_dir().to_string();
                let timeout = ssg.get_form_timeout().max(1) as i64;
                let notify = ssg.get_form_notify();
                let os = ssg.get_form_os();
                let now = now_ms();
                let job_id = {
                    let st = s.store.lock().unwrap_or_else(|e| e.into_inner());
                    if id < 0 {
                        st.add_job(&name, kind, &command, &cron, &dir, timeout, notify, os, now)
                            .ok()
                    } else {
                        let _ = st.update_job(
                            id as i64, &name, kind, &command, &cron, &dir, timeout, notify, os, now,
                        );
                        Some(id as i64)
                    }
                };
                // Apply the OS schedule (or remove it). On failure, fall back to While-open.
                if let Some(jid) = job_id {
                    let job = s
                        .store
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .get_job(jid)
                        .ok()
                        .flatten();
                    if let Some(job) = job {
                        if let Err(e) = apply_os_schedule(&job) {
                            magpie_platform::notify("Magpie — OS scheduling failed", &e);
                            let st = s.store.lock().unwrap_or_else(|e| e.into_inner());
                            let _ = st.update_job(
                                jid, &name, kind, &command, &cron, &dir, timeout, notify, false,
                                now,
                            );
                        }
                    }
                }
                ssg.set_confirming(false);
                ssg.set_editing(false);
                refresh_schedule(&ui, &s);
            }
        });
    }
    {
        let w = ui.as_weak();
        ss.on_cancel_edit(move || {
            if let Some(ui) = w.upgrade() {
                let ss = ui.global::<ScheduleStore>();
                ss.set_confirming(false);
                ss.set_editing(false);
            }
        });
    }
}

/// The cron the form resolves to: a valid cron typed directly, else natural language.
fn resolve_form_cron(ss: &ScheduleStore) -> String {
    let when = ss.get_form_when().to_string();
    if magpie_core::cron::parse_cron(&when).is_ok() {
        when
    } else {
        magpie_core::cron::natural_to_cron(&when).unwrap_or_default()
    }
}

/// Recompute the form's resolved cron + next-5-runs preview from the "when" field.
fn schedule_preview(ui: &LauncherWindow, state: &AppState) {
    let ss = ui.global::<ScheduleStore>();
    let cron = resolve_form_cron(&ss);
    ss.set_form_cron(cron.clone().into());
    let tz = state.world_clock_home.clone();
    let preview = magpie_core::cron::parse_cron(&cron)
        .ok()
        .map(|c| {
            magpie_core::cron::next_runs_in_tz(&c, now_ms() / 1000, &tz, 5)
                .into_iter()
                .map(|ts| magpie_core::cron::fmt_local(ts, &tz))
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    ss.set_next_runs(preview.into());
}

/// Build synthetic `Task`s for every syncable project's file tasks so they appear in
/// the global Tasks view and board alongside note tasks (Phase 18b cross-view). Each
/// file gets a unique negative `note_id`; markdown files reuse the note parser (full
/// inline `!prio @due #tag` syntax), org/todo.txt use the format-aware line parser.
/// Also records `note_id → file` in `state.project_task_refs` so a toggle routes to
/// the file. Returns the tasks (project set to the project's name).
fn project_file_tasks(state: &AppState, now: i64) -> Vec<magpie_app::tasks::Task> {
    use magpie_app::tasks::{Status, Task};
    let g = state.project_scan.lock().unwrap_or_else(|e| e.into_inner());
    let mut tasks = Vec::new();
    let mut refs: std::collections::HashMap<i32, magpie_app::app_state::ProjTaskRef> =
        std::collections::HashMap::new();
    let mut next_id: i32 = -1000;
    for h in g.hits.iter().filter(|h| h.syncable()) {
        // Distinct task files in first-seen order.
        let mut files: Vec<(String, std::path::PathBuf)> = Vec::new();
        for t in &h.tasks {
            if !files.iter().any(|(r, _)| r == &t.file_rel) {
                files.push((t.file_rel.clone(), t.file_abs.clone()));
            }
        }
        for (rel, file_abs) in files {
            let content = g
                .file_snapshots
                .get(&file_abs)
                .cloned()
                .or_else(|| std::fs::read_to_string(&file_abs).ok())
                .unwrap_or_default();
            let fmt = magpie_core::projects::TaskFormat::for_file(&rel);
            let mut file_tasks: Vec<Task> = if fmt == magpie_core::projects::TaskFormat::Markdown {
                let note = magpie_core::notes::Note {
                    id: next_id as i64,
                    name: rel.clone(),
                    is_daily: false,
                    body: content,
                    created_at_ms: 0,
                    updated_at_ms: 0,
                    source_app_id: None,
                    source_entry_id: None,
                };
                magpie_app::tasks::parse_tasks_in(&note, now)
            } else {
                // org / todo.txt: basic tasks (no inline prio/due parsing).
                magpie_core::projects::parse_task_lines(&content, fmt)
                    .into_iter()
                    .map(|p| Task {
                        note_id: next_id as i64,
                        note_name: rel.clone(),
                        line_index: p.line,
                        done: p.done,
                        status: if p.done { Status::Done } else { Status::Todo },
                        title: p.title,
                        priority: magpie_app::tasks::Priority::None,
                        due_ms: None,
                        due_time_min: None,
                        recur: None,
                        bookmarked: false,
                        project: None,
                        source_app_id: None,
                        source_entry_id: None,
                    })
                    .collect()
            };
            for ft in &mut file_tasks {
                ft.project = Some(h.name.clone());
                ft.note_name = format!("⇄ {rel}");
            }
            if !file_tasks.is_empty() {
                refs.insert(
                    next_id,
                    magpie_app::app_state::ProjTaskRef {
                        root: h.path.clone(),
                        rel: rel.clone(),
                        file_abs: file_abs.clone(),
                    },
                );
                tasks.append(&mut file_tasks);
            }
            next_id -= 1;
        }
    }
    drop(g);
    if let Ok(mut m) = state.project_task_refs.lock() {
        *m = refs;
    }
    tasks
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
    // Free-text filter from the Tasks search box (task title).
    let query = ui.get_task_search().to_string().trim().to_lowercase();
    // Project file-tasks (computed before the store lock — it locks project_scan).
    let proj_tasks = project_file_tasks(state, now);
    // The running timer (set inside the lock, reused for the header below).
    let active;
    let rows: Vec<TaskRow> = {
        let store = match state.store.lock() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };
        active = store.active_timer().ok().flatten();
        let mut tasks = magpie_app::tasks::all_tasks(&store, now);
        tasks.extend(proj_tasks);
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
            .filter(|t| query.is_empty() || t.title.to_lowercase().contains(&query))
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
        query: ui.get_task_search().to_string(),
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
    // Project file-tasks (before the store lock — locks project_scan).
    let proj_tasks = project_file_tasks(state, now);
    let (columns, projects): (Vec<BoardColumn>, Vec<SharedString>) = {
        let store = match state.store.lock() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };
        let mut all = magpie_app::tasks::all_tasks(&store, now);
        all.extend(proj_tasks);
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
    let task_total: i32 = columns.iter().map(|c| c.count).sum();
    let project_count = projects.len();
    ui.set_board(ModelRc::new(VecModel::from(columns)));
    ui.set_board_projects(ModelRc::new(VecModel::from(projects)));
    let ptxt = if project_count == 1 {
        "project"
    } else {
        "projects"
    };
    let ttxt = if task_total == 1 { "task" } else { "tasks" };
    ui.set_board_caption(SharedString::from(format!(
        "{task_total} {ttxt} across {project_count} {ptxt}"
    )));
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

/// Size + position the popover before showing it: apply the persisted panel size,
/// then dock a pinned panel to the right screen edge (below the menu bar) or, when
/// unpinned, anchor it under the tray icon (`tray_anchor`, physical px). Pass
/// `tray_anchor = None` to reposition in place (e.g. when toggling the pin).
fn present_popover(popover: &Popover, state: &AppState, tray_anchor: Option<(i32, i32)>) {
    let (w, h) = state.popover_size.lock().map(|g| *g).unwrap_or((360, 440));
    popover
        .window()
        .set_size(slint::LogicalSize::new(w as f32, h as f32));
    let pinned = state
        .popover_pinned
        .load(std::sync::atomic::Ordering::Relaxed);
    if pinned {
        if let Some((sw, sh, menubar)) = magpie_platform::main_screen_size() {
            let (x, y) = magpie_app::panel::dock_right((sw, sh), (w, h), menubar);
            popover
                .window()
                .set_position(slint::LogicalPosition::new(x as f32, y as f32));
            return;
        }
    }
    if let Some((tx, ty)) = tray_anchor {
        popover
            .window()
            .set_position(slint::PhysicalPosition::new(tx, ty));
    }
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
    // Shared task search box (same free-text filter the List and Board tabs use).
    let query = ui.get_task_search().to_string().trim().to_lowercase();
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
            .filter(|t| query.is_empty() || t.title.to_lowercase().contains(&query))
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
const ACTIONS: &[(&str, &str, &str, &str, &str)] = &[
    ("paste", "", "Paste", "⏎", "Entry"),
    ("keep", "", "Paste & keep open", "⌘⏎", "Entry"),
    ("copy", "", "Copy without pasting", "⌘C", "Entry"),
    ("edit", "", "Edit", "⌘E", "Entry"),
    ("pin", "", "Pin / Unpin", "⌘P", "Entry"),
    ("favorite", "", "Add to favorites", "⌘D", "Entry"),
    ("slot", "", "Assign quick-paste slot…", "⌘S", "Entry"),
    ("merge", "", "Add to merge", "⌘G", "Entry"),
    ("note", "", "New note from this entry", "⌘J", "Entry"),
    ("snippet", "", "New snippet", "⌘N", "Entry"),
    ("transform", "", "Transform…", "⌘T", "Convert"),
    ("open-as-table", "", "Open as table", "", "Convert"),
    ("save-secret", "", "Save as secret", "", "Convert"),
    ("bookmark", "", "Bookmark this link", "", "Convert"),
    ("task-from-link", "", "Create task from link", "", "Convert"),
    ("export", "", "Export data…", "", "Convert"),
    ("theme", "", "Theme…", "", "Convert"),
    ("delete", "", "Delete entry", "⌘⌫", ""),
];

/// The ⌘K rows whose label matches `query` (case-insensitive substring; "" =
/// all), in declaration order. Pure, so the match logic is unit-testable.
#[allow(clippy::type_complexity)]
fn filter_actions(
    query: &str,
) -> Vec<&'static (
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
)> {
    let q = query.to_lowercase();
    ACTIONS
        .iter()
        .filter(|(_, _, label, _, _)| q.is_empty() || label.to_lowercase().contains(&q))
        .collect()
}

/// The Filter popover's Match control (screen 02): 0 = Words, 1 = Fuzzy, 2 = Regex.
fn match_mode_from_index(i: i32) -> magpie_core::SearchMode {
    match i {
        1 => magpie_core::SearchMode::Fuzzy,
        2 => magpie_core::SearchMode::Regex,
        _ => magpie_core::SearchMode::Word,
    }
}

fn match_mode_index(m: magpie_core::SearchMode) -> i32 {
    match m {
        magpie_core::SearchMode::Fuzzy => 1,
        magpie_core::SearchMode::Regex => 2,
        _ => 0,
    }
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
        .map(|(id, icon, label, key, group)| ActionItem {
            icon: SharedString::from(*icon),
            label: SharedString::from(*label),
            key: SharedString::from(*key),
            id: SharedString::from(*id),
            group: SharedString::from(*group),
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

/// Open a file `path` in its preferred application: the one configured for the
/// path's category (`open -a <App>`) if set, else the system default (`open`).
fn open_path_with_app(state: &AppState, path: &str) {
    let category = magpie_core::openwith::category_for_path(path);
    match state.open_with.get(category) {
        Some(app) if !app.is_empty() => {
            let _ = std::process::Command::new("open")
                .arg("-a")
                .arg(app)
                .arg(path)
                .spawn();
        }
        _ => {
            let _ = std::process::Command::new("open").arg(path).spawn();
        }
    }
}

/// Open the selected clip in its default / configured app: a URL in the browser, an
/// image entry's stored bitmap in the image app, a file-path clip (or a `File`
/// entry, each path) in the per-category app. Hides the launcher first.
fn open_selected_default(ui: &LauncherWindow, state: &AppState) {
    let idx = ui.get_selected();
    let results = current_results(state, now_ms());
    let Some(e) = (idx >= 0).then(|| results.get(idx as usize)).flatten() else {
        return;
    };
    match e.kind {
        Kind::Link => {
            let url = e.full_text.trim().to_string();
            if url.starts_with("http://") || url.starts_with("https://") {
                open_url_and_hide(ui, &url);
            }
            return;
        }
        Kind::Image => {
            // The stored bitmap (a .bin of PNG bytes) — macOS `open` detects it.
            if let Some(bin) = e.image_path.as_deref() {
                hide_launcher(ui);
                open_path_with_app(state, bin);
                return;
            }
        }
        Kind::File => {
            hide_launcher(ui);
            for line in e.full_text.lines().filter(|l| !l.trim().is_empty()) {
                open_path_with_app(state, line.trim());
            }
            return;
        }
        _ => {}
    }
    // A plain-text clip that is actually a filesystem path.
    if magpie_core::openwith::looks_like_path(&e.full_text) {
        let path = magpie_core::openwith::category_for_path(&e.full_text);
        let _ = path;
        let expanded = magpie_app::file_search::expand_tilde(e.full_text.trim());
        hide_launcher(ui);
        open_path_with_app(state, &expanded.to_string_lossy());
    }
}

/// The Enter / ⌘Enter flow: copy the entry at `idx`, hide Magpie so the previous
/// app regains focus, then paste into it. `keep_open` re-shows Magpie afterward.
/// If `entry` is a promoted secret (its history text is masked), return a copy whose
/// `full_text` is the real value from the vault and register that value for one-time
/// capture suppression (so pasting it doesn't re-enter history as plaintext). Otherwise
/// returns the entry unchanged.
fn resolve_secret_entry(state: &AppState, entry: &Entry) -> Entry {
    let store = state.store.lock().unwrap_or_else(|e| e.into_inner());
    if let Ok(Some(sec)) = store.secret_for_entry(entry.id) {
        drop(store);
        let hash = magpie_core::model::content_hash(&Content::Text(sec.value.clone()));
        state
            .suppress_capture
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(hash);
        let mut e = entry.clone();
        e.full_text = sec.value;
        e
    } else {
        entry.clone()
    }
}

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
    // A promoted secret's history text is masked — paste the REAL value from the vault,
    // and suppress re-capturing that plaintext back into clipboard history.
    let resolved = resolve_secret_entry(state, entry);
    if let Ok(mut clip) = magpie_platform::platform_clipboard() {
        // Copy only here; the keystroke is sent after the window hides.
        let _ = perform_paste(
            &mut clip,
            &EnigoPaster,
            &resolved,
            PasteKind::Formatted,
            false,
        );
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
            // Capture pause (sidebar Pause button): skip polling entirely, but keep
            // the loop alive so resuming is instant.
            if state.paused.load(std::sync::atomic::Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(300));
                continue;
            }
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

/// Live project-file watcher (Phase 18b): watches each syncable project's task-file
/// directories with filesystem events (`notify`, non-recursive so it never crawls a
/// whole root) and refreshes the Projects view when a file changes on disk. The
/// watch set is re-synced from the latest scan every couple of seconds, so opening or
/// ignoring projects updates what's watched. Idle (watches nothing) until the user
/// has opened Projects at least once.
/// In-app ("While open") job scheduler (Phase 21): wakes each minute and runs the
/// enabled non-OS-managed jobs that are due this minute, in the user's home timezone.
/// OS-managed jobs fire via the OS scheduler (which calls `magpie --run-job`), so they
/// are skipped here. Store locks are held only for the quick DB ops, never across a
/// job's execution.
fn spawn_job_scheduler(state: Arc<AppState>) {
    use std::panic::AssertUnwindSafe;
    std::thread::spawn(move || {
        let tz = state.world_clock_home.clone();
        let log = data_dir().join("logs").join("magpie.log");
        loop {
            // Sleep to just past the next minute boundary.
            let now = now_ms() / 1000;
            let to_next = 60 - (now % 60);
            std::thread::sleep(Duration::from_secs(to_next as u64 + 1));
            let step = std::panic::catch_unwind(AssertUnwindSafe(|| {
                let tick = now_ms() / 1000;
                let minute_start = tick - (tick % 60);
                let jobs = {
                    let st = state.store.lock().unwrap_or_else(|e| e.into_inner());
                    st.list_jobs().unwrap_or_default()
                };
                for job in jobs.into_iter().filter(|j| j.enabled && !j.os_managed) {
                    let Ok(cron) = magpie_core::cron::parse_cron(&job.cron) else {
                        continue;
                    };
                    let due = magpie_core::cron::next_runs_in_tz(&cron, minute_start - 1, &tz, 1)
                        .first()
                        .map(|&n| n < minute_start + 60)
                        .unwrap_or(false);
                    if !due {
                        continue;
                    }
                    // Verify + start under the lock, execute WITHOUT it, finish under it.
                    let vjob = {
                        let st = state.store.lock().unwrap_or_else(|e| e.into_inner());
                        st.job_for_run(job.id).ok().flatten()
                    };
                    let Some(vjob) = vjob else { continue };
                    let run = {
                        let st = state.store.lock().unwrap_or_else(|e| e.into_inner());
                        st.start_job_run(vjob.id, now_ms()).ok()
                    };
                    let res = magpie_app::job_runner::execute_job(&vjob);
                    if let Some(run) = run {
                        let st = state.store.lock().unwrap_or_else(|e| e.into_inner());
                        let _ = st.finish_job_run(
                            run,
                            vjob.id,
                            now_ms(),
                            res.exit_code,
                            &res.stdout,
                            &res.stderr,
                            20,
                        );
                    }
                    let ok = res.exit_code == Some(0)
                        || matches!(vjob.kind, magpie_core::jobs::JobKind::Notification);
                    if !ok && vjob.notify_on_failure {
                        magpie_platform::notify(
                            &format!("Magpie job failed: {}", vjob.name),
                            &res.exit_code
                                .map(|c| format!("exit {c}"))
                                .unwrap_or_else(|| "did not complete".to_string()),
                        );
                    }
                }
            }));
            if step.is_err() {
                magpie_app::diagnostics::log_line(&log, "job scheduler tick panicked; continuing");
            }
        }
    });
}

fn spawn_project_watcher(state: Arc<AppState>, weak: slint::Weak<LauncherWindow>) {
    use notify::{EventKind, RecursiveMode, Watcher};
    std::thread::spawn(move || {
        let log_path = data_dir().join("logs").join("magpie.log");
        let (tx, rx) = std::sync::mpsc::channel();
        let mut watcher = match notify::recommended_watcher(move |res| {
            let _ = tx.send(res);
        }) {
            Ok(w) => w,
            Err(e) => {
                magpie_app::diagnostics::log_line(
                    &log_path,
                    &format!("project watcher init failed: {e}"),
                );
                return;
            }
        };
        let mut watched: std::collections::HashSet<std::path::PathBuf> = Default::default();
        loop {
            // Re-sync the watch set from the current syncable projects.
            let desired: std::collections::HashSet<std::path::PathBuf> = {
                let g = state.project_scan.lock().unwrap_or_else(|e| e.into_inner());
                let mut dirs = std::collections::HashSet::new();
                for h in g.hits.iter().filter(|h| h.syncable()) {
                    dirs.insert(h.path.clone());
                    let magpie = h.path.join(".magpie");
                    if magpie.is_dir() {
                        dirs.insert(magpie);
                    }
                }
                dirs
            };
            for d in desired.difference(&watched).cloned().collect::<Vec<_>>() {
                if watcher.watch(&d, RecursiveMode::NonRecursive).is_ok() {
                    watched.insert(d);
                }
            }
            for d in watched.difference(&desired).cloned().collect::<Vec<_>>() {
                let _ = watcher.unwatch(&d);
                watched.remove(&d);
            }

            // Wait for an event, timing out so the watch set is re-synced periodically.
            match rx.recv_timeout(Duration::from_secs(2)) {
                Ok(Ok(ev)) => {
                    if !matches!(
                        ev.kind,
                        EventKind::Modify(_) | EventKind::Create(_) | EventKind::Remove(_)
                    ) {
                        continue;
                    }
                    // Coalesce a burst of events (editor save = write + rename + chmod).
                    while rx.recv_timeout(Duration::from_millis(200)).is_ok() {}
                    let w = weak.clone();
                    let s = state.clone();
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(ui) = w.upgrade() {
                            // Only re-scan when the Projects view is showing, so an
                            // unrelated background save never triggers a root walk.
                            if ui.get_section().as_str() == "projects" {
                                refresh_projects(&ui, &s, true);
                            }
                        }
                    });
                }
                Ok(Err(e)) => {
                    magpie_app::diagnostics::log_line(
                        &log_path,
                        &format!("project watch error: {e}"),
                    );
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
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
            let st = state.clone();
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = w.upgrade() {
                    // Reset to the list first so each step starts from a known state.
                    ui.set_mode(SharedString::from("list"));
                    ui.set_view(SharedString::from("list"));
                    match step.as_str() {
                        // Seed a few favorites/quicklinks so the sidebar group shows.
                        "favorites" => {
                            if let Ok(store) = st.store.lock() {
                                let _ = store.add_favorite(
                                    "quicklink",
                                    "https://github.com/search?q={query}",
                                    "GitHub search",
                                    "gh",
                                    now_ms(),
                                );
                                let _ = store.add_favorite(
                                    "bookmark",
                                    "https://github.com/MarkusSagen/magpie",
                                    "magpie on GitHub",
                                    "",
                                    now_ms(),
                                );
                            }
                            refresh(&ui, &st);
                        }
                        "quicklink" => {
                            if let Ok(store) = st.store.lock() {
                                let _ = store.add_favorite(
                                    "quicklink",
                                    "https://github.com/search?q={query}",
                                    "GitHub search",
                                    "gh",
                                    now_ms(),
                                );
                            }
                            let q = SharedString::from("gh rust clipboard");
                            ui.set_query(q.clone());
                            ui.invoke_search_changed(q);
                        }
                        "actions" => ui.invoke_open_actions(),
                        "filters" => ui.invoke_open_search(),
                        // Filter popover with an active time filter (chip + badge visible).
                        "filters-demo" => {
                            ui.invoke_set_time_filter(2);
                            ui.invoke_open_search();
                        }
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
                        // Transforms (⌘T) palette with a representative input so the
                        // result pane is populated for screenshots.
                        "transform" => {
                            ui.set_transform_input(SharedString::from(
                                "banana\napple\napple\ncherry\n",
                            ));
                            ui.set_transform_selected(0);
                            ui.set_mode(SharedString::from("transform"));
                        }
                        // Case conversion via ⌘T (Phase 13): snake_case of an identifier.
                        "transform-case" => {
                            ui.set_transform_input(SharedString::from("parseHTTPResponse value"));
                            ui.set_transform_selected(11); // snake_case
                            ui.set_mode(SharedString::from("transform"));
                        }
                        "notes" => ui.invoke_set_mode_notes(true),
                        // Notes v2 rich preview (Phase 14): seed a note exercising
                        // headings, checkboxes, task tokens, wiki/url chips, code
                        // and a blockquote, then open it (preview is the default).
                        "notes-rich" => {
                            let body = "# Magpie roadmap\n\n## This week\n\n- [x] Split launcher.slint into components\n- [ ] Graphite tokens and palette !high @mon #ui\n- [ ] Shared search bar component @fri #ui +1w\n- [ ] Favicons and titles for links #clipboard\n\n## Notes\n\n- Design lives in [[Graphite redesign]] and on the canvas\n- Repo https://github.com/MarkusSagen/magpie\n- Run the gate before every commit: `cargo test --workspace`\n\n> One accent, one job.";
                            if let Ok(store) = st.store.lock() {
                                if let Ok(n) = store.upsert_note_by_name("Magpie roadmap", now_ms())
                                {
                                    let _ = store.update_note_body(n.id, body, now_ms());
                                    drop(store);
                                    ui.invoke_set_mode_notes(true);
                                    ui.set_note_id(n.id as i32);
                                }
                            }
                            refresh_notes(&ui, &st);
                        }
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
                        // Inline answer card: type a date expression into the search box.
                        "answer" => {
                            let q = SharedString::from("next monday + (195-17)d");
                            ui.set_query(q.clone());
                            ui.invoke_search_changed(q);
                        }
                        // Random generator (Phase 13): a password request → candidate
                        // chips (Tab re-rolls, ↵ pastes the selected one).
                        "random" => {
                            let q = SharedString::from("password 24");
                            ui.set_query(q.clone());
                            ui.invoke_search_changed(q);
                        }
                        // Date answer + calendar (Phase 11): a holiday countdown.
                        "calendar" => {
                            let q = SharedString::from("days until christmas");
                            ui.set_query(q.clone());
                            ui.invoke_search_changed(q);
                        }
                        // Local projects (Phase 18): seed fixture project folders, open Projects.
                        "projects" => {
                            let base = std::env::temp_dir().join("magpie-proj-demo");
                            let write = |rel: &str, body: &str| {
                                let p = base.join(rel);
                                if let Some(parent) = p.parent() {
                                    let _ = std::fs::create_dir_all(parent);
                                }
                                let _ = std::fs::write(p, body);
                            };
                            write("magpie/TODO.md", "# magpie\n\n- [ ] Task card redesign !high @mon #ui\n- [ ] Link favicons + titles #clipboard\n- [x] Split launcher.slint\n");
                            write("magpie/.magpie/tasks.md", "- [ ] Decide tz boundary data source !med\n- [ ] Wordlist license check\n");
                            write("magpie/.git/HEAD", "ref: refs/heads/graphite/p9b\n");
                            // Phase 18b showcase: project.toml (name+tags), notes, code comments.
                            write(
                                "magpie/.magpie/project.toml",
                                "name = \"magpie\"\ntags = [\"hobby\", \"rust\"]\n",
                            );
                            write(
                                "magpie/.magpie/notes/architecture.md",
                                "# Architecture\n\nlocal-first, three crates.\n",
                            );
                            write(
                                "magpie/.magpie/notes/ideas.md",
                                "# Ideas\n\n- split-screen quick paste\n",
                            );
                            write("magpie/src/main.rs", "fn main() {\n    // TODO: wire the sync watcher into projects\n    let _ = 1; // FIXME: handle CRLF on Windows\n}\n");
                            write(
                                "dotfiles/TODO.md",
                                "- [ ] tidy zshrc\n- [ ] prune brew\n- [ ] backup gpg keys\n",
                            );
                            write("dotfiles/.git/HEAD", "ref: refs/heads/main\n");
                            write("blog/.magpie/tasks.md", "- [ ] draft org-mode post\n- [ ] fix RSS\n- [ ] alt text pass\n- [ ] deploy\n- [ ] newsletter\n");
                            write("blog/.git/HEAD", "ref: refs/heads/main\n");
                            write(
                                "Home renovation/.magpie/tasks.md",
                                "- [ ] call plumber\n- [ ] paint samples\n- [ ] measure kitchen\n",
                            );
                            write("rust-playground/TODO.md", "- [ ] try axum\n");
                            let base_s = base.to_string_lossy().to_string();
                            let (hits, snapshots) =
                                magpie_app::projects::scan(&[base_s], 4, true, &[]);
                            if let Ok(mut g) = st.project_scan.lock() {
                                g.hits = hits;
                                g.file_snapshots = snapshots;
                            }
                            // Demo conflict so the Review banner renders (PNG 25).
                            if let Ok(mut cs) = st.project_conflicts.lock() {
                                let root = base.join("magpie");
                                cs.clear();
                                cs.push(magpie_app::app_state::ProjConflict {
                                    root: root.clone(),
                                    rel: "TODO.md".into(),
                                    file_abs: root.join("TODO.md"),
                                    line: 14,
                                    base: String::new(),
                                    mine: String::new(),
                                    disk: String::new(),
                                    summary: "Line 14 changed in Magpie and on disk — \u{201c}Link favicons\u{201d} vs \u{201c}Link favicons + titles\u{201d}".into(),
                                });
                            }
                            ui.set_section(SharedString::from("projects"));
                            ui.set_view(SharedString::from("projects"));
                            // Select the conflicted "magpie" project so its banner shows.
                            ui.global::<ProjectsStore>().set_selected(3);
                            populate_projects(&ui, &st);
                        }
                        // Time tracking v2 (Phase 17): seed a running timer, open Time tab.
                        "time" => {
                            if let Ok(store) = st.store.lock() {
                                let base = now_ms();
                                // A couple of finished entries + a running one.
                                let _ = store.start_timer(
                                    "Standup #Studio555",
                                    "Standup #Studio555",
                                    None,
                                    base - 9_000_000,
                                );
                                let _ = store.stop_active(base - 8_100_000);
                                let _ = store.start_timer(
                                    "Phase 9 — data import #magpie",
                                    "Phase 9 — data import #magpie",
                                    None,
                                    base - 7_000_000,
                                );
                                let _ = store.stop_active(base - 200_000);
                                let _ = store.start_timer(
                                    "Graphite 9b — task card redesign #magpie #ui",
                                    "Graphite 9b — task card redesign #magpie #ui",
                                    None,
                                    base - 2_520_000,
                                );
                            }
                            ui.invoke_set_section(SharedString::from("tasks"));
                            ui.invoke_set_task_view(SharedString::from("time"));
                        }
                        // Schedule (Phase 21): seed a few demo jobs, open the Schedule tab.
                        "schedule" => {
                            use magpie_core::jobs::JobKind;
                            if let Ok(store) = st.store.lock() {
                                if store.list_jobs().map(|v| v.is_empty()).unwrap_or(false) {
                                    let now = now_ms();
                                    let a = store
                                        .add_job(
                                            "Morning standup ping",
                                            JobKind::Notification,
                                            "Time for standup",
                                            "40 8 * * 1-5",
                                            "",
                                            300,
                                            true,
                                            false,
                                            now,
                                        )
                                        .unwrap_or(0);
                                    let r = store.start_job_run(a, now - 3_600_000).unwrap_or(0);
                                    let _ = store.finish_job_run(
                                        r,
                                        a,
                                        now - 3_600_000,
                                        Some(0),
                                        "",
                                        "",
                                        20,
                                    );
                                    let b = store
                                        .add_job(
                                            "Nightly backup",
                                            JobKind::Shell,
                                            "rsync -a ~/code ~/backup",
                                            "0 2 * * *",
                                            "",
                                            600,
                                            true,
                                            true,
                                            now,
                                        )
                                        .unwrap_or(0);
                                    let r = store.start_job_run(b, now - 80_000_000).unwrap_or(0);
                                    let _ = store.finish_job_run(
                                        r,
                                        b,
                                        now - 80_000_000,
                                        Some(1),
                                        "",
                                        "rsync: connection refused",
                                        20,
                                    );
                                    let _ = store.add_job(
                                        "Open dashboard",
                                        JobKind::Url,
                                        "https://grafana.local",
                                        "0 9 * * 1",
                                        "",
                                        60,
                                        false,
                                        false,
                                        now,
                                    );
                                }
                            }
                            ui.invoke_set_section(SharedString::from("tasks"));
                            ui.invoke_set_task_view(SharedString::from("schedule"));
                        }
                        // File search (Phase 15): search the repo for a term.
                        "files" => {
                            {
                                let mut g = st.search.lock().unwrap_or_else(|e| e.into_inner());
                                g.locations = vec!["~/code/hobby/magpie/crates".to_string()];
                            }
                            let q = SharedString::from("user_version");
                            ui.set_query(q.clone());
                            if let Ok(mut u) = st.ui.lock() {
                                u.text = "user_version".to_string();
                            }
                            ui.invoke_set_section(SharedString::from("files"));
                        }
                        // Snippets (Phase 16a): seed a starter set, open the view.
                        "snippets" => {
                            if let Ok(store) = st.store.lock() {
                                if store.list_snippets().map(|s| s.is_empty()).unwrap_or(false) {
                                    let seed: &[(&str, &str, &str, &str)] = &[
                                        ("Meeting notes", ";meet", "## {date:%a %d %b} — {input:Meeting with}\n\n**Attendees:** {cursor}\n**Notes**\n-\n**Action items**\n- [ ]", "snippet"),
                                        ("Today's date", ";today", "{date:%A, %d %B %Y}", "snippet"),
                                        ("Email signature", ";sig", "Best,\nMarkus", "snippet"),
                                        ("Shrug", ";shrug", "¯\\_(ツ)_/¯", "snippet"),
                                        ("Wrap clipboard in code fence", ";code", "```\n{clipboard}\n```", "snippet"),
                                        ("git: undo last commit, keep changes", "", "git reset --soft HEAD~1", "command"),
                                        ("docker: remove stopped containers", "", "docker container prune -f", "command"),
                                    ];
                                    for (name, kw, body, kind) in seed {
                                        let _ = store.add_snippet(name, kw, body, kind, now_ms());
                                    }
                                }
                            }
                            ui.invoke_set_section(SharedString::from("snippets"));
                        }
                        "secrets" => {
                            if let Ok(store) = st.store.lock() {
                                if store.list_secrets().map(|s| s.is_empty()).unwrap_or(false) {
                                    let seed: &[(&str, &str, &str)] = &[
                                        ("STRIPE_API_KEY", "sk-live-4eC39Hq...", "prod"),
                                        ("DB_PASSWORD", "hunter2!", ""),
                                        ("GITHUB_TOKEN", "ghp_0123456789abcdef", ""),
                                    ];
                                    for (k, v, n) in seed {
                                        let _ = store.add_secret(k, v, n, None, now_ms());
                                    }
                                }
                            }
                            ui.invoke_set_section(SharedString::from("secrets"));
                        }
                        // World clock (Phase 12): Calculator → World clock tab.
                        "worldclock" => {
                            let q = SharedString::from("time in tokyo");
                            ui.set_query(q.clone());
                            ui.invoke_search_changed(q);
                            ui.invoke_set_section(SharedString::from("calculator"));
                            ui.global::<CalcStore>().invoke_open_worldclock();
                        }
                        // Data view: the typed grid (13), optionally with the chart (14).
                        "data" => {
                            ui.invoke_set_section(SharedString::from("data"));
                        }
                        "data-chart" => {
                            ui.invoke_set_section(SharedString::from("data"));
                            ui.global::<DataStore>().invoke_chart_toggle();
                        }
                        // Calculator history (11): seed a query so the answer card shows.
                        "calculator" => {
                            ui.invoke_set_section(SharedString::from("calculator"));
                            let q = SharedString::from("17% of 2340 + 3/8");
                            ui.set_query(q.clone());
                            ui.invoke_search_changed(q);
                            ui.global::<CalcStore>().invoke_commit();
                        }
                        // Calculator typesetting (Phase 8): a LaTeX expression.
                        "calc-latex" => {
                            ui.invoke_set_section(SharedString::from("calculator"));
                            let q = SharedString::from("\\sum_{k=1}^{10} k^2 + \\frac{3}{8}");
                            ui.set_query(q.clone());
                            ui.invoke_search_changed(q);
                        }
                        // Calculator graphs (12): plot sin(x) = x/3.
                        "calc-graph" => {
                            ui.invoke_set_section(SharedString::from("calculator"));
                            let q = SharedString::from("sin(x) = x/3");
                            ui.set_query(q.clone());
                            ui.invoke_search_changed(q);
                            ui.global::<CalcStore>().invoke_commit();
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
    Scratch,
}

/// A short title for an Earlier-scratch row: its first non-empty line.
fn scratch_title(body: &str) -> String {
    let first = body
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim();
    if first.is_empty() {
        "Empty scratch".to_string()
    } else {
        first.chars().take(60).collect()
    }
}

/// Load the shown scratch (body, pin, Earlier list) into the window and show the Now
/// tab. Resolves the shown note: the last shown if it still exists, else the newest
/// scratch (creating an empty one if there are none).
fn refresh_scratch(sc: &ScratchWindow, state: &AppState) {
    let now = now_ms();
    let prev = *state
        .scratch_shown_id
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let (id, body, pinned, earlier) = {
        let store = state.store.lock().unwrap_or_else(|e| e.into_inner());
        let note = (prev >= 0)
            .then_some(prev)
            .and_then(|id| store.get_note(id).ok().flatten())
            .or_else(|| store.scratch_current(now).ok());
        let note = match note {
            Some(n) => n,
            None => return,
        };
        let pinned = store.scratch_is_pinned(note.id).unwrap_or(false);
        let earlier: Vec<ScratchRow> = store
            .scratch_earlier(60)
            .unwrap_or_default()
            .into_iter()
            .map(|n| ScratchRow {
                id: n.id as i32,
                title: SharedString::from(scratch_title(&n.body)),
                when: SharedString::from(relative_time(n.updated_at_ms, now)),
                pinned: store.scratch_is_pinned(n.id).unwrap_or(false),
            })
            .collect();
        (note.id, note.body, pinned, earlier)
    };
    *state
        .scratch_shown_id
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = id;
    let ss = sc.global::<ScratchStore>();
    ss.set_body(SharedString::from(body));
    ss.set_pinned(pinned);
    ss.set_earlier(ModelRc::new(VecModel::from(earlier)));
    ss.set_tab(0);
    ss.set_filing(false);
}

/// Show the scratch window (loading the current scratch) or hide it, returning focus
/// to the previous app on hide.
fn toggle_scratch(sc: &ScratchWindow, state: &AppState) {
    if sc.window().is_visible() {
        let _ = sc.hide();
        magpie_platform::hide_and_yield_focus();
    } else {
        refresh_scratch(sc, state);
        sc.window().set_size(slint::LogicalSize::new(720.0, 520.0));
        let _ = sc.show();
        magpie_platform::raise_to_front();
    }
}

/// The non-empty lines of the scratch body (what "File all" acts on).
fn scratch_lines(body: &str) -> Vec<String> {
    body.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.to_string())
        .collect()
}

/// Ensure a line is a Markdown task (`- [ ] …`) for task destinations.
fn as_task_line(line: &str) -> String {
    let t = line.trim_start();
    if t.starts_with("- [ ] ") || t.starts_with("- [x] ") || t.starts_with("- [/] ") {
        line.to_string()
    } else {
        format!("- [ ] {}", line.trim())
    }
}

/// The "File N lines to…" targets (Phase 20), in the fixed order the menu renders.
fn scratch_file_targets(state: &AppState) -> Vec<FileTarget> {
    let proj = {
        let g = state.project_scan.lock().unwrap_or_else(|e| e.into_inner());
        g.hits.iter().find(|h| h.syncable()).map(|h| h.name.clone())
    };
    let append_note = {
        let store = state.store.lock().unwrap_or_else(|e| e.into_inner());
        store
            .recent_notes(30)
            .unwrap_or_default()
            .into_iter()
            .find(|n| !n.is_daily && !n.name.starts_with("scratch-"))
            .map(|n| n.name)
    };
    let today = abs_date(now_ms());
    let mk = |label: &str, detail: String, key: &str| FileTarget {
        label: SharedString::from(label),
        detail: SharedString::from(detail),
        key: SharedString::from(key),
    };
    vec![
        mk(
            &proj
                .as_ref()
                .map(|p| format!("Tasks in {p}"))
                .unwrap_or_else(|| "Tasks in project".to_string()),
            proj.clone().map(|_| String::new()).unwrap_or_default(),
            "1",
        ),
        mk("Tasks · Inbox", "no project".to_string(), "2"),
        mk(
            "Append to note…",
            append_note.clone().unwrap_or_default(),
            "3",
        ),
        mk("Today's daily note", today, "4"),
        mk("New note", String::new(), "5"),
        mk("Snippet", String::new(), "6"),
    ]
}

/// File `lines` to target index `idx` (see `scratch_file_targets`). Reuses the note /
/// snippet / project-file creation paths. Never runs anything; just writes text.
fn file_scratch_lines(state: &AppState, idx: i32, lines: &[String]) {
    let now = now_ms();
    let joined_tasks = lines
        .iter()
        .map(|l| as_task_line(l))
        .collect::<Vec<_>>()
        .join("\n");
    let joined_raw = lines.join("\n");
    let append_to_note = |name: &str, text: &str, as_tasks: bool| {
        let store = state.store.lock().unwrap_or_else(|e| e.into_inner());
        if let Ok(note) = store.upsert_note_by_name(name, now) {
            let mut body = note.body.clone();
            if !body.is_empty() && !body.ends_with('\n') {
                body.push('\n');
            }
            body.push_str(if as_tasks { &joined_tasks } else { text });
            body.push('\n');
            let _ = store.update_note_body(note.id, &body, now);
        }
    };
    match idx {
        0 => {
            // Tasks in the first syncable project: append to its primary task file.
            let target = {
                let g = state.project_scan.lock().unwrap_or_else(|e| e.into_inner());
                g.hits.iter().find(|h| h.syncable()).and_then(|h| {
                    h.tasks
                        .first()
                        .map(|t| (h.path.clone(), t.file_rel.clone(), t.file_abs.clone()))
                        .or_else(|| {
                            Some((
                                h.path.clone(),
                                ".magpie/tasks.md".to_string(),
                                h.path.join(".magpie/tasks.md"),
                            ))
                        })
                })
            };
            if let Some((root, rel, file_abs)) = target {
                if let Some(parent) = file_abs.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let mut content = std::fs::read_to_string(&file_abs).unwrap_or_default();
                if !content.is_empty() && !content.ends_with('\n') {
                    content.push('\n');
                }
                content.push_str(&joined_tasks);
                content.push('\n');
                if std::fs::write(&file_abs, &content).is_ok() {
                    let store = state.store.lock().unwrap_or_else(|e| e.into_inner());
                    let _ =
                        store.set_project_baseline(&root.to_string_lossy(), &rel, &content, now);
                }
            } else {
                append_to_note("Inbox", "", true);
            }
        }
        1 => append_to_note("Inbox", "", true),
        2 => {
            let name = {
                let store = state.store.lock().unwrap_or_else(|e| e.into_inner());
                store
                    .recent_notes(30)
                    .unwrap_or_default()
                    .into_iter()
                    .find(|n| !n.is_daily && !n.name.starts_with("scratch-"))
                    .map(|n| n.name)
            };
            append_to_note(
                &name.unwrap_or_else(|| "Notes".to_string()),
                &joined_raw,
                false,
            );
        }
        3 => {
            let store = state.store.lock().unwrap_or_else(|e| e.into_inner());
            if let Ok(note) = store.daily_note(&abs_date(now), now) {
                let mut body = note.body.clone();
                if !body.is_empty() && !body.ends_with('\n') {
                    body.push('\n');
                }
                body.push_str(&joined_tasks);
                body.push('\n');
                let _ = store.update_note_body(note.id, &body, now);
            }
        }
        4 => {
            let title = scratch_title(&joined_raw);
            let store = state.store.lock().unwrap_or_else(|e| e.into_inner());
            if let Ok(note) = store.upsert_note_by_name(&title, now) {
                let _ = store.update_note_body(note.id, &joined_raw, now);
            }
        }
        5 => {
            let name = scratch_title(&joined_raw);
            let store = state.store.lock().unwrap_or_else(|e| e.into_inner());
            let _ = store.add_snippet(&name, "", &joined_raw, "snippet", now);
        }
        _ => {}
    }
}

/// Wire the scratch window's callbacks (autosave, tabs, pin, new, dismiss, File-to).
fn wire_scratch(sc: &ScratchWindow, state: &Arc<AppState>) {
    let ss = sc.global::<ScratchStore>();
    {
        let s = state.clone();
        ss.on_edited(move |text| {
            let id = *s.scratch_shown_id.lock().unwrap_or_else(|e| e.into_inner());
            if id >= 0 {
                let store = s.store.lock().unwrap_or_else(|e| e.into_inner());
                let _ = store.update_note_body(id, &text, now_ms());
            }
        });
    }
    {
        let sw = sc.as_weak();
        ss.on_set_tab(move |t| {
            if let Some(sc) = sw.upgrade() {
                sc.global::<ScratchStore>().set_tab(t);
            }
        });
    }
    {
        let s = state.clone();
        let sw = sc.as_weak();
        ss.on_open_earlier(move |i| {
            if let Some(sc) = sw.upgrade() {
                let id = {
                    let store = s.store.lock().unwrap_or_else(|e| e.into_inner());
                    store
                        .scratch_earlier(60)
                        .ok()
                        .and_then(|v| v.get(i as usize).map(|n| n.id))
                };
                if let Some(id) = id {
                    *s.scratch_shown_id.lock().unwrap_or_else(|e| e.into_inner()) = id;
                    refresh_scratch(&sc, &s);
                }
            }
        });
    }
    {
        let s = state.clone();
        let sw = sc.as_weak();
        ss.on_toggle_pin(move || {
            if let Some(sc) = sw.upgrade() {
                let id = *s.scratch_shown_id.lock().unwrap_or_else(|e| e.into_inner());
                if id >= 0 {
                    let store = s.store.lock().unwrap_or_else(|e| e.into_inner());
                    let cur = store.scratch_is_pinned(id).unwrap_or(false);
                    let _ = store.set_scratch_pinned(id, !cur);
                    drop(store);
                    refresh_scratch(&sc, &s);
                }
            }
        });
    }
    {
        let s = state.clone();
        let sw = sc.as_weak();
        ss.on_new_scratch(move || {
            if let Some(sc) = sw.upgrade() {
                let id = {
                    let store = s.store.lock().unwrap_or_else(|e| e.into_inner());
                    store.new_scratch(now_ms()).map(|n| n.id).unwrap_or(-1)
                };
                *s.scratch_shown_id.lock().unwrap_or_else(|e| e.into_inner()) = id;
                refresh_scratch(&sc, &s);
            }
        });
    }
    {
        let sw = sc.as_weak();
        ss.on_dismiss(move || {
            if let Some(sc) = sw.upgrade() {
                let _ = sc.hide();
            }
            magpie_platform::hide_and_yield_focus();
        });
    }
    {
        let s = state.clone();
        let sw = sc.as_weak();
        ss.on_begin_file(move |_all| {
            if let Some(sc) = sw.upgrade() {
                let store = sc.global::<ScratchStore>();
                let lines = scratch_lines(&store.get_body());
                if lines.is_empty() {
                    return;
                }
                store.set_filing_count(lines.len() as i32);
                store.set_targets(ModelRc::new(VecModel::from(scratch_file_targets(&s))));
                store.set_filing(true);
            }
        });
    }
    {
        let sw = sc.as_weak();
        ss.on_cancel_file(move || {
            if let Some(sc) = sw.upgrade() {
                sc.global::<ScratchStore>().set_filing(false);
            }
        });
    }
    {
        let s = state.clone();
        let sw = sc.as_weak();
        ss.on_choose_target(move |idx, keep| {
            if let Some(sc) = sw.upgrade() {
                let lines = scratch_lines(&sc.global::<ScratchStore>().get_body());
                file_scratch_lines(&s, idx, &lines);
                sc.global::<ScratchStore>().set_filing(false);
                if keep {
                    refresh_scratch(&sc, &s);
                } else {
                    // Filed lines leave the scratch: clear the body + hide.
                    let id = *s.scratch_shown_id.lock().unwrap_or_else(|e| e.into_inner());
                    if id >= 0 {
                        let store = s.store.lock().unwrap_or_else(|e| e.into_inner());
                        let _ = store.update_note_body(id, "", now_ms());
                    }
                    refresh_scratch(&sc, &s);
                    let _ = sc.hide();
                    magpie_platform::hide_and_yield_focus();
                }
            }
        });
    }
}

/// Register launcher + quick-paste + scratch hotkeys and drain their events on a thread.
fn spawn_hotkeys(
    cfg: &Config,
    state: Arc<AppState>,
    weak: slint::Weak<LauncherWindow>,
    scratch: slint::Weak<ScratchWindow>,
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
    // Scratch pad hotkey (⌘⌥Space default). If it's already taken, fall back to the
    // tray "Scratch pad" item + ⌘K action and notify once.
    if let Ok(spec) = parse_hotkey(&cfg.scratch_hotkey) {
        match hk.register(&spec) {
            Ok(id) => {
                actions.insert(id, HotAction::Scratch);
            }
            Err(_) => {
                magpie_platform::notify(
                    "Magpie — scratch hotkey unavailable",
                    "That shortcut is already in use. Open the scratch pad from the tray menu, and rebind it in the config.",
                );
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
                    Some(HotAction::Scratch) => {
                        let sw = scratch.clone();
                        let s = state.clone();
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(sc) = sw.upgrade() {
                                toggle_scratch(&sc, &s);
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
    // A kill/pkill (e.g. install replacing a running copy), Ctrl-C, or logout marks the
    // session clean instead of looking like a crash on the next launch.
    diag::install_clean_exit_on_signal(&session_path);
    diag::log_line(&log_path, "startup");

    let cfg = magpie_app::config::load_or_default(&data_dir().join("config.toml"));
    let mut denylist = default_app_denylist();
    denylist.extend(cfg.app_denylist.clone());

    let state = build_state(&cfg);
    let ui = LauncherWindow::new().expect("create window");
    // Bundle the Graphite UI typefaces (Geist / Geist Mono, OFL) so typography is
    // identical on every OS. Registered after the backend is initialized (window
    // creation) via the fontique collection Slint uses for text rendering.
    {
        use slint::fontique_010::fontique;
        let mut fonts = slint::fontique_010::shared_collection();
        for bytes in [
            include_bytes!("../fonts/Geist.ttf").as_slice(),
            include_bytes!("../fonts/GeistMono.ttf").as_slice(),
            // JetBrainsMono Nerd Font Mono (OFL) — the app's monospace face. Its
            // Private-Use-Area glyphs let copied Nerd Font icons render instead of
            // tofu; ligatures + full coverage make it the code/preview font too.
            include_bytes!("../fonts/JetBrainsMonoNFM-Regular.ttf").as_slice(),
            include_bytes!("../fonts/JetBrainsMonoNFM-Bold.ttf").as_slice(),
        ] {
            let blob = fontique::Blob::new(std::sync::Arc::new(bytes.to_vec()));
            let _ = fonts.register_fonts(blob, None);
        }
    }
    let weak = ui.as_weak();
    // Apply the persisted theme once, at window creation — see the comment in
    // `show_window` for why this isn't repeated on every summon.
    let theme_idx = magpie_app::themes::index_of(&state.theme);
    ui.set_theme_index(theme_idx as i32);

    // Restore the persisted sidebar group-collapse state.
    {
        let collapsed = |k: &str| cfg.sidebar_collapsed_groups.iter().any(|g| g == k);
        ui.set_grp_capture_collapsed(collapsed("capture"));
        ui.set_grp_organize_collapsed(collapsed("organize"));
        ui.set_grp_tools_collapsed(collapsed("tools"));
        ui.set_grp_favorites_collapsed(collapsed("favorites"));
    }
    {
        ui.on_group_toggled(move |key, collapsed| {
            let cfg_path = data_dir().join("config.toml");
            let mut cfg = magpie_app::config::load_or_default(&cfg_path);
            let key = key.to_string();
            cfg.sidebar_collapsed_groups.retain(|g| g != &key);
            if collapsed {
                cfg.sidebar_collapsed_groups.push(key);
            }
            let _ = magpie_app::config::save(&cfg, &cfg_path);
        });
    }

    // The menubar popover: a second, chromeless, always-on-top window shown from
    // the tray left-click. Kept alive for the whole run alongside `ui` (dropping it
    // early would tear the window down). It's a separate top-level component with its
    // OWN `Theme` global, so its theme must be set + toggled independently of `ui`.
    // Created here (before the theme-toggle handler) so the toggle can update both.
    let popover = Popover::new().expect("create popover");
    popover.set_theme_index(theme_idx as i32);

    // The floating scratch pad (Phase 20): a third top-level window, always-on-top,
    // summoned by the scratch hotkey. Kept alive for the whole run; its own `Theme`
    // global is mirrored like the popover's.
    let scratch = ScratchWindow::new().expect("create scratch window");
    scratch.set_theme_index(theme_idx as i32);
    wire_scratch(&scratch, &state);
    wire_schedule(&ui, &state);
    init_settings_props(&ui, &state, &cfg);
    wire_settings(&ui, &state);
    popover.set_pinned(cfg.popover_pinned);

    // Match the native window chrome (titlebar / traffic-light strip) to the
    // theme, so a light theme isn't undercut by a dark OS titlebar.
    magpie_platform::set_appearance(magpie_app::themes::is_dark(theme_idx));

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
        let scw = scratch.as_weak();
        ui.on_set_theme(move |idx| {
            let count = magpie_app::themes::THEMES.len() as i32;
            let idx = idx.clamp(0, count - 1);
            if let Some(ui) = w.upgrade() {
                ui.set_theme_index(idx);
            }
            if let Some(p) = pw.upgrade() {
                p.set_theme_index(idx);
            }
            if let Some(sc) = scw.upgrade() {
                sc.set_theme_index(idx);
            }
            magpie_platform::set_appearance(magpie_app::themes::is_dark(idx as usize));
            let cfg_path = data_dir().join("config.toml");
            let mut cfg = magpie_app::config::load_or_default(&cfg_path);
            cfg.theme = magpie_app::themes::id_of(idx as usize).to_string();
            let _ = magpie_app::config::save(&cfg, &cfg_path);
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
    // Pin the quick-clipboard panel: dock it to the screen edge + persist so it
    // re-opens on the next launch. Unpinning leaves it where it is (transient again).
    {
        let p = popover.as_weak();
        let s = state.clone();
        popover.on_toggle_pin(move || {
            let now = !s.popover_pinned.load(std::sync::atomic::Ordering::Relaxed);
            s.popover_pinned
                .store(now, std::sync::atomic::Ordering::Relaxed);
            let cfg_path = data_dir().join("config.toml");
            let mut cfg = magpie_app::config::load_or_default(&cfg_path);
            cfg.popover_pinned = now;
            let _ = magpie_app::config::save(&cfg, &cfg_path);
            if let Some(p) = p.upgrade() {
                p.set_pinned(now);
                // Re-dock immediately when pinned; leave the position alone when
                // unpinning (no tray anchor here, so pass None).
                if now {
                    present_popover(&p, &s, None);
                }
            }
        });
    }
    // Resize grip: snapshot the size on press, resize live during the drag, persist
    // once on release.
    {
        let s = state.clone();
        popover.on_resize_start(move || {
            let cur = s.popover_size.lock().map(|g| *g).unwrap_or((360, 440));
            if let Ok(mut a) = s.popover_resize_anchor.lock() {
                *a = cur;
            }
        });
    }
    {
        let p = popover.as_weak();
        let s = state.clone();
        popover.on_resize_by(move |dx, dy| {
            let anchor = s
                .popover_resize_anchor
                .lock()
                .map(|a| *a)
                .unwrap_or((360, 440));
            let (w, h) = magpie_app::panel::clamp_size(anchor.0 + dx as i32, anchor.1 + dy as i32);
            if let Ok(mut sz) = s.popover_size.lock() {
                *sz = (w, h);
            }
            if let Some(p) = p.upgrade() {
                p.window()
                    .set_size(slint::LogicalSize::new(w as f32, h as f32));
            }
        });
    }
    {
        let s = state.clone();
        popover.on_resize_end(move || {
            let (w, h) = s.popover_size.lock().map(|g| *g).unwrap_or((360, 440));
            let cfg_path = data_dir().join("config.toml");
            let mut cfg = magpie_app::config::load_or_default(&cfg_path);
            cfg.popover_width = w as i64;
            cfg.popover_height = h as i64;
            let _ = magpie_app::config::save(&cfg, &cfg_path);
        });
    }
    // Transforms (⌘T) palette. `TransformOpts.run`/`.stats` are pure previews of the
    // selected transform applied to the input (join rows honour the Quote / Separator /
    // Wrap fields); `transform_action` pastes / copies / saves the result.
    {
        use magpie_core::transforms::{
            default_overrides, run_index_display_with, run_index_stats_with, JoinOverrides,
        };
        let opts = ui.global::<TransformOpts>();
        let ov = |q: SharedString, s: SharedString, w: SharedString| JoinOverrides {
            quote: q.into(),
            separator: s.into(),
            wrap: w.into(),
        };
        opts.on_run(move |idx, input, q, s, w| {
            let o = ov(q, s, w);
            run_index_display_with(idx as usize, input.as_str(), Some(&o)).into()
        });
        opts.on_stats(move |idx, input, q, s, w| {
            let o = ov(q, s, w);
            run_index_stats_with(idx as usize, input.as_str(), Some(&o)).into()
        });
        opts.on_default_opt(|idx, field| {
            default_overrides(idx as usize)
                .map(|o| match field {
                    0 => o.quote,
                    1 => o.separator,
                    _ => o.wrap,
                })
                .unwrap_or_default()
                .into()
        });
        opts.on_has_opts(|idx| default_overrides(idx as usize).is_some());
    }
    {
        ui.on_transform_input_summary(move |input| {
            SharedString::from(magpie_core::transforms::input_summary(input.as_str()))
        });
    }
    // ---- Date answer card (screen 15): Christmas eve/day region switch ----
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.global::<DateStore>().on_set_eve(move |i| {
            if let Some(ui) = w.upgrade() {
                populate_date_card(&ui, &s, i);
            }
        });
    }
    // ---- World clock (screen 16) ----
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.global::<CalcStore>().on_open_worldclock(move || {
            if let Some(ui) = w.upgrade() {
                ui.global::<CalcStore>().set_worldclock_open(true);
                refresh_worldclock(&ui, &s);
            }
        });
    }
    {
        ui.global::<CalcStore>().on_close_worldclock(move || {});
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.global::<WorldClockStore>().on_shift_hour(move |d| {
            let cur = {
                let epoch = now_ms() / 1000;
                let stored = s.wc_chosen_hour.lock().map(|h| *h).unwrap_or(-1);
                if stored < 0 {
                    magpie_core::worldclock::home_current_hour(&s.world_clock_home, epoch) as i32
                } else {
                    stored
                }
            };
            let next = (cur + d).rem_euclid(24);
            if let Ok(mut h) = s.wc_chosen_hour.lock() {
                *h = next;
            }
            if let Some(ui) = w.upgrade() {
                refresh_worldclock(&ui, &s);
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.global::<WorldClockStore>().on_set_hour(move |h| {
            if let Ok(mut c) = s.wc_chosen_hour.lock() {
                *c = h.clamp(0, 23);
            }
            if let Some(ui) = w.upgrade() {
                refresh_worldclock(&ui, &s);
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.global::<WorldClockStore>().on_add_city(move || {
            // Add the currently-typed city (if it resolves) to the persisted list.
            let query = w
                .upgrade()
                .map(|ui| ui.get_query().to_string())
                .unwrap_or_default();
            let name = query.trim();
            if let Some(tz) = magpie_core::worldclock::resolve_zone(name) {
                let id = tz.name().to_string();
                if let Ok(mut c) = s.world_clock_cities.lock() {
                    if !c.contains(&id) {
                        c.push(id);
                        let cfg_path = data_dir().join("config.toml");
                        let mut cfg = magpie_app::config::load_or_default(&cfg_path);
                        cfg.world_clock_cities = c.clone();
                        let _ = magpie_app::config::save(&cfg, &cfg_path);
                    }
                }
                if let Some(ui) = w.upgrade() {
                    refresh_worldclock(&ui, &s);
                }
            }
        });
    }
    {
        let w = ui.as_weak();
        ui.global::<WorldClockStore>().on_copy_summary(move || {
            if let Some(ui) = w.upgrade() {
                let summary = ui.global::<WorldClockStore>().get_summary().to_string();
                if !summary.is_empty() {
                    if let Ok(mut clip) = magpie_platform::platform_clipboard() {
                        let _ = clip.set_text(&summary);
                    }
                }
            }
        });
    }
    // ---- Note version history (backlog #4) ----
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.global::<NoteRender>().on_toggle_history(move || {
            if let Some(ui) = w.upgrade() {
                let nr = ui.global::<NoteRender>();
                let open = !nr.get_history();
                nr.set_history(open);
                if open {
                    let id = ui.get_note_id();
                    let rows: Vec<NoteVersionRow> = if id >= 0 {
                        let store = s.store.lock().unwrap_or_else(|e| e.into_inner());
                        store
                            .note_versions(id as i64, 50)
                            .unwrap_or_default()
                            .into_iter()
                            .map(|(vid, ms, body)| NoteVersionRow {
                                id: vid as i32,
                                when: relative_time(ms, now_ms()).into(),
                                preview: body
                                    .lines()
                                    .find(|l| !l.trim().is_empty())
                                    .unwrap_or("(empty)")
                                    .chars()
                                    .take(80)
                                    .collect::<String>()
                                    .into(),
                            })
                            .collect()
                    } else {
                        Vec::new()
                    };
                    nr.set_versions(ModelRc::new(VecModel::from(rows)));
                }
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.global::<NoteRender>().on_restore_version(move |row| {
            if let Some(ui) = w.upgrade() {
                let id = ui.get_note_id();
                if id < 0 {
                    return;
                }
                let body = {
                    let store = s.store.lock().unwrap_or_else(|e| e.into_inner());
                    store
                        .note_versions(id as i64, 50)
                        .ok()
                        .and_then(|v| v.get(row as usize).map(|(_, _, b)| b.clone()))
                };
                if let Some(body) = body {
                    {
                        let store = s.store.lock().unwrap_or_else(|e| e.into_inner());
                        let _ = store.update_note_body(id as i64, &body, now_ms());
                        // Record the restore itself as a new restore point (force a new row).
                        let _ = store.snapshot_note_version(id as i64, &body, now_ms(), 0, 50);
                    }
                    ui.set_note_body(body.clone().into());
                    ui.global::<NoteRender>().set_history(false);
                    refresh_notes(&ui, &s);
                }
            }
        });
    }
    // ---- Notes v2 rich preview (screen 18) ----
    {
        let w = ui.as_weak();
        ui.global::<NoteRender>().on_toggle_preview(move || {
            if let Some(ui) = w.upgrade() {
                let nr = ui.global::<NoteRender>();
                let now_preview = !nr.get_preview();
                nr.set_preview(now_preview);
                if now_preview {
                    // Rebuild from the (possibly just-edited) body.
                    let body = ui.get_note_body();
                    refresh_note_render(&ui, body.as_str());
                }
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.global::<NoteRender>().on_open_wiki(move |name| {
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
        let w = ui.as_weak();
        ui.global::<NoteRender>().on_open_url(move |href| {
            if let Some(ui) = w.upgrade() {
                let url = href.to_string();
                if url.starts_with("http://") || url.starts_with("https://") {
                    open_url_and_hide(&ui, &url);
                }
            }
        });
    }
    // ---- Snippets (screen 20) ----
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.global::<SnippetsStore>().on_set_tab(move |t| {
            if let Some(ui) = w.upgrade() {
                ui.global::<SnippetsStore>().set_tab(t);
                ui.global::<SnippetsStore>().set_selected(0);
                refresh_snippets(&ui, &s);
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.global::<SnippetsStore>().on_select(move |i| {
            if let Some(ui) = w.upgrade() {
                ui.global::<SnippetsStore>().set_selected(i);
                refresh_snippets(&ui, &s);
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.global::<SnippetsStore>().on_paste_selected(move || {
            if let Some(ui) = w.upgrade() {
                let body = ui.global::<SnippetsStore>().get_detail_body().to_string();
                if body.is_empty() {
                    return;
                }
                let text = expand_snippet(&s, &body).text;
                if let Ok(mut clip) = magpie_platform::platform_clipboard() {
                    let _ = clip.set_text(&text);
                }
                hide_launcher(&ui);
                let _ = slint::invoke_from_event_loop(|| {
                    let _ = magpie_platform::Paster::paste(&EnigoPaster);
                });
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.global::<SnippetsStore>().on_new_snippet(move || {
            if let Some(ui) = w.upgrade() {
                let id = {
                    let store = match s.store.lock() {
                        Ok(g) => g,
                        Err(e) => e.into_inner(),
                    };
                    store
                        .add_snippet("New snippet", "", "{cursor}", "snippet", now_ms())
                        .ok()
                };
                if id.is_some() {
                    ui.global::<SnippetsStore>().set_selected(0);
                    refresh_snippets(&ui, &s);
                    refresh_expand_keywords(&s);
                }
            }
        });
    }
    {
        ui.global::<SnippetsStore>().on_edit_selected(move || {
            // Inline snippet editing is out of scope for this pass; the body is
            // shown read-only. Import or the New action create snippets.
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.global::<SnippetsStore>().on_import_espanso(move || {
            import_snippets(&w, &s, true);
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.global::<SnippetsStore>().on_import_navi(move || {
            import_snippets(&w, &s, false);
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        let wk = ui.as_weak();
        ui.global::<SnippetsStore>().on_toggle_expand(move || {
            if let Some(ui) = w.upgrade() {
                let ss = ui.global::<SnippetsStore>();
                let want = !ss.get_expand_as_you_type();
                let actual = if want {
                    // Start the global monitor; only reflect ON if it actually started
                    // (macOS needs Input Monitoring — the toggle stays off otherwise).
                    start_expand_monitor(&wk, &s)
                } else {
                    stop_expand_monitor(&s);
                    false
                };
                ss.set_expand_as_you_type(actual);
                // Persist the intent.
                let cfg_path = data_dir().join("config.toml");
                let mut cfg = magpie_app::config::load_or_default(&cfg_path);
                cfg.expand_as_you_type = actual;
                let _ = magpie_app::config::save(&cfg, &cfg_path);
            }
        });
    }
    // ---- Secrets vault ----
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.global::<SecretsStore>().on_search(move |q| {
            if let Some(ui) = w.upgrade() {
                ui.global::<SecretsStore>().set_query(q);
                ui.global::<SecretsStore>().set_selected(0);
                ui.global::<SecretsStore>().set_detail_revealed(false);
                refresh_secrets(&ui, &s);
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.global::<SecretsStore>().on_select(move |i| {
            if let Some(ui) = w.upgrade() {
                ui.global::<SecretsStore>().set_selected(i);
                ui.global::<SecretsStore>().set_detail_revealed(false);
                refresh_secrets(&ui, &s);
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.global::<SecretsStore>().on_reveal_selected(move || {
            if let Some(ui) = w.upgrade() {
                let ss = ui.global::<SecretsStore>();
                ss.set_detail_revealed(!ss.get_detail_revealed());
                refresh_secrets(&ui, &s);
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.global::<SecretsStore>().on_copy_selected(move || {
            if let Some(ui) = w.upgrade() {
                if let Some(id) = selected_secret_id(&ui) {
                    let value = {
                        let store = s.store.lock().unwrap_or_else(|e| e.into_inner());
                        store.get_secret(id).ok().flatten().map(|sec| sec.value)
                    };
                    if let Some(value) = value {
                        // Suppress the resulting capture so the plaintext doesn't re-enter
                        // history, then place it on the clipboard and paste.
                        let hash = magpie_core::model::content_hash(&Content::Text(value.clone()));
                        s.suppress_capture
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .insert(hash);
                        if let Ok(mut clip) = magpie_platform::platform_clipboard() {
                            let _ = clip.set_text(&value);
                        }
                        hide_launcher(&ui);
                        let _ = slint::invoke_from_event_loop(|| {
                            let _ = magpie_platform::Paster::paste(&EnigoPaster);
                        });
                    }
                }
            }
        });
    }
    {
        let w = ui.as_weak();
        ui.global::<SecretsStore>().on_new_secret(move || {
            if let Some(ui) = w.upgrade() {
                let ss = ui.global::<SecretsStore>();
                ss.set_edit_id(0);
                ss.set_edit_key(SharedString::from(""));
                ss.set_edit_value(SharedString::from(""));
                ss.set_edit_notes(SharedString::from(""));
                ss.set_editing(true);
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.global::<SecretsStore>().on_edit_selected(move || {
            if let Some(ui) = w.upgrade() {
                if let Some(id) = selected_secret_id(&ui) {
                    let sec = {
                        let store = s.store.lock().unwrap_or_else(|e| e.into_inner());
                        store.get_secret(id).ok().flatten()
                    };
                    if let Some(sec) = sec {
                        let ss = ui.global::<SecretsStore>();
                        ss.set_edit_id(sec.id as i32);
                        ss.set_edit_key(sec.key.into());
                        ss.set_edit_value(sec.value.into());
                        ss.set_edit_notes(sec.notes.into());
                        ss.set_editing(true);
                    }
                }
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.global::<SecretsStore>().on_delete_selected(move || {
            if let Some(ui) = w.upgrade() {
                if let Some(id) = selected_secret_id(&ui) {
                    {
                        let store = s.store.lock().unwrap_or_else(|e| e.into_inner());
                        let _ = store.delete_secret(id);
                    }
                    ui.global::<SecretsStore>().set_selected(0);
                    refresh_secrets(&ui, &s);
                }
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.global::<SecretsStore>().on_save_edit(move || {
            if let Some(ui) = w.upgrade() {
                let ss = ui.global::<SecretsStore>();
                let id = ss.get_edit_id();
                let key = ss.get_edit_key().to_string();
                let key = if key.trim().is_empty() {
                    "Secret".to_string()
                } else {
                    key
                };
                let value = ss.get_edit_value().to_string();
                let notes = ss.get_edit_notes().to_string();
                {
                    let store = s.store.lock().unwrap_or_else(|e| e.into_inner());
                    if id < 0 {
                        // Promote the clipboard entry (-id): value comes from the clip.
                        let _ = store.promote_entry_to_secret((-id) as i64, &key, now_ms());
                    } else if id == 0 {
                        let _ = store.add_secret(&key, &value, &notes, None, now_ms());
                    } else {
                        let _ = store.update_secret(id as i64, &key, &value, &notes, now_ms());
                    }
                }
                ss.set_editing(false);
                ss.set_selected(0);
                refresh_secrets(&ui, &s);
                // The promoted clip is now masked — refresh the clipboard list too.
                refresh(&ui, &s);
            }
        });
    }
    {
        let w = ui.as_weak();
        ui.global::<SecretsStore>().on_cancel_edit(move || {
            if let Some(ui) = w.upgrade() {
                ui.global::<SecretsStore>().set_editing(false);
            }
        });
    }
    // ---- File search (screens 19/21/22/23) ----
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.global::<SearchStore>().on_set_tab(move |t| {
            s.search.lock().unwrap_or_else(|e| e.into_inner()).tab = t;
            if let Some(ui) = w.upgrade() {
                refresh_files(&ui, &s);
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.global::<SearchStore>().on_set_mode(move |m| {
            s.search.lock().unwrap_or_else(|e| e.into_inner()).mode = m;
            if let Some(ui) = w.upgrade() {
                refresh_files(&ui, &s);
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.global::<SearchStore>().on_cycle_case(move || {
            {
                let mut g = s.search.lock().unwrap_or_else(|e| e.into_inner());
                g.case = (g.case + 1) % 3;
            }
            if let Some(ui) = w.upgrade() {
                refresh_files(&ui, &s);
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.global::<SearchStore>().on_add_location(move || {
            // Add the typed query as a path if it looks like one; else the home dir.
            let typed = w
                .upgrade()
                .map(|ui| ui.get_query().to_string())
                .unwrap_or_default();
            let path = typed.trim().to_string();
            let candidate = if path.starts_with('~') || path.starts_with('/') {
                path
            } else {
                "~/Documents".to_string()
            };
            {
                let mut g = s.search.lock().unwrap_or_else(|e| e.into_inner());
                if !g.locations.contains(&candidate) {
                    g.locations.push(candidate);
                }
            }
            if let Some(ui) = w.upgrade() {
                refresh_files(&ui, &s);
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.global::<SearchStore>().on_remove_location(move |i| {
            {
                let mut g = s.search.lock().unwrap_or_else(|e| e.into_inner());
                let idx = i as usize;
                if idx < g.locations.len() {
                    g.locations.remove(idx);
                }
            }
            if let Some(ui) = w.upgrade() {
                refresh_files(&ui, &s);
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.global::<SearchStore>().on_toggle_hide_viewed(move || {
            {
                let mut g = s.search.lock().unwrap_or_else(|e| e.into_inner());
                g.hide_viewed = !g.hide_viewed;
            }
            if let Some(ui) = w.upgrade() {
                refresh_files(&ui, &s);
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.global::<SearchStore>().on_toggle_viewed(move |i| {
            {
                let mut g = s.search.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(p) = g.result_paths.get(i as usize).cloned() {
                    if g.viewed.contains(&p) {
                        g.viewed.remove(&p);
                    } else {
                        g.viewed.insert(p);
                    }
                }
            }
            if let Some(ui) = w.upgrade() {
                refresh_files(&ui, &s);
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.global::<SearchStore>().on_open_file(move |i| {
            let path = s
                .search
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .result_paths
                .get(i as usize)
                .cloned();
            if let (Some(p), Some(ui)) = (path, w.upgrade()) {
                hide_launcher(&ui);
                let _ = std::process::Command::new("open").arg(&p).spawn();
            }
        });
    }
    {
        let s = state.clone();
        ui.global::<SearchStore>().on_reveal_file(move |i| {
            let path = s
                .search
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .result_paths
                .get(i as usize)
                .cloned();
            if let Some(p) = path {
                let _ = std::process::Command::new("open").arg("-R").arg(&p).spawn();
            }
        });
    }
    {
        let s = state.clone();
        ui.global::<SearchStore>().on_edit_file(move |i| {
            let path = s
                .search
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .result_paths
                .get(i as usize)
                .cloned();
            if let Some(p) = path {
                // Open in the default text editor.
                let _ = std::process::Command::new("open").arg("-t").arg(&p).spawn();
            }
        });
    }
    {
        let s = state.clone();
        ui.global::<SearchStore>().on_copy_name(move |i| {
            if let Some(p) = s
                .search
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .result_paths
                .get(i as usize)
                .cloned()
            {
                let name = std::path::Path::new(&p)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or(p);
                if let Ok(mut clip) = magpie_platform::platform_clipboard() {
                    let _ = clip.set_text(&name);
                }
            }
        });
    }
    {
        let s = state.clone();
        ui.global::<SearchStore>().on_copy_path(move |i| {
            if let Some(p) = s
                .search
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .result_paths
                .get(i as usize)
                .cloned()
            {
                if let Ok(mut clip) = magpie_platform::platform_clipboard() {
                    let _ = clip.set_text(&p);
                }
            }
        });
    }
    {
        ui.global::<SearchStore>().on_collapse_all(move || {});
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.global::<SearchStore>().on_apply_replace(move || {
            apply_sed_replace(&w, &s);
        });
    }
    // ---- Time tracking v2 (screen 24) ----
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.global::<TimeStore>().on_stop_timer(move || {
            {
                let store = match s.store.lock() {
                    Ok(g) => g,
                    Err(e) => e.into_inner(),
                };
                let _ = store.stop_active(now_ms());
            }
            if let Some(ui) = w.upgrade() {
                refresh_time(&ui, &s);
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.global::<TimeStore>().on_continue_last(move || {
            continue_time_entry(&w, &s, 0);
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.global::<TimeStore>().on_continue_row(move |i| {
            continue_time_entry(&w, &s, i);
        });
    }
    {
        ui.global::<TimeStore>().on_add_entry(move || {
            // Manual-entry form is deferred; entries come from the task timer.
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.global::<TimeStore>().on_report(move || {
            if let Some(ui) = w.upgrade() {
                nav_to(&ui, &s, "stats", "");
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.global::<TimeStore>().on_pomo_toggle(move || {
            {
                let mut p = s.pomodoro.lock().unwrap_or_else(|e| e.into_inner());
                p.running = !p.running;
            }
            if let Some(ui) = w.upgrade() {
                push_pomodoro(&ui, &s);
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.global::<TimeStore>().on_pomo_skip(move || {
            {
                let mut p = s.pomodoro.lock().unwrap_or_else(|e| e.into_inner());
                p.advance();
            }
            if let Some(ui) = w.upgrade() {
                push_pomodoro(&ui, &s);
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.global::<TimeStore>().on_pomo_tick(move || {
            let mut phase_ended = false;
            {
                let mut p = s.pomodoro.lock().unwrap_or_else(|e| e.into_inner());
                if p.running {
                    p.remaining_sec -= 1;
                    if p.remaining_sec <= 0 {
                        let finished_focus = p.phase == 0;
                        p.advance();
                        phase_ended = true;
                        let _ = finished_focus;
                    }
                }
            }
            if phase_ended {
                magpie_platform::notify("Pomodoro", "Phase complete");
            }
            if let Some(ui) = w.upgrade() {
                push_pomodoro(&ui, &s);
                // Keep the running-timer elapsed live while on the Time tab.
                if ui.get_tasks_view() == "time" {
                    refresh_time(&ui, &s);
                }
            }
        });
    }
    // ---- Local projects (screens 25/26) ----
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.global::<ProjectsStore>().on_select(move |i| {
            if let Some(ui) = w.upgrade() {
                ui.global::<ProjectsStore>().set_selected(i);
                populate_projects(&ui, &s);
            }
        });
    }
    // ---- Per-project task hub (List/Board scoped to the selected project) ----
    {
        let w = ui.as_weak();
        let s = state.clone();
        ui.global::<ProjectsStore>().on_set_detail_view(move |v| {
            if let Some(ui) = w.upgrade() {
                ui.global::<ProjectsStore>().set_detail_view(v);
                refresh_project_tasks(&ui, &s);
            }
        });
    }
    {
        let w = ui.as_weak();
        let s = state.clone();
        ui.global::<ProjectsStore>().on_toggle_show_done(move || {
            if let Some(ui) = w.upgrade() {
                let ps = ui.global::<ProjectsStore>();
                ps.set_detail_show_done(!ps.get_detail_show_done());
                refresh_project_tasks(&ui, &s);
            }
        });
    }
    {
        let w = ui.as_weak();
        let s = state.clone();
        ui.global::<ProjectsStore>()
            .on_board_controls_changed(move || {
                if let Some(ui) = w.upgrade() {
                    refresh_project_tasks(&ui, &s);
                }
            });
    }
    {
        let w = ui.as_weak();
        let s = state.clone();
        ui.global::<ProjectsStore>()
            .on_task_toggle(move |note_id, line| {
                if let Some(ui) = w.upgrade() {
                    proj_toggle_task(&s, note_id, line as usize);
                    refresh_project_tasks(&ui, &s);
                }
            });
    }
    {
        let w = ui.as_weak();
        let s = state.clone();
        ui.global::<ProjectsStore>().on_task_open(move |note_id| {
            if note_id < 0 {
                open_project_task_by_id(&s, note_id);
            } else if let Some(ui) = w.upgrade() {
                open_note_into_notes_mode(&ui, &s, note_id);
            }
        });
    }
    {
        let w = ui.as_weak();
        let s = state.clone();
        ui.global::<ProjectsStore>()
            .on_task_bookmark(move |note_id, line| {
                if let Some(ui) = w.upgrade() {
                    proj_toggle_bookmark(&s, note_id, line as usize);
                    refresh_project_tasks(&ui, &s);
                }
            });
    }
    {
        let w = ui.as_weak();
        let s = state.clone();
        ui.global::<ProjectsStore>()
            .on_task_timer(move |note_id, title| {
                if let Some(ui) = w.upgrade() {
                    // Reuse the full timer logic (incl. org CLOCK logging), then
                    // re-render the project hub.
                    ui.invoke_toggle_timer(note_id, title);
                    refresh_project_tasks(&ui, &s);
                }
            });
    }
    {
        let w = ui.as_weak();
        let s = state.clone();
        ui.global::<ProjectsStore>()
            .on_task_rename(move |note_id, line, title| {
                if let Some(ui) = w.upgrade() {
                    proj_rename_task(&s, note_id, line as usize, title.as_str());
                    refresh_project_tasks(&ui, &s);
                }
            });
    }
    {
        let w = ui.as_weak();
        let s = state.clone();
        ui.global::<ProjectsStore>()
            .on_task_set_status(move |note_id, line, status| {
                if let Some(ui) = w.upgrade() {
                    proj_set_status(&s, note_id, line as usize, status);
                    refresh_project_tasks(&ui, &s);
                }
            });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.global::<ProjectsStore>().on_rescan(move || {
            if let Some(ui) = w.upgrade() {
                refresh_projects(&ui, &s, true);
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.global::<ProjectsStore>().on_add_found(move |i| {
            // Adding a found (repo/editor-only) folder makes it a project by creating
            // an empty `.magpie/` dir (safe mkdir; the folder becomes syncable).
            let path = {
                let g = s.project_scan.lock().unwrap_or_else(|e| e.into_inner());
                g.hits
                    .iter()
                    .filter(|h| !h.syncable())
                    .nth(i as usize)
                    .map(|h| h.path.clone())
            };
            if let Some(p) = path {
                let _ = std::fs::create_dir_all(p.join(".magpie"));
            }
            if let Some(ui) = w.upgrade() {
                refresh_projects(&ui, &s, true);
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.global::<ProjectsStore>().on_ignore_found(move |i| {
            let display = {
                let g = s.project_scan.lock().unwrap_or_else(|e| e.into_inner());
                g.hits
                    .iter()
                    .filter(|h| !h.syncable())
                    .nth(i as usize)
                    .map(|h| magpie_app::projects::display_path(&h.path))
            };
            if let Some(d) = display {
                {
                    let mut g = s.project_scan.lock().unwrap_or_else(|e| e.into_inner());
                    if !g.ignored.contains(&d) {
                        g.ignored.push(d.clone());
                    }
                }
                let cfg_path = data_dir().join("config.toml");
                let mut cfg = magpie_app::config::load_or_default(&cfg_path);
                if !cfg.projects_ignored.contains(&d) {
                    cfg.projects_ignored.push(d);
                }
                let _ = magpie_app::config::save(&cfg, &cfg_path);
            }
            if let Some(ui) = w.upgrade() {
                refresh_projects(&ui, &s, true);
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.global::<ProjectsStore>().on_toggle_task(move |idx| {
            // Line-level checkbox toggle, reconciled through the 3-way merge so a
            // concurrent disk edit isn't clobbered (Phase 18b).
            let target = {
                let g = s.project_scan.lock().unwrap_or_else(|e| e.into_inner());
                let sel = w
                    .upgrade()
                    .map(|ui| ui.global::<ProjectsStore>().get_selected())
                    .unwrap_or(0);
                let syncable: Vec<_> = g.hits.iter().filter(|h| h.syncable()).collect();
                syncable.get(sel.max(0) as usize).and_then(|h| {
                    h.tasks.get(idx as usize).map(|t| {
                        (
                            h.path.clone(),
                            t.file_rel.clone(),
                            t.file_abs.clone(),
                            t.line,
                        )
                    })
                })
            };
            let conflicted = if let Some((root, rel, file, line)) = target {
                toggle_project_task(&s, &root, &rel, &file, line)
            } else {
                false
            };
            if conflicted {
                magpie_platform::notify(
                    "Magpie — file changed on disk",
                    "Your change wasn't applied because this task was also edited on disk. Reopen the file to reconcile.",
                );
            }
            if let Some(ui) = w.upgrade() {
                refresh_projects(&ui, &s, true);
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.global::<ProjectsStore>().on_open_task(move |idx| {
            let file = {
                let g = s.project_scan.lock().unwrap_or_else(|e| e.into_inner());
                let sel = w
                    .upgrade()
                    .map(|ui| ui.global::<ProjectsStore>().get_selected())
                    .unwrap_or(0);
                let syncable: Vec<_> = g.hits.iter().filter(|h| h.syncable()).collect();
                syncable
                    .get(sel.max(0) as usize)
                    .and_then(|h| h.tasks.get(idx as usize))
                    .map(|t| t.file_abs.clone())
            };
            if let Some(f) = file {
                let _ = std::process::Command::new("open").arg("-t").arg(&f).spawn();
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        // Open a read-only code-comment ("rel:line") in the editor, resolved against
        // the selected project's path.
        ui.global::<ProjectsStore>().on_open_source(move |source| {
            let root = {
                let g = s.project_scan.lock().unwrap_or_else(|e| e.into_inner());
                let sel = w
                    .upgrade()
                    .map(|ui| ui.global::<ProjectsStore>().get_selected())
                    .unwrap_or(0);
                g.hits
                    .iter()
                    .filter(|h| h.syncable())
                    .nth(sel.max(0) as usize)
                    .map(|h| h.path.clone())
            };
            if let Some(root) = root {
                let s = source.to_string();
                let rel = s.rsplit_once(':').map(|(p, _)| p).unwrap_or(&s);
                let _ = std::process::Command::new("open")
                    .arg("-t")
                    .arg(root.join(rel))
                    .spawn();
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.global::<ProjectsStore>().on_open_folder(move || {
            let path = {
                let g = s.project_scan.lock().unwrap_or_else(|e| e.into_inner());
                let sel = w
                    .upgrade()
                    .map(|ui| ui.global::<ProjectsStore>().get_selected())
                    .unwrap_or(0);
                g.hits
                    .iter()
                    .filter(|h| h.syncable())
                    .nth(sel.max(0) as usize)
                    .map(|h| h.path.clone())
            };
            if let Some(p) = path {
                let _ = std::process::Command::new("open").arg(&p).spawn();
            }
        });
    }
    // ---- Project sync conflict Review (Phase 18b, PNG 25) ----
    {
        let w = ui.as_weak();
        ui.global::<ProjectsStore>().on_review(move || {
            if let Some(ui) = w.upgrade() {
                let ps = ui.global::<ProjectsStore>();
                ps.set_reviewing(!ps.get_reviewing());
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.global::<ProjectsStore>().on_resolve_mine(move || {
            resolve_project_conflict(&s, true);
            if let Some(ui) = w.upgrade() {
                refresh_projects(&ui, &s, true);
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.global::<ProjectsStore>().on_resolve_theirs(move || {
            resolve_project_conflict(&s, false);
            if let Some(ui) = w.upgrade() {
                refresh_projects(&ui, &s, true);
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.global::<ProjectsStore>().on_resolve_edit(move || {
            // Open the conflicted file in the editor for a manual merge, then drop it
            // from the Review queue (the watcher picks up the hand-edited result).
            let root = {
                let g = s.project_scan.lock().unwrap_or_else(|e| e.into_inner());
                g.hits
                    .iter()
                    .filter(|h| h.syncable())
                    .nth(g.selected.max(0) as usize)
                    .map(|h| h.path.clone())
            };
            let file = root.and_then(|rp| {
                let cs = s
                    .project_conflicts
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                cs.iter().find(|c| c.root == rp).map(|c| c.file_abs.clone())
            });
            if let Some(f) = &file {
                let _ = std::process::Command::new("open").arg("-t").arg(f).spawn();
                let mut cs = s
                    .project_conflicts
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                cs.retain(|c| Some(&c.file_abs) != file.as_ref());
            }
            if let Some(ui) = w.upgrade() {
                refresh_projects(&ui, &s, true);
            }
        });
    }
    // ---- Data view (screens 13/14) ----
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.global::<DataStore>().on_set_read_as(move |i| {
            if let Ok(mut g) = s.data_read_as.lock() {
                *g = read_as_from_index(i);
            }
            if let Some(ui) = w.upgrade() {
                refresh_data(&ui, &s);
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.global::<DataStore>().on_open_recent(move |i| {
            let src = s
                .data_recent
                .lock()
                .ok()
                .and_then(|r| r.get(i as usize).cloned());
            if let (Some(src), Some(ui)) = (src, w.upgrade()) {
                open_data(&ui, &s, &src.name, &src.input);
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.global::<DataStore>().on_open_from_clip(move || {
            let recent = current_results(&s, now_ms());
            if let (Some(e), Some(ui)) = (recent.first(), w.upgrade()) {
                open_data(&ui, &s, "Clipboard entry", &e.full_text);
            }
        });
    }
    {
        let s = state.clone();
        ui.global::<DataStore>().on_export_data(move || {
            use magpie_core::table;
            let input = s.data_input.lock().map(|g| g.clone()).unwrap_or_default();
            let read_as = s
                .data_read_as
                .lock()
                .map(|g| *g)
                .unwrap_or(table::ReadAs::Auto);
            if let Ok(t) = table::parse(&input, read_as) {
                let out = table::export(&t.columns, table::ExportFormat::Csv);
                if let Ok(mut clip) = magpie_platform::platform_clipboard() {
                    let _ = clip.set_text(&out);
                }
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.global::<DataStore>().on_chart_toggle(move || {
            if let Ok(mut g) = s.data_chart.lock() {
                g.open = !g.open;
            }
            if let Some(ui) = w.upgrade() {
                compute_chart(&ui, &s);
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.global::<DataStore>().on_chart_set_type(move |t| {
            if let Ok(mut g) = s.data_chart.lock() {
                g.kind = t;
            }
            if let Some(ui) = w.upgrade() {
                compute_chart(&ui, &s);
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.global::<DataStore>().on_chart_cycle_x(move || {
            cycle_chart_col(&s, true);
            if let Some(ui) = w.upgrade() {
                compute_chart(&ui, &s);
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.global::<DataStore>().on_chart_cycle_y(move || {
            cycle_chart_col(&s, false);
            if let Some(ui) = w.upgrade() {
                compute_chart(&ui, &s);
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.global::<DataStore>().on_chart_set_log(move |on| {
            if let Ok(mut g) = s.data_chart.lock() {
                g.log = on;
            }
            if let Some(ui) = w.upgrade() {
                compute_chart(&ui, &s);
            }
        });
    }
    // ---- Calculator (screens 11/12) ----
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.global::<CalcStore>().on_set_tab(move |t| {
            if let Some(ui) = w.upgrade() {
                ui.global::<CalcStore>().set_tab(t);
                ui.global::<CalcStore>().set_selected(0);
                refresh_calc(&ui, &s);
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.global::<CalcStore>().on_reuse(move |i| {
            if let Some(ui) = w.upgrade() {
                let model = ui.global::<CalcStore>().get_history();
                if let Some(row) = model.row_data(i as usize) {
                    ui.set_query(row.input.clone());
                    ui.invoke_search_changed(row.input);
                }
                let _ = &s;
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.global::<CalcStore>().on_delete_row(move |i| {
            if let Some(ui) = w.upgrade() {
                let model = ui.global::<CalcStore>().get_history();
                if let Some(row) = model.row_data(i as usize) {
                    if let Ok(st) = s.store.lock() {
                        let _ = st.delete_calc_entry(row.id as i64);
                    }
                }
                refresh_calc(&ui, &s);
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.global::<CalcStore>().on_clear_history(move || {
            if let Ok(st) = s.store.lock() {
                let _ = st.clear_calc_history();
            }
            if let Some(ui) = w.upgrade() {
                refresh_calc(&ui, &s);
            }
        });
    }
    {
        let w = ui.as_weak();
        ui.global::<CalcStore>().on_copy_row(move |i| {
            if let Some(ui) = w.upgrade() {
                let model = ui.global::<CalcStore>().get_history();
                if let Some(row) = model.row_data(i as usize) {
                    if let Ok(mut clip) = magpie_platform::platform_clipboard() {
                        let _ = clip.set_text(&row.result);
                    }
                }
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.global::<CalcStore>().on_commit(move || {
            if let Some(ui) = w.upgrade() {
                calc_commit(&ui, &s);
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.global::<CalcStore>().on_open_graphs(move |i| {
            if let Some(ui) = w.upgrade() {
                let model = ui.global::<CalcStore>().get_saved_graphs();
                if let Some(row) = model.row_data(i as usize) {
                    let (f, g) = split_fg(row.input.as_str());
                    if let Ok(mut lock) = s.calc_graph_f.lock() {
                        *lock = f;
                    }
                    if let Ok(mut lock) = s.calc_graph_g.lock() {
                        *lock = g;
                    }
                    ui.global::<CalcStore>().set_graphs_open(true);
                    compute_graph(&ui, &s);
                }
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.global::<CalcStore>().on_close_graphs(move || {
            if let Some(ui) = w.upgrade() {
                ui.global::<CalcStore>().set_graphs_open(false);
                refresh_calc(&ui, &s);
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.global::<CalcStore>().on_set_syntax(move |m| {
            if let Some(ui) = w.upgrade() {
                ui.global::<CalcStore>().set_syntax_mode(m);
                update_calc_math(&ui, &s);
            }
        });
    }
    {
        let w = ui.as_weak();
        ui.global::<CalcStore>().on_copy_latex(move || {
            if let Some(ui) = w.upgrade() {
                if let Ok(mut clip) = magpie_platform::platform_clipboard() {
                    let _ = clip.set_text(&ui.get_query());
                }
            }
        });
    }
    {
        let w = ui.as_weak();
        ui.global::<CalcStore>().on_copy_typst(move || {
            if let Some(ui) = w.upgrade() {
                let typst = latex_to_typst(&ui.get_query());
                if let Ok(mut clip) = magpie_platform::platform_clipboard() {
                    let _ = clip.set_text(&typst);
                }
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_transform_action(move |result, action| {
            let result = result.to_string();
            if result.is_empty() {
                return;
            }
            match action {
                1 => {
                    if let Ok(mut clip) = magpie_platform::platform_clipboard() {
                        let _ = clip.set_text(&result);
                    }
                }
                2 => {
                    if let Ok(store) = s.store.lock() {
                        let _ = store.create_snippet(&result, now_ms());
                    }
                    if let Some(ui) = w.upgrade() {
                        refresh(&ui, &s);
                    }
                }
                _ => {
                    if let Ok(mut clip) = magpie_platform::platform_clipboard() {
                        let _ = clip.set_text(&result);
                    }
                    if let Some(ui) = w.upgrade() {
                        hide_launcher(&ui);
                    }
                    let _ = slint::invoke_from_event_loop(|| {
                        let _ = magpie_platform::Paster::paste(&EnigoPaster);
                    });
                }
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
            let Some(ui) = w.upgrade() else { return };
            // `>` prefix opens the command palette (actions / transforms). Clear the top
            // bar and hand control to the palette, which owns its own input (like ⌘K).
            if text.starts_with('>') {
                ui.set_query(SharedString::from(""));
                if let Ok(mut u) = s.ui.lock() {
                    u.text = String::new();
                }
                let rest = text.trim_start_matches('>').trim().to_string();
                ui.invoke_open_actions();
                if !rest.is_empty() {
                    set_actions_filtered(&ui, &rest);
                }
                return;
            }
            // The shared top bar filters whatever screen is active (context-aware).
            match ui.get_section().as_str() {
                "secrets" => {
                    ui.global::<SecretsStore>().set_query(text.clone());
                    ui.global::<SecretsStore>().set_selected(0);
                    refresh_secrets(&ui, &s);
                }
                "snippets" => {
                    ui.global::<SnippetsStore>().set_selected(0);
                    refresh_snippets(&ui, &s);
                }
                "files" => {
                    if let Ok(mut u) = s.ui.lock() {
                        u.text = text.to_string();
                    }
                    refresh_files(&ui, &s);
                }
                "notes" => {
                    // refresh_notes reads ui.get_query() for its free-text filter.
                    refresh_notes(&ui, &s);
                }
                "tasks" => {
                    // All task tabs share one free-text filter (`task_search`).
                    ui.set_task_search(text.clone());
                    match ui.get_tasks_view().as_str() {
                        "board" => refresh_board(&ui, &s),
                        "today" => refresh_today(&ui, &s),
                        _ => refresh_tasks(&ui, &s),
                    }
                }
                "bookmarks" => {
                    ui.set_bookmark_query(text.clone());
                    refresh_bookmarks(&ui, &s);
                }
                "projects" => {
                    // populate_projects reads ui.get_query() (no rescan).
                    refresh_projects(&ui, &s, false);
                }
                "calculator" => {
                    if let Ok(mut u) = s.ui.lock() {
                        u.text = text.to_string();
                    }
                    let mode = match magpie_core::mathsvg::detect_syntax(text.as_str()) {
                        magpie_core::mathsvg::Syntax::Latex => 1,
                        magpie_core::mathsvg::Syntax::Typst => 2,
                        magpie_core::mathsvg::Syntax::Plain => 0,
                    };
                    ui.global::<CalcStore>().set_syntax_mode(mode);
                    refresh(&ui, &s);
                    update_calc_math(&ui, &s);
                }
                _ => {
                    // Clipboard (and any list section): text search + inline answer card.
                    if let Ok(mut u) = s.ui.lock() {
                        u.text = text.to_string();
                    }
                    ui.set_answer_format_index(0);
                    refresh(&ui, &s);
                }
            }
            // Live token suggestions for the clipboard search bar.
            refresh_search_suggestions(&ui, &s, text.as_str());
        });
    }
    {
        // Copy an arbitrary string (a color format chip) to the system clipboard.
        ui.on_copy_text(move |t| {
            if let Ok(mut clip) = magpie_platform::platform_clipboard() {
                let _ = clip.set_text(t.as_str());
            }
        });
    }
    {
        // A search-suggestion chip was chosen: complete the query and re-run.
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_apply_suggestion(move |insert| {
            if let Some(ui) = w.upgrade() {
                ui.set_query(insert.clone());
                if let Ok(mut u) = s.ui.lock() {
                    u.text = insert.to_string();
                }
                ui.set_answer_format_index(0);
                refresh(&ui, &s);
                refresh_search_suggestions(&ui, &s, insert.as_str());
            }
        });
    }
    {
        // Build the (bounded) preview line model for the selected entry. Only the
        // visible lines are shaped by the ListView, so huge entries stay instant.
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_preview_select(move |idx, masked, revealed| {
            // Coalesce rapid selection changes: bump the generation, defer the expensive
            // rebuild to the event loop, and only render if no newer selection arrived
            // meanwhile. Holding an arrow key then flies through the list, rebuilding the
            // preview once when it settles instead of on every keystroke.
            let g = s
                .preview_gen
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                + 1;
            let s = s.clone();
            let w = w.clone();
            let _ = slint::invoke_from_event_loop(move || {
                if s.preview_gen.load(std::sync::atomic::Ordering::Relaxed) != g {
                    return; // superseded by a newer selection
                }
                // Bounded per-rebuild work: cap the lines built each time (the list is
                // virtualized anyway; huge entries show a truncated marker + Open-in-editor).
                const PREVIEW_LINE_CAP: usize = 4_000;
                // Read the selected entry from the cached results (populated by refresh) so
                // arrow-key navigation never re-runs the DB search — the old per-keystroke
                // `current_results` call was the preview lag.
                let entry: Option<Entry> = if idx >= 0 {
                    s.last_results
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .get(idx as usize)
                        .cloned()
                } else {
                    None
                };
                let entry = entry.as_ref();
                let (lines, truncated) = match entry {
                    Some(e) if masked && !revealed => {
                        let visible = s.mask_visible_chars.max(0) as usize;
                        (
                            vec![SharedString::from(mask_render(&e.full_text, visible))],
                            false,
                        )
                    }
                    Some(e) => {
                        // Cap each line's length so a single pathological very-long line
                        // (e.g. minified JSON) can't reintroduce whole-string shaping in the
                        // wrapped preview rows. The full entry is still copyable / openable.
                        const LINE_CHAR_CAP: usize = 1200;
                        let mut lines: Vec<SharedString> = Vec::new();
                        let mut truncated = false;
                        for (n, line) in e.full_text.split('\n').enumerate() {
                            if n >= PREVIEW_LINE_CAP {
                                truncated = true;
                                break;
                            }
                            if line.chars().count() > LINE_CHAR_CAP {
                                let head: String = line.chars().take(LINE_CHAR_CAP).collect();
                                lines.push(SharedString::from(format!("{head}…")));
                            } else {
                                lines.push(SharedString::from(line));
                            }
                        }
                        (lines, truncated)
                    }
                    None => (Vec::new(), false),
                };
                // Only SMALL entries get the selectable single-TextInput preview — that
                // control shapes its whole string every frame (Slint has no intra-Text
                // virtualization), so it's the scroll bottleneck. Everything larger uses the
                // virtualized line ListView (only visible rows shaped), which scrolls
                // instantly; whole-entry copy still works via ⌘C / the copy action.
                const SELECTABLE_MAX_LINES: usize = 40;
                const SELECTABLE_MAX_CHARS: usize = 2_000;
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
                // Search match highlighting: split the preview into lines with the matched
                // query terms marked, and note the first matching line so the preview can
                // scroll to it. Only when there's a text query and the entry is visible.
                let hl_terms: Vec<String> = {
                    let u = s.ui.lock().unwrap_or_else(|e| e.into_inner());
                    magpie_core::parse_search_tokens(&u.text)
                        .text
                        .split(|c: char| !c.is_alphanumeric())
                        .filter(|t| t.chars().count() >= 2)
                        .map(|t| t.to_lowercase())
                        .collect()
                };
                let (hl_lines, hl_first) = match entry {
                    Some(e) if (!masked || revealed) && !hl_terms.is_empty() => {
                        highlight_preview_lines(&e.full_text, &hl_terms, 4000)
                    }
                    _ => (Vec::new(), -1),
                };
                // Image entries have no text at all, so without this the preview pane
                // was blank. A masked entry stays hidden until Reveal.
                let (preview_image, has_preview_image) = match entry {
                    Some(e) if !masked || revealed => {
                        let (img, has) = load_entry_image(&s.images, e);
                        if has {
                            (img, has)
                        } else if let Some(p) = image_file_of(e) {
                            // A copied *path* to an image file → show the image itself.
                            match slint::Image::load_from_path(&p) {
                                Ok(i) => (i, true),
                                Err(_) => (img, has),
                            }
                        } else if let Some(path) = file_path_of(e) {
                            // Any other file (PDF/video/audio/epub/doc…) → a QuickLook
                            // thumbnail. Use the cache if present (instant); otherwise generate
                            // it on a background thread and refresh the preview when ready, so
                            // selection never blocks on qlmanage.
                            let dir = data_dir().join("thumbnails");
                            match magpie_platform::thumbnails::cached_thumbnail(&dir, &path, 512)
                                .and_then(|tp| slint::Image::load_from_path(&tp).ok())
                            {
                                Some(i) => (i, true),
                                None => {
                                    let weak = w.clone();
                                    std::thread::spawn(move || {
                                        if magpie_platform::thumbnails::ensure_file_thumbnail(
                                            &dir, &path, 512,
                                        )
                                        .is_some()
                                        {
                                            let _ = slint::invoke_from_event_loop(move || {
                                                if let Some(ui) = weak.upgrade() {
                                                    ui.invoke_rebuild_preview();
                                                }
                                            });
                                        }
                                    });
                                    (img, has)
                                }
                            }
                        } else {
                            (img, has)
                        }
                    }
                    _ => (slint::Image::default(), false),
                };
                // Web-link preview extras (screen 04): the domain, plus a cached OG
                // thumbnail IF this URL was previously bookmarked. Never fetched here —
                // page previews are fetched exclusively on explicit bookmark save.
                let (is_link, link_domain, link_image, has_link_image) = match entry {
                    Some(e) if matches!(e.kind, Kind::Link) => {
                        let domain = favicon::domain_of(&e.full_text).unwrap_or_default();
                        let p = magpie_app::link_meta::preview_cache_path(
                            &data_dir().join("previews"),
                            e.full_text.trim(),
                        );
                        let (img, has) = match p.exists().then(|| slint::Image::load_from_path(&p))
                        {
                            Some(Ok(img)) => (img, true),
                            _ => (slint::Image::default(), false),
                        };
                        (true, domain, img, has)
                    }
                    _ => (false, String::new(), slint::Image::default(), false),
                };
                // Colour card (screen 17): scale + harmonies + format strings.
                let to_slint = |c: magpie_core::color::Color| {
                    let (r, g, b) = c.rgb_u8();
                    slint::Color::from_rgb_u8(r, g, b)
                };
                let (is_color, cscale, charmony, cformats, cinverted) = match entry {
                    Some(e) if !masked || revealed => match magpie_core::color::parse(&e.full_text)
                    {
                        Some(c) => {
                            let scale: Vec<slint::Color> = magpie_core::color::scale(c)
                                .into_iter()
                                .map(to_slint)
                                .collect();
                            let (t1, t2) = magpie_core::color::triad(c);
                            let harmony: Vec<slint::Color> =
                                [magpie_core::color::complement(c), t1, t2]
                                    .into_iter()
                                    .map(to_slint)
                                    .collect();
                            let formats: Vec<SharedString> = vec![
                                SharedString::from(c.to_hex()),
                                SharedString::from(c.to_rgb_string()),
                                SharedString::from(c.to_rgba_string()),
                                SharedString::from(c.to_hsl_string()),
                                SharedString::from(c.to_oklch_string()),
                                SharedString::from(c.to_oklab_string()),
                            ];
                            (true, scale, harmony, formats, to_slint(c.invert()))
                        }
                        None => (
                            false,
                            Vec::new(),
                            Vec::new(),
                            Vec::new(),
                            slint::Color::default(),
                        ),
                    },
                    _ => (
                        false,
                        Vec::new(),
                        Vec::new(),
                        Vec::new(),
                        slint::Color::default(),
                    ),
                };
                if let Some(ui) = w.upgrade() {
                    ui.set_preview_lines(ModelRc::new(VecModel::from(lines)));
                    ui.set_preview_truncated(truncated);
                    ui.set_preview_text(SharedString::from(selectable));
                    ui.set_preview_image(preview_image);
                    ui.set_preview_has_image(has_preview_image);
                    ui.set_preview_is_color(is_color);
                    ui.set_color_scale(ModelRc::new(VecModel::from(cscale)));
                    ui.set_color_harmony(ModelRc::new(VecModel::from(charmony)));
                    ui.set_color_formats(ModelRc::new(VecModel::from(cformats)));
                    ui.set_color_inverted(cinverted);
                    ui.set_preview_is_link(is_link);
                    ui.set_preview_domain(SharedString::from(link_domain));
                    ui.set_preview_link_image(link_image);
                    ui.set_preview_has_link_image(has_link_image);
                    ui.set_preview_hl_lines(ModelRc::new(VecModel::from(hl_lines)));
                    ui.set_preview_hl_first(hl_first);
                }
            });
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
                    .and_then(|st| st.apps_in_use_with_counts().ok())
                    .unwrap_or_default()
                    .into_iter()
                    .map(|(id, name, count)| AppItem {
                        id: id as i32,
                        name: SharedString::from(name),
                        count: count as i32,
                    })
                    .collect();
                ui.set_apps(ModelRc::new(VecModel::from(apps)));
                // Mirror the current match mode into the popover control.
                ui.set_match_index(s.ui.lock().map(|u| match_mode_index(u.mode)).unwrap_or(0));
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
        ui.on_set_match_mode(move |idx| {
            if let Ok(mut u) = s.ui.lock() {
                u.mode = match_mode_from_index(idx);
            }
            if let Some(ui) = w.upgrade() {
                ui.set_match_index(idx);
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
        // Sidebar Pause button: flip the shared flag the watcher polls each tick.
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_toggle_pause(move || {
            use std::sync::atomic::Ordering;
            let now = !s.paused.load(Ordering::Relaxed);
            s.paused.store(now, Ordering::Relaxed);
            if let Some(ui) = w.upgrade() {
                ui.set_paused(now);
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
            // A "From projects" row (negative id) opens its file, not a DB note.
            if id < 0 {
                let path = s
                    .project_note_refs
                    .lock()
                    .ok()
                    .and_then(|m| m.get(&id).cloned());
                if let Some(p) = path {
                    let _ = std::process::Command::new("open").arg("-t").arg(&p).spawn();
                }
                return;
            }
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
                        // Version history (backlog #4): a coalesced restore point per
                        // ~2 min of edits, last 50 kept.
                        let _ = store.snapshot_note_version(
                            id as i64,
                            body.as_str(),
                            now_ms(),
                            120_000,
                            50,
                        );
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
            if note_id < 0 {
                // A project file-task: route the toggle to its file.
                toggle_project_task_by_id(&s, note_id, line_index as usize);
            } else {
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
            // Project file-tasks (negative id) have no DB row to bookmark — skip.
            if note_id >= 0 {
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
            // A project file-task opens its file; a note task opens the note.
            if note_id < 0 {
                open_project_task_by_id(&s, note_id);
            } else if let Some(ui) = w.upgrade() {
                open_note_into_notes_mode(&ui, &s, note_id);
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_refresh_tasks(move || {
            if let Some(ui) = w.upgrade() {
                // The task views share one skeleton + one search box, so route the
                // shared "search edited" signal to whichever tab is active. All
                // three read the same `task_search` free-text filter.
                match ui.get_tasks_view().as_str() {
                    "board" => refresh_board(&ui, &s),
                    "today" => refresh_today(&ui, &s),
                    _ => refresh_tasks(&ui, &s),
                }
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_rename_task(move |note_id, line_index, title| {
            if let Some(ui) = w.upgrade() {
                {
                    let store = match s.store.lock() {
                        Ok(g) => g,
                        Err(e) => e.into_inner(),
                    };
                    let _ = magpie_app::tasks::rename_task(
                        &store,
                        note_id as i64,
                        line_index as usize,
                        title.as_str(),
                        now_ms(),
                    );
                }
                // Re-render whichever task tab is showing the edited row.
                match ui.get_tasks_view().as_str() {
                    "board" => refresh_board(&ui, &s),
                    "today" => refresh_today(&ui, &s),
                    _ => refresh_tasks(&ui, &s),
                }
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
        // ⌘K → Open as table: parse the selected clip as a dataset and show Data.
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_open_selected_as_table(move || {
            if let Some(ui) = w.upgrade() {
                let idx = ui.get_selected();
                let recent = current_results(&s, now_ms());
                if let Some(e) = (idx >= 0).then(|| recent.get(idx as usize)).flatten() {
                    let text = e.full_text.clone();
                    open_data(&ui, &s, "Clipboard entry", &text);
                    nav_to(&ui, &s, "data", "");
                }
            }
        });
    }
    {
        // ⌘O — open the selected entry in its default / configured app: a web link in
        // the browser, an image in the image app, a file-path clip / File entry in the
        // per-category app (config `open_with`).
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_open_selected_url(move || {
            if let Some(ui) = w.upgrade() {
                open_selected_default(&ui, &s);
            }
        });
    }
    {
        // ⌘D — favorite / unfavorite the selected clip (a link becomes a bookmark
        // favorite that opens in the browser; anything else a `clip` that pastes).
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_favorite_selected(move || {
            if let Some(ui) = w.upgrade() {
                let idx = ui.get_selected();
                let recent = current_results(&s, now_ms());
                if let Some(e) = (idx >= 0).then(|| recent.get(idx as usize)).flatten() {
                    let is_url = e.full_text.trim().starts_with("http://")
                        || e.full_text.trim().starts_with("https://");
                    let kind = if is_url { "bookmark" } else { "clip" };
                    let payload = e.full_text.clone();
                    let label = e
                        .full_text
                        .lines()
                        .map(str::trim)
                        .find(|l| !l.is_empty())
                        .map(|l| l.chars().take(40).collect::<String>())
                        .unwrap_or_else(|| "Favorite".to_string());
                    if let Ok(st) = s.store.lock() {
                        let _ = st.toggle_favorite(kind, &payload, &label, now_ms());
                    }
                    refresh_favorites(&ui, &s);
                }
            }
        });
    }
    {
        // ⌘K → Save as secret: promote the selected clip to the vault. Opens the Secrets
        // section with the create/promote modal (value from the clip, key = suggestion).
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_save_secret_selected(move || {
            if let Some(ui) = w.upgrade() {
                let idx = ui.get_selected();
                let recent = current_results(&s, now_ms());
                if let Some(e) = (idx >= 0).then(|| recent.get(idx as usize)).flatten() {
                    let already = {
                        let st = s.store.lock().unwrap_or_else(|e| e.into_inner());
                        st.secret_for_entry(e.id).ok().flatten().is_some()
                    };
                    let suggested =
                        magpie_core::looks_like_secret(&e.full_text).unwrap_or("Secret");
                    let value = e.full_text.clone();
                    let eid = e.id;
                    nav_apply(&ui, &s, "secrets", "list");
                    if !already {
                        let ss = ui.global::<SecretsStore>();
                        ss.set_edit_id(-(eid as i32));
                        ss.set_edit_key(SharedString::from(suggested));
                        ss.set_edit_value(value.into());
                        ss.set_edit_notes(SharedString::from(""));
                        ss.set_editing(true);
                    }
                }
            }
        });
    }
    {
        // Activate a sidebar favorite: open URLs in the browser, else paste the value.
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_activate_favorite(move |id| {
            if let Some(ui) = w.upgrade() {
                let fav = s
                    .store
                    .lock()
                    .ok()
                    .and_then(|st| st.list_favorites().ok())
                    .and_then(|v| v.into_iter().find(|f| f.id == id as i64));
                if let Some(f) = fav {
                    if f.kind == "bookmark" || f.kind == "quicklink" {
                        let url = magpie_core::favorites::expand_quicklink(&f.payload, "");
                        open_url_and_hide(&ui, &url);
                    } else {
                        if let Ok(mut clip) = magpie_platform::platform_clipboard() {
                            let _ = clip.set_text(&f.payload);
                        }
                        hide_launcher(&ui);
                        let _ = slint::invoke_from_event_loop(|| {
                            let _ = magpie_platform::Paster::paste(&EnigoPaster);
                        });
                    }
                }
            }
        });
    }
    {
        // Quicklink activation: "<keyword> query" → open the expanded target.
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_activate_quicklink(move |query| {
            if let Some(ui) = w.upgrade() {
                if let Some((kw, rest)) = query.as_str().split_once(' ') {
                    let f = s
                        .store
                        .lock()
                        .ok()
                        .and_then(|st| st.favorite_by_keyword(kw).ok().flatten());
                    if let Some(f) = f {
                        let target = magpie_core::favorites::expand_quicklink(&f.payload, rest);
                        open_url_and_hide(&ui, &target);
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
            if note_id < 0 {
                open_project_task_by_id(&s, note_id);
            } else if let Some(ui) = w.upgrade() {
                open_note_into_notes_mode(&ui, &s, note_id);
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_board_set_status(move |note_id, line_index, status| {
            if note_id < 0 {
                // Project file-task: only a done⇄open flip is meaningful on a file.
                // Route drops onto Done/To-do through the line toggle; ignore "Doing".
                let target = magpie_app::tasks::status_from_column(status);
                if !matches!(target, magpie_app::tasks::Status::Doing) {
                    let want_done = matches!(target, magpie_app::tasks::Status::Done);
                    let is_done = project_task_is_done(&s, note_id, line_index as usize);
                    if is_done != Some(want_done) {
                        toggle_project_task_by_id(&s, note_id, line_index as usize);
                    }
                }
            } else {
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
    // Live project file sync (Phase 18b): watch the syncable projects' task files and
    // refresh the Projects view when they change on disk. Idle until a scan populates
    // projects, and only ever watches specific project dirs (never whole roots).
    spawn_project_watcher(state.clone(), weak.clone());
    // In-app scheduler for "While open" jobs (Phase 21).
    spawn_job_scheduler(state.clone());
    // Keep the hotkey manager alive for the whole run.
    let _hotkeys = spawn_hotkeys(&cfg, state.clone(), weak.clone(), scratch.as_weak());
    // Keep the tray icon alive for the whole run. It lives in a thread-local
    // (not a local binding) so the ticker below can reach it from the event
    // loop; see the `TRAY` thread_local for why (`TrayIcon` isn't `Send`).
    TRAY.with(|t| {
        *t.borrow_mut() = build_tray(
            weak.clone(),
            state.clone(),
            popover.as_weak(),
            scratch.as_weak(),
        )
    });

    // Dev hook: show the scratch window on launch for screenshots (no-op otherwise).
    if std::env::var("MAGPIE_SHOW_SCRATCH").is_ok() {
        let sw = scratch.as_weak();
        let s = state.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(sc) = sw.upgrade() {
                toggle_scratch(&sc, &s);
            }
        });
    }

    // If the panel was pinned last session, re-open it docked once the loop starts.
    // Don't raise-to-front here — re-opening on launch must not steal focus.
    if state
        .popover_pinned
        .load(std::sync::atomic::Ordering::Relaxed)
    {
        let pw = popover.as_weak();
        let s = state.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(p) = pw.upgrade() {
                refresh_popover(&p, &s);
                p.set_tab(
                    s.popover_default_tab
                        .load(std::sync::atomic::Ordering::Relaxed),
                );
                p.set_pinned(true);
                present_popover(&p, &s, None);
                let _ = p.show();
            }
        });
    }

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
            // A few starter snippets (the command packs seed separately, so gate on
            // our own keyword rather than an empty list).
            if store.snippet_by_keyword(";meet").ok().flatten().is_none() {
                let seed: &[(&str, &str, &str, &str)] = &[
                    ("Meeting notes", ";meet", "## {date:%a %d %b} — {input:Meeting with}\n\n**Attendees:** {cursor}\n**Notes**\n-\n**Action items**\n- [ ]", "snippet"),
                    ("Today's date", ";today", "{date:%A, %d %B %Y}", "snippet"),
                    ("Email signature", ";sig", "Best,\nMarkus", "snippet"),
                    ("Shrug", ";shrug", "¯\\_(ツ)_/¯", "snippet"),
                    ("Wrap clipboard in code fence", ";code", "```\n{clipboard}\n```", "snippet"),
                    ("git: undo last commit, keep changes", "", "git reset --soft HEAD~1", "command"),
                    ("docker: remove stopped containers", "", "docker container prune -f", "command"),
                ];
                for (name, kw, body, kind) in seed {
                    let _ = store.add_snippet(name, kw, body, kind, now_ms());
                }
            }
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

    // Seed the kubectl/gcloud/bq command packs once (idempotent by keyword), so
    // every user — new or existing — gets them; later deletions stick.
    if !cfg.command_packs_seeded {
        {
            let store = match state.store.lock() {
                Ok(g) => g,
                Err(e) => e.into_inner(),
            };
            for (name, keyword, body) in magpie_core::snippets::default_command_packs() {
                if store.snippet_by_keyword(keyword).ok().flatten().is_none() {
                    let _ = store.add_snippet(name, keyword, body, "command", now_ms());
                }
            }
        }
        refresh_expand_keywords(&state);
        let cfg_path = data_dir().join("config.toml");
        let mut c = magpie_app::config::load_or_default(&cfg_path);
        c.command_packs_seeded = true;
        let _ = magpie_app::config::save(&c, &cfg_path);
    }

    // Expand-as-you-type (Phase 16b): reflect the saved preference in the toggle and,
    // if it was on, start the global monitor now (needs macOS Input Monitoring).
    if cfg.expand_as_you_type {
        let started = start_expand_monitor(&weak, &state);
        if let Some(ui) = weak.upgrade() {
            ui.global::<SnippetsStore>().set_expand_as_you_type(started);
        }
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
    scratch: slint::Weak<ScratchWindow>,
) -> Option<tray_icon::TrayIcon> {
    use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
    use tray_icon::TrayIconBuilder;

    let menu = Menu::new();
    let show = MenuItem::new("Open Magpie", true, None);
    let clipboard_item = MenuItem::new("Quick clipboard…", true, None);
    let scratch_item = MenuItem::new("Scratch pad", true, None);
    let quit = MenuItem::new("Quit Magpie", true, None);
    menu.append(&show).ok()?;
    menu.append(&clipboard_item).ok()?;
    menu.append(&scratch_item).ok()?;
    menu.append(&PredefinedMenuItem::separator()).ok()?;
    menu.append(&quit).ok()?;
    let show_id = show.id().clone();
    let clipboard_id = clipboard_item.id().clone();
    let scratch_id = scratch_item.id().clone();
    let quit_id = quit.id().clone();

    let mut builder = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        // Clicking the menu-bar icon shows this dropdown (standard macOS behavior) — it
        // does NOT open the mini clipboard GUI directly; the user picks from the menu.
        .with_menu_on_left_click(true)
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

    // Menu clicks: Open Magpie / Quick clipboard / Scratch pad / Quit. This IS the
    // tray dropdown (shown on left-click now) — no separate popover-on-click handler.
    {
        let w = weak.clone();
        let s = state.clone();
        let pop = popover.clone();
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
                } else if ev.id == clipboard_id {
                    // The quick clipboard popover, opened only when chosen from the menu.
                    let pw = pop.clone();
                    let s = s.clone();
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(p) = pw.upgrade() {
                            if p.window().is_visible() {
                                let _ = p.hide();
                            } else {
                                refresh_popover(&p, &s);
                                p.set_tab(
                                    s.popover_default_tab
                                        .load(std::sync::atomic::Ordering::Relaxed),
                                );
                                p.set_pinned(
                                    s.popover_pinned.load(std::sync::atomic::Ordering::Relaxed),
                                );
                                present_popover(&p, &s, None);
                                let _ = p.show();
                                magpie_platform::raise_popover_front();
                            }
                        }
                    });
                } else if ev.id == scratch_id {
                    let sw = scratch.clone();
                    let s = s.clone();
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(sc) = sw.upgrade() {
                            toggle_scratch(&sc, &s);
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
        // "paste" matches "Paste", "Paste & keep open" and "Assign quick-paste slot".
        let ids: Vec<&str> = filter_actions("paste").iter().map(|r| r.0).collect();
        assert_eq!(ids, vec!["paste", "keep", "slot"]);
        // Case-insensitive, single match.
        let ids: Vec<&str> = filter_actions("PIN").iter().map(|r| r.0).collect();
        assert_eq!(ids, vec!["pin"]);
        // No match: empty.
        assert!(filter_actions("zzzznomatch").is_empty());
    }
}
