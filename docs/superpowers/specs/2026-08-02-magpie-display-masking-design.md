# Magpie Display Masking + Screensharing Mode — Design

**Date:** 2026-08-02 · **Status:** Approved · **Phase:** 1

## Summary

Display-only privacy: obscure how captured items are *shown* without changing what
is *stored* or *pasted*. Three triggers, OR'd: a session **screensharing mode**
(masks everything), a **mask-apps** list, and **mask-patterns** regexes. Masked
text shows the first N chars then bullets; a per-entry **Reveal** unmasks
temporarily. This is separate from the existing *skip* path (never-capture), whose
defaults are strengthened to cover LastPass + `sk-*`.

## Two independent mechanisms

1. **Skip (existing, strengthened):** concealed markers + `app_denylist` +
   `ignore_regexes` → item is never captured. Defaults gain **LastPass** and
   **Dashlane** in the denylist and **`^sk-` / `^sk_`** in the ignore-regexes.
   Password managers + `sk-*` therefore never reach the DB. Unchanged mechanism.
2. **Mask (new, display-only, opt-in):** the item IS captured; only its UI
   rendering is obscured. `full_text` is untouched, so paste/slots/merge use the
   real value.

## Masking triggers (an entry is masked iff any hold)

- **Screensharing mode** is ON → mask everything.
- The entry's **source app name** ∈ `mask_apps` (config; empty by default).
- The entry's **`full_text`** matches any compiled `mask_patterns` regex (config;
  empty by default; invalid regexes are ignored).

Masking never affects `skip`: skipped items were never captured, so they can't be
masked. Masking applies only to captured items.

## Render

`mask_render(text, visible) -> String`:
- If `text` has ≤ `visible` chars → all bullets (`•` × char count, min 1) — short
  secrets are fully hidden.
- Else → first `visible` chars + `•` × `min(remaining, 12)`.
- Unicode-safe (operates on `chars()`).

`visible` = `Config.mask_visible_chars` (default 3, min 0).

## Reveal

The detail pane, for a masked selected entry, shows the masked detail + a
**Reveal** button. Clicking sets an ephemeral `revealed-id` (window state) to that
entry's id; while it matches the selected entry, the detail shows the real text.
Changing selection or re-masking clears it. Reveal is UI-only and per-entry (row
titles stay masked). Reveal always works (it's the user's machine).

## Screensharing mode

- Session-scoped boolean in `AppState` (`Mutex<bool>`), **not persisted** (resets
  on restart).
- Toggled by a launcher header button (like the Stats toggle): **🔒 Screenshare:
  on/off**, with a clear active indicator. Optional future: a tray item.
- While ON, `should_mask` returns true for every entry.

## Config

`Config` gains (all `#[serde(default)]`, so older configs still load):
- `mask_apps: Vec<String>` — source-app names to mask (empty default).
- `mask_patterns: Vec<String>` — regexes to mask matching content (empty default).
- `mask_visible_chars: i64` — leading chars shown when masking (default 3).

Defaults strengthened (in `magpie-platform` defaults, used by the watcher): the
skip denylist adds `LastPass`, `Dashlane`; the ignore-regexes add `^sk-`, `^sk_`.

## Architecture

Masking is a display concern, kept out of core storage.

- **Pure app module `mask_view`** (unit-tested):
  - `mask_render(text: &str, visible: usize) -> String`.
  - `struct MaskRules { screenshare: bool, apps: Vec<String>, patterns: Vec<regex::Regex> }`
    with `MaskRules::from(cfg, screenshare)` (compiles patterns, skips invalid).
  - `should_mask(rules: &MaskRules, app_name: Option<&str>, full_text: &str) -> bool`.
- **Core helper** `Store::app_name_pairs() -> Result<Vec<(i64, String)>>` — an
  `entry_id → source app display_name` map (join `entries`→`apps`), like
  `tag_pairs`, so the runtime can evaluate the app-mask rule per entry.
- **Runtime**: builds a `MaskRules` from `Config` + the session screenshare flag,
  and an `entry_id → app_name` map; for each row computes a possibly-masked title;
  computes the (possibly-masked, or revealed) detail. Adds the screenshare toggle
  callback + a `masked`/`screenshare` display state. **Regexes are compiled once
  per refresh**, not per entry.

`regex` is already a workspace dependency (used by core/platform); the app gains
it as a direct dependency.

## Data Flow

```
capture: (skip path unchanged) -> stored full_text
display refresh:
  MaskRules = from(config, screenshare_flag)
  app_names = store.app_name_pairs()
  per row: title = should_mask(rules, app_name(id), full_text) ? mask_render(title) : title
  detail = (masked && revealed-id != sel.id) ? mask_render(full) : full
paste: always uses real full_text (never masked)
```

## Error Handling

- Invalid `mask_patterns` regex → skipped at compile (logged), never crashes.
- `mask_visible_chars` clamped to `>= 0`.
- Store/lock errors in the app-name lookup → treat as "no app name" (app rule
  can't match; pattern + screenshare still apply).

## Testing

**core:** `app_name_pairs` returns `(entry_id, app_name)` for entries with a source
app (NULL-app entries excluded).

**app (`mask_view` unit + `tests/mask.rs`):**
- `mask_render`: `visible=3` on `"sk-live-abcdef"` → `"sk-" + bullets`; short text
  (`"hi"`, visible 3) → all bullets; `visible=0` → all bullets; unicode (`"café…"`)
  masks by char.
- `should_mask`: screenshare true → always masks; app in `apps` → masks; text
  matching a pattern → masks; none → false; invalid pattern ignored.
- expanded skip defaults: `default_app_denylist` contains `LastPass`;
  `default_ignore_regexes` matches `sk-abc123` and `sk_abc123`.

**run-verify:** launch; toggle 🔒 Screenshare → all previews obscure; select a
masked entry, click Reveal → its detail shows; paste a masked entry → real text
pastes.

## File Structure

- Modify: `crates/magpie-platform/src/defaults.rs` (LastPass/Dashlane + `sk-` regexes) + tests.
- Create: `crates/magpie-core/src/mask_support.rs` — `impl Store { app_name_pairs }`, declared `pub mod mask_support` in `lib.rs`. (New focused module, keeps `tags.rs` untouched.)
- Create: `crates/magpie-app/src/mask_view.rs` (`mask_render`, `MaskRules`, `should_mask`).
- Modify: `crates/magpie-app/Cargo.toml` (add `regex`), `src/lib.rs`, `src/config.rs`, `src/app_state.rs` (`screenshare: Mutex<bool>`), `ui/launcher.slint` (screenshare toggle + Reveal + masked rendering via runtime), `src/runtime.rs`.
- Create: `crates/magpie-app/tests/mask.rs`.

## Risks / Notes

- **Masking ≠ encryption:** masked items are still plaintext in the DB. This is
  display privacy for screensharing, not at-rest protection (that's the separate
  encryption feature). Documented so users don't over-trust it for secrets — hence
  password managers/`sk-*` stay on the *skip* path by default.
- Row titles are masked but the list still shows counts/kinds/tags — those are not
  masked in v1 (only the text preview + detail). Acceptable; noted.
