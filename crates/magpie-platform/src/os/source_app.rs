use crate::app_icons::ensure_app_icon;
use crate::traits::SourceApp;
use magpie_core::AppInfo;
use std::path::PathBuf;

pub struct ActiveWinSource {
    /// Directory where resolved application icons are cached as PNGs.
    pub cache_dir: PathBuf,
}

impl SourceApp for ActiveWinSource {
    fn frontmost(&self) -> Option<AppInfo> {
        let w = active_win_pos_rs::get_active_window().ok()?;
        let ident = if w.app_name.is_empty() {
            format!("pid:{}", w.process_id)
        } else {
            w.app_name.clone()
        };
        // Resolve (and cache) the app icon once per app; best-effort.
        let icon_path = ensure_app_icon(&self.cache_dir, &ident, w.process_path.as_path())
            .and_then(|p| p.to_str().map(|s| s.to_string()));
        Some(AppInfo {
            identifier: ident.clone(),
            display_name: ident,
            icon_path,
        })
    }
}
