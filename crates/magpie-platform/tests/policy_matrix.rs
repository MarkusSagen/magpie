//! Exhaustive precedence tests for the capture policy.

use magpie_core::{AppInfo, Content};
use magpie_platform::{CapturePolicy, ClipboardSnapshot, Decision, SkipReason};

fn snap(text: Option<&str>, concealed: bool) -> ClipboardSnapshot {
    ClipboardSnapshot {
        content: text.map(|t| Content::Text(t.into())),
        change_token: 1,
        concealed,
    }
}
fn app(id: &str) -> AppInfo {
    AppInfo { identifier: id.into(), display_name: id.into(), icon_path: None }
}

fn policy() -> CapturePolicy {
    let mut p = CapturePolicy::new();
    p.app_denylist = vec!["1Password".into()];
    p.ignore_regexes = vec![regex::Regex::new(r"SECRET-\d+").unwrap()];
    p
}

#[test]
fn precedence_paused_beats_everything() {
    let mut p = policy();
    p.paused = true;
    // Even a concealed, denylisted, secret-matching copy reports Paused first.
    let d = p.decide(&snap(Some("SECRET-123"), true), Some(&app("1Password")));
    assert_eq!(d, Decision::Skip(SkipReason::Paused));
}

#[test]
fn precedence_empty_beats_concealed_and_rest() {
    let p = policy();
    // No content -> Empty regardless of concealed flag.
    assert_eq!(p.decide(&snap(None, true), Some(&app("1Password"))), Decision::Skip(SkipReason::Empty));
}

#[test]
fn precedence_concealed_beats_denylist_and_regex() {
    let p = policy();
    let d = p.decide(&snap(Some("SECRET-123"), true), Some(&app("1Password")));
    assert_eq!(d, Decision::Skip(SkipReason::Concealed));
}

#[test]
fn precedence_denylist_beats_regex() {
    let p = policy();
    let d = p.decide(&snap(Some("SECRET-123"), false), Some(&app("1Password")));
    assert_eq!(d, Decision::Skip(SkipReason::DeniedApp));
}

#[test]
fn regex_applies_when_nothing_higher_matches() {
    let p = policy();
    let d = p.decide(&snap(Some("SECRET-999"), false), Some(&app("Ghostty")));
    assert_eq!(d, Decision::Skip(SkipReason::IgnoredContent));
}

#[test]
fn clean_copy_is_kept() {
    let p = policy();
    assert_eq!(p.decide(&snap(Some("a normal note"), false), Some(&app("Ghostty"))), Decision::Keep);
}

#[test]
fn no_source_app_is_fine_when_not_denylisted() {
    let p = policy();
    assert_eq!(p.decide(&snap(Some("hello"), false), None), Decision::Keep);
}

#[test]
fn image_content_is_kept_unless_concealed() {
    let p = policy();
    let s = ClipboardSnapshot { content: Some(Content::Image { bytes: vec![1, 2, 3] }), change_token: 5, concealed: false };
    assert_eq!(p.decide(&s, Some(&app("Preview"))), Decision::Keep);
    let s2 = ClipboardSnapshot { content: Some(Content::Image { bytes: vec![1, 2, 3] }), change_token: 6, concealed: true };
    assert_eq!(p.decide(&s2, None), Decision::Skip(SkipReason::Concealed));
}
