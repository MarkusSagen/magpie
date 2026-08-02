//! Property-based tests for the pure platform logic: hotkey parsing, capture
//! policy, and the Linux content token.

use magpie_core::{AppInfo, Content};
use magpie_platform::os::linux::content_token;
use magpie_platform::{parse_hotkey, CapturePolicy, ClipboardSnapshot, Decision, SkipReason};
use proptest::prelude::*;

fn snap(text: Option<String>, concealed: bool) -> ClipboardSnapshot {
    ClipboardSnapshot {
        content: text.map(Content::Text),
        change_token: 1,
        concealed,
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(250))]

    /// A well-formed combo (>=1 modifier + a letter/digit key) parses back to the
    /// exact modifiers and key we assembled.
    #[test]
    fn hotkey_roundtrip(
        ctrl in any::<bool>(),
        alt in any::<bool>(),
        shift in any::<bool>(),
        meta in any::<bool>(),
        key in "[A-Z0-9]",
    ) {
        prop_assume!(ctrl || alt || shift || meta);
        let mut parts = Vec::new();
        if ctrl { parts.push("ctrl"); }
        if alt { parts.push("alt"); }
        if shift { parts.push("shift"); }
        if meta { parts.push("super"); }
        let key_lc = key.to_lowercase();
        parts.push(&key_lc);
        let combo = parts.join("+");

        let h = parse_hotkey(&combo).unwrap();
        prop_assert_eq!(h.mods.ctrl, ctrl);
        prop_assert_eq!(h.mods.alt, alt);
        prop_assert_eq!(h.mods.shift, shift);
        prop_assert_eq!(h.mods.meta, meta);
        prop_assert_eq!(h.key, key);
    }

    /// Parsing arbitrary junk never panics.
    #[test]
    fn hotkey_parse_never_panics(s in ".{0,40}") {
        let _ = parse_hotkey(&s);
    }

    /// Paused policy skips everything; otherwise empty content is Empty.
    #[test]
    fn policy_invariants(
        text in prop::option::of(".{0,40}"),
        concealed in any::<bool>(),
        paused in any::<bool>(),
        has_app in any::<bool>(),
    ) {
        let mut p = CapturePolicy::new();
        p.paused = paused;
        let app = has_app.then(|| AppInfo { identifier: "X".into(), display_name: "X".into(), icon_path: None });
        let d = p.decide(&snap(text.clone(), concealed), app.as_ref());

        if paused {
            prop_assert_eq!(d, Decision::Skip(SkipReason::Paused));
        } else if text.is_none() {
            prop_assert_eq!(d, Decision::Skip(SkipReason::Empty));
        } else if concealed {
            prop_assert_eq!(d, Decision::Skip(SkipReason::Concealed));
        } else {
            // no denylist/regex configured -> a present, non-concealed copy is kept
            prop_assert_eq!(d, Decision::Keep);
        }
    }

    /// content_token is deterministic and only zero for the empty clipboard.
    #[test]
    fn content_token_is_deterministic(text in ".{0,50}") {
        let a = content_token(Some(&text), None);
        let b = content_token(Some(&text), None);
        prop_assert_eq!(a, b);
        prop_assert_eq!(content_token(None, None), 0);
        // any non-empty text yields a non-zero token in practice
        if !text.is_empty() {
            prop_assert_ne!(content_token(Some(&text), None), 0);
        }
    }
}
