//! `magpie-platform` — OS clipboard/window/hotkey/paste adapters behind traits,
//! plus the pure capture-policy and watcher logic that drives them.

pub mod defaults;
pub mod hotkey;
pub mod os;
pub mod policy;
pub mod traits;
pub mod watcher;

pub use defaults::{default_app_denylist, default_ignore_regexes};
pub use hotkey::{parse_hotkey, HotkeySpec, Mods};
pub use policy::{CapturePolicy, Decision, SkipReason};
pub use traits::{Autostart, Clipboard, ClipboardSnapshot, Paster, SourceApp};
pub use watcher::Watcher;
