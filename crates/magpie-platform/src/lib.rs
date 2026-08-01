//! `magpie-platform` — OS clipboard/window/hotkey/paste adapters behind traits,
//! plus the pure capture-policy and watcher logic that drives them.

pub mod hotkey;
pub mod policy;
pub mod traits;
pub mod watcher;

pub use traits::{Autostart, Clipboard, ClipboardSnapshot, Paster, SourceApp};

#[cfg(test)]
mod smoke {
    #[test]
    fn builds() {
        assert!(true);
    }
}
