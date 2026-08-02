# Magpie Editable Items + Snippets — Design

**Date:** 2026-08-02
**Status:** Approved (brainstorm complete)
**Phase:** 1 (fourth sub-project)

## Summary

Two related, editor-sharing features:
- **Editable items** — edit an existing entry's text in place before pasting.
- **Snippets/templates** — author reusable text (not from the clipboard) that lives
  as a **pinned entry** in the main list, so it's searchable, slottable,
  quick-pasteable, and retention-exempt like any pin.

Both are driven by one inline editor in the launcher's detail pane and two small
core methods. No new dependencies. Editing keeps the entry's id but bumps it to
the top; creating a snippet of already-present text simply pins that entry.

## Goals & Non-Goals

**Goals**
- `Store::update_entry_text` — recompute kind/metrics/hash, update in place, keep
  the FTS index consistent, and refuse edits that would duplicate another entry.
- `Store::create_snippet` — insert a pinned text entry (idempotent on identical text).
- An inline editor (multi-line) in the detail pane for both Edit and New snippet.

**Non-Goals (v1)**
- A separate snippets view/tab (snippets are pinned entries in the main list).
- Rich-text / templating placeholders (`{name}`) — plain text only.
- Editing images or file entries (Edit is offered only for text-ish rows).

## Core: `magpie_core::edit` (methods on `Store`)

```rust
impl Store {
    /// Replace an entry's text in place: recompute kind/metrics/content_hash and
    /// bump last_copied_at_ms to `now_ms`. Returns Ok(false) without changing
    /// anything if the new text's hash matches a DIFFERENT existing entry
    /// (editing must not create a duplicate). Ok(true) on success.
    pub fn update_entry_text(&self, id: i64, new_text: &str, now_ms: i64) -> Result<bool>;

    /// Create a pinned, user-authored text entry. If an entry with identical text
    /// already exists, pin it and return its id (idempotent). Returns the entry id.
    pub fn create_snippet(&self, text: &str, now_ms: i64) -> Result<i64>;
}
```

Implementation notes:
- `update_entry_text`: collision check
  `SELECT 1 FROM entries WHERE content_hash = ?hash AND id != ?id` → if present,
  `Ok(false)`. Otherwise `UPDATE entries SET content_hash, kind, preview_text,
  full_text, byte_size, char_count, word_count, line_count, last_copied_at_ms
  WHERE id = ?`. The `entries_au` trigger reindexes FTS automatically.
- `create_snippet`: if `SELECT id FROM entries WHERE content_hash = ?hash` exists,
  `set_pinned(id, true)` and return it. Otherwise `INSERT` a text entry with
  `pinned = 1`, `source_app_id = NULL`, `copy_count = 1`, `first = last = now_ms`,
  `image_path = NULL`. No `copy_events` row (a snippet is authored, not copied).
- Both reuse `content_hash(&Content::Text(..))`, `detect_text_kind`, and
  `text_metrics`. `preview_text` is the first 200 chars (matching `prepare`).
- Time is injected (`now_ms`); core calls no wall clock.

## UI: inline editor in the detail pane

`LauncherWindow` gains:
- `in-out property <string> edit-mode: "none";` (`"none"` | `"entry"` | `"snippet"`)
- `in-out property <string> edit-text;` (two-way with a multi-line `TextEdit`)
- callbacks: `start-edit(int)`, `new-snippet()`, `save-edit(string)`, `cancel-edit()`.

Detail pane behavior:
- **View mode** (`edit-mode == "none"`): the existing detail text, the slot cells,
  and two buttons — **Edit** (calls `start-edit(selected)`) and **New snippet**
  (calls `new-snippet()`).
- **Edit mode** (`edit-mode != "none"`): a `TextEdit` bound to `edit-text`, plus
  **Save** (`save-edit(edit-text)`) and **Cancel** (`cancel-edit()`).

`TextEdit` comes from `std-widgets.slint`.

## Runtime wiring

- `on_start_edit(i)`: read `current_results()[i].full_text`, `ui.set_edit_text(..)`,
  `ui.set_edit_mode("entry")`. (Store the selected index implicitly via the row the
  user edited; save uses `ui.get_selected()`.)
- `on_new_snippet()`: `ui.set_edit_text("")`, `ui.set_edit_mode("snippet")`.
- `on_save_edit(text)`: read `edit-mode`; if `"entry"`, resolve the selected
  entry id from `current_results()[ui.get_selected()]` and call
  `update_entry_text(id, &text, now_ms())`; if `"snippet"`, call
  `create_snippet(&text, now_ms())`. Then `ui.set_edit_mode("none")` and `refresh`.
- `on_cancel_edit()`: `ui.set_edit_mode("none")`.

## Data Flow

```
Edit:  start-edit(i) -> edit_text=results[i].full_text, edit_mode="entry"
       save-edit(t)  -> update_entry_text(selected_id, t, now) [core] -> refresh
New:   new-snippet() -> edit_text="", edit_mode="snippet"
       save-edit(t)  -> create_snippet(t, now) [core, pinned]      -> refresh
```

## Error Handling

- `update_entry_text` returning `Ok(false)` (duplicate) is surfaced as a no-op —
  the editor closes and the list is unchanged (v1 has no toast; documented).
- Store lock / SQL errors in the callbacks are swallowed (logged), never crash.
- Empty edit text: `save-edit("")` on `entry` updates to empty text (allowed);
  on `snippet` creates an empty snippet — the UI guards by ignoring a save when
  `edit-text` is empty (no empty snippets/entries).

## Testing

**core (`edit.rs` unit + `tests/edit.rs`):**
- `update_entry_text` changes full_text/preview/kind/metrics + hash; the entry is
  found by searching the new text and NOT the old (FTS trigger fired); id unchanged.
- `update_entry_text` collision (new text equals another entry) → `Ok(false)`,
  target entry unchanged.
- `create_snippet` inserts a pinned entry that appears in `recent`, is pinned, has
  the detected kind (e.g. a URL snippet → `link`).
- `create_snippet` of text identical to an existing entry pins + returns that id
  (no duplicate).

**app:** the edit-mode dispatch is thin runtime glue; covered by manual run.

**UI/run:** manual — and the binary is **actually launched** (`timeout 8
./target/debug/magpie`, exit 124 = alive, no panic) after wiring, per the lesson
that compile-clean ≠ runs.

## File Structure

- Create: `crates/magpie-core/src/edit.rs` (`update_entry_text`, `create_snippet`).
- Modify: `crates/magpie-core/src/lib.rs` (`pub mod edit;`).
- Create: `crates/magpie-core/tests/edit.rs`.
- Modify: `crates/magpie-app/ui/launcher.slint` (edit-mode, edit-text, TextEdit, buttons, callbacks).
- Modify: `crates/magpie-app/src/runtime.rs` (wire start-edit/new-snippet/save-edit/cancel-edit).

## Risks / Notes

- **Editing an image row** would coerce it to text (kind recomputed) — avoided by
  only showing **Edit** when the selected row's kind is not `image` (the UI checks
  `entries[selected].kind != "image"`).
- **Snippet analytics:** snippets have no `copy_events`, so they don't inflate
  copies-over-time; they appear in most-copied only once actually copied. Intended.
