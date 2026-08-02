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
}

impl Default for Config {
    fn default() -> Self {
        Config {
            launcher_hotkey: "super+ctrl+v".into(),
            quick_paste_hotkeys: (1..=9).map(|n| format!("super+ctrl+{n}")).collect(),
            paste_on_select: true,
            app_denylist: Vec::new(),
            max_entries: None,
            max_age_days: None,
            max_image_mb: None,
        }
    }
}

pub fn load_or_default(path: &Path) -> Config {
    match std::fs::read_to_string(path) {
        Ok(s) => toml::from_str(&s).unwrap_or_default(),
        Err(_) => Config::default(),
    }
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
        assert_eq!(c.launcher_hotkey, "super+ctrl+v");
    }
}
