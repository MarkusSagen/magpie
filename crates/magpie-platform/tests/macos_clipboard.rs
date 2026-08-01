//! Real macOS clipboard round-trip (host-only). The system pasteboard is a
//! single shared resource, so this is ONE sequential test (parallel pasteboard
//! access traps). Saves and restores the prior clipboard text to stay polite.
#![cfg(target_os = "macos")]

use magpie_core::Content;
use magpie_platform::os::macos::MacClipboard;
use magpie_platform::Clipboard;

#[test]
fn clipboard_roundtrip_and_change_token() {
    let mut c = MacClipboard::new().unwrap();
    let prior = c.snapshot().content;

    // 1) text round-trips and the change token advances after a write.
    let before = c.snapshot().change_token;
    let payload = "magpie-roundtrip-⚡-42";
    c.set_text(payload).unwrap();
    let after = c.snapshot();
    assert!(
        after.change_token >= before,
        "changeCount does not go backwards on write"
    );
    match &after.content {
        Some(Content::Text(t)) => assert_eq!(t, payload),
        _ => panic!("expected our text back from the pasteboard"),
    }
    assert!(!after.concealed, "our own write is not marked concealed");

    // 2) set_content(Files) lands as newline-joined text in v1.
    c.set_content(&Content::Files(vec!["/tmp/a".into(), "/tmp/b".into()]))
        .unwrap();
    match c.snapshot().content {
        Some(Content::Text(t)) => assert_eq!(t, "/tmp/a\n/tmp/b"),
        _ => panic!("files should round-trip as newline-joined text in v1"),
    }

    // restore prior clipboard text if there was any
    if let Some(Content::Text(t)) = prior {
        let _ = c.set_text(&t);
    }
}
