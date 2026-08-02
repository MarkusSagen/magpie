//! Windows adapter tests (run on the windows-latest CI runner). The system
//! clipboard is global, so the clipboard round-trip is a single test.
#![cfg(target_os = "windows")]

use magpie_core::Content;
use magpie_platform::os::autostart_windows::WinAutostart;
use magpie_platform::os::windows::WinClipboard;
use magpie_platform::{Autostart, Clipboard};

#[test]
fn windows_clipboard_roundtrip_and_sequence_number() {
    let mut c = WinClipboard::new().unwrap();
    let prior = c.snapshot().content;

    let before = c.snapshot().change_token;
    let payload = "magpie-win-⚡-42";
    c.set_text(payload).unwrap();
    let after = c.snapshot();

    assert!(
        after.change_token >= before,
        "clipboard sequence number does not go backwards"
    );
    match &after.content {
        Some(Content::Text(t)) => assert_eq!(t, payload),
        _ => panic!("expected our text back from the Windows clipboard"),
    }

    // set_content(Files) lands as newline-joined text in v1
    c.set_content(&Content::Files(vec!["C:/a".into(), "C:/b".into()]))
        .unwrap();
    match c.snapshot().content {
        Some(Content::Text(t)) => assert_eq!(t, "C:/a\nC:/b"),
        _ => panic!("files should round-trip as newline-joined text"),
    }

    if let Some(Content::Text(t)) = prior {
        let _ = c.set_text(&t);
    }
}

#[test]
fn windows_registry_autostart_enable_disable() {
    // Unique value name so we never touch the user's real autostart entry.
    let name = format!("MagpieTest{}", std::process::id());
    let a = WinAutostart {
        value_name: name,
        exe_path: r"C:\Temp\magpie.exe".into(),
    };

    assert!(!a.is_enabled(), "fresh test value is absent");
    a.set_enabled(true).unwrap();
    assert!(a.is_enabled(), "value present after enable");
    a.set_enabled(false).unwrap();
    a.set_enabled(false).unwrap(); // idempotent
    assert!(!a.is_enabled(), "value removed after disable");
}
