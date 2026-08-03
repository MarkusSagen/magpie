# Magpie — Settings & Theming Design

**Date:** 2026-08-03
**Status:** Approved design → implementation plan next

## Overview

Add an in-app **settings panel** (opened with ⌘,) that persists to `config.toml`,
covering two areas:

1. **Appearance** — a themeable UI driven by a single Slint `Theme` global, with a
   set of preconfigured presets plus an optional accent-color override. Theme
   changes apply **live**, without restart.
2. **Editor** — a picker for which editor "Open in editor" uses, defaulting to the
   user's `$VISUAL`/`$EDITOR` but selectable from **detected installed editors**;
   plus, for terminal editors, a **terminal** picker (defaulting to auto-detect).

All new config keys are additive (`#[serde(default)]`) so existing config files and
first-run behavior are unchanged.

## Non-goals / out of scope (tracked as separate specs)

- **Tag row layout** (single row + horizontal scroll on overflow) — appended to the
  existing tags design (`2026-08-02-magpie-tags-design.md`).
- **"Keyboard shortcuts everywhere"** (jump to image/link/tag #N, move between
  panes) — its own future spec.
- Per-terminal *new-tab* injection (vs. new window) — remains out of scope; the
  terminal picker only chooses *which* terminal, still opening a new window.

## 1. Config schema (`crates/magpie-app/src/config.rs`)

Add four additive fields to `Config`:

```rust
/// UI theme preset id. "system" (default) follows the OS light/dark setting.
#[serde(default = "default_theme_preset")]
pub theme_preset: String,
/// Optional accent color as "#rrggbb". Empty = use the preset's own accent.
#[serde(default)]
pub accent: String,
/// Editor command for "Open in editor". None = auto ($VISUAL → $EDITOR).
/// Some("code -n -w") = explicit command (full string, args preserved).
#[serde(default)]
pub editor: Option<String>,
/// Terminal for terminal editors. None = auto-detect ($TERMINAL → $TERM_PROGRAM
/// → first found). Some("kitty") = explicit.
#[serde(default)]
pub terminal: Option<String>,
```

`default_theme_preset()` returns `"system"`. Defaults preserve today's behavior
exactly: `theme_preset="system"`, `accent=""`, `editor=None`, `terminal=None`.

**Valid preset ids** (unknown id falls back to `system`):
`system`, `dark`, `light`, `gruvbox`, `tokyo-night`, `tokyo-night-storm`,
`catppuccin-latte`, `catppuccin-frappe`, `catppuccin-macchiato`,
`catppuccin-mocha`, `ayu`, `dracula`, `atom-one-dark`.

## 2. Editor & terminal discovery (`crates/magpie-app/src/external_editor.rs`)

### Types
```rust
pub struct EditorChoice {
    pub label: String,    // human label, e.g. "VS Code", "Neovim"
    pub command: String,  // command to run, e.g. "code", "nvim"
    pub is_terminal: bool, // reuses is_terminal_editor()
}
```

### `detect_editors() -> Vec<EditorChoice>`
Probe a curated known-editor table against the system; return only **installed**
ones (deduped, stable order). Detection:
- **unix:** `command -v <bin>` (via `which`-style PATH scan, no shell needed) for
  CLI bins; plus standard GUI locations on macOS (`/Applications/<App>.app`,
  `~/Applications/<App>.app`) and Linux (`/usr/bin`, `/usr/local/bin`, PATH).
- **windows:** check PATH + known install dirs for `<bin>.exe`.

Curated table (label → command → is_terminal), all optional depending on presence:
Neovim `nvim`(T), Vim `vim`(T), Nano `nano`(T), Helix `hx`(T), Micro `micro`(T),
Emacs (terminal) `emacsclient -t`(T), Neovide `neovide`(G), VS Code `code`(G),
Cursor `cursor`(G), Windsurf `windsurf`(G), Zed `zed`(G), Sublime `subl`(G),
Emacs (GUI) `emacs`(G), JetBrains IDEA `idea`(G), PyCharm `pycharm`(G),
WebStorm `webstorm`(G), GoLand `goland`(G), CLion `clion`(G), RustRover
`rustrover`(G) — extend as cheaply detectable. `is_terminal` comes from the
existing `is_terminal_editor()` classifier so the two never drift.

### `detect_terminals() -> Vec<String>`
Return terminals found on PATH among `ghostty`, `kitty`, `wezterm`, `alacritty`,
`xterm`, plus any from `$TERMINAL`/`$TERM_PROGRAM` (deduped, preferred first).

### `launch` refactor
`open_text(text, key, editor_override: Option<&str>, terminal_override: Option<&str>)`.
- `editor_override` `Some` wins over env; `None` keeps the current
  `env_editor()` fallback. Same for `terminal_override` inside the
  terminal-candidate list (prepended when set).
- All existing pure helpers (`is_terminal_editor`, `terminal_argv`,
  `command_bin`, …) and their tests stay unchanged.

### Tests
- `detect_editors` against a controlled fake PATH (temp dir with stub executables)
  returns exactly the stubs, correctly classified terminal vs GUI.
- `detect_terminals` similarly.
- `launch` with an explicit `editor_override` uses it over env (assert via the
  argv-building helpers; no real spawn in unit tests). Existing ignored
  `e2e_open_text_spawns` updated to pass `None, None`.

## 3. Theme system

### Slint `Theme` global (`crates/magpie-app/ui/`)
```
export global Theme {
    in property <color> bg;          // window background
    in property <color> surface;     // rows, panels, metadata block
    in property <color> surface-alt; // hover / secondary fills
    in property <color> selection;   // selected row fill
    in property <color> text;        // primary text
    in property <color> text-dim;    // metadata, placeholders, divider labels
    in property <color> accent;      // active state, buttons, highlights
    in property <color> divider;     // rule lines
    in property <color> border;      // subtle borders
}
```
Every currently-hardcoded color in `launcher.slint` (~50 sites) is replaced by a
`Theme.<role>` reference. No literal colors remain in the UI except where a role
genuinely doesn't apply (documented per-site).

### Palette source of truth (Rust `theme::palette(preset, accent_override)`)
Six roles are transcribed per preset from the upstream palette; the remaining
three are **derived by rule**: `divider = border = surface`,
`surface-alt = selection`. `accent_override` (when non-empty, valid `#rrggbb`)
replaces `accent`. `system` resolves to `dark` or `light` per OS setting.

| preset | bg | surface | text | text-dim | accent | selection |
|---|---|---|---|---|---|---|
| dark | #1c1c1e | #2c2c2e | #f2f2f7 | #8e8e93 | #4c8bf5 | #3a3a3c |
| light | #ffffff | #f2f2f7 | #1c1c1e | #8e8e93 | #4c8bf5 | #e5e5ea |
| gruvbox | #282828 | #3c3836 | #ebdbb2 | #928374 | #fe8019 | #504945 |
| tokyo-night | #1a1b26 | #24283b | #c0caf5 | #565f89 | #7aa2f7 | #283457 |
| tokyo-night-storm | #24283b | #1f2335 | #c0caf5 | #565f89 | #7aa2f7 | #2e3c64 |
| catppuccin-latte | #eff1f5 | #ccd0da | #4c4f69 | #6c6f85 | #1e66f5 | #bcc0cc |
| catppuccin-frappe | #303446 | #414559 | #c6d0f5 | #a5adce | #8caaee | #51576d |
| catppuccin-macchiato | #24273a | #363a4f | #cad3f5 | #a5adcb | #8aadf4 | #494d64 |
| catppuccin-mocha | #1e1e2e | #313244 | #cdd6f4 | #a6adc8 | #89b4fa | #45475a |
| ayu | #0d1017 | #131721 | #bfbdb6 | #565b66 | #e6b450 | #1c2532 |
| dracula | #282a36 | #343746 | #f8f8f2 | #6272a4 | #bd93f9 | #44475a |
| atom-one-dark | #282c34 | #21252b | #abb2bf | #5c6370 | #61afef | #3e4451 |

`dark`/`light` are Magpie's own palette; if the current UI colors differ, Task 3
transcribes the *current* look into `dark` so the default appearance is unchanged.

### Live application
`ui.global::<Theme>().set_*` is called (a) on startup from loaded config, and (b)
whenever a settings change occurs. Pure `theme::palette()` is unit-tested (correct
role hex per preset; accent override; unknown preset → dark; system → dark|light
given an injected OS-mode arg).

## 4. Settings panel UI (`mode == "settings"`, ⌘,)

A modal overlay in the same style as the existing Help (`mode=="help"`) and ⌘K
actions overlays. Opened by a `Ctrl`-modified `,` key (Slint's control = ⌘ on mac /
Ctrl elsewhere), closed by Esc. Two sections:

**Appearance**
- Preset selector (list/dropdown of the 13 presets; live-applies on select).
- Accent: a row of swatches + a `#rrggbb` custom field; empty = preset default.

**Shortcuts**
- **Summon hotkey capture** — shows the current binding (default
  `Cmd+Shift+Space`). A **"Record shortcut"** button puts the field into
  capture mode: the next key-chord the user presses is captured (via the Slint
  `FocusScope` `key-pressed` modifiers + key) and rendered as a spec string
  (e.g. `super+shift+space`). Esc cancels capture; the field never accepts a
  bare key with no modifier.
- **Validate before saving.** On capture, run `validate_hotkey(spec)` which:
  1. **Parses** it (`parse_hotkey`) — reject if malformed / no modifier / an
     unsupported key (surface the `to_global_hotkey` error, e.g. an
     unmapped key).
  2. **Checks availability** by attempting a real `Hotkeys::register` of the
     candidate in a throwaway manager: `Ok` → the combo is free and works
     (immediately unregister); `Err` → it's already taken by the OS or another
     app — show "This shortcut is unavailable (in use by another app)". This is
     the only reliable cross-platform "is it free?" test — the OS refuses a
     duplicate registration.
  3. Only on success: persist `launcher_hotkey`, **re-register live**
     (unregister the old id, register the new) so it takes effect without
     restart, and show a confirmation.
- Quick-paste hotkeys stay in `config.toml` for now (editable there);
  exposing them in-panel is a documented later addition.

**Editor**
- Radio list of detected editors. First row is always
  **"System default ($VISUAL → <resolved>)"** = `editor:None`.
- A **"Custom…"** row with a text field (full command; e.g. `code -n -w`).
- **Terminal** sub-picker, shown only when the selected editor is a terminal
  editor: "Auto ($TERMINAL → <resolved>)" (=`terminal:None`) / detected terminals
  / "Custom…" field.

Each control edits the in-memory `Config` and **persists immediately** to
`config.toml` via `config::save` (no separate Save button). Theme changes also push
to the `Theme` global immediately.

## 5. Data flow

- **Startup:** `load_or_default` → `theme::palette(cfg.theme_preset, cfg.accent)` →
  set `Theme` global; store `cfg.editor`/`cfg.terminal` in runtime state.
- **Settings change:** callback (e.g. `on_set_theme(preset)`,
  `on_set_accent(hex)`, `on_set_editor(cmd_or_empty)`, `on_set_terminal(...)`)
  updates `Config`, applies live (Theme), and calls `config::save`.
- **Open in editor:** `runtime`'s `on_open_in_editor` passes
  `cfg.editor.as_deref()`, `cfg.terminal.as_deref()` into `open_text`.

## 6. Security / privacy (unchanged)

No change to the never-capture skip list, display-only masking, favicon
same-origin rule, or full-fidelity paste. The editor writes the same temp file as
today; only *which* program opens it becomes configurable.

## 7. Testing summary

- `config.rs`: roundtrip of the four new fields; old config (without keys) loads
  with correct defaults; unknown `theme_preset` tolerated.
- `theme.rs`: `palette()` role values per preset; accent override; system
  resolution; unknown → dark.
- `external_editor.rs`: `detect_editors`/`detect_terminals` against a fake PATH;
  `open_text` honors overrides; existing pure tests unchanged.
- `hotkey`/`validate_hotkey`: rejects malformed/no-modifier/unsupported-key; a
  free combo validates `Ok`; a combo already registered by the same process
  probe reports unavailable. (The cross-app "in use elsewhere" branch is a
  manual/host check — CI has no competing registrant.)
- Gates: `cargo test`, `clippy --all-targets -D warnings`, `fmt --check`, timeout
  launch (exit 124 = alive). Windows/Linux discovery is cfg-gated / compile-only.

## 8. Implementation tasks (one plan)

1. **Config fields** — add the four keys + defaults + roundtrip/back-compat tests.
2. **Editor/terminal discovery + `launch` refactor** — `EditorChoice`,
   `detect_editors`, `detect_terminals`, `open_text` override params + tests; wire
   `on_open_in_editor` to pass config.
3. **Theme system** — `theme.rs` palette table + `palette()` + tests; `Theme`
   global; migrate ~50 hardcoded colors; set global on startup.
4. **Settings panel** — `mode=="settings"` modal, ⌘, open/Esc close, Appearance +
   Editor sections, callbacks that edit `Config`, apply live, and persist.
5. **Shortcut capture** — `magpie-platform::validate_hotkey(spec)` (parse +
   register-probe availability, immediately unregistering the probe); a
   "Record shortcut" control in the panel's **Shortcuts** section that captures a
   chord, validates it, and on success persists `launcher_hotkey` + a runtime hook
   that live re-registers (unregister old id → register new) so it works without
   restart. `launcher_hotkey` already exists in `Config` (no new field).
