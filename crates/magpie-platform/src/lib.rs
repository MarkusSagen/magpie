//! `magpie-platform` — OS clipboard/window/hotkey/paste adapters behind traits,
//! plus the pure capture-policy and watcher logic that drives them.

pub mod app_icons;
pub mod defaults;
pub mod hotkey;
pub mod os;
pub mod policy;
pub mod traits;
pub mod watcher;

pub use defaults::{default_app_denylist, default_ignore_regexes};
pub use hotkey::{parse_hotkey, HotkeySpec, Mods};
pub use os::accessibility::{
    accessibility_trusted, open_accessibility_settings, prompt_accessibility,
};
pub use os::db_key::db_key;
pub use os::factory::{platform_autostart, platform_clipboard};
pub use os::notify::{
    cancel_scheduled_reminders, notify, request_notification_authorization, schedule_notification,
};
pub use os::window::{hide_and_yield_focus, raise_to_front};
pub use policy::{CapturePolicy, Decision, SkipReason};
pub use traits::{Autostart, Clipboard, ClipboardSnapshot, Paster, SourceApp};
pub use watcher::Watcher;
