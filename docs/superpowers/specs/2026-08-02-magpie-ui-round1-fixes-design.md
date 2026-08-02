# Magpie UI Round 1 — Tray, Multiline, Preview Fixes

**Date:** 2026-08-02 · **Status:** Approved · **Phase:** 1

## Summary

Three surgical fixes to the current launcher, ahead of the full Raycast-style
visual redesign (Round 2, separate spec). No layout overhaul here:

1. **Tray click behavior** — left-click the menu-bar 🐦 opens the window;
   right-click shows a menu (`Show Magpie` / `Quit Magpie`).
2. **Multiline list rows** — a multiline copy shows its first line plus an
   `N lines` badge instead of silently hiding the rest.
3. **Preview follows selection** — the right-hand preview shows the *selected*
   entry's full, scrollable, multiline text (today it is hardwired to the first
   result and never updates on selection).

Round 2 (deferred, its own spec): new two-pane layout, type-icon rows, metadata
block, bottom action bar with keyboard hints, type filter, keyboard-first
navigation (↑/↓/⏎/Esc) alongside mouse.

## 1. Tray click behavior

**Today** (`build_tray`, `runtime.rs`): `TrayIconBuilder` attaches a menu. On
macOS tray-icon opens that menu on *any* click, so left-click cannot open the
window. Only a `Quit Magpie` item exists.

**Change:**
- Menu items (right-click): `Show Magpie`, separator, `Quit Magpie`.
- `TrayIconBuilder::with_menu_on_left_click(false)` so left-click does not open
  the menu.
- Spawn a thread on `tray_icon::TrayIconEvent::receiver()`. On a left mouse-button
  click event, `slint::invoke_from_event_loop` → `refresh(&ui, &state)` then
  `ui.show()` (the exact path `HotAction::Launcher` already uses).
- The existing `MenuEvent` thread keeps handling menu clicks: `Quit Magpie` →
  `quit_event_loop` (unchanged); `Show Magpie` → same show path as left-click.
- `build_tray` must therefore receive what it needs to show the window: the
  `slint::Weak<LauncherWindow>` and `Arc<AppState>` (today it takes nothing).

**Cross-platform note:** left-click `TrayIconEvent`s are reliable on macOS and
Windows but not consistently delivered on Linux (GTK). `Show Magpie` in the
right-click menu is the portable fallback, so the window is always reachable.

**Show helper:** extract the "refresh + show + focus" sequence used by the
launcher hotkey and both tray paths into one `show_window(&ui, &state)` helper so
all three call sites stay identical.

## 2. Multiline list rows

**Today** (`preview_title`, `runtime.rs`): returns `full_text.lines().next()` — the
first line only, with no indication more exists.

**Change:**
- Keep the first-line title.
- Add `EntryRow.badge: string`. Compute from the entry: count non-empty lines; if
  `> 1`, `badge = "<n> lines"`, else `badge = ""`.
- In `launcher.slint`, render `badge` (when non-empty) as a small muted pill at
  the right end of the row's title line, before the merge `+`/`✓` button.

`badge` derives only from the entry's own text — a pure function
`line_badge(full_text: &str) -> String`, unit-testable.

## 3. Preview follows selection + full multiline

**Today:** `refresh` sets `detail_text` to `results.first()` (`runtime.rs`), and the
preview `Text` binds to `root.detail-text`. Selection (`root.selected`) changes the
row highlight but never the preview — so the preview shows an unrelated entry.

**Change:**
- Add `EntryRow.full: string` = the entry's full text, populated in `to_rows`.
- In `launcher.slint`, bind the preview to
  `entries[root.selected].full` (guarded when `entries.length > 0 &&
  selected < entries.length`), making it reactive to selection with no extra
  callback. Remove the now-unused `detail-text` push from `refresh` (keep the
  property only if other code still reads it; otherwise delete it).
- Wrap the preview `Text` in a `ScrollView` (or `Flickable`) with `wrap:
  word-wrap` so long/multiline content is fully readable and scrolls.

Paste/slots/merge/edit are untouched — they already act on the selected entry via
the store, not the preview string.

## Architecture

- Pure helpers in `runtime.rs` (or a small sibling module), unit-tested:
  `line_badge(full_text: &str) -> String`.
- `EntryRow` (in `launcher.slint`) gains `full: string` and `badge: string`.
- `to_rows` populates the two new fields.
- `build_tray(weak, state)` gains parameters and the left-click/show wiring;
  `show_window(&ui, &state)` helper shared by hotkey + tray.

## Data Flow

```
capture (unchanged) -> full_text stored
refresh -> to_rows(entries…) sets each row.full + row.badge
list row -> shows title + (badge if multiline)
select row i -> preview binds entries[i].full (scrollable, wrapped)
tray left-click / "Show Magpie" -> show_window(refresh + show)
tray right-click -> menu
```

## Error Handling

- If `TrayIconEvent::receiver()` / left-click events are unavailable on the
  platform, the menu's `Show Magpie` still works (documented fallback).
- Preview binding is guarded against empty/out-of-range selection (shows empty).
- `line_badge` on empty or single-line text returns `""` (no pill).

## Testing

**Unit (core-style, in `runtime.rs` tests or a helpers module):**
- `line_badge("a\nb\nc")` → `"3 lines"`; `line_badge("one line")` → `""`;
  `line_badge("")` → `""`; blank interior lines don't inflate the count
  (`"a\n\nb"` → `"2 lines"`).

**Manual run-verify (`just run`):**
- Left-click the 🐦 tray icon → window opens. Right-click → menu with `Show
  Magpie` + `Quit Magpie`; `Show Magpie` opens it, `Quit` exits.
- Copy a multiline snippet → its row shows `N lines`; selecting it shows the full
  text in the preview, scrollable.
- Click different rows → preview updates to each selected entry (no stale first
  entry).

## File Structure

- Modify: `crates/magpie-app/src/runtime.rs` — `build_tray` signature + left-click
  wiring; `show_window` helper; `line_badge`; `to_rows` (full + badge); drop the
  `detail-text` push. Add `#[cfg(test)]` tests for `line_badge`.
- Modify: `crates/magpie-app/ui/launcher.slint` — `EntryRow { … full, badge }`;
  badge pill in the row; preview bound to `entries[selected].full` inside a
  `ScrollView`.

## Risks / Notes

- **Round 2 dependency:** these changes are structured so the redesign builds on
  them (per-selection preview, row `full`/`badge`) rather than being thrown away.
- Removing `detail-text`: verify no other binding/callback reads it before
  deleting; if anything does, leave the property and just stop pushing
  `results.first()` into it.
- `tray-icon` 0.24 API (`with_menu_on_left_click`, `TrayIconEvent`,
  `MouseButton`/`ClickType` naming) to be confirmed against the installed version
  during implementation.
