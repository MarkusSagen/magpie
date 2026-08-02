use magpie_app::config::Config;
use magpie_app::retention::policy_from_config;

#[test]
fn defaults_are_all_none() {
    let p = policy_from_config(&Config::default());
    assert!(p.max_entries.is_none() && p.max_age_ms.is_none() && p.max_image_bytes.is_none());
    assert!(p.is_noop());
}

#[test]
fn conversions_days_and_mb() {
    let cfg = Config {
        max_entries: Some(500),
        max_age_days: Some(30),
        max_image_mb: Some(200),
        ..Config::default()
    };
    let p = policy_from_config(&cfg);
    assert_eq!(p.max_entries, Some(500));
    assert_eq!(p.max_age_ms, Some(30 * 86_400_000));
    assert_eq!(p.max_image_bytes, Some(200 * 1_048_576));
}

#[test]
fn partial_config_toml_loads_with_retention_defaults() {
    let dir = std::env::temp_dir().join(format!("magpie-ret-cfg-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("config.toml");
    std::fs::write(
        &path,
        "launcher_hotkey = \"super+ctrl+v\"\nquick_paste_hotkeys = []\npaste_on_select = true\napp_denylist = []\n",
    )
    .unwrap();
    let cfg = magpie_app::config::load_or_default(&path);
    assert!(cfg.max_entries.is_none());
    std::fs::remove_dir_all(&dir).ok();
}
