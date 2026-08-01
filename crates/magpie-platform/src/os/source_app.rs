use crate::traits::SourceApp;
use magpie_core::AppInfo;

pub struct ActiveWinSource;

impl SourceApp for ActiveWinSource {
    fn frontmost(&self) -> Option<AppInfo> {
        match active_win_pos_rs::get_active_window() {
            Ok(w) => {
                let ident = if w.app_name.is_empty() {
                    format!("pid:{}", w.process_id)
                } else {
                    w.app_name.clone()
                };
                let name = if w.app_name.is_empty() {
                    format!("pid:{}", w.process_id)
                } else {
                    w.app_name
                };
                Some(AppInfo {
                    identifier: ident,
                    display_name: name,
                    icon_path: None,
                })
            }
            Err(_) => None,
        }
    }
}
