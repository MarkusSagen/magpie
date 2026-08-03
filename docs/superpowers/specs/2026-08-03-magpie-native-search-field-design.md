# Magpie — Native search field (real cursor + full text editing)

**Date:** 2026-08-03 · **Status:** Approved · **Phase:** 1

## Goal

Replace the custom command-bar search Text with a real Slint **`TextInput`** so the
search gets a native cursor and all standard editing shortcuts — while keeping our
keyboard model (↑/↓ list nav, ⏎ paste, Esc, ⌘K/⌘F/⌘C/⌘E/⌘N/⌘M/⌘P/⌘//⌘1-9,
type-to-filter). This gives the user's requested word/line cursor movement for free,
natively and cross-platform.

## Mechanism

`TextInput` has a `key-pressed(event) -> EventResult` **pre-handler**: return
`accept` to handle a key ourselves (TextInput ignores it), `reject` to let
TextInput edit natively. So the focused search `TextInput` owns editing, and its
`key-pressed` intercepts only our nav/shortcut keys:

- **accept (we handle):** `Up`/`Down` (list nav), `Return` (⌘Return→keep, else
  activate), `Escape` (clear text if non-empty, else hide), and with `control`
  (=⌘ on macOS / Ctrl elsewhere): `k` `f` `e` `n` `m` `p` `c` `/` and digits `1..9`.
- **reject (native editing):** everything else — printable chars, `Backspace`,
  `⌥Backspace` (delete word), `⌘Backspace` (delete to line start), `⌥←/→`
  (word), `⌘←/→` (line start/end), `⌘A` (select all), `⌘V`/`⌘X` (paste/cut),
  Left/Right, Home/End. All handled by `TextInput` natively.

`⌘V`-into-search is now native (removes our `paste-into-search`). `⌘C` stays "copy
the selected entry" (we accept it) — a search-text copy is not needed.

## Text ownership

- `query` becomes `in-out`; the TextInput binds `text <=> root.query`.
- `edited => root.search-changed(self.text)` drives the live filter (Rust
  `on_search_changed` sets `UiState.text` + `refresh`, unchanged).
- Esc-clear sets `searchinput.text = ""` then calls `search-changed("")`
  (programmatic text change doesn't fire `edited`).
- **Removed** callbacks + Rust handlers + helpers: `key-char`, `key-backspace`,
  `key-delete-word`, `clear-query`, `paste-into-search`, `apply_query`,
  `delete_last_word` (+ its test). The TextInput owns the text now.

## Focus + structure

- The Window `forward-focus: searchinput`; the search input is focused on show.
  Overlays (⌘K actions, help, ⌘F search, stats) are non-focus-grabbing overlays, so
  the TextInput keeps focus and routes keys while a modal is open (its `key-pressed`
  `accept`s all keys in `mode == "actions"/"help"/"search"`).
- **`searchinput` must not live inside a conditional** (Slint forbids referencing a
  conditional id from `forward-focus`/root handlers). So: remove the `if view ==
  "list"` wrapper — the search row, filters, slots strip, list, detail pane, and
  action bar are **always present**; **Stats becomes a full-window overlay**
  (`if root.view == "stats": Rectangle { opaque bg; <existing stats content> }`),
  consistent with the other modals.
- The root `FocusScope fs` is removed; `searchinput` is the keyboard owner.
- **Edit mode:** `start-edit`/`new-snippet` focus the inline editor
  (`editor.focus()` on init, as today); on save/cancel `mode = "list"` and a
  `changed mode` handler re-focuses `searchinput`.
- **`cmd-held`** (1-9 badge highlight): set in `searchinput.key-pressed` /
  `key-released` from `event.modifiers.control`.

## Styling

The search box Rectangle keeps its 🔍 + border. Inside: a placeholder `Text`
("Search clipboard…") shown when `query == ""`, and the `TextInput`
(`single-line: true`, white text, our font-size, transparent bg). The native caret
+ selection use the theme's colors (`selection-background-color`).

## Files
- `crates/magpie-app/ui/launcher.slint` — TextInput search; move key handling to
  `searchinput.key-pressed`/`key-released`; remove `fs`; `forward-focus`;
  de-conditionalize the list body; Stats → overlay; `changed mode` refocus.
- `crates/magpie-app/src/runtime.rs` — `on_search_changed` stays; remove the
  removed callbacks' handlers + `apply_query` + `delete_last_word` (+ test);
  `on_close_requested` unchanged.

## Testing
- Core/pure: unaffected (no logic change); the removed `delete_last_word` test goes
  with the helper (native ⌥⌫ replaces it).
- Manual (macOS): type/filter; cursor visible; ⌥←/→ word, ⌘←/→ line, ⌥⌫ word,
  ⌘⌫ clear-to-start, ⌘A select-all, ⌘V paste-into-search all work natively; ↑/↓
  still navigate the list; ⏎/⌘⏎ paste; Esc clears then hides; ⌘K/⌘F/⌘C/⌘E/etc.
  still work; type-to-filter works; 1-9 badges light on ⌘; Stats opens/closes.

## Risks
- Biggest rework of the keyboard model; implement incrementally with builds.
- `⌘C` dual meaning (copy-entry vs copy-text-selection): we keep copy-entry
  (accept). Acceptable for a search box.
- Modifier-only key events must update `cmd-held`; if a lone ⌘ press doesn't fire
  `key-pressed`, the badge highlight lags until the next key — minor.
- Native caret returns (the user wanted the *ugly static bar* gone; a real editable
  cursor is expected here and was the point of choosing this option).
