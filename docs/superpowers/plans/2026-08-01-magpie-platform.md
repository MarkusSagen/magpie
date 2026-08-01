# magpie-platform Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build `magpie-platform`, the crate that turns OS clipboard/window/hotkey/paste events into `magpie-core` types — behind traits, with all decision logic pure and unit-tested, and the thin OS adapters isolated for manual verification.

**Architecture:** Every OS capability is a trait (`Clipboard`, `SourceApp`, `SecretMarkers`, `Paster`, `Autostart`, `HotkeyManager`). Pure decision logic — the capture policy (secret matrix / denylist / regex-ignore / pause), hotkey-spec parsing, and the watcher's change-detection + event-building — lives in plain functions/structs tested with fakes and injected `now_ms`. Real OS adapters use cross-platform crates where possible (`arboard`, `active-win-pos-rs`, `global-hotkey`, `enigo`) to minimize FFI; the only raw macOS FFI is concealed-clipboard-type detection. OS adapters are `#[cfg(target_os = "…")]` and verified by documented manual smoke tests, not CI (mirrors Lumen's optional `ai` feature).

**Tech Stack:** Rust, `magpie-core` (path dep), `arboard` (clipboard read/write incl. images), `active-win-pos-rs` (frontmost app, cross-platform), `global-hotkey` (global shortcuts), `enigo` (synthetic paste), `regex`. macOS-only: `objc2` + `objc2-app-kit` (concealed-type detection). Tests use fakes + in-memory logic.

## Global Constraints

- **Depends on `magpie-core`** for `Content`, `CaptureEvent`, `AppInfo` — never redefine them.
- **Pure logic is testable:** the capture policy, hotkey parsing, and watcher event-building must not call the OS and must take `now_ms: i64` as a parameter (no wall-clock). Only the trait *adapter* impls touch the OS.
- **OS adapters are `#[cfg(target_os = "…")]`** and carry a `## Manual verification` block instead of a CI test. This plan implements **macOS** adapters only; Windows/Linux adapters are Plan 4.
- **Cross-platform crates first:** use `arboard`/`active-win-pos-rs`/`global-hotkey`/`enigo` rather than hand-rolled FFI wherever they suffice.
- **Privacy is default-deny on doubt:** if the policy cannot determine an app or sees a concealed marker, it skips capture.
- **Minimal dependencies:** only the crates named in Tech Stack.
- **Commit style:** conventional commits, one per task.

---

### Task 1: Crate scaffold + trait definitions

**Files:**
- Modify: `Cargo.toml` (workspace `members`)
- Create: `crates/magpie-platform/Cargo.toml`
- Create: `crates/magpie-platform/src/lib.rs`
- Create: `crates/magpie-platform/src/traits.rs`

**Interfaces:**
- Consumes: `magpie_core::{Content, AppInfo, CaptureEvent}`.
- Produces (in `traits.rs`, all `pub`):
  - `struct ClipboardSnapshot { pub content: Option<Content>, pub change_token: u64, pub concealed: bool }`
  - `trait Clipboard: Send { fn snapshot(&mut self) -> ClipboardSnapshot; fn set_text(&mut self, text: &str) -> anyhow::Result<()>; fn set_content(&mut self, content: &Content) -> anyhow::Result<()>; }` — **do not** add `anyhow`; use `Result<T, String>` to stay dependency-light. (So: `fn set_text(&mut self, text: &str) -> Result<(), String>;` etc.)
  - `trait SourceApp: Send { fn frontmost(&self) -> Option<AppInfo>; }`
  - `trait Paster: Send { fn paste(&self) -> Result<(), String>; }`
  - `trait Autostart { fn is_enabled(&self) -> bool; fn set_enabled(&self, on: bool) -> Result<(), String>; }`
  - `lib.rs` declares `pub mod traits;` and re-exports the trait items.

- [ ] **Step 1: Add the crate to the workspace `members`**

Edit root `Cargo.toml` `members` to `["crates/magpie-core", "crates/magpie-platform"]`.

- [ ] **Step 2: Create `crates/magpie-platform/Cargo.toml`**

```toml
[package]
name = "magpie-platform"
edition.workspace = true
version.workspace = true
license.workspace = true

[dependencies]
magpie-core = { path = "../magpie-core" }
regex = "1"
arboard = "3"
active-win-pos-rs = "0.8"
global-hotkey = "0.6"
enigo = "0.2"

[target.'cfg(target_os = "macos")'.dependencies]
objc2 = "0.5"
objc2-app-kit = { version = "0.2", features = ["NSPasteboard", "NSWorkspace", "NSRunningApplication"] }
objc2-foundation = { version = "0.2", features = ["NSString", "NSArray"] }
```

- [ ] **Step 3: Create `traits.rs`**

```rust
use magpie_core::{AppInfo, Content};

pub struct ClipboardSnapshot {
    pub content: Option<Content>,
    pub change_token: u64,
    pub concealed: bool,
}

pub trait Clipboard: Send {
    fn snapshot(&mut self) -> ClipboardSnapshot;
    fn set_text(&mut self, text: &str) -> Result<(), String>;
    fn set_content(&mut self, content: &Content) -> Result<(), String>;
}

pub trait SourceApp: Send {
    fn frontmost(&self) -> Option<AppInfo>;
}

pub trait Paster: Send {
    fn paste(&self) -> Result<(), String>;
}

pub trait Autostart {
    fn is_enabled(&self) -> bool;
    fn set_enabled(&self, on: bool) -> Result<(), String>;
}
```

- [ ] **Step 4: Create `lib.rs`**

```rust
//! `magpie-platform` — OS clipboard/window/hotkey/paste adapters behind traits,
//! plus the pure capture-policy and watcher logic that drives them.

pub mod traits;

pub use traits::{Autostart, Clipboard, ClipboardSnapshot, Paster, SourceApp};

#[cfg(test)]
mod smoke {
    #[test]
    fn builds() {
        assert!(true);
    }
}
```

- [ ] **Step 5: Build and test**

Run: `cargo test -p magpie-platform`
Expected: PASS (1 test). Note: first build pulls `arboard`/`enigo`/etc. — may take a minute.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml crates/magpie-platform
git commit -m "chore: scaffold magpie-platform crate + OS traits"
```

---

### Task 2: Capture policy (secret matrix / denylist / regex-ignore / pause)

**Files:**
- Create: `crates/magpie-platform/src/policy.rs`
- Modify: `crates/magpie-platform/src/lib.rs` (`pub mod policy;`)
- Test: `policy.rs` tests module.

**Interfaces:**
- Consumes: `ClipboardSnapshot`, `magpie_core::{AppInfo, Content}`, `regex::Regex`.
- Produces:
  - `pub enum SkipReason { Paused, Concealed, DeniedApp, IgnoredContent, Empty }`
  - `pub enum Decision { Keep, Skip(SkipReason) }`
  - `pub struct CapturePolicy { pub paused: bool, pub app_denylist: Vec<String>, pub ignore_regexes: Vec<Regex> }` with `pub fn new() -> Self` (all empty, not paused) and `pub fn decide(&self, snap: &ClipboardSnapshot, app: Option<&AppInfo>) -> Decision`.
  - Rule order (first match wins): `paused` → `Paused`; `snap.content` is `None` → `Empty`; `snap.concealed` → `Concealed`; `app` identifier ∈ `app_denylist` → `DeniedApp`; content's text (see `content_text` helper below) matches any `ignore_regexes` → `IgnoredContent`; else `Keep`.
  - `pub fn content_text(c: &Content) -> String` — the searchable text of a content (Text/Rich → the text; Files → joined `\n`; Image → `""`).

- [ ] **Step 1: Write failing tests in `policy.rs`**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use magpie_core::{AppInfo, Content};

    fn snap(content: Option<Content>, concealed: bool) -> ClipboardSnapshot {
        ClipboardSnapshot { content, change_token: 1, concealed }
    }
    fn app(id: &str) -> AppInfo {
        AppInfo { identifier: id.into(), display_name: id.into(), icon_path: None }
    }

    #[test]
    fn paused_skips_everything() {
        let mut p = CapturePolicy::new();
        p.paused = true;
        assert!(matches!(p.decide(&snap(Some(Content::Text("x".into())), false), None),
                         Decision::Skip(SkipReason::Paused)));
    }

    #[test]
    fn empty_content_is_skipped() {
        let p = CapturePolicy::new();
        assert!(matches!(p.decide(&snap(None, false), None), Decision::Skip(SkipReason::Empty)));
    }

    #[test]
    fn concealed_is_skipped() {
        let p = CapturePolicy::new();
        assert!(matches!(p.decide(&snap(Some(Content::Text("secret".into())), true), None),
                         Decision::Skip(SkipReason::Concealed)));
    }

    #[test]
    fn denylisted_app_is_skipped() {
        let mut p = CapturePolicy::new();
        p.app_denylist = vec!["com.1password".into()];
        let d = p.decide(&snap(Some(Content::Text("x".into())), false), Some(&app("com.1password")));
        assert!(matches!(d, Decision::Skip(SkipReason::DeniedApp)));
    }

    #[test]
    fn ignore_regex_skips_matching_content() {
        let mut p = CapturePolicy::new();
        p.ignore_regexes = vec![regex::Regex::new(r"AKIA[0-9A-Z]{16}").unwrap()];
        let d = p.decide(&snap(Some(Content::Text("AKIA1234567890ABCDEF".into())), false), None);
        assert!(matches!(d, Decision::Skip(SkipReason::IgnoredContent)));
    }

    #[test]
    fn normal_text_is_kept() {
        let p = CapturePolicy::new();
        let d = p.decide(&snap(Some(Content::Text("just a note".into())), false), Some(&app("com.ghostty")));
        assert!(matches!(d, Decision::Keep));
    }
}
```

- [ ] **Step 2: Add `pub mod policy;` to `lib.rs`; run to verify fail**

Run: `cargo test -p magpie-platform policy`
Expected: FAIL — items not found.

- [ ] **Step 3: Implement `policy.rs`**

```rust
use crate::traits::ClipboardSnapshot;
use magpie_core::{AppInfo, Content};
use regex::Regex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkipReason { Paused, Concealed, DeniedApp, IgnoredContent, Empty }

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision { Keep, Skip(SkipReason) }

pub struct CapturePolicy {
    pub paused: bool,
    pub app_denylist: Vec<String>,
    pub ignore_regexes: Vec<Regex>,
}

impl Default for CapturePolicy {
    fn default() -> Self { Self::new() }
}

impl CapturePolicy {
    pub fn new() -> Self {
        CapturePolicy { paused: false, app_denylist: Vec::new(), ignore_regexes: Vec::new() }
    }

    pub fn decide(&self, snap: &ClipboardSnapshot, app: Option<&AppInfo>) -> Decision {
        if self.paused {
            return Decision::Skip(SkipReason::Paused);
        }
        let content = match &snap.content {
            Some(c) => c,
            None => return Decision::Skip(SkipReason::Empty),
        };
        if snap.concealed {
            return Decision::Skip(SkipReason::Concealed);
        }
        if let Some(a) = app {
            if self.app_denylist.iter().any(|d| d == &a.identifier) {
                return Decision::Skip(SkipReason::DeniedApp);
            }
        }
        let text = content_text(content);
        if !text.is_empty() && self.ignore_regexes.iter().any(|re| re.is_match(&text)) {
            return Decision::Skip(SkipReason::IgnoredContent);
        }
        Decision::Keep
    }
}

pub fn content_text(c: &Content) -> String {
    match c {
        Content::Text(t) => t.clone(),
        Content::Rich { text, .. } => text.clone(),
        Content::Files(paths) => paths.join("\n"),
        Content::Image { .. } => String::new(),
    }
}
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p magpie-platform policy`
Expected: PASS (6 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/magpie-platform/src/policy.rs crates/magpie-platform/src/lib.rs
git commit -m "feat(platform): capture policy (secret matrix/denylist/regex/pause)"
```

---

### Task 3: Hotkey-spec parsing

**Files:**
- Create: `crates/magpie-platform/src/hotkey.rs`
- Modify: `crates/magpie-platform/src/lib.rs` (`pub mod hotkey;`)
- Test: `hotkey.rs` tests module.

**Interfaces:**
- Consumes: nothing (pure).
- Produces:
  - `pub struct Mods { pub ctrl: bool, pub alt: bool, pub shift: bool, pub meta: bool }` (meta = Cmd/Super/Win).
  - `pub struct HotkeySpec { pub mods: Mods, pub key: String }` (key normalized uppercase, e.g. `"V"`, `"1"`).
  - `pub fn parse_hotkey(s: &str) -> Result<HotkeySpec, String>` — parses `+`-separated tokens, case-insensitive. Modifier aliases: `ctrl|control`→ctrl, `alt|opt|option`→alt, `shift`→shift, `super|cmd|command|win|meta`→meta. Exactly one non-modifier token = the key; zero or two+ keys → `Err`. No modifiers → `Err` (global hotkeys need at least one).

- [ ] **Step 1: Write failing tests in `hotkey.rs`**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_super_ctrl_1() {
        let h = parse_hotkey("super+ctrl+1").unwrap();
        assert!(h.mods.meta && h.mods.ctrl && !h.mods.alt && !h.mods.shift);
        assert_eq!(h.key, "1");
    }

    #[test]
    fn aliases_and_case_insensitive() {
        let h = parse_hotkey("Cmd+Opt+Shift+v").unwrap();
        assert!(h.mods.meta && h.mods.alt && h.mods.shift);
        assert_eq!(h.key, "V");
    }

    #[test]
    fn rejects_no_modifier() {
        assert!(parse_hotkey("v").is_err());
    }

    #[test]
    fn rejects_no_or_multiple_keys() {
        assert!(parse_hotkey("ctrl").is_err());
        assert!(parse_hotkey("ctrl+a+b").is_err());
    }
}
```

- [ ] **Step 2: Add `pub mod hotkey;` to `lib.rs`; run to verify fail**

Run: `cargo test -p magpie-platform hotkey`
Expected: FAIL.

- [ ] **Step 3: Implement `hotkey.rs`**

```rust
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Mods {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub meta: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HotkeySpec {
    pub mods: Mods,
    pub key: String,
}

pub fn parse_hotkey(s: &str) -> Result<HotkeySpec, String> {
    let mut mods = Mods::default();
    let mut key: Option<String> = None;
    for raw in s.split('+') {
        let tok = raw.trim().to_lowercase();
        if tok.is_empty() {
            return Err(format!("empty token in '{s}'"));
        }
        match tok.as_str() {
            "ctrl" | "control" => mods.ctrl = true,
            "alt" | "opt" | "option" => mods.alt = true,
            "shift" => mods.shift = true,
            "super" | "cmd" | "command" | "win" | "meta" => mods.meta = true,
            _ => {
                if key.is_some() {
                    return Err(format!("more than one key in '{s}'"));
                }
                key = Some(tok.to_uppercase());
            }
        }
    }
    let key = key.ok_or_else(|| format!("no key in '{s}'"))?;
    if !(mods.ctrl || mods.alt || mods.shift || mods.meta) {
        return Err(format!("no modifier in '{s}'"));
    }
    Ok(HotkeySpec { mods, key })
}
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p magpie-platform hotkey`
Expected: PASS (4 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/magpie-platform/src/hotkey.rs crates/magpie-platform/src/lib.rs
git commit -m "feat(platform): hotkey-spec parsing"
```

---

### Task 4: Watcher — change detection + event building (pure, fake-driven)

**Files:**
- Create: `crates/magpie-platform/src/watcher.rs`
- Modify: `crates/magpie-platform/src/lib.rs` (`pub mod watcher;`)
- Test: `watcher.rs` tests module.

**Interfaces:**
- Consumes: `Clipboard`, `SourceApp`, `ClipboardSnapshot`, `CapturePolicy`/`Decision`, `magpie_core::CaptureEvent`.
- Produces:
  - `pub struct Watcher<C: Clipboard, S: SourceApp> { clipboard: C, source: S, policy: CapturePolicy, last_token: Option<u64> }`
  - `impl<C,S> Watcher<C,S> { pub fn new(clipboard: C, source: S, policy: CapturePolicy) -> Self; pub fn poll_once(&mut self, now_ms: i64) -> Option<CaptureEvent>; pub fn policy_mut(&mut self) -> &mut CapturePolicy }`
  - `poll_once`: reads a snapshot; if `change_token == last_token` returns `None` (no change); otherwise updates `last_token`, runs the policy with the current frontmost app, and on `Keep` returns `Some(CaptureEvent { content, source_app, copied_at_ms: now_ms })`; on `Skip` returns `None`. The first poll (last_token `None`) with content is treated as a change.

- [ ] **Step 1: Write failing tests in `watcher.rs`**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::CapturePolicy;
    use crate::traits::{Clipboard, ClipboardSnapshot, SourceApp};
    use magpie_core::{AppInfo, Content};

    struct FakeClip { snaps: Vec<ClipboardSnapshot>, i: usize }
    impl Clipboard for FakeClip {
        fn snapshot(&mut self) -> ClipboardSnapshot {
            let s = self.snaps[self.i.min(self.snaps.len() - 1)].clone_for_test();
            if self.i < self.snaps.len() - 1 { self.i += 1; }
            s
        }
        fn set_text(&mut self, _t: &str) -> Result<(), String> { Ok(()) }
        fn set_content(&mut self, _c: &Content) -> Result<(), String> { Ok(()) }
    }
    impl ClipboardSnapshot {
        fn clone_for_test(&self) -> ClipboardSnapshot {
            ClipboardSnapshot { content: self.content.clone(), change_token: self.change_token, concealed: self.concealed }
        }
    }
    struct FakeApp(Option<AppInfo>);
    impl SourceApp for FakeApp {
        fn frontmost(&self) -> Option<AppInfo> { self.0.clone() }
    }

    fn snap(text: &str, token: u64) -> ClipboardSnapshot {
        ClipboardSnapshot { content: Some(Content::Text(text.into())), change_token: token, concealed: false }
    }

    #[test]
    fn first_change_emits_event_with_injected_time_and_app() {
        let clip = FakeClip { snaps: vec![snap("hello", 1)], i: 0 };
        let app = AppInfo { identifier: "com.ghostty".into(), display_name: "Ghostty".into(), icon_path: None };
        let mut w = Watcher::new(clip, FakeApp(Some(app)), CapturePolicy::new());
        let ev = w.poll_once(1234).expect("event");
        assert_eq!(ev.copied_at_ms, 1234);
        assert_eq!(ev.source_app.unwrap().identifier, "com.ghostty");
    }

    #[test]
    fn same_token_does_not_re_emit() {
        let clip = FakeClip { snaps: vec![snap("hello", 7), snap("hello", 7)], i: 0 };
        let mut w = Watcher::new(clip, FakeApp(None), CapturePolicy::new());
        assert!(w.poll_once(1).is_some()); // first
        assert!(w.poll_once(2).is_none()); // unchanged token
    }

    #[test]
    fn policy_skip_yields_no_event() {
        let clip = FakeClip { snaps: vec![snap("hello", 1)], i: 0 };
        let mut policy = CapturePolicy::new();
        policy.paused = true;
        let mut w = Watcher::new(clip, FakeApp(None), policy);
        assert!(w.poll_once(1).is_none());
    }
}
```

- [ ] **Step 2: Add `pub mod watcher;` to `lib.rs`; run to verify fail**

Run: `cargo test -p magpie-platform watcher`
Expected: FAIL.

- [ ] **Step 3: Implement `watcher.rs`**

```rust
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
        Watcher { clipboard, source, policy, last_token: None }
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
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p magpie-platform watcher`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/magpie-platform/src/watcher.rs crates/magpie-platform/src/lib.rs
git commit -m "feat(platform): watcher change-detection + event building"
```

---

### Task 5: macOS clipboard adapter (arboard + concealed-type detection)

**Files:**
- Create: `crates/magpie-platform/src/os/mod.rs`
- Create: `crates/magpie-platform/src/os/macos.rs`
- Modify: `crates/magpie-platform/src/lib.rs` (`pub mod os;`)

**Interfaces:**
- Consumes: `Clipboard`, `ClipboardSnapshot`, `magpie_core::Content`.
- Produces (macOS): `pub struct MacClipboard { inner: arboard::Clipboard, last_change_count: i64 }` implementing `Clipboard`. `change_token` = `NSPasteboard.generalPasteboard.changeCount`; `concealed` = whether the pasteboard's `types` contains any of `org.nspasteboard.ConcealedType`, `org.nspasteboard.TransientType`, `org.nspasteboard.AutoGeneratedType`. Content read: prefer text (incl. detecting URL string), else image (via `arboard::Clipboard::get_image` → PNG bytes), else `None`. `set_text`/`set_content` write through `arboard`.

> This task is OS glue and cannot run in CI. It has a **Manual verification** block instead of an automated test. The `objc2` API surface below is the intended shape; confirm exact method names against the pinned `objc2-app-kit` version and adjust if the compiler disagrees — the trait contract and the `concealed`/`change_token` semantics are what matter.

- [ ] **Step 1: Create `os/mod.rs`**

```rust
#[cfg(target_os = "macos")]
pub mod macos;
```

- [ ] **Step 2: Implement `os/macos.rs`**

```rust
use crate::traits::{Clipboard, ClipboardSnapshot};
use magpie_core::Content;
use objc2_app_kit::NSPasteboard;
use objc2_foundation::NSString;

const CONCEALED_TYPES: &[&str] = &[
    "org.nspasteboard.ConcealedType",
    "org.nspasteboard.TransientType",
    "org.nspasteboard.AutoGeneratedType",
];

pub struct MacClipboard {
    inner: arboard::Clipboard,
}

impl MacClipboard {
    pub fn new() -> Result<Self, String> {
        Ok(MacClipboard {
            inner: arboard::Clipboard::new().map_err(|e| e.to_string())?,
        })
    }

    fn change_count() -> i64 {
        unsafe {
            let pb = NSPasteboard::generalPasteboard();
            pb.changeCount() as i64
        }
    }

    fn is_concealed() -> bool {
        unsafe {
            let pb = NSPasteboard::generalPasteboard();
            let Some(types) = pb.types() else { return false };
            for t in types.iter() {
                let s = t.to_string();
                if CONCEALED_TYPES.iter().any(|c| *c == s) {
                    return true;
                }
            }
            false
        }
    }
}

impl Clipboard for MacClipboard {
    fn snapshot(&mut self) -> ClipboardSnapshot {
        let change_token = Self::change_count() as u64;
        let concealed = Self::is_concealed();
        let content = if let Ok(text) = self.inner.get_text() {
            if text.is_empty() { None } else { Some(Content::Text(text)) }
        } else if let Ok(img) = self.inner.get_image() {
            // Encode RGBA -> PNG bytes for storage/hashing.
            match png_bytes(&img) {
                Some(bytes) => Some(Content::Image { bytes }),
                None => None,
            }
        } else {
            None
        };
        ClipboardSnapshot { content, change_token, concealed }
    }

    fn set_text(&mut self, text: &str) -> Result<(), String> {
        self.inner.set_text(text.to_string()).map_err(|e| e.to_string())
    }

    fn set_content(&mut self, content: &Content) -> Result<(), String> {
        match content {
            Content::Text(t) => self.inner.set_text(t.clone()).map_err(|e| e.to_string()),
            Content::Rich { text, .. } => self.inner.set_text(text.clone()).map_err(|e| e.to_string()),
            Content::Files(paths) => self.inner.set_text(paths.join("\n")).map_err(|e| e.to_string()),
            Content::Image { .. } => Err("image set not supported in v1".into()),
        }
    }
}

fn png_bytes(img: &arboard::ImageData) -> Option<Vec<u8>> {
    // arboard gives raw RGBA; we store PNG. Keep the dep surface minimal by
    // hashing raw bytes is acceptable, but a stable encoding is nicer. For v1,
    // store the raw RGBA prefixed with dimensions so identical images hash equal.
    let mut out = Vec::with_capacity(8 + img.bytes.len());
    out.extend_from_slice(&(img.width as u32).to_le_bytes());
    out.extend_from_slice(&(img.height as u32).to_le_bytes());
    out.extend_from_slice(&img.bytes);
    Some(out)
}
```

> Note the `png_bytes` helper does not actually PNG-encode in v1 — it produces a stable, dimension-tagged raw-RGBA blob so identical images dedup by hash. Real PNG encoding (for a smaller cache file and thumbnailing) is deferred to the app crate's image cache. This keeps the platform crate dependency-light.

- [ ] **Step 3: Add `pub mod os;` to `lib.rs` and compile**

Run: `cargo build -p magpie-platform`
Expected: compiles on macOS. If `objc2` method names differ, fix per the compiler and keep the semantics.

- [ ] **Step 4: Manual verification** (cannot be a CI test)

Write a throwaway example `crates/magpie-platform/examples/clip_probe.rs`:

```rust
#[cfg(target_os = "macos")]
fn main() {
    use magpie_platform::os::macos::MacClipboard;
    use magpie_platform::Clipboard;
    let mut c = MacClipboard::new().unwrap();
    let s = c.snapshot();
    println!("token={} concealed={} some={}", s.change_token, s.concealed, s.content.is_some());
}
#[cfg(not(target_os = "macos"))]
fn main() {}
```

Run `cargo run -p magpie-platform --example clip_probe`, copy some text, run again — confirm `token` increases and `some=true`. Copy from a password manager and confirm `concealed=true` (or that the app is on the denylist). Record the observed output in your report.

- [ ] **Step 5: Commit**

```bash
git add crates/magpie-platform/src/os crates/magpie-platform/src/lib.rs crates/magpie-platform/examples/clip_probe.rs
git commit -m "feat(platform): macOS clipboard adapter (arboard + concealed detection)"
```

---

### Task 6: macOS source-app adapter (active-win-pos-rs)

**Files:**
- Create: `crates/magpie-platform/src/os/source_app.rs`
- Modify: `crates/magpie-platform/src/os/mod.rs`

**Interfaces:**
- Consumes: `SourceApp`, `magpie_core::AppInfo`.
- Produces: `pub struct ActiveWinSource;` implementing `SourceApp` cross-platform via `active_win_pos_rs::get_active_window()`, mapping to `AppInfo { identifier: <app_name or process id>, display_name: <app_name>, icon_path: None }`. Returns `None` if no active window (e.g. Wayland restriction). Icons are Phase 1.

- [ ] **Step 1: Add to `os/mod.rs`**

```rust
pub mod source_app;
#[cfg(target_os = "macos")]
pub mod macos;
```

- [ ] **Step 2: Implement `os/source_app.rs`**

```rust
use crate::traits::SourceApp;
use magpie_core::AppInfo;

pub struct ActiveWinSource;

impl SourceApp for ActiveWinSource {
    fn frontmost(&self) -> Option<AppInfo> {
        match active_win_pos_rs::get_active_window() {
            Ok(w) => {
                let name = if w.app_name.is_empty() { format!("pid:{}", w.process_id) } else { w.app_name.clone() };
                Some(AppInfo {
                    identifier: if w.app_name.is_empty() { format!("pid:{}", w.process_id) } else { w.app_name },
                    display_name: name,
                    icon_path: None,
                })
            }
            Err(_) => None,
        }
    }
}
```

- [ ] **Step 3: Compile**

Run: `cargo build -p magpie-platform`
Expected: compiles.

- [ ] **Step 4: Manual verification**

Extend `clip_probe.rs` (or a new example) to print `ActiveWinSource.frontmost()` and confirm it names the frontmost app (e.g. "Ghostty"). Record output in your report.

- [ ] **Step 5: Commit**

```bash
git add crates/magpie-platform/src/os
git commit -m "feat(platform): source-app adapter via active-win-pos-rs"
```

---

### Task 7: macOS paster (enigo Cmd+V)

**Files:**
- Create: `crates/magpie-platform/src/os/paste.rs`
- Modify: `crates/magpie-platform/src/os/mod.rs`

**Interfaces:**
- Consumes: `Paster`.
- Produces: `pub struct EnigoPaster;` implementing `Paster::paste` by synthesizing the platform paste chord (macOS: Cmd+V; other OSes: Ctrl+V) via `enigo`. On macOS this requires Accessibility permission (documented).

- [ ] **Step 1: Add `pub mod paste;` to `os/mod.rs`**

- [ ] **Step 2: Implement `os/paste.rs`**

```rust
use crate::traits::Paster;
use enigo::{Direction, Enigo, Key, Keyboard, Settings};

pub struct EnigoPaster;

impl Paster for EnigoPaster {
    fn paste(&self) -> Result<(), String> {
        let mut enigo = Enigo::new(&Settings::default()).map_err(|e| e.to_string())?;
        #[cfg(target_os = "macos")]
        let modifier = Key::Meta; // Cmd
        #[cfg(not(target_os = "macos"))]
        let modifier = Key::Control;

        enigo.key(modifier, Direction::Press).map_err(|e| e.to_string())?;
        enigo.key(Key::Unicode('v'), Direction::Click).map_err(|e| e.to_string())?;
        enigo.key(modifier, Direction::Release).map_err(|e| e.to_string())?;
        Ok(())
    }
}
```

- [ ] **Step 3: Compile**

Run: `cargo build -p magpie-platform`
Expected: compiles.

- [ ] **Step 4: Manual verification**

In an example, set the clipboard to a known string, focus a text field, call `EnigoPaster.paste()`, and confirm the string is pasted. On macOS, grant Accessibility permission when prompted. Record the result in your report.

- [ ] **Step 5: Commit**

```bash
git add crates/magpie-platform/src/os
git commit -m "feat(platform): synthetic paste via enigo"
```

---

### Task 8: Global hotkey manager (global-hotkey)

**Files:**
- Create: `crates/magpie-platform/src/os/hotkeys.rs`
- Modify: `crates/magpie-platform/src/os/mod.rs`

**Interfaces:**
- Consumes: `HotkeySpec`/`Mods` from `hotkey.rs`, `global_hotkey`.
- Produces:
  - `pub fn to_global_hotkey(spec: &crate::hotkey::HotkeySpec) -> Result<global_hotkey::hotkey::HotKey, String>` — maps our `Mods`+key to `global_hotkey::hotkey::{HotKey, Modifiers, Code}`. Support keys `A–Z`, `0–9`. (This mapping is pure and IS unit-tested — building a `HotKey` value doesn't touch the OS.)
  - `pub struct Hotkeys { manager: global_hotkey::GlobalHotKeyManager }` with `new()`, `pub fn register(&self, spec: &HotkeySpec) -> Result<u32 /*id*/, String>` (returns the HotKey id). Receiving events is done in the app's event loop via `global_hotkey::GlobalHotKeyEvent::receiver()` — not wrapped here.

- [ ] **Step 1: Add `pub mod hotkeys;` to `os/mod.rs`**

- [ ] **Step 2: Write a failing unit test for the mapping in `os/hotkeys.rs`**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::hotkey::parse_hotkey;

    #[test]
    fn maps_letter_and_digit_keys() {
        let v = to_global_hotkey(&parse_hotkey("super+ctrl+v").unwrap()).unwrap();
        let one = to_global_hotkey(&parse_hotkey("super+ctrl+1").unwrap()).unwrap();
        assert_ne!(v.id(), one.id()); // distinct hotkeys
    }

    #[test]
    fn rejects_unsupported_key() {
        assert!(to_global_hotkey(&parse_hotkey("ctrl+f13").unwrap()).is_err());
    }
}
```

- [ ] **Step 3: Run to verify fail**

Run: `cargo test -p magpie-platform hotkeys`
Expected: FAIL.

- [ ] **Step 4: Implement `os/hotkeys.rs`**

```rust
use crate::hotkey::HotkeySpec;
use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::GlobalHotKeyManager;

pub fn to_global_hotkey(spec: &HotkeySpec) -> Result<HotKey, String> {
    let mut mods = Modifiers::empty();
    if spec.mods.ctrl { mods |= Modifiers::CONTROL; }
    if spec.mods.alt { mods |= Modifiers::ALT; }
    if spec.mods.shift { mods |= Modifiers::SHIFT; }
    if spec.mods.meta { mods |= Modifiers::META; }

    let code = key_to_code(&spec.key)?;
    Ok(HotKey::new(Some(mods), code))
}

fn key_to_code(key: &str) -> Result<Code, String> {
    let c = match key {
        "A" => Code::KeyA, "B" => Code::KeyB, "C" => Code::KeyC, "D" => Code::KeyD,
        "E" => Code::KeyE, "F" => Code::KeyF, "G" => Code::KeyG, "H" => Code::KeyH,
        "I" => Code::KeyI, "J" => Code::KeyJ, "K" => Code::KeyK, "L" => Code::KeyL,
        "M" => Code::KeyM, "N" => Code::KeyN, "O" => Code::KeyO, "P" => Code::KeyP,
        "Q" => Code::KeyQ, "R" => Code::KeyR, "S" => Code::KeyS, "T" => Code::KeyT,
        "U" => Code::KeyU, "V" => Code::KeyV, "W" => Code::KeyW, "X" => Code::KeyX,
        "Y" => Code::KeyY, "Z" => Code::KeyZ,
        "0" => Code::Digit0, "1" => Code::Digit1, "2" => Code::Digit2, "3" => Code::Digit3,
        "4" => Code::Digit4, "5" => Code::Digit5, "6" => Code::Digit6, "7" => Code::Digit7,
        "8" => Code::Digit8, "9" => Code::Digit9,
        other => return Err(format!("unsupported hotkey key: {other}")),
    };
    Ok(c)
}

pub struct Hotkeys {
    manager: GlobalHotKeyManager,
}

impl Hotkeys {
    pub fn new() -> Result<Self, String> {
        Ok(Hotkeys { manager: GlobalHotKeyManager::new().map_err(|e| e.to_string())? })
    }

    pub fn register(&self, spec: &HotkeySpec) -> Result<u32, String> {
        let hk = to_global_hotkey(spec)?;
        self.manager.register(hk).map_err(|e| e.to_string())?;
        Ok(hk.id())
    }
}
```

- [ ] **Step 5: Run to verify pass**

Run: `cargo test -p magpie-platform hotkeys`
Expected: PASS (2 tests).

- [ ] **Step 6: Commit**

```bash
git add crates/magpie-platform/src/os
git commit -m "feat(platform): global hotkey mapping + manager"
```

---

### Task 9: macOS autostart (LaunchAgent plist)

**Files:**
- Create: `crates/magpie-platform/src/os/autostart.rs`
- Modify: `crates/magpie-platform/src/os/mod.rs`
- Test: `autostart.rs` tests module (uses a temp dir — this IS unit-testable).

**Interfaces:**
- Consumes: `Autostart`.
- Produces:
  - `pub fn launch_agent_plist(label: &str, program: &str) -> String` — returns the plist XML for a `RunAtLoad` LaunchAgent (pure, unit-tested).
  - `pub struct MacAutostart { pub plist_path: std::path::PathBuf, pub label: String, pub program: String }` implementing `Autostart`: `is_enabled` = plist file exists; `set_enabled(true)` writes the plist; `set_enabled(false)` removes it. File IO on a real path, but the pure `launch_agent_plist` carries the logic under test.

- [ ] **Step 1: Add `#[cfg(target_os = "macos")] pub mod autostart;` to `os/mod.rs`**

- [ ] **Step 2: Write failing tests in `autostart.rs`**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plist_contains_label_program_and_runatload() {
        let xml = launch_agent_plist("io.magpie.agent", "/Applications/Magpie.app/Contents/MacOS/magpie");
        assert!(xml.contains("io.magpie.agent"));
        assert!(xml.contains("/Applications/Magpie.app/Contents/MacOS/magpie"));
        assert!(xml.contains("RunAtLoad"));
    }

    #[test]
    fn set_enabled_writes_and_removes_plist() {
        let dir = std::env::temp_dir().join(format!("magpie-autostart-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let plist = dir.join("io.magpie.agent.plist");
        let a = MacAutostart { plist_path: plist.clone(), label: "io.magpie.agent".into(), program: "/bin/magpie".into() };
        assert!(!a.is_enabled());
        a.set_enabled(true).unwrap();
        assert!(a.is_enabled() && plist.exists());
        a.set_enabled(false).unwrap();
        assert!(!a.is_enabled() && !plist.exists());
        std::fs::remove_dir_all(&dir).ok();
    }
}
```

- [ ] **Step 3: Run to verify fail**

Run: `cargo test -p magpie-platform autostart`
Expected: FAIL.

- [ ] **Step 4: Implement `autostart.rs`**

```rust
use crate::traits::Autostart;
use std::path::PathBuf;

pub fn launch_agent_plist(label: &str, program: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{label}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{program}</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
</dict>
</plist>
"#
    )
}

pub struct MacAutostart {
    pub plist_path: PathBuf,
    pub label: String,
    pub program: String,
}

impl Autostart for MacAutostart {
    fn is_enabled(&self) -> bool {
        self.plist_path.exists()
    }

    fn set_enabled(&self, on: bool) -> Result<(), String> {
        if on {
            if let Some(parent) = self.plist_path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            let xml = launch_agent_plist(&self.label, &self.program);
            std::fs::write(&self.plist_path, xml).map_err(|e| e.to_string())
        } else {
            match std::fs::remove_file(&self.plist_path) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(e.to_string()),
            }
        }
    }
}
```

- [ ] **Step 5: Run to verify pass**

Run: `cargo test -p magpie-platform autostart`
Expected: PASS (2 tests).

- [ ] **Step 6: Commit**

```bash
git add crates/magpie-platform/src/os
git commit -m "feat(platform): macOS LaunchAgent autostart"
```

---

### Task 10: Public API surface + default secret set

**Files:**
- Modify: `crates/magpie-platform/src/lib.rs`
- Create: `crates/magpie-platform/src/defaults.rs`
- Test: `defaults.rs` tests module.

**Interfaces:**
- Produces:
  - `pub fn default_ignore_regexes() -> Vec<regex::Regex>` — a small built-in set for obvious secrets (AWS access key `AKIA[0-9A-Z]{16}`, generic `-----BEGIN [A-Z ]+PRIVATE KEY-----`, and a JWT-ish `eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}`). Each must compile.
  - `pub fn default_app_denylist() -> Vec<String>` — known macOS password-manager bundle identifiers (e.g. `"1Password"`, `"Bitwarden"`, `"KeePassXC"`). (These match `active-win-pos-rs` app names; refined in Plan 4 per OS.)
  - `lib.rs` re-exports: `pub use policy::{CapturePolicy, Decision, SkipReason}; pub use hotkey::{parse_hotkey, HotkeySpec, Mods}; pub use watcher::Watcher; pub use defaults::{default_ignore_regexes, default_app_denylist};`

- [ ] **Step 1: Write failing tests in `defaults.rs`**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ignore_regexes_compile_and_match_known_secrets() {
        let res = default_ignore_regexes();
        assert!(!res.is_empty());
        assert!(res.iter().any(|r| r.is_match("AKIAABCDEFGHIJKLMNOP")));
    }

    #[test]
    fn denylist_nonempty() {
        assert!(!default_app_denylist().is_empty());
    }
}
```

- [ ] **Step 2: Run to verify fail**

Run: `cargo test -p magpie-platform defaults`
Expected: FAIL.

- [ ] **Step 3: Implement `defaults.rs`**

```rust
use regex::Regex;

pub fn default_ignore_regexes() -> Vec<Regex> {
    [
        r"AKIA[0-9A-Z]{16}",
        r"-----BEGIN [A-Z ]+PRIVATE KEY-----",
        r"eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}",
    ]
    .iter()
    .map(|p| Regex::new(p).expect("built-in ignore regex must compile"))
    .collect()
}

pub fn default_app_denylist() -> Vec<String> {
    vec!["1Password".into(), "Bitwarden".into(), "KeePassXC".into()]
}
```

- [ ] **Step 4: Add the re-exports to `lib.rs`; run to verify pass**

Run: `cargo test -p magpie-platform`
Expected: PASS (all platform tests).

- [ ] **Step 5: Commit**

```bash
git add crates/magpie-platform/src/defaults.rs crates/magpie-platform/src/lib.rs
git commit -m "feat(platform): default secret regexes + denylist + public API"
```

---

## Self-Review

**Spec coverage:**
- Clipboard capture (text/image; rich/files via set) → Tasks 5 (read) + core `Content`. ✅
- Source-app attribution (cross-platform, Wayland-degrades-to-None) → Task 6. ✅
- Secret-marker matrix (macOS concealed types) + denylist + content-regex + pause → Tasks 2, 5, 10. ✅ (Windows/Linux markers are Plan 4.)
- Global hotkeys → Tasks 3 (parse) + 8 (map/register). ✅
- Synthetic paste → Task 7. ✅
- Autostart → Task 9 (macOS). ✅
- Change-detection + event building (pure, testable) → Task 4. ✅
- Deferred to Plan 4: Windows + Linux adapters and their secret markers. Deferred to app: image PNG encoding/thumbnailing, hotkey-event pumping in the event loop.

**Placeholder scan:** OS-glue tasks (5,6,7) use `## Manual verification` blocks by necessity (no display/permissions in CI) with concrete probe code — not TODOs. Pure logic (2,3,4,8-map,9,10) is fully TDD'd. The `png_bytes` v1 behavior is documented, not a placeholder.

**Type consistency:** `ClipboardSnapshot`, `Clipboard`, `SourceApp`, `Paster`, `Autostart`, `CapturePolicy`, `Decision`, `SkipReason`, `HotkeySpec`, `Mods`, `parse_hotkey`, `Watcher`, `to_global_hotkey`, `Hotkeys`, `MacAutostart`, `launch_agent_plist` are used consistently. Trait method signatures match across adapters. `Result<(), String>` used uniformly (no `anyhow`).

## Next Plans

- **Plan 3 `magpie-app`** consumes: `Watcher::poll_once`, `MacClipboard`, `ActiveWinSource`, `EnigoPaster`, `Hotkeys`, `MacAutostart`, `CapturePolicy`, `parse_hotkey`, `default_ignore_regexes`, `default_app_denylist`.
- **Plan 4** adds Windows + Linux adapters implementing the same traits + their secret markers.
