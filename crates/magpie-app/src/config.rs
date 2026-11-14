use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub launcher_hotkey: String,
    pub quick_paste_hotkeys: Vec<String>,
    pub paste_on_select: bool,
    pub app_denylist: Vec<String>,
    // Opt-in retention caps (off by default). `#[serde(default)]` keeps older
    // config files (without these keys) loading.
    #[serde(default)]
    pub max_entries: Option<i64>,
    #[serde(default)]
    pub max_age_days: Option<i64>,
    #[serde(default)]
    pub max_image_mb: Option<i64>,
    // Display-only masking (opt-in; empty by default). Separate from the
    // never-capture skip path.
    #[serde(default)]
    pub mask_apps: Vec<String>,
    #[serde(default)]
    pub mask_patterns: Vec<String>,
    #[serde(default = "default_mask_visible_chars")]
    pub mask_visible_chars: i64,
    /// Fetch a link's site favicon (from the site's own /favicon.ico) for display.
    #[serde(default = "default_true")]
    pub fetch_link_favicons: bool,
    /// Fetch a rich preview thumbnail (OG image) for a bookmark ON EXPLICIT SAVE
    /// only — never for merely-copied URLs (privacy).
    #[serde(default = "default_true")]
    pub fetch_link_previews: bool,
    /// Open the Today dashboard (instead of the clipboard list) each time the
    /// launcher is summoned.
    #[serde(default)]
    pub open_to_today: bool,
    /// Folder synced with notes via `--sync-vault` (Markdown files, one per note).
    #[serde(default)]
    pub vault_path: Option<String>,
    /// Opt-in: run a periodic background 3-way reconcile against `vault_path`
    /// while Magpie is running, so external edits to the vault's Markdown files
    /// are pulled in live (and note edits pushed out) without a manual
    /// `--sync-vault`. Off by default; requires `vault_path` to also be set.
    #[serde(default)]
    pub vault_watch: bool,
    /// Record a completed task-timer session as an org `CLOCK:` line in the
    /// note's `:LOGBOOK:` drawer, in addition to the `time_entries` DB row.
    #[serde(default = "default_true")]
    pub log_clock_entries: bool,
    /// Legacy dark/light flag. Superseded by `theme` (a named-theme id); kept
    /// only so an older config's choice migrates into `theme` on load. Not read
    /// once `theme` is set.
    #[serde(default = "default_true")]
    pub theme_dark: bool,
    /// Selected colour-theme id (see `themes::THEMES`), e.g. "dark", "light",
    /// "catppuccin-mocha". Empty in an older config → migrated from `theme_dark`
    /// in `load_or_default`. Chosen from the sidebar theme picker.
    #[serde(default)]
    pub theme: String,
    /// Region for locale-dependent holiday answers (`SE`/`US`/`UK`/`DE`). Empty →
    /// detected from the OS locale on load (Sweden fallback). Controls the
    /// Christmas eve switch and holiday labels in the date answer card.
    #[serde(default)]
    pub holiday_region: String,
    /// Home zone for the world clock (an IANA id or a city name). Empty → the
    /// starter default. Marked "you" and used as the 24-hour strip's day anchor.
    #[serde(default)]
    pub world_clock_home: String,
    /// Cities shown in the world clock (IANA ids or city names), persisted as you
    /// Add/remove them. Empty → a starter set on first load.
    #[serde(default)]
    pub world_clock_cities: Vec<String>,
    /// Expand snippet keywords system-wide as you type (Phase 16b). OFF by default;
    /// needs macOS Input Monitoring + Accessibility. Experimental.
    #[serde(default)]
    pub expand_as_you_type: bool,
    /// Roots scanned for local projects (Phase 18). Empty → `~/code` + `~/Documents`.
    #[serde(default)]
    pub projects_roots: Vec<String>,
    /// Project folders the user chose to ignore (display form, e.g. `~/code/x`).
    #[serde(default)]
    pub projects_ignored: Vec<String>,
    /// One-time marker: the kubectl/gcloud/bq command packs have been seeded into
    /// snippets. Set true after the first seed so user deletions stick.
    #[serde(default)]
    pub command_packs_seeded: bool,
    /// Which tray-popover tab opens by default: `clipboard` (default) / `today` /
    /// `tasks` / `daily`. The popover is the quick, from-anywhere clipboard menu.
    #[serde(default = "default_popover_tab")]
    pub popover_default_tab: String,
    /// Per-category "open with" application map (Phase: open files in preferred
    /// apps). Keys are categories — `text`, `code`, `image`, `pdf`, `ebook`,
    /// `archive`, `audio`, `video`, `other` — values are macOS app names passed to
    /// `open -a <App>` (or a bundle path). Missing category → the system default.
    #[serde(default)]
    pub open_with: std::collections::BTreeMap<String, String>,
    /// The quick-clipboard popover is pinned as a persistent side panel: it docks
    /// to the right screen edge and re-opens automatically on launch. When false
    /// it's the transient, tray-anchored popover. Toggled by the pin button.
    #[serde(default)]
    pub popover_pinned: bool,
    /// Persisted popover panel size in logical pixels (driven by the resize grip).
    /// Clamped by `panel::clamp_size` on load and on every resize.
    #[serde(default = "default_popover_width")]
    pub popover_width: i64,
    #[serde(default = "default_popover_height")]
    pub popover_height: i64,
    /// Sidebar group headers the user has collapsed (keys: `capture`, `organize`,
    /// `tools`, `favorites`). Persisted so the sidebar reopens as left off.
    #[serde(default)]
    pub sidebar_collapsed_groups: Vec<String>,
    /// Sidebar sections the user hid via Settings → Sidebar (keys: `snippets`, `notes`,
    /// `tasks`, `projects`, `bookmarks`, `secrets`, `files`, `calculator`, `data`,
    /// `stats`). Hidden sections stay reachable via `G x` / `⌘K`. Clipboard can't hide.
    #[serde(default)]
    pub sidebar_hidden: Vec<String>,
    /// Pomodoro: roll straight into the next phase (true, default) or pause at each
    /// phase end so the next one starts by hand.
    #[serde(default = "default_true")]
    pub pomodoro_auto_continue: bool,
    /// Ask "keep or discard?" after this many idle minutes while a timer runs
    /// (macOS; 0 = off). Read at startup.
    #[serde(default = "default_idle_threshold_min")]
    pub idle_threshold_min: i64,
    /// How to resolve an overlapping edit when syncing a project file: `ask` (hold
    /// the file + surface the conflict, default) or `file` (the disk always wins).
    #[serde(default = "default_project_sync_conflict")]
    pub project_sync_conflict: String,
    /// Keep each project's `.magpie/` out of git by appending it to that repo's
    /// private `.git/info/exclude` on scan (never edits a tracked `.gitignore`).
    #[serde(default)]
    pub projects_keep_magpie_out_of_git: bool,
    /// Global hotkey that shows/hides the floating scratch pad (Phase 20). Separate
    /// from `launcher_hotkey`. Default ⌘⌥Space. If already taken, Magpie falls back to
    /// the tray menu + ⌘K action and notifies once.
    #[serde(default = "default_scratch_hotkey")]
    pub scratch_hotkey: String,
}

fn default_scratch_hotkey() -> String {
    "super+alt+space".into()
}

fn default_project_sync_conflict() -> String {
    "ask".into()
}

fn default_popover_width() -> i64 {
    360
}

fn default_popover_height() -> i64 {
    440
}

fn default_popover_tab() -> String {
    "clipboard".into()
}

fn default_mask_visible_chars() -> i64 {
    3
}

fn default_idle_threshold_min() -> i64 {
    10
}

fn default_true() -> bool {
    true
}

impl Default for Config {
    fn default() -> Self {
        Config {
            launcher_hotkey: "super+shift+space".into(),
            quick_paste_hotkeys: (1..=9).map(|n| format!("super+ctrl+{n}")).collect(),
            paste_on_select: true,
            app_denylist: Vec::new(),
            max_entries: None,
            max_age_days: None,
            max_image_mb: None,
            mask_apps: Vec::new(),
            mask_patterns: Vec::new(),
            mask_visible_chars: 3,
            fetch_link_favicons: true,
            fetch_link_previews: true,
            open_to_today: false,
            vault_path: None,
            vault_watch: false,
            log_clock_entries: true,
            theme_dark: true,
            theme: "dark".into(),
            holiday_region: String::new(),
            world_clock_home: String::new(),
            world_clock_cities: Vec::new(),
            expand_as_you_type: false,
            projects_roots: Vec::new(),
            projects_ignored: Vec::new(),
            command_packs_seeded: false,
            popover_default_tab: default_popover_tab(),
            open_with: std::collections::BTreeMap::new(),
            popover_pinned: false,
            popover_width: default_popover_width(),
            popover_height: default_popover_height(),
            sidebar_collapsed_groups: Vec::new(),
            sidebar_hidden: Vec::new(),
            pomodoro_auto_continue: true,
            idle_threshold_min: default_idle_threshold_min(),
            project_sync_conflict: default_project_sync_conflict(),
            projects_keep_magpie_out_of_git: false,
            scratch_hotkey: default_scratch_hotkey(),
        }
    }
}

/// The world-clock starter set used when the config has none yet (matches the
/// reference screen). IANA ids so they're unambiguous across locales.
pub fn default_world_clock_cities() -> Vec<String> {
    [
        "Europe/London",
        "America/New_York",
        "America/Los_Angeles",
        "Asia/Tokyo",
        "Australia/Sydney",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// Detect the OS locale for a first-run `holiday_region` default. Reads the usual
/// POSIX env vars; `magpie_core::holidays::Region::from_locale` falls back to SE.
fn detect_locale() -> String {
    for key in ["LC_ALL", "LC_TIME", "LANG", "LANGUAGE"] {
        if let Ok(v) = std::env::var(key) {
            if !v.is_empty() {
                return v;
            }
        }
    }
    String::new()
}

pub fn load_or_default(path: &Path) -> Config {
    let mut cfg = match std::fs::read_to_string(path) {
        Ok(s) => toml::from_str(&s).unwrap_or_default(),
        Err(_) => Config::default(),
    };
    // Migrate an older config that only had the dark/light bool.
    if cfg.theme.is_empty() {
        cfg.theme = if cfg.theme_dark { "dark" } else { "light" }.into();
    }
    // First run (or older config): default the holiday region from the OS locale.
    if cfg.holiday_region.is_empty() {
        cfg.holiday_region = magpie_core::holidays::Region::from_locale(&detect_locale())
            .id()
            .to_string();
    }
    // First run: seed a world-clock home + starter cities.
    if cfg.world_clock_home.is_empty() {
        cfg.world_clock_home = "Europe/Stockholm".to_string();
    }
    if cfg.world_clock_cities.is_empty() {
        cfg.world_clock_cities = default_world_clock_cities();
    }
    if cfg.projects_roots.is_empty() {
        cfg.projects_roots = vec!["~/code".to_string(), "~/Documents".to_string()];
    }
    cfg
}

pub fn save(cfg: &Config, path: &Path) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let s = toml::to_string_pretty(cfg).map_err(std::io::Error::other)?;
    std::fs::write(path, s)
}

/// Sidebar sections that Settings → Sidebar can hide, as `(key, label)` in sidebar order.
pub const SIDEBAR_SECTIONS: [(&str, &str); 10] = [
    ("snippets", "Snippets"),
    ("notes", "Notes"),
    ("tasks", "Tasks"),
    ("projects", "Projects"),
    ("bookmarks", "Bookmarks"),
    ("secrets", "Secrets"),
    ("files", "Files"),
    ("calculator", "Calculator"),
    ("data", "Data"),
    ("stats", "Stats"),
];

/// Whether sidebar section `key` is shown. Clipboard is always shown.
pub fn sidebar_shown(cfg: &Config, key: &str) -> bool {
    key == "clipboard" || !cfg.sidebar_hidden.iter().any(|k| k == key)
}

/// Show or hide sidebar section `key` (no-op for clipboard / duplicate entries).
pub fn set_sidebar_shown(cfg: &mut Config, key: &str, shown: bool) {
    cfg.sidebar_hidden.retain(|k| k != key);
    if !shown && key != "clipboard" {
        cfg.sidebar_hidden.push(key.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_has_nine_quick_paste_hotkeys() {
        let c = Config::default();
        assert_eq!(c.quick_paste_hotkeys.len(), 9);
        assert_eq!(c.quick_paste_hotkeys[0], "super+ctrl+1");
        assert!(c.paste_on_select);
    }

    #[test]
    fn save_then_load_roundtrips() {
        let dir = std::env::temp_dir().join(format!("magpie-cfg-{}", std::process::id()));
        let path = dir.join("config.toml");
        let c = Config {
            paste_on_select: false,
            app_denylist: vec!["Secret".into()],
            ..Config::default()
        };
        save(&c, &path).unwrap();
        let loaded = load_or_default(&path);
        assert!(!loaded.paste_on_select);
        assert_eq!(loaded.app_denylist, vec!["Secret".to_string()]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn missing_file_yields_default() {
        let c = load_or_default(std::path::Path::new("/nonexistent/magpie/nope.toml"));
        assert_eq!(c.launcher_hotkey, "super+shift+space");
    }

    #[test]
    fn masking_defaults_are_empty_with_three_visible() {
        let c = Config::default();
        assert!(c.mask_apps.is_empty());
        assert!(c.mask_patterns.is_empty());
        assert_eq!(c.mask_visible_chars, 3);
    }

    #[test]
    fn old_config_without_masking_keys_loads_with_defaults() {
        let toml = "launcher_hotkey = \"super+ctrl+v\"\nquick_paste_hotkeys = []\npaste_on_select = true\napp_denylist = []\n";
        let c: Config = toml::from_str(toml).unwrap();
        assert!(c.mask_apps.is_empty());
        assert_eq!(c.mask_visible_chars, 3);
        assert!(c.fetch_link_favicons); // defaults on
        assert!(c.fetch_link_previews); // defaults on
        assert!(c.vault_path.is_none());
    }

    #[test]
    fn sidebar_visibility_round_trips_and_clipboard_never_hides() {
        let mut c = Config::default();
        assert!(SIDEBAR_SECTIONS.iter().all(|(k, _)| sidebar_shown(&c, k)));
        set_sidebar_shown(&mut c, "stats", false);
        set_sidebar_shown(&mut c, "stats", false); // idempotent
        set_sidebar_shown(&mut c, "clipboard", false); // ignored
        assert_eq!(c.sidebar_hidden, vec!["stats".to_string()]);
        assert!(!sidebar_shown(&c, "stats"));
        assert!(sidebar_shown(&c, "clipboard"));
        let back: Config = toml::from_str(&toml::to_string(&c).unwrap()).unwrap();
        assert!(!sidebar_shown(&back, "stats"));
        set_sidebar_shown(&mut c, "stats", true);
        assert!(c.sidebar_hidden.is_empty());
        // Older configs without the key load with everything shown.
        let text: String = toml::to_string(&Config::default())
            .unwrap()
            .lines()
            .filter(|l| !l.starts_with("sidebar_hidden"))
            .map(|l| format!("{l}\n"))
            .collect();
        let old: Config = toml::from_str(&text).unwrap();
        assert!(old.sidebar_hidden.is_empty());
    }
}
