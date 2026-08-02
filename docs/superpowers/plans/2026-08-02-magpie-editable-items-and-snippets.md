# Magpie Editable Items + Snippets Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let users edit an existing entry's text in place and author reusable snippets (pinned text entries), both through one inline editor in the launcher's detail pane.

**Architecture:** A new `magpie_core::edit` module adds `update_entry_text` (recompute + in-place UPDATE, FTS trigger reindexes, duplicate-guarded) and `create_snippet` (pinned authored text entry, idempotent). The Slint detail pane gains an `edit-mode`/`edit-text` state with a `TextEdit` and Edit / New snippet / Save / Cancel controls; `runtime` dispatches saves to the two core methods.

**Tech Stack:** Rust, `rusqlite` (existing), Slint + `std-widgets` `TextEdit` (existing). No new dependencies.

## Global Constraints

- **No schema changes** — edits/inserts over existing `entries`; the `entries_au`/`entries_ai` FTS triggers keep `entries_fts` consistent.
- **Determinism:** `now_ms: i64` is injected; core calls no wall clock.
- **Editing keeps the entry id** and bumps `last_copied_at_ms = now_ms` (moves to top).
- **Editing must not create a duplicate:** if the new text's hash matches a *different* entry, `update_entry_text` returns `Ok(false)` and changes nothing.
- **Snippets are pinned text entries** (`pinned = 1`, `source_app_id = NULL`, no `copy_events`); `create_snippet` of identical existing text pins + returns that entry (idempotent).
- **Edit is offered only for non-image rows** (the UI checks `kind != "image"`).
- **Reuse** `content_hash`, `detect_text_kind`, `text_metrics`, `set_pinned` — do not reimplement.
- **Verify by running:** after UI wiring, actually launch the binary (`timeout 8 ./target/debug/magpie`; exit 124 = alive, no panic).
- **Commit style:** conventional commits, one per task.

---

### Task 1: `edit` module + `update_entry_text`

**Files:**
- Create: `crates/magpie-core/src/edit.rs`
- Modify: `crates/magpie-core/src/lib.rs` (`pub mod edit;`)
- Test: `edit.rs` tests module.

**Interfaces:**
- Consumes: `Store`, `Result`; `crate::model::{content_hash, Content}`, `crate::detect::detect_text_kind`, `crate::metrics::text_metrics`; `rusqlite::{params, OptionalExtension}`; (tests) `open_in_memory`, `CaptureEvent`, `ImageStore`, `default_query`.
- Produces: `impl Store { pub fn update_entry_text(&self, id: i64, new_text: &str, now_ms: i64) -> Result<bool> }` — `Ok(false)` (no change) when the new hash collides with a different entry; else UPDATE + `Ok(true)`.

- [ ] **Step 1: Write failing tests in `edit.rs`**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{CaptureEvent, Content};
    use crate::search::default_query;
    use crate::store::{open_in_memory, ImageStore, Store};

    struct Noop;
    impl ImageStore for Noop {
        fn put(&self, h: &str, _b: &[u8]) -> std::io::Result<String> { Ok(h.to_string()) }
    }
    fn ingest(s: &Store, t: &str, ms: i64) -> i64 {
        s.ingest(&CaptureEvent { content: Content::Text(t.into()), source_app: None, copied_at_ms: ms }, &Noop)
            .unwrap()
            .entry_id
    }

    #[test]
    fn update_changes_text_metrics_and_reindexes_fts() {
        let s = open_in_memory().unwrap();
        let id = ingest(&s, "hello world", 1);
        let ok = s.update_entry_text(id, "https://example.com/new", 5_000).unwrap();
        assert!(ok);

        let e = s.recent(1).unwrap().into_iter().next().unwrap();
        assert_eq!(e.id, id); // same id
        assert_eq!(e.full_text, "https://example.com/new");
        assert_eq!(e.kind.as_str(), "link"); // kind recomputed
        assert_eq!(e.last_copied_at_ms, 5_000); // bumped to top

        // FTS: old text gone, new text found
        let mut q = default_query();
        q.text = "hello".into();
        assert_eq!(s.search(&q).unwrap().len(), 0);
        q.text = "example".into();
        assert_eq!(s.search(&q).unwrap().len(), 1);
    }

    #[test]
    fn update_to_duplicate_is_rejected_noop() {
        let s = open_in_memory().unwrap();
        let a = ingest(&s, "alpha", 1);
        let _b = ingest(&s, "beta", 2);
        // editing "alpha" to "beta" would duplicate -> rejected
        let ok = s.update_entry_text(a, "beta", 3).unwrap();
        assert!(!ok);
        let e = s.recent(100).unwrap().into_iter().find(|e| e.id == a).unwrap();
        assert_eq!(e.full_text, "alpha"); // unchanged
    }
}
```

- [ ] **Step 2: Add `pub mod edit;` to `lib.rs`; run to verify fail**

Run: `cargo test -p magpie-core --lib edit::`
Expected: FAIL — `update_entry_text` not found.

- [ ] **Step 3: Implement `edit.rs`**

```rust
use crate::detect::detect_text_kind;
use crate::metrics::text_metrics;
use crate::model::{content_hash, Content};
use crate::store::{Result, Store};
use rusqlite::{params, OptionalExtension};

impl Store {
    pub fn update_entry_text(&self, id: i64, new_text: &str, now_ms: i64) -> Result<bool> {
        let hash = content_hash(&Content::Text(new_text.to_string()));
        let collision: Option<i64> = self
            .conn()
            .query_row(
                "SELECT id FROM entries WHERE content_hash = ?1 AND id != ?2",
                params![hash, id],
                |r| r.get(0),
            )
            .optional()?;
        if collision.is_some() {
            return Ok(false);
        }
        let kind = detect_text_kind(new_text);
        let m = text_metrics(new_text);
        let preview: String = new_text.chars().take(200).collect();
        self.conn().execute(
            "UPDATE entries SET
               content_hash = ?1, kind = ?2, preview_text = ?3, full_text = ?4,
               byte_size = ?5, char_count = ?6, word_count = ?7, line_count = ?8,
               last_copied_at_ms = ?9
             WHERE id = ?10",
            params![
                hash, kind.as_str(), preview, new_text,
                new_text.len() as i64, m.char_count, m.word_count, m.line_count,
                now_ms, id
            ],
        )?;
        Ok(true)
    }
}
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p magpie-core --lib edit::`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/magpie-core/src/edit.rs crates/magpie-core/src/lib.rs
git commit -m "feat(core): update_entry_text (in-place edit + FTS reindex + dup guard)"
```

---

### Task 2: `create_snippet`

**Files:**
- Modify: `crates/magpie-core/src/edit.rs`
- Test: `edit.rs` tests module.

**Interfaces:**
- Consumes: Task 1's imports + `set_pinned`.
- Produces: `impl Store { pub fn create_snippet(&self, text: &str, now_ms: i64) -> Result<i64> }` — pins+returns an existing identical entry, else inserts a pinned text entry and returns its id.

- [ ] **Step 1: Add failing tests to `edit.rs` tests module**

```rust
    #[test]
    fn create_snippet_inserts_pinned_text_entry() {
        let s = open_in_memory().unwrap();
        let id = s.create_snippet("Dear team, thanks!", 100).unwrap();
        let e = s.recent(1).unwrap().into_iter().next().unwrap();
        assert_eq!(e.id, id);
        assert_eq!(e.full_text, "Dear team, thanks!");
        assert!(e.pinned);
        assert_eq!(e.kind.as_str(), "text");
        assert!(e.source_app_id.is_none());
    }

    #[test]
    fn create_snippet_of_existing_text_pins_and_returns_existing() {
        let s = open_in_memory().unwrap();
        let existing = ingest(&s, "https://reuse.me", 1);
        let got = s.create_snippet("https://reuse.me", 2).unwrap();
        assert_eq!(got, existing); // same entry, not a duplicate
        assert_eq!(s.recent(100).unwrap().len(), 1);
        let e = s.recent(1).unwrap().into_iter().next().unwrap();
        assert!(e.pinned);
    }
```

- [ ] **Step 2: Run to verify fail**

Run: `cargo test -p magpie-core --lib edit::tests::create_snippet`
Expected: FAIL — `create_snippet` not found.

- [ ] **Step 3: Implement `create_snippet`** (add to the `impl Store` block in `edit.rs`)

```rust
    pub fn create_snippet(&self, text: &str, now_ms: i64) -> Result<i64> {
        let hash = content_hash(&Content::Text(text.to_string()));
        if let Some(id) = self
            .conn()
            .query_row("SELECT id FROM entries WHERE content_hash = ?1", [&hash], |r| r.get::<_, i64>(0))
            .optional()?
        {
            self.set_pinned(id, true)?;
            return Ok(id);
        }
        let kind = detect_text_kind(text);
        let m = text_metrics(text);
        let preview: String = text.chars().take(200).collect();
        self.conn().execute(
            "INSERT INTO entries
               (content_hash, kind, preview_text, full_text, image_path,
                byte_size, char_count, word_count, line_count,
                first_copied_at_ms, last_copied_at_ms, copy_count, pinned, source_app_id)
             VALUES (?1,?2,?3,?4,NULL,?5,?6,?7,?8,?9,?9,1,1,NULL)",
            params![
                hash, kind.as_str(), preview, text,
                text.len() as i64, m.char_count, m.word_count, m.line_count, now_ms
            ],
        )?;
        Ok(self.conn().last_insert_rowid())
    }
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p magpie-core --lib edit::`
Expected: PASS (4 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/magpie-core/src/edit.rs
git commit -m "feat(core): create_snippet (pinned authored entry, idempotent)"
```

---

### Task 3: Core integration test

**Files:**
- Create: `crates/magpie-core/tests/edit.rs`

**Interfaces:**
- Consumes: public `update_entry_text`, `create_snippet`, `open_in_memory`, `default_query`.
- Produces: end-to-end coverage that an edited snippet is searchable + slottable, and that a snippet survives retention.

- [ ] **Step 1: Write the integration test file**

```rust
//! Integration tests for editable items + snippets.

use magpie_core::{
    default_query, open_in_memory, CaptureEvent, Content, ImageStore, RetentionPolicy, Store,
};

struct Noop;
impl ImageStore for Noop {
    fn put(&self, h: &str, _b: &[u8]) -> std::io::Result<String> { Ok(h.to_string()) }
}
fn ingest(s: &Store, t: &str, ms: i64) -> i64 {
    s.ingest(&CaptureEvent { content: Content::Text(t.into()), source_app: None, copied_at_ms: ms }, &Noop)
        .unwrap()
        .entry_id
}

#[test]
fn edited_entry_is_searchable_by_new_text_and_slottable() {
    let s = open_in_memory().unwrap();
    let id = ingest(&s, "typpo here", 1);
    assert!(s.update_entry_text(id, "typo fixed here", 2).unwrap());

    let mut q = default_query();
    q.text = "fixed".into();
    let hits = s.search(&q).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].id, id);

    // still assignable to a slot
    s.assign_slot(4, id).unwrap();
    assert_eq!(s.slot_entry(4).unwrap().unwrap().full_text, "typo fixed here");
}

#[test]
fn snippet_is_pinned_and_survives_retention() {
    let s = open_in_memory().unwrap();
    let snip = s.create_snippet("my reusable template", 1).unwrap();
    ingest(&s, "junk", 2);

    // aggressive retention that would delete non-pinned
    let policy = RetentionPolicy { max_entries: None, max_age_ms: Some(0), max_image_bytes: None };
    s.enforce_retention(&policy, 1_000_000).unwrap();

    let remaining: Vec<i64> = s.recent(100).unwrap().into_iter().map(|e| e.id).collect();
    assert!(remaining.contains(&snip)); // pinned snippet kept
    let mut q = default_query();
    q.text = "reusable".into();
    assert_eq!(s.search(&q).unwrap().len(), 1);
}
```

- [ ] **Step 2: Run to verify it passes**

Run: `cargo test -p magpie-core --test edit`
Expected: PASS (2 tests).

- [ ] **Step 3: Commit**

```bash
git add crates/magpie-core/tests/edit.rs
git commit -m "test(core): edit searchable+slottable + snippet retention"
```

---

### Task 4: Slint inline editor markup

**Files:**
- Modify: `crates/magpie-app/ui/launcher.slint`

**Interfaces:**
- Produces on `LauncherWindow`:
  - `in-out property <string> edit-mode: "none";`
  - `in-out property <string> edit-text;`
  - `callback start-edit(int);` `callback new-snippet();` `callback save-edit(string);` `callback cancel-edit();`
  - In the list-view detail pane: view-mode action buttons (**Edit** shown only when the selected row's `kind != "image"`, and **New snippet**), and an edit-mode block with a `TextEdit` (two-way `edit-text`) + **Save** / **Cancel**.

> UI markup; the gate is that it **compiles** via `slint-build`. Visual + behavior check is manual (Task 5 runs the binary).

- [ ] **Step 1: Import `TextEdit`** — change the first import line to include it

```slint
import { LineEdit, ListView, TextEdit } from "std-widgets.slint";
```

- [ ] **Step 2: Add the properties + callbacks to `LauncherWindow`** (next to the other callbacks)

```slint
    in-out property <string> edit-mode: "none";
    in-out property <string> edit-text;
    callback start-edit(int);
    callback new-snippet();
    callback save-edit(string);
    callback cancel-edit();
```

- [ ] **Step 3: Add the editor to the detail pane** (inside the detail `Rectangle`'s `VerticalLayout`, after the slot-cells `if` block)

```slint
                // View-mode actions
                if root.edit-mode == "none": HorizontalLayout {
                    spacing: 6px;
                    alignment: start;
                    if entries.length > 0 && root.selected < entries.length && entries[root.selected].kind != "image": Rectangle {
                        width: 60px;
                        height: 26px;
                        background: #2a2a30;
                        border-radius: 6px;
                        TouchArea { clicked => { root.start-edit(root.selected); } }
                        Text { text: "Edit"; color: white; font-size: 12px; horizontal-alignment: center; vertical-alignment: center; }
                    }
                    Rectangle {
                        width: 96px;
                        height: 26px;
                        background: #2a2a30;
                        border-radius: 6px;
                        TouchArea { clicked => { root.new-snippet(); } }
                        Text { text: "New snippet"; color: white; font-size: 12px; horizontal-alignment: center; vertical-alignment: center; }
                    }
                }

                // Edit-mode editor
                if root.edit-mode != "none": VerticalLayout {
                    spacing: 6px;
                    editor := TextEdit {
                        min-height: 120px;
                        text <=> root.edit-text;
                    }
                    HorizontalLayout {
                        spacing: 6px;
                        alignment: start;
                        Rectangle {
                            width: 60px;
                            height: 26px;
                            background: #6a8cff;
                            border-radius: 6px;
                            TouchArea { clicked => { root.save-edit(root.edit-text); } }
                            Text { text: "Save"; color: white; font-size: 12px; horizontal-alignment: center; vertical-alignment: center; }
                        }
                        Rectangle {
                            width: 60px;
                            height: 26px;
                            background: #2a2a30;
                            border-radius: 6px;
                            TouchArea { clicked => { root.cancel-edit(); } }
                            Text { text: "Cancel"; color: white; font-size: 12px; horizontal-alignment: center; vertical-alignment: center; }
                        }
                    }
                }
```

- [ ] **Step 4: Compile (Slint codegen)**

Run: `cargo build -p magpie-app`
Expected: compiles (new setters/callbacks generated: `set_edit_mode`, `get_edit_mode`, `set_edit_text`, `get_edit_text`, `on_start_edit`, `on_new_snippet`, `on_save_edit`, `on_cancel_edit`). No Rust wiring yet, so callbacks are inert — that's fine until Task 5.

- [ ] **Step 5: Commit**

```bash
git add crates/magpie-app/ui/launcher.slint
git commit -m "feat(app): Slint inline editor (TextEdit + Edit/New snippet/Save/Cancel)"
```

---

### Task 5: Runtime wiring + run verification

**Files:**
- Modify: `crates/magpie-app/src/runtime.rs`

**Interfaces:**
- Consumes: `Store::{update_entry_text, create_snippet}`, generated `on_start_edit`/`on_new_snippet`/`on_save_edit`/`on_cancel_edit`, `get_edit_mode`/`set_edit_mode`/`set_edit_text`/`get_selected`, existing `current_results`, `refresh`, `now_ms`.
- Produces: the four callbacks wired in `start()`.

- [ ] **Step 1: Wire the callbacks in `start()`** (near the other `ui.on_*` blocks)

```rust
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_start_edit(move |index| {
            if let Some(ui) = w.upgrade() {
                let results = current_results(&s, now_ms());
                if let Some(e) = results.get(index as usize) {
                    ui.set_edit_text(SharedString::from(e.full_text.clone()));
                    ui.set_edit_mode(SharedString::from("entry"));
                }
            }
        });
    }
    {
        let w = ui.as_weak();
        ui.on_new_snippet(move || {
            if let Some(ui) = w.upgrade() {
                ui.set_edit_text(SharedString::from(""));
                ui.set_edit_mode(SharedString::from("snippet"));
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_save_edit(move |text| {
            let text = text.to_string();
            if let Some(ui) = w.upgrade() {
                let mode = ui.get_edit_mode().to_string();
                if !text.trim().is_empty() {
                    if let Ok(store) = s.store.lock() {
                        if mode == "entry" {
                            let idx = ui.get_selected() as usize;
                            let results = current_results(&s, now_ms());
                            if let Some(e) = results.get(idx) {
                                let _ = store.update_entry_text(e.id, &text, now_ms());
                            }
                        } else if mode == "snippet" {
                            let _ = store.create_snippet(&text, now_ms());
                        }
                    }
                }
                ui.set_edit_mode(SharedString::from("none"));
                refresh(&ui, &s);
            }
        });
    }
    {
        let w = ui.as_weak();
        ui.on_cancel_edit(move || {
            if let Some(ui) = w.upgrade() {
                ui.set_edit_mode(SharedString::from("none"));
            }
        });
    }
```

Note: `on_save_edit` takes the store lock and also calls `current_results` (which locks the store). Compute `results` BEFORE locking to avoid a re-entrant lock — reorder so `current_results(&s, ...)` runs before `s.store.lock()`:

```rust
        ui.on_save_edit(move |text| {
            let text = text.to_string();
            if let Some(ui) = w.upgrade() {
                let mode = ui.get_edit_mode().to_string();
                let idx = ui.get_selected() as usize;
                let results = current_results(&s, now_ms()); // locks+unlocks store
                if !text.trim().is_empty() {
                    if let Ok(store) = s.store.lock() {
                        if mode == "entry" {
                            if let Some(e) = results.get(idx) {
                                let _ = store.update_entry_text(e.id, &text, now_ms());
                            }
                        } else if mode == "snippet" {
                            let _ = store.create_snippet(&text, now_ms());
                        }
                    }
                }
                ui.set_edit_mode(SharedString::from("none"));
                refresh(&ui, &s);
            }
        });
```

Use this reordered version (it is the one to implement).

- [ ] **Step 2: Compile**

Run: `cargo build -p magpie-app`
Expected: compiles clean.

- [ ] **Step 3: Run the binary to verify it launches without panicking**

Run: `timeout 8 ./target/debug/magpie; echo "exit=$?"`
Expected: `exit=124` (still alive after 8s — no panic, no early exit). If it panics, read the message and fix before committing.

- [ ] **Step 4: Manual verification** (interactive, record in report)

Launch `cargo run -p magpie-app`; open the launcher; select a text entry; click **Edit**, change the text, **Save** — confirm the row updates and search finds the new text. Click **New snippet**, type text, **Save** — confirm it appears as a pinned entry. Click **Cancel** mid-edit — confirm no change.

- [ ] **Step 5: Commit**

```bash
git add crates/magpie-app/src/runtime.rs
git commit -m "feat(app): wire inline editor (edit entry + create snippet)"
```

---

## Self-Review

**Spec coverage:**
- `update_entry_text` (recompute, in-place, bump-to-top, FTS reindex, dup guard) → Task 1. ✅
- `create_snippet` (pinned authored entry, idempotent) → Task 2. ✅
- Edited entry searchable + slottable; snippet pinned + retention-exempt → Task 3. ✅
- Inline editor UI (edit-mode/edit-text, TextEdit, Edit/New snippet/Save/Cancel, image-guard on Edit) → Task 4. ✅
- Runtime dispatch (start-edit/new-snippet/save-edit/cancel-edit) → Task 5. ✅
- Empty-text guard (no empty snippet/entry) → Task 5 (`!text.trim().is_empty()`). ✅
- Run-verification per the "compile-clean ≠ runs" lesson → Task 5 Step 3. ✅

**Placeholder scan:** Task 4/5 UI + run use compile + `timeout` run + `## Manual verification`; all core logic (1–3) is TDD'd. No TODOs. The reordered `on_save_edit` is explicitly the version to implement (not two competing snippets).

**Type consistency:** `update_entry_text(i64, &str, i64) -> Result<bool>`, `create_snippet(&str, i64) -> Result<i64>`, `edit-mode`/`edit-text` string props, `start-edit(int)`/`save-edit(string)`/`new-snippet()`/`cancel-edit()` callbacks, and their generated Rust names (`set_edit_mode`, `get_selected`, etc.) are consistent across Tasks 4–5.

## Notes

- The FTS reindex on `update_entry_text` is automatic via the existing
  `entries_au` trigger (fires on any `entries` UPDATE) — no manual FTS write.
- `on_save_edit` must call `current_results` before taking the store lock to avoid
  a re-entrant `Mutex` deadlock (both lock `state.store`).
