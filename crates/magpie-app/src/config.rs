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
}

fn default_mask_visible_chars() -> i64 {
    3
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
}
