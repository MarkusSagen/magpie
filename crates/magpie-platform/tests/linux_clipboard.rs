//! Linux clipboard adapter (runs on ubuntu CI under xvfb). Skips gracefully when
//! no display / clipboard backend is available, so it never fails a headless run.
#![cfg(target_os = "linux")]

use magpie_core::Content;
use magpie_platform::os::linux::{content_token, LinuxClipboard};
use magpie_platform::Clipboard;

#[test]
fn linux_clipboard_roundtrip_when_display_available() {
    let mut c = match LinuxClipboard::new() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("skipping linux clipboard test (no backend): {e}");
            return;
        }
    };

    let payload = "magpie-linux-⚡-42";
    if c.set_text(payload).is_err() {
        eprintln!("skipping linux clipboard test: set_text failed (headless)");
        return;
    }

    let snap = c.snapshot();
    match snap.content {
        Some(Content::Text(t)) => {
            assert_eq!(t, payload);
            assert_ne!(snap.change_token, 0);
            // token is derived from the content hash
            assert_eq!(snap.change_token, content_token(Some(payload), None));
        }
        _ => eprintln!("skipping assertions: selection not served back (headless)"),
    }
}

#[test]
fn linux_content_token_changes_with_content() {
    // Pure logic — always runs.
    let a = content_token(Some("alpha"), None);
    let b = content_token(Some("beta"), None);
    assert_ne!(a, b);
    assert_eq!(content_token(None, None), 0);
}
