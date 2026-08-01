//! Proves the cfg-factory path works: a `Box<dyn Clipboard>` drives the real
//! `Watcher` via the blanket impl, exactly as `platform_clipboard()` returns.

use magpie_core::Content;
use magpie_platform::{CapturePolicy, Clipboard, ClipboardSnapshot, SourceApp, Watcher};

struct FakeClip {
    token: u64,
    text: String,
}
impl Clipboard for FakeClip {
    fn snapshot(&mut self) -> ClipboardSnapshot {
        ClipboardSnapshot {
            content: Some(Content::Text(self.text.clone())),
            change_token: self.token,
            concealed: false,
        }
    }
    fn set_text(&mut self, _t: &str) -> Result<(), String> {
        Ok(())
    }
    fn set_content(&mut self, _c: &Content) -> Result<(), String> {
        Ok(())
    }
}

struct NoApp;
impl SourceApp for NoApp {
    fn frontmost(&self) -> Option<magpie_core::AppInfo> {
        None
    }
}

#[test]
fn watcher_accepts_boxed_clipboard_and_emits() {
    let boxed: Box<dyn Clipboard> = Box::new(FakeClip {
        token: 1,
        text: "boxed capture".into(),
    });
    let mut w = Watcher::new(boxed, NoApp, CapturePolicy::new());
    let ev = w.poll_once(500).expect("event from boxed clipboard");
    assert_eq!(ev.copied_at_ms, 500);
    match ev.content {
        Content::Text(t) => assert_eq!(t, "boxed capture"),
        _ => panic!("expected text"),
    }
    // same token -> no re-emit (dedup works through the box too)
    assert!(w.poll_once(600).is_none());
}
