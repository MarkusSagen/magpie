//! End-to-end capture pipeline: a scripted fake clipboard + source app are
//! driven through the real `Watcher`; emitted `CaptureEvent`s are ingested into
//! a real `magpie_core::Store`; then we search/filter as the UI would. This is
//! the whole capture path with only the OS clipboard/window replaced by fakes.

use magpie_core::{default_query, open_in_memory, AppInfo, Content, ImageStore, Kind, Store};
use magpie_platform::{CapturePolicy, Clipboard, ClipboardSnapshot, SourceApp, Watcher};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

struct Noop;
impl ImageStore for Noop {
    fn put(&self, hash: &str, _b: &[u8]) -> std::io::Result<String> {
        Ok(hash.to_string())
    }
}

/// One observable clipboard state in a session.
#[derive(Clone)]
struct Step {
    content: Option<Content>,
    token: u64,
    concealed: bool,
    app: Option<AppInfo>,
}

fn app(name: &str) -> Option<AppInfo> {
    Some(AppInfo {
        identifier: name.into(),
        display_name: name.into(),
        icon_path: None,
    })
}
fn tstep(text: &str, token: u64, appname: &str) -> Step {
    Step {
        content: Some(Content::Text(text.into())),
        token,
        concealed: false,
        app: app(appname),
    }
}

struct FakeClip {
    steps: Arc<Vec<Step>>,
    cursor: Arc<AtomicUsize>,
}
impl Clipboard for FakeClip {
    fn snapshot(&mut self) -> ClipboardSnapshot {
        let i = self.cursor.load(Ordering::SeqCst).min(self.steps.len() - 1);
        let s = &self.steps[i];
        ClipboardSnapshot {
            content: s.content.clone(),
            change_token: s.token,
            concealed: s.concealed,
        }
    }
    fn set_text(&mut self, _t: &str) -> Result<(), String> {
        Ok(())
    }
    fn set_content(&mut self, _c: &Content) -> Result<(), String> {
        Ok(())
    }
}

struct FakeSrc {
    steps: Arc<Vec<Step>>,
    cursor: Arc<AtomicUsize>,
}
impl SourceApp for FakeSrc {
    fn frontmost(&self) -> Option<AppInfo> {
        let i = self.cursor.load(Ordering::SeqCst).min(self.steps.len() - 1);
        self.steps[i].app.clone()
    }
}

/// Drive a scripted session through Watcher -> core; return the store.
fn run_session(steps: Vec<Step>, policy: CapturePolicy) -> Store {
    let store = open_in_memory().unwrap();
    let steps = Arc::new(steps);
    let cursor = Arc::new(AtomicUsize::new(0));
    let mut watcher = Watcher::new(
        FakeClip {
            steps: steps.clone(),
            cursor: cursor.clone(),
        },
        FakeSrc {
            steps: steps.clone(),
            cursor: cursor.clone(),
        },
        policy,
    );
    for n in 0..steps.len() {
        cursor.store(n, Ordering::SeqCst);
        if let Some(ev) = watcher.poll_once(1_000_000 + n as i64 * 1000) {
            store.ingest(&ev, &Noop).unwrap();
        }
    }
    store
}

#[test]
fn realistic_session_captures_dedups_attributes_and_is_searchable() {
    // Mirrors the kind of history in the design screenshot.
    let steps = vec![
        tstep(
            "docs/superpowers/specs/2026-07-31-telemetry-env-split-design.md",
            1,
            "Ghostty",
        ),
        tstep("Vi har source_service på alla tabeller", 2, "Slack"),
        tstep("https://app.asana.com/1/1210421", 3, "Vivaldi"),
        tstep(
            "docs/superpowers/specs/2026-07-31-telemetry-env-split-design.md",
            4,
            "Ghostty",
        ), // re-copy
        tstep("maybe 24h can be a good start", 5, "Slack"),
        tstep("https://app.asana.com/1/1210421", 6, "Vivaldi"), // re-copy
    ];
    let store = run_session(steps, CapturePolicy::new());

    // 4 unique entries, two of them copied twice.
    let all = store.search(&default_query()).unwrap();
    assert_eq!(all.len(), 4);
    let doc = all
        .iter()
        .find(|e| e.full_text.starts_with("docs/"))
        .unwrap();
    assert_eq!(doc.copy_count, 2);
    let asana = all.iter().find(|e| e.full_text.contains("asana")).unwrap();
    assert_eq!(asana.copy_count, 2);
    assert_eq!(asana.kind, Kind::Link);

    // Filter by source app (Ghostty) -> only the doc path.
    let ghostty_id = doc.source_app_id.unwrap();
    let mut byapp = default_query();
    byapp.source_app_id = Some(ghostty_id);
    let g = store.search(&byapp).unwrap();
    assert_eq!(g.len(), 1);
    assert!(g[0].full_text.starts_with("docs/"));

    // Filter by type = link.
    let mut bylink = default_query();
    bylink.kind = Some(Kind::Link);
    assert_eq!(store.search(&bylink).unwrap().len(), 1);

    // Word search.
    let mut q = default_query();
    q.text = "telemetry design".into();
    let hits = store.search(&q).unwrap();
    assert_eq!(hits.len(), 1);
    assert!(hits[0].full_text.contains("telemetry"));
}

#[test]
fn concealed_and_denylisted_copies_never_reach_the_store() {
    let mut policy = CapturePolicy::new();
    policy.app_denylist = vec!["1Password".into()];

    let steps = vec![
        tstep("normal note", 1, "Ghostty"),
        Step {
            content: Some(Content::Text("hunter2".into())),
            token: 2,
            concealed: true,
            app: app("Bitwarden"),
        },
        tstep("password: s3cr3t-from-pw-manager", 3, "1Password"), // denylisted app
        tstep("another normal note", 4, "Ghostty"),
    ];
    let store = run_session(steps, policy);

    let all = store.search(&default_query()).unwrap();
    assert_eq!(all.len(), 2, "concealed + denylisted copies are dropped");
    assert!(all.iter().all(|e| e.full_text.contains("normal note")));
    assert!(all
        .iter()
        .all(|e| !e.full_text.contains("hunter2") && !e.full_text.contains("s3cr3t")));
}

#[test]
fn regex_ignored_secrets_never_reach_the_store() {
    let mut policy = CapturePolicy::new();
    policy.ignore_regexes = magpie_platform::default_ignore_regexes();

    let steps = vec![
        tstep("just a commit message", 1, "Ghostty"),
        tstep("AKIAIOSFODNN7EXAMPLE", 2, "Ghostty"), // AWS access key shape
        tstep("normal text again", 3, "Ghostty"),
    ];
    let store = run_session(steps, policy);
    let all = store.search(&default_query()).unwrap();
    assert_eq!(all.len(), 2);
    assert!(all.iter().all(|e| !e.full_text.starts_with("AKIA")));
}

#[test]
fn unchanged_change_token_is_not_recaptured() {
    // Two polls with the same token (clipboard didn't change) => one entry.
    let steps = vec![
        tstep("sticky", 7, "Ghostty"),
        tstep("sticky", 7, "Ghostty"), // same token: no re-emit
    ];
    let store = run_session(steps, CapturePolicy::new());
    let all = store.search(&default_query()).unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(
        all[0].copy_count, 1,
        "same token means the copy is not recounted"
    );
}

#[test]
fn paused_policy_captures_nothing() {
    let mut policy = CapturePolicy::new();
    policy.paused = true;
    let steps = vec![tstep("a", 1, "Ghostty"), tstep("b", 2, "Ghostty")];
    let store = run_session(steps, policy);
    assert_eq!(store.search(&default_query()).unwrap().len(), 0);
}
