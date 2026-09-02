# Magpie — Notes Foundation (Phase 1) Design

**Date:** 2026-09-02
**Status:** Approved design → implementation plan next
**Scope:** Phase 1 of a larger notes + tasks + links subsystem. This spec covers
**Notes foundation only**; later phases get their own specs.

## Overview

Extend Magpie beyond clipboard history into a notes/task workspace, keeping its
differentiator — **provenance** (Magpie already records *what* you copied plus
*where & when*). Notes and (later) tasks inherit the same provenance: "this note
was made from that Slack message at 1:48 PM."

### Unifying model (org-mode / Logseq flavored, notes-first)
- **Notes are "pages."** Daily notes (one per calendar day, auto-created) and named
  pages are the same kind of thing. A note's body is the source of truth.
- **A project is just a page.** `[[prod-incidents]]` links to (and auto-creates) a
  page, so notes, projects, and daily notes are all pages — no separate "project"
  concept.
- **A task will be a `- [ ]` line inside a note** (Phase 2), parsed into a Tasks
  index — notes and tasks never drift out of sync. Not built in Phase 1.
- **Provenance everywhere** — notes carry the source app + time and the originating
  clipboard entry when captured from a clip.
- **Additive SQLite**, mirroring the clipboard core's patterns.

### Phase decomposition (each = its own spec → plan → build)
1. **Notes foundation** ← THIS SPEC (pages, editor, `[[link]]` navigation, capture
   from a clipboard entry with provenance).
2. **Tasks** — parse `- [ ]` lines → Tasks index + view; promote-line-to-task;
   toggle-done writes back; inline `!priority @due #project`.
3. **Backlinks & link graph** — backlinks panel, unlinked mentions.
4. **Reminders** — `@due` → notifications (reuses the diagnostics notify infra).
5. **Keyboard navigation** — jump between notes/tasks/panes (the queued kbd spec).

## Data model

Add to `crates/magpie-core/src/schema.sql` (runs via `CREATE TABLE IF NOT EXISTS`
on open → **no migration**):

```sql
CREATE TABLE IF NOT EXISTS notes (
  id              INTEGER PRIMARY KEY,
  name            TEXT NOT NULL,               -- page name; daily notes = ISO date "2026-09-02"
  is_daily        INTEGER NOT NULL DEFAULT 0,  -- 1 for daily notes
  body            TEXT NOT NULL DEFAULT '',
  created_at_ms   INTEGER NOT NULL,
  updated_at_ms   INTEGER NOT NULL,
  source_app_id   INTEGER,                     -- provenance: frontmost app at capture (nullable)
  source_entry_id INTEGER                      -- provenance: originating clipboard entry (nullable)
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_notes_name ON notes(name);
CREATE INDEX IF NOT EXISTS idx_notes_updated ON notes(updated_at_ms);
```

`name` is unique: one page per name (Logseq-style), so `[[X]]` always resolves to
the same page. Daily notes are named by their ISO date and have `is_daily = 1`.
`source_app_id` references the existing `apps` table (no FK constraint, matching the
codebase's existing loose-reference style); `source_entry_id` references `entries`.
Notes are **not** deleted by clipboard retention (retention only touches
`entries`/`copy_events`/`entry_tags`).

## Core: `magpie_core::notes` (methods on `Store`)

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note {
    pub id: i64,
    pub name: String,
    pub is_daily: bool,
    pub body: String,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub source_app_id: Option<i64>,
    pub source_entry_id: Option<i64>,
}

impl Store {
    /// Get-or-create the daily note for `day` (ISO "YYYY-MM-DD"). is_daily = true.
    pub fn daily_note(&self, day: &str, now_ms: i64) -> Result<Note>;
    /// Get-or-create a named page (powers [[links]]). Trims the name; empty → Err.
    pub fn upsert_note_by_name(&self, name: &str, now_ms: i64) -> Result<Note>;
    pub fn get_note(&self, id: i64) -> Result<Option<Note>>;
    pub fn note_by_name(&self, name: &str) -> Result<Option<Note>>;
    /// Overwrite body, bump updated_at_ms. Ok(false) if the note id doesn't exist.
    pub fn update_note_body(&self, id: i64, body: &str, now_ms: i64) -> Result<bool>;
    /// Most-recently-updated notes first, capped at `limit`.
    pub fn recent_notes(&self, limit: i64) -> Result<Vec<Note>>;
    /// Distinct page names (for [[ ]] autocomplete + navigation), sorted.
    pub fn all_note_names(&self) -> Result<Vec<String>>;
    /// Capture: new note whose body is the entry's full_text, name derived from its
    /// first non-empty line (fallback "Note <created_at ISO>"), provenance copied
    /// (source_app_id from the entry, source_entry_id = entry id). Name collisions
    /// get a " (2)", " (3)" … suffix so capture never fails on a dup name.
    pub fn create_note_from_entry(&self, entry_id: i64, now_ms: i64) -> Result<Note>;
}
```

- `daily_note`/`upsert_note_by_name` are idempotent get-or-create (INSERT then
  SELECT, or SELECT-then-INSERT guarded by the unique index).
- Time is injected as `now_ms` (core stays wall-clock-free, matching the codebase).
  The ISO day string is computed by the **app** (it already has `format_time`).

## App: pure helpers (`crates/magpie-app/src/notes_view.rs`) — tested

```rust
/// Extract `[[Page Name]]` references from a note body: deduped, trimmed, in first
/// -seen order. `[[ ]]`/empty are skipped. Used for the Links strip + navigation.
pub fn wiki_links(body: &str) -> Vec<String>;

/// A short, human title for a note in the list: the name, or for an untitled
/// capture the first line of the body, elided.
pub fn note_list_title(note_name: &str, body: &str) -> String;
```

`viewmodel`/runtime maps `Note` → a Slint `NoteRow` struct for the list.

## UI (Slint) — a new **Notes mode**

The window gains a top-level mode toggle between **Clipboard** (today's view) and
**Notes**. In the top tab bar, add a **Notes** entry; a shortcut flips modes
(`Ctrl`-modified key, mac ⌘ — see below). Entering Notes mode lands on **today's
daily note**.

Notes-mode layout (reuses existing two-pane structure):
- **Left — notes list:** today's daily note pinned at top, then `recent_notes`
  (most-recent first); a name-filter `TextInput` (filters the list by name
  substring); a "New note" affordance. Each row: title (`note_list_title`) +
  relative updated time + a 📌 daily marker.
- **Right — note editor:** a read/write multi-line `TextInput` (same explicit-color
  pattern as the clipboard preview, wrapped for scrolling) bound to the selected
  note's body; edits call `update_note_body` (debounced/on-change). Above/below it:
  - a **provenance line** ("from Slack · 1:48 PM" when `source_*` set; "created in
    Magpie · <time>" otherwise),
  - a **Links strip:** `wiki_links(body)` rendered as clickable chips → open (via
    `upsert_note_by_name`, so it auto-creates) and select that page. A `[[`
    autocomplete against `all_note_names()` is included if it fits cleanly;
    otherwise deferred (chips alone satisfy navigation).

New Slint props/callbacks on `LauncherWindow` (names indicative):
`in property <[NoteRow]> notes;` `in-out property <int> note-selected;`
`in property <string> note-body;` `in property <string> note-provenance;`
`in property <[string]> note-links;` `in-out property <string> note-filter;`
`callback open-note(int);` `callback edit-note-body(string);`
`callback new-note();` `callback open-page(string);` (open/create by name)
`callback note-from-entry(int);` `callback set-mode-notes(bool);`

## Capture from a clipboard entry

In the clipboard view, a ⌘K action **"New note from this entry"** (+ a direct
shortcut) calls `note-from-entry(selected)` → `create_note_from_entry` → switches to
Notes mode with the new note open. Provenance (source app + originating entry) is
recorded. Paste/masking/security paths are untouched.

## Data flow

- **Enter Notes mode:** compute today's ISO date (`format_time`), `daily_note(...)`,
  load `recent_notes` + `all_note_names`, select the daily note, push body + links.
- **Open a page / click a `[[link]]`:** `upsert_note_by_name` → select + push.
- **Edit:** on body change, `update_note_body`; refresh the Links strip from
  `wiki_links`.
- **Capture:** `create_note_from_entry` → switch mode + open.
- Runtime holds no note state beyond the DB; the selected note id + a cached list
  drive the view (re-query on changes), mirroring the clipboard view's pattern.

## Security / privacy

Notes are stored **plaintext** in the same SQLite DB as clips (documented; not an
at-rest boundary — SQLCipher is the separate deferred item). No new network paths.
Display-masking and the never-capture skip list apply to *clipboard capture* and are
unrelated to notes the user authors. Provenance stores only the app name/id already
recorded for clips.

## Testing

**core (`notes.rs` unit + `tests/notes.rs`):**
- `daily_note` get-or-create is idempotent (same day → same id, `is_daily`).
- `upsert_note_by_name` idempotent + trims; empty name → Err.
- `update_note_body` overwrites + bumps `updated_at_ms`; unknown id → Ok(false).
- `create_note_from_entry` copies provenance (source_app_id, source_entry_id), body
  = entry full_text, name from first line; dup name → suffixed.
- `recent_notes` ordering; `all_note_names` distinct + sorted.

**app (`notes_view.rs` unit):**
- `wiki_links`: extracts `[[A]] text [[B]]` → `["A","B"]`, dedups, trims, skips
  empty `[[]]`.
- `note_list_title`: name when present; first-line elision for untitled captures.

**run-verify:** `timeout 8 ./target/debug/magpie` (exit 124); open Notes mode, create
a note, type a `[[link]]`, click the chip to navigate.

## File structure

- Modify: `crates/magpie-core/src/schema.sql` (add `notes` + indexes).
- Create: `crates/magpie-core/src/notes.rs` (`Note` + the `Store` methods).
- Modify: `crates/magpie-core/src/lib.rs` (`pub mod notes;` + re-export `Note`).
- Create: `crates/magpie-core/tests/notes.rs`.
- Create: `crates/magpie-app/src/notes_view.rs` (`wiki_links`, `note_list_title`).
- Modify: `crates/magpie-app/src/lib.rs` (`pub mod notes_view;`).
- Modify: `crates/magpie-app/ui/launcher.slint` (Notes mode: list + editor + links
  strip + provenance; `NoteRow` struct; props/callbacks).
- Modify: `crates/magpie-app/src/runtime.rs` (mode toggle, wire note callbacks,
  capture action, ⌘K "New note from this entry").

## Risks / notes

- **`update_note_body` write frequency** — debounce on-change writes (or write on
  blur / selection change) so rapid typing isn't one SQLite write per keystroke.
- **Editable-field perf** — a `TextInput` shapes its whole string (same constraint
  as the clipboard preview). Notes are user-authored and typically small; if a note
  grows huge, the same viewport concern applies — acceptable for Phase 1, revisit if
  needed.
- **Mode-switch shortcut** — must use `event.modifiers.control` (⌘ on mac via
  Slint's winit mapping), consistent with every other Magpie shortcut.
- **`[[link]]` names** — trimmed; case-sensitivity of page names follows the unique
  index (exact match). Case-insensitive page resolution is a possible later refinement.
