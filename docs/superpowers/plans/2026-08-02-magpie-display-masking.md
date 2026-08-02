# Magpie Display Masking + Screensharing Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Display-only privacy — obscure how captured items are shown (screensharing mode + per-app + per-pattern masking, with per-entry Reveal) without changing what's stored or pasted; and strengthen the never-capture *skip* defaults.

**Architecture:** A pure, tested app module `mask_view` (`mask_render`, `MaskRules`, `should_mask`). Runtime builds a `MaskRules` once per refresh from config + a session screenshare flag and computes each row's masked title + masked preview; the real `full_text` is always kept for paste. Screenshare is session-only state on `AppState`. `app_name_pairs` (already implemented in Round 2) supplies the entry→app map.

**Tech Stack:** Rust, Slint 1.17.1, `regex`.

## Global Constraints

- Toolchain pinned `rust-toolchain.toml` (1.97.1). Run cargo as `env -u RUSTUP_TOOLCHAIN ~/.cargo/bin/cargo …`.
- Gates: `cargo test` (all green), `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`.
- **Masking is display-only.** Paste / slots / merge always use the real `full_text`. Masked items remain plaintext in the DB (documented; that's the separate encryption feature).
- Skip (never-capture) defaults are separate from masking and stay the strongest default for secrets.

---

### Task 1: Strengthen skip defaults (LastPass/Dashlane + `sk-*`)

**Files:**
- Modify: `crates/magpie-platform/src/defaults.rs`

- [ ] **Step 1: Failing test** — add to the `tests` mod in `defaults.rs`:

```rust
    #[test]
    fn denylist_covers_major_password_managers() {
        let d = default_app_denylist();
        for app in ["1Password", "Bitwarden", "KeePassXC", "LastPass", "Dashlane"] {
            assert!(d.iter().any(|x| x == app), "missing {app}");
        }
    }

    #[test]
    fn ignore_regexes_match_sk_prefixed_secrets() {
        let res = default_ignore_regexes();
        assert!(res.iter().any(|r| r.is_match("sk-abc123")));
        assert!(res.iter().any(|r| r.is_match("sk_abc123")));
    }
```

- [ ] **Step 2: Run → fails.** `env -u RUSTUP_TOOLCHAIN ~/.cargo/bin/cargo test -p magpie-platform defaults`.

- [ ] **Step 3: Implement** — extend the two functions:

```rust
pub fn default_ignore_regexes() -> Vec<Regex> {
    [
        r"AKIA[0-9A-Z]{16}",
        r"-----BEGIN [A-Z ]+PRIVATE KEY-----",
        r"eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}",
        r"^sk-",
        r"^sk_",
    ]
    .iter()
    .map(|p| Regex::new(p).expect("built-in ignore regex must compile"))
    .collect()
}

pub fn default_app_denylist() -> Vec<String> {
    vec![
        "1Password".into(),
        "Bitwarden".into(),
        "KeePassXC".into(),
        "LastPass".into(),
        "Dashlane".into(),
    ]
}
```

- [ ] **Step 4: Run → PASS.** Also `cargo test -p magpie-platform` (defaults_secrets.rs integration may assert counts — update if it hard-codes the list length).

- [ ] **Step 5: Commit** `feat(platform): skip LastPass/Dashlane + sk- secrets by default`.

---

### Task 2: Config masking fields

**Files:**
- Modify: `crates/magpie-app/src/config.rs`

**Interfaces:**
- Produces: `Config.mask_apps: Vec<String>`, `Config.mask_patterns: Vec<String>`, `Config.mask_visible_chars: i64` (default 3).

- [ ] **Step 1: Failing test** — add:

```rust
    #[test]
    fn masking_defaults_are_empty_with_three_visible() {
        let c = Config::default();
        assert!(c.mask_apps.is_empty());
        assert!(c.mask_patterns.is_empty());
        assert_eq!(c.mask_visible_chars, 3);
    }

    #[test]
    fn old_config_without_masking_keys_loads_with_defaults() {
        // A config file predating masking still loads.
        let toml = "launcher_hotkey = \"super+ctrl+v\"\nquick_paste_hotkeys = []\npaste_on_select = true\napp_denylist = []\n";
        let c: Config = toml::from_str(toml).unwrap();
        assert!(c.mask_apps.is_empty());
        assert_eq!(c.mask_visible_chars, 3);
    }
```

- [ ] **Step 2: Run → fails** (fields missing).

- [ ] **Step 3: Implement** — add fields + a default fn, and set them in `Default`:

```rust
fn default_mask_visible_chars() -> i64 {
    3
}
```

In the struct (after `max_image_mb`):
```rust
    #[serde(default)]
    pub mask_apps: Vec<String>,
    #[serde(default)]
    pub mask_patterns: Vec<String>,
    #[serde(default = "default_mask_visible_chars")]
    pub mask_visible_chars: i64,
```

In `Default::default()` (after `max_image_mb: None,`):
```rust
            mask_apps: Vec::new(),
            mask_patterns: Vec::new(),
            mask_visible_chars: 3,
```

- [ ] **Step 4: Run → PASS.** `cargo test -p magpie-app --lib config`.

- [ ] **Step 5: Commit** `feat(app): config masking fields (mask_apps/patterns/visible_chars)`.

---

### Task 3: `mask_view` module (render + rules + should_mask)

**Files:**
- Modify: `crates/magpie-app/Cargo.toml` (add `regex`)
- Create: `crates/magpie-app/src/mask_view.rs`
- Modify: `crates/magpie-app/src/lib.rs` (`pub mod mask_view;`)
- Create: `crates/magpie-app/tests/mask.rs`

**Interfaces:**
- Produces: `fn mask_render(text: &str, visible: usize) -> String`; `struct MaskRules { screenshare: bool, apps: Vec<String>, patterns: Vec<regex::Regex> }` with `MaskRules::build(apps: &[String], patterns: &[String], screenshare: bool) -> MaskRules`; `fn should_mask(rules: &MaskRules, app_name: Option<&str>, full_text: &str) -> bool`.

- [ ] **Step 1: Add `regex` dep** — under `[dependencies]` in `crates/magpie-app/Cargo.toml`:

```toml
regex = "1"
```
(Confirm the workspace version pin: `grep -n "^regex" crates/magpie-core/Cargo.toml crates/magpie-platform/Cargo.toml` and match it, e.g. `regex = "1"` or a workspace dep.)

- [ ] **Step 2: Write the module with failing-first tests** — create `mask_view.rs`:

```rust
//! Display-only masking for screensharing / sensitive sources. Never changes
//! stored or pasted text — only how an entry is rendered.

use regex::Regex;

/// First `visible` chars of `text` then capped bullets. Text no longer than
/// `visible` is fully masked (min one bullet). Unicode-safe (operates on chars).
pub fn mask_render(text: &str, visible: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    if n <= visible {
        return "•".repeat(n.max(1));
    }
    let head: String = chars[..visible].iter().collect();
    let bullets = "•".repeat((n - visible).min(12));
    format!("{head}{bullets}")
}

/// Compiled masking rules for one refresh. Invalid regexes are dropped.
pub struct MaskRules {
    pub screenshare: bool,
    pub apps: Vec<String>,
    pub patterns: Vec<Regex>,
}

impl MaskRules {
    pub fn build(apps: &[String], patterns: &[String], screenshare: bool) -> MaskRules {
        let patterns = patterns.iter().filter_map(|p| Regex::new(p).ok()).collect();
        MaskRules {
            screenshare,
            apps: apps.to_vec(),
            patterns,
        }
    }
}

/// An entry is masked if screenshare is on, its source app is in `apps`, or its
/// text matches any pattern.
pub fn should_mask(rules: &MaskRules, app_name: Option<&str>, full_text: &str) -> bool {
    if rules.screenshare {
        return true;
    }
    if let Some(a) = app_name {
        if rules.apps.iter().any(|x| x == a) {
            return true;
        }
    }
    rules.patterns.iter().any(|re| re.is_match(full_text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_prefix_then_bullets() {
        assert_eq!(mask_render("sk-live-abcdef", 3), format!("sk-{}", "•".repeat(11)));
    }

    #[test]
    fn render_short_text_fully_masked() {
        assert_eq!(mask_render("hi", 3), "••");
        assert_eq!(mask_render("", 3), "•");
    }

    #[test]
    fn render_visible_zero_all_bullets() {
        assert_eq!(mask_render("hello", 0), "•••••");
    }

    #[test]
    fn render_caps_bullets_at_12() {
        let out = mask_render(&"x".repeat(100), 2);
        assert_eq!(out.chars().filter(|&c| c == '•').count(), 12);
    }

    #[test]
    fn render_unicode_by_char() {
        assert_eq!(mask_render("café…", 3), "caf••");
    }

    #[test]
    fn should_mask_screenshare_always() {
        let r = MaskRules::build(&[], &[], true);
        assert!(should_mask(&r, None, "anything"));
    }

    #[test]
    fn should_mask_by_app() {
        let r = MaskRules::build(&["Slack".into()], &[], false);
        assert!(should_mask(&r, Some("Slack"), "x"));
        assert!(!should_mask(&r, Some("Terminal"), "x"));
    }

    #[test]
    fn should_mask_by_pattern_and_skips_invalid() {
        let r = MaskRules::build(&[], &["secret".into(), "(".into()], false);
        assert!(should_mask(&r, None, "a secret value"));
        assert!(!should_mask(&r, None, "nothing here"));
    }

    #[test]
    fn should_not_mask_when_no_rule_hits() {
        let r = MaskRules::build(&[], &[], false);
        assert!(!should_mask(&r, Some("Terminal"), "plain"));
    }
}
```

- [ ] **Step 3: Add `pub mod mask_view;`** to `lib.rs` (alphabetical, after `image_cache`/before `merge_view`).

- [ ] **Step 4: Add an integration test** `crates/magpie-app/tests/mask.rs` exercising the public surface:

```rust
use magpie_app::mask_view::{mask_render, should_mask, MaskRules};

#[test]
fn end_to_end_masking_shape() {
    let rules = MaskRules::build(&["1Password".into()], &[r"^ghp_".into()], false);
    assert!(should_mask(&rules, Some("1Password"), "hunter2"));
    assert!(should_mask(&rules, None, "ghp_tokenvalue"));
    assert!(!should_mask(&rules, Some("Notes"), "grocery list"));
    assert_eq!(mask_render("ghp_secret", 3), format!("ghp{}", "•".repeat(7)));
}
```

- [ ] **Step 5: Run → PASS.** `cargo test -p magpie-app mask`.

- [ ] **Step 6: Commit** `feat(app): mask_view (render + rules + should_mask)`.

---

### Task 4: Screenshare + mask config on AppState

**Files:**
- Modify: `crates/magpie-app/src/app_state.rs`
- Modify: `crates/magpie-app/src/runtime.rs` (`build_state` takes `&Config`)

**Interfaces:**
- Produces: `AppState { …, screenshare: Mutex<bool>, mask_apps: Vec<String>, mask_patterns: Vec<String>, mask_visible_chars: i64 }`; `build_state(cfg: &Config) -> Arc<AppState>`.

- [ ] **Step 1: Extend `AppState`** in `app_state.rs`:

```rust
    /// Session-only screensharing mode (masks everything). Not persisted.
    pub screenshare: Mutex<bool>,
    /// Masking config snapshot (from `Config`, immutable for the session).
    pub mask_apps: Vec<String>,
    pub mask_patterns: Vec<String>,
    pub mask_visible_chars: i64,
```

- [ ] **Step 2: Fix every `AppState { … }` construction.** Find them: `grep -rn "AppState {" crates/magpie-app`. In each (the `app_state.rs` test helper `state()`, any `tests/*.rs`, and `runtime::build_state`), add:
```rust
        screenshare: std::sync::Mutex::new(false),
        mask_apps: Vec::new(),
        mask_patterns: Vec::new(),
        mask_visible_chars: 3,
```

- [ ] **Step 3: `build_state(cfg: &Config)`** — change the signature and populate the mask fields from `cfg`:

```rust
pub fn build_state(cfg: &Config) -> Arc<AppState> {
    let dir = data_dir();
    std::fs::create_dir_all(&dir).ok();
    let store = open(&dir.join("magpie.sqlite3")).expect("open db");
    Arc::new(AppState {
        store: std::sync::Mutex::new(store),
        images: magpie_app::image_cache::FsImageStore {
            dir: dir.join("images"),
        },
        ui: std::sync::Mutex::new(magpie_app::viewmodel::UiState::new()),
        merge_set: std::sync::Mutex::new(Vec::new()),
        screenshare: std::sync::Mutex::new(false),
        mask_apps: cfg.mask_apps.clone(),
        mask_patterns: cfg.mask_patterns.clone(),
        mask_visible_chars: cfg.mask_visible_chars.max(0),
    })
}
```

- [ ] **Step 4: Update the call site** in `start`: `let state = build_state(&cfg);` (cfg is already loaded above it).

- [ ] **Step 5: Build + test** — `cargo build -p magpie-app`, `cargo test -p magpie-app --lib app_state`. Fix any other constructor the grep found.

- [ ] **Step 6: Commit** `feat(app): screenshare + mask config on AppState`.

---

### Task 5: Runtime + UI integration (masked rows/preview, screenshare toggle, reveal)

**Files:**
- Modify: `crates/magpie-app/ui/launcher.slint`
- Modify: `crates/magpie-app/src/runtime.rs`

**Interfaces:**
- Consumes: `mask_view::{MaskRules, should_mask, mask_render}`, `AppState` mask fields.
- Produces: `EntryRow { …, masked: bool, full_masked: string }`; callback `toggle-screenshare()`; property `screenshare: bool`, `revealed: bool`; `to_rows(.., rules, visible)`.

- [ ] **Step 1: EntryRow gains masking fields** — in `launcher.slint`:

```slint
    masked: bool,
    full_masked: string,
```
(append inside the `EntryRow` struct).

- [ ] **Step 2: Mask in `to_rows`** — add params and compute. New signature:

```rust
fn to_rows(
    entries: &[Entry],
    slots: &HashMap<i64, i64>,
    tags: &HashMap<i64, Vec<String>>,
    merge_set: &[i32],
    app_names: &HashMap<i64, String>,
    now: i64,
    rules: &MaskRules,
    visible: usize,
) -> Vec<EntryRow> {
```
Inside the map closure, after computing `source` and before building `subtitle`:
```rust
            let masked = should_mask(rules, app_names.get(&e.id).map(|s| s.as_str()), &e.full_text);
            let base_title = preview_title(e);
            let title = if masked {
                mask_render(&base_title, visible)
            } else {
                base_title
            };
            let full_masked = if masked {
                mask_render(&e.full_text, visible)
            } else {
                String::new()
            };
```
Change the `EntryRow` literal: `title: SharedString::from(title),` (was `preview_title(e)`), and add:
```rust
                masked,
                full_masked: SharedString::from(full_masked),
```
Add imports: `use magpie_app::mask_view::{mask_render, should_mask, MaskRules};`.

- [ ] **Step 3: Build `MaskRules` in `refresh` and pass to `to_rows`** — near the top of `refresh` (after `results`):

```rust
    let screenshare = state.screenshare.lock().map(|g| *g).unwrap_or(false);
    let rules = MaskRules::build(&state.mask_apps, &state.mask_patterns, screenshare);
    let visible = state.mask_visible_chars.max(0) as usize;
```
Then update the `to_rows(...)` call: `&results, &slots, &tag_map, &merge_set, &app_names, now_ms(), &rules, visible`.
Also push the screenshare flag to the UI: `ui.set_screenshare(screenshare);`.

- [ ] **Step 4: Slint — screenshare toggle + indicator** — add property + callback to `LauncherWindow`:
```slint
    in property <bool> screenshare: false;
    in-out property <bool> revealed: false;
    callback toggle-screenshare();
```
In the type-filter row (next to the `Stats` button), add a screenshare toggle:
```slint
                    Rectangle {
                        width: 40px;
                        height: 26px;
                        background: root.screenshare ? #b04a4a : #202024;
                        border-radius: 6px;
                        TouchArea { clicked => { root.toggle-screenshare(); } }
                        Text { text: "🔒"; font-size: 13px; horizontal-alignment: center; vertical-alignment: center; }
                    }
```

- [ ] **Step 5: Slint — masked preview + Reveal** — replace the preview `Text`'s `text:` binding:
```slint
                                text: (entries.length > 0 && root.selected < entries.length)
                                    ? (entries[root.selected].masked && !root.revealed
                                        ? entries[root.selected].full-masked
                                        : entries[root.selected].full)
                                    : "";
```
Immediately after the `ScrollView`, add a Reveal button shown only when the selected entry is masked and not revealed:
```slint
                            if entries.length > 0 && root.selected < entries.length
                                && entries[root.selected].masked && !root.revealed: Rectangle {
                                height: 26px;
                                width: 90px;
                                background: #2a2a30;
                                border-radius: 6px;
                                TouchArea { clicked => { root.revealed = true; } }
                                Text { text: "👁 Reveal"; color: white; font-size: 12px; horizontal-alignment: center; vertical-alignment: center; }
                            }
```

- [ ] **Step 6: Slint — reset reveal on selection change** — in the existing `changed sel-follow` handler (inside the two-pane HorizontalLayout), add:
```slint
                        root.revealed = false;
```

- [ ] **Step 7: Runtime — screenshare toggle callback** — in `start`:
```rust
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_toggle_screenshare(move || {
            if let Ok(mut g) = s.screenshare.lock() {
                *g = !*g;
            }
            if let Some(ui) = w.upgrade() {
                refresh(&ui, &s);
            }
        });
    }
```

- [ ] **Step 8: Build + clippy + fmt + `timeout 8` launch.** Manual: toggle 🔒 → previews + row titles obscure; select masked entry → 👁 Reveal shows the text; changing selection re-masks; paste still yields real text.

- [ ] **Step 9: Commit** `feat(app): display masking + screenshare toggle + reveal`.

---

### Task 6: Full gate + run-verify + memory

- [ ] **Step 1: Full gates**
```bash
env -u RUSTUP_TOOLCHAIN ~/.cargo/bin/cargo test
env -u RUSTUP_TOOLCHAIN ~/.cargo/bin/cargo clippy --all-targets -- -D warnings
env -u RUSTUP_TOOLCHAIN ~/.cargo/bin/cargo fmt --check
timeout 8 env -u RUSTUP_TOOLCHAIN ~/.cargo/bin/cargo run -p magpie-app   # exit 124 = alive
```

- [ ] **Step 2: Update memory** `magpie-clipboard-manager.md`; commit docs.

## Self-Review

- **Spec coverage:** strengthened skip defaults → T1; config fields → T2; `mask_render`/`MaskRules`/`should_mask` → T3; screenshare session state → T4; masked title+preview, screenshare toggle, Reveal, real-text-paste → T5. `app_name_pairs` already implemented (Round 2). All spec sections mapped.
- **Placeholder scan:** none — testable code is concrete; UI steps give full Slint. The `regex` version and extra `AppState { }` constructors are explicit "grep then match" checks.
- **Type consistency:** `to_rows(.., rules: &MaskRules, visible: usize)` matches its `refresh` call (T5S2/T5S3). `MaskRules::build(&[String], &[String], bool)` used consistently in T3 tests, T5 refresh. `EntryRow` gains `masked: bool` + `full_masked: string` in the Slint struct (T5S1) and `to_rows` (T5S2), consumed by the preview + Reveal bindings (T5S5). `build_state(&Config)` signature matches its `start` call (T4S4).
- **Risk flags:** (a) other `AppState { }` constructors in `tests/` must all get the 4 new fields (grep in T4S2). (b) `defaults_secrets.rs` may hard-code the denylist/regex counts — update if so (T1S4). (c) reveal reset lives in the `changed sel-follow` handler, which only fires on selection change — correct per spec.
