use crate::policy::{CapturePolicy, Decision};
use crate::traits::{Clipboard, SourceApp};
use magpie_core::CaptureEvent;

pub struct Watcher<C: Clipboard, S: SourceApp> {
    clipboard: C,
    source: S,
    policy: CapturePolicy,
    last_token: Option<u64>,
}

impl<C: Clipboard, S: SourceApp> Watcher<C, S> {
    pub fn new(clipboard: C, source: S, policy: CapturePolicy) -> Self {
        Watcher {
            clipboard,
            source,
            policy,
            last_token: None,
        }
    }

    pub fn policy_mut(&mut self) -> &mut CapturePolicy {
        &mut self.policy
    }

    pub fn poll_once(&mut self, now_ms: i64) -> Option<CaptureEvent> {
        let snap = self.clipboard.snapshot();
        if self.last_token == Some(snap.change_token) {
            return None;
        }
        self.last_token = Some(snap.change_token);
        let app = self.source.frontmost();
        match self.policy.decide(&snap, app.as_ref()) {
            Decision::Keep => snap.content.map(|content| CaptureEvent {
                content,
                source_app: app,
                copied_at_ms: now_ms,
            }),
            Decision::Skip(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::CapturePolicy;
    use crate::traits::{Clipboard, ClipboardSnapshot, SourceApp};
    use magpie_core::{AppInfo, Content};

    struct FakeClip {
        snaps: Vec<ClipboardSnapshot>,
        i: usize,
    }
    impl Clipboard for FakeClip {
        fn snapshot(&mut self) -> ClipboardSnapshot {
            let s = self.snaps[self.i.min(self.snaps.len() - 1)].clone_for_test();
            if self.i < self.snaps.len() - 1 {
                self.i += 1;
            }
            s
        }
        fn set_text(&mut self, _t: &str) -> Result<(), String> {
            Ok(())
        }
        fn set_content(&mut self, _c: &Content) -> Result<(), String> {
            Ok(())
        }
    }
    impl ClipboardSnapshot {
        fn clone_for_test(&self) -> ClipboardSnapshot {
            ClipboardSnapshot {
                content: self.content.clone(),
                change_token: self.change_token,
                concealed: self.concealed,
            }
        }
    }
    struct FakeApp(Option<AppInfo>);
    impl SourceApp for FakeApp {
        fn frontmost(&self) -> Option<AppInfo> {
            self.0.clone()
        }
    }

    fn snap(text: &str, token: u64) -> ClipboardSnapshot {
        ClipboardSnapshot {
            content: Some(Content::Text(text.into())),
            change_token: token,
            concealed: false,
        }
    }

    #[test]
    fn first_change_emits_event_with_injected_time_and_app() {
        let clip = FakeClip {
            snaps: vec![snap("hello", 1)],
            i: 0,
        };
        let app = AppInfo {
            identifier: "com.ghostty".into(),
            display_name: "Ghostty".into(),
            icon_path: None,
        };
        let mut w = Watcher::new(clip, FakeApp(Some(app)), CapturePolicy::new());
        let ev = w.poll_once(1234).expect("event");
        assert_eq!(ev.copied_at_ms, 1234);
        assert_eq!(ev.source_app.unwrap().identifier, "com.ghostty");
    }

    #[test]
    fn same_token_does_not_re_emit() {
        let clip = FakeClip {
            snaps: vec![snap("hello", 7), snap("hello", 7)],
            i: 0,
        };
        let mut w = Watcher::new(clip, FakeApp(None), CapturePolicy::new());
        assert!(w.poll_once(1).is_some()); // first
        assert!(w.poll_once(2).is_none()); // unchanged token
    }

    #[test]
    fn policy_skip_yields_no_event() {
        let clip = FakeClip {
            snaps: vec![snap("hello", 1)],
            i: 0,
        };
        let mut policy = CapturePolicy::new();
        policy.paused = true;
        let mut w = Watcher::new(clip, FakeApp(None), policy);
        assert!(w.poll_once(1).is_none());
    }
}
