# Magpie Pinned Quick-Paste Slots Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Persistent numbered slots (1–9) so `Super+Ctrl+N` pastes a specifically-assigned entry (falling back to the Nth most-recent when the slot is empty), assignable by clicking a numbered cell in the launcher's detail pane.

**Architecture:** A new additive `slots` table maps slot→entry (no migration; `CREATE TABLE IF NOT EXISTS` runs on open). Core gains `assign_slot`/`clear_slot`/`slot_entry`/`slot_map`; assigning pins the entry (retention-exempt). A pure `resolve_slot_or_recent` helper picks slot-or-recency for the hotkey. The Slint launcher annotates each row with its slot and renders 9 clickable assign cells.

**Tech Stack:** Rust, `rusqlite` (existing), Slint (existing). No new dependencies.

## Global Constraints

- **Additive schema only:** add the `slots` table via `CREATE TABLE IF NOT EXISTS`; no `ALTER`, no migration framework.
- **Slot range is 1..=9;** out-of-range slot → `Err` (write methods) or `None` (`slot_entry`), no write.
- **Invariants:** one entry per slot (PK), one slot per entry (`assign_slot` removes the entry from any prior slot first).
- **Assigning a slot sets `pinned = 1`** (retention-exempt); `clear_slot` leaves `pinned` untouched.
- **No foreign key** on `slots`; `slot_entry` uses an INNER JOIN so a missing entry reads as an empty slot.
- **Determinism:** core takes no wall clock; time already injected elsewhere.
- **Hotkey behavior:** `Super+Ctrl+N` = slot N entry if present, else Nth most-recent.
- **Commit style:** conventional commits, one per task.

---

### Task 1: schema `slots` table + `assign_slot` / `clear_slot` / `slot_map`

**Files:**
- Modify: `crates/magpie-core/src/schema.sql`
- Create: `crates/magpie-core/src/slots.rs`
- Modify: `crates/magpie-core/src/lib.rs` (`pub mod slots;`)
- Test: `slots.rs` tests module.

**Interfaces:**
- Consumes: `Store`, `Result`; (tests) `open_in_memory`, `CaptureEvent`, `Content`, `ImageStore`.
- Produces:
  - `impl Store { pub fn assign_slot(&self, slot: i64, entry_id: i64) -> Result<()> }` — validates 1..=9, deletes the entry from any prior slot, upserts `slot→entry`, sets `pinned = 1`; one transaction.
  - `impl Store { pub fn clear_slot(&self, slot: i64) -> Result<()> }` — validates 1..=9, deletes the slot row (idempotent).
  - `impl Store { pub fn slot_map(&self) -> Result<Vec<(i64, i64)>> }` — `(slot, entry_id)` pairs ordered by slot.
  - `fn slot_err() -> rusqlite::Error` (module-private) — the out-of-range error.

- [ ] **Step 1: Add the table to `schema.sql`** (after the `copy_events` indexes, before the FTS section)

```sql
CREATE TABLE IF NOT EXISTS slots (
  slot     INTEGER PRIMARY KEY CHECK(slot BETWEEN 1 AND 9),
  entry_id INTEGER NOT NULL
);
```

- [ ] **Step 2: Write failing tests in `slots.rs`**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{CaptureEvent, Content};
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
    fn pinned(s: &Store, id: i64) -> bool {
        s.recent(100).unwrap().into_iter().find(|e| e.id == id).unwrap().pinned
    }

    #[test]
    fn assign_inserts_mapping_and_pins_entry() {
        let s = open_in_memory().unwrap();
        let id = ingest(&s, "a", 1);
        s.assign_slot(3, id).unwrap();
        assert_eq!(s.slot_map().unwrap(), vec![(3, id)]);
        assert!(pinned(&s, id));
    }

    #[test]
    fn reassigning_an_entry_moves_it_between_slots() {
        let s = open_in_memory().unwrap();
        let id = ingest(&s, "a", 1);
        s.assign_slot(3, id).unwrap();
        s.assign_slot(5, id).unwrap();
        assert_eq!(s.slot_map().unwrap(), vec![(5, id)]); // no stale slot 3
    }

    #[test]
    fn assigning_to_occupied_slot_replaces_occupant() {
        let s = open_in_memory().unwrap();
        let a = ingest(&s, "a", 1);
        let b = ingest(&s, "b", 2);
        s.assign_slot(3, a).unwrap();
        s.assign_slot(3, b).unwrap();
        assert_eq!(s.slot_map().unwrap(), vec![(3, b)]);
    }

    #[test]
    fn clear_slot_removes_and_is_idempotent() {
        let s = open_in_memory().unwrap();
        let id = ingest(&s, "a", 1);
        s.assign_slot(3, id).unwrap();
        s.clear_slot(3).unwrap();
        s.clear_slot(3).unwrap(); // idempotent
        assert!(s.slot_map().unwrap().is_empty());
    }

    #[test]
    fn out_of_range_slot_errors_and_writes_nothing() {
        let s = open_in_memory().unwrap();
        let id = ingest(&s, "a", 1);
        assert!(s.assign_slot(0, id).is_err());
        assert!(s.assign_slot(10, id).is_err());
        assert!(s.clear_slot(0).is_err());
        assert!(s.slot_map().unwrap().is_empty());
    }
}
```

- [ ] **Step 3: Add `pub mod slots;` to `lib.rs`; run to verify fail**

Run: `cargo test -p magpie-core --lib slots::`
Expected: FAIL — items not found.

- [ ] **Step 4: Implement `slots.rs`** (assign/clear/slot_map + the error)

```rust
use crate::store::{Result, Store};

fn slot_err() -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(Box::new(std::io::Error::new(
        std::io::ErrorKind::InvalidInput,
        "slot must be in 1..=9",
    )))
}

impl Store {
    pub fn assign_slot(&self, slot: i64, entry_id: i64) -> Result<()> {
        if !(1..=9).contains(&slot) {
            return Err(slot_err());
        }
        let tx = self.conn().unchecked_transaction()?;
        // one slot per entry: drop any prior slot holding this entry
        tx.execute("DELETE FROM slots WHERE entry_id = ?1", [entry_id])?;
        // one entry per slot: upsert
        tx.execute(
            "INSERT INTO slots (slot, entry_id) VALUES (?1, ?2)
             ON CONFLICT(slot) DO UPDATE SET entry_id = excluded.entry_id",
            [slot, entry_id],
        )?;
        // slots are kept: pin the entry
        tx.execute("UPDATE entries SET pinned = 1 WHERE id = ?1", [entry_id])?;
        tx.commit()?;
        Ok(())
    }

    pub fn clear_slot(&self, slot: i64) -> Result<()> {
        if !(1..=9).contains(&slot) {
            return Err(slot_err());
        }
        self.conn().execute("DELETE FROM slots WHERE slot = ?1", [slot])?;
        Ok(())
    }

    pub fn slot_map(&self) -> Result<Vec<(i64, i64)>> {
        let mut stmt = self.conn().prepare("SELECT slot, entry_id FROM slots ORDER BY slot")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))?;
        rows.collect()
    }
}
```

- [ ] **Step 5: Run to verify pass**

Run: `cargo test -p magpie-core --lib slots::`
Expected: PASS (5 tests).

- [ ] **Step 6: Commit**

```bash
git add crates/magpie-core/src/schema.sql crates/magpie-core/src/slots.rs crates/magpie-core/src/lib.rs
git commit -m "feat(core): slots table + assign_slot/clear_slot/slot_map"
```

---

### Task 2: `slot_entry`

**Files:**
- Modify: `crates/magpie-core/src/slots.rs`
- Test: `slots.rs` tests module.

**Interfaces:**
- Consumes: `Entry`, `crate::search::{row_to_entry, ENTRY_COLUMNS}` (both `pub(crate)`).
- Produces: `impl Store { pub fn slot_entry(&self, slot: i64) -> Result<Option<Entry>> }` — the entry currently in `slot` via `entries JOIN slots`; `None` when out of range, empty, or the entry is gone.

- [ ] **Step 1: Add failing tests to `slots.rs` tests module**

```rust
    #[test]
    fn slot_entry_returns_assigned_entry_or_none() {
        let s = open_in_memory().unwrap();
        let id = ingest(&s, "hello", 1);
        assert!(s.slot_entry(3).unwrap().is_none()); // empty
        s.assign_slot(3, id).unwrap();
        let e = s.slot_entry(3).unwrap().unwrap();
        assert_eq!(e.id, id);
        assert_eq!(e.full_text, "hello");
        assert!(s.slot_entry(0).unwrap().is_none()); // out of range -> None
    }
```

- [ ] **Step 2: Run to verify fail**

Run: `cargo test -p magpie-core --lib slots::tests::slot_entry`
Expected: FAIL — `slot_entry` not found.

- [ ] **Step 3: Implement `slot_entry`** (add to the `impl Store` block; add the imports at the top of `slots.rs`)

```rust
use crate::model::Entry;
use crate::search::{row_to_entry, ENTRY_COLUMNS};
```

```rust
    pub fn slot_entry(&self, slot: i64) -> Result<Option<Entry>> {
        if !(1..=9).contains(&slot) {
            return Ok(None);
        }
        let cols = ENTRY_COLUMNS
            .split(", ")
            .map(|c| format!("e.{c}"))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "SELECT {cols} FROM entries e JOIN slots s ON s.entry_id = e.id WHERE s.slot = ?1"
        );
        let mut stmt = self.conn().prepare(&sql)?;
        let mut rows = stmt.query_map([slot], row_to_entry)?;
        match rows.next() {
            Some(r) => Ok(Some(r?)),
            None => Ok(None),
        }
    }
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p magpie-core --lib slots::`
Expected: PASS (6 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/magpie-core/src/slots.rs
git commit -m "feat(core): slot_entry lookup"
```

---

### Task 3: Core integration test

**Files:**
- Create: `crates/magpie-core/tests/slots.rs`

**Interfaces:**
- Consumes: public `assign_slot`/`clear_slot`/`slot_entry`/`slot_map`, `open_in_memory`.
- Produces: end-to-end coverage that assignment survives newer copies and that pinning protects the slot entry from retention.

- [ ] **Step 1: Write the integration test file**

```rust
//! Integration tests for pinned quick-paste slots.

use magpie_core::{open_in_memory, CaptureEvent, Content, ImageStore, RetentionPolicy};

struct Noop;
impl ImageStore for Noop {
    fn put(&self, h: &str, _b: &[u8]) -> std::io::Result<String> { Ok(h.to_string()) }
}
fn ingest(s: &magpie_core::Store, t: &str, ms: i64) -> i64 {
    s.ingest(&CaptureEvent { content: Content::Text(t.into()), source_app: None, copied_at_ms: ms }, &Noop)
        .unwrap()
        .entry_id
}

#[test]
fn slot_entry_is_stable_as_newer_things_are_copied() {
    let s = open_in_memory().unwrap();
    let fav = ingest(&s, "my snippet", 1);
    s.assign_slot(2, fav).unwrap();
    // copy several newer things
    for i in 0..10 {
        ingest(&s, &format!("noise {i}"), 100 + i);
    }
    // slot 2 still points at the original favourite
    let e = s.slot_entry(2).unwrap().unwrap();
    assert_eq!(e.full_text, "my snippet");
}

#[test]
fn slotted_entry_survives_retention_because_it_is_pinned() {
    let s = open_in_memory().unwrap();
    let fav = ingest(&s, "keep me", 1);
    s.assign_slot(1, fav).unwrap(); // pins it
    ingest(&s, "throwaway", 2);

    // aggressive age cap that would otherwise delete everything
    let policy = RetentionPolicy { max_entries: None, max_age_ms: Some(0), max_image_bytes: None };
    s.enforce_retention(&policy, 1_000_000).unwrap();

    // the slotted (pinned) entry is still there and still in its slot
    assert!(s.slot_entry(1).unwrap().is_some());
    assert_eq!(s.slot_entry(1).unwrap().unwrap().full_text, "keep me");
}
```

- [ ] **Step 2: Run to verify it passes**

Run: `cargo test -p magpie-core --test slots`
Expected: PASS (2 tests).

- [ ] **Step 3: Commit**

```bash
git add crates/magpie-core/tests/slots.rs
git commit -m "test(core): slot stability + retention exemption"
```

---

### Task 4: App `resolve_slot_or_recent` helper

**Files:**
- Modify: `crates/magpie-app/src/paste_action.rs`
- Test: `crates/magpie-app/tests/slots.rs`

**Interfaces:**
- Consumes: `magpie_core::Entry`, existing `resolve_quick_paste`.
- Produces: `pub fn resolve_slot_or_recent(slot_entry: Option<Entry>, recent: &[Entry], slot: usize) -> Option<Entry>` — returns `slot_entry` when `Some`; otherwise the `slot`-th most-recent (1-based, cloned); `None` when neither applies.

- [ ] **Step 1: Write failing tests in `crates/magpie-app/tests/slots.rs`**

```rust
use magpie_app::paste_action::resolve_slot_or_recent;
use magpie_core::{Entry, Kind};

fn entry(id: i64, text: &str) -> Entry {
    Entry {
        id, content_hash: format!("h{id}"), kind: Kind::Text, preview_text: text.into(),
        full_text: text.into(), image_path: None, byte_size: 0, char_count: 0, word_count: 0,
        line_count: 0, first_copied_at_ms: 0, last_copied_at_ms: 0, copy_count: 1,
        pinned: false, source_app_id: None,
    }
}

#[test]
fn slot_entry_takes_precedence() {
    let recent = vec![entry(1, "recent1"), entry(2, "recent2")];
    let slotted = entry(99, "slotted");
    let got = resolve_slot_or_recent(Some(slotted), &recent, 1).unwrap();
    assert_eq!(got.full_text, "slotted");
}

#[test]
fn falls_back_to_nth_recent_when_slot_empty() {
    let recent = vec![entry(1, "recent1"), entry(2, "recent2")];
    let got = resolve_slot_or_recent(None, &recent, 2).unwrap();
    assert_eq!(got.full_text, "recent2");
}

#[test]
fn none_when_slot_empty_and_out_of_range() {
    let recent = vec![entry(1, "recent1")];
    assert!(resolve_slot_or_recent(None, &recent, 0).is_none());
    assert!(resolve_slot_or_recent(None, &recent, 5).is_none());
}
```

- [ ] **Step 2: Run to verify fail**

Run: `cargo test -p magpie-app --test slots`
Expected: FAIL — `resolve_slot_or_recent` not found.

- [ ] **Step 3: Implement in `paste_action.rs`** (below `resolve_quick_paste`)

```rust
/// Pick the slot entry if assigned, else the `slot`-th most-recent (1-based).
pub fn resolve_slot_or_recent(slot_entry: Option<Entry>, recent: &[Entry], slot: usize) -> Option<Entry> {
    match slot_entry {
        Some(e) => Some(e),
        None => resolve_quick_paste(recent, slot).cloned(),
    }
}
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p magpie-app --test slots`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/magpie-app/src/paste_action.rs crates/magpie-app/tests/slots.rs
git commit -m "feat(app): resolve_slot_or_recent helper"
```

---

### Task 5: Slint — `EntryRow.slot` + detail-pane assign cells

**Files:**
- Modify: `crates/magpie-app/ui/launcher.slint`

**Interfaces:**
- Produces:
  - `EntryRow` gains `slot: int` (0 = unassigned).
  - `LauncherWindow` gains `callback assign-slot(int, int);` (entry index, slot number).
  - In the list-view detail pane: a row of 9 numbered cells; the cell equal to the selected entry's `slot` is highlighted; clicking cell N calls `assign-slot(root.selected, N)`.

> UI markup renders only with a display; the gate is that it **compiles** via `slint-build`. Visual check is manual.

- [ ] **Step 1: Add `slot: int` to the `EntryRow` struct**

```slint
struct EntryRow {
    title: string,
    subtitle: string,
    kind: string,
    slot: int,
}
```

- [ ] **Step 2: Add the callback to `LauncherWindow`** (next to the other callbacks)

```slint
    callback assign-slot(int, int);
```

- [ ] **Step 3: Add the assign-cells row to the detail pane** (inside the list-view detail `Rectangle`'s `VerticalLayout`, after the `detail-text` `Text`)

```slint
                if entries.length > 0 && root.selected < entries.length: HorizontalLayout {
                    spacing: 4px;
                    alignment: start;
                    Text {
                        text: "Slot:";
                        color: #9a9aa0;
                        font-size: 11px;
                        vertical-alignment: center;
                    }
                    for n in [1, 2, 3, 4, 5, 6, 7, 8, 9]: Rectangle {
                        width: 24px;
                        height: 24px;
                        background: entries[root.selected].slot == n ? #6a8cff : #2a2a30;
                        border-radius: 4px;
                        TouchArea {
                            clicked => {
                                root.assign-slot(root.selected, n);
                            }
                        }
                        Text {
                            text: n;
                            color: white;
                            font-size: 11px;
                            horizontal-alignment: center;
                            vertical-alignment: center;
                        }
                    }
                }
```

- [ ] **Step 4: Compile (Slint codegen)**

Run: `cargo build -p magpie-app`
Expected: compiles. The generated `EntryRow` now has a `slot` field (breaks `runtime::to_rows` until Task 6 sets it — that's expected; Task 6 lands next and fixes the call).

Note: if the build fails only because `to_rows` doesn't set `.slot`, that is fine — proceed to Task 6, which updates `to_rows`. To keep this task independently green, temporarily add `slot: 0` to the `EntryRow { .. }` literal in `runtime::to_rows` in this task and finalize the real value in Task 6.

- [ ] **Step 5: Set `slot: 0` in `runtime::to_rows`** so the crate compiles now

In `crates/magpie-app/src/runtime.rs`, in `to_rows`, add `slot: 0,` to the `EntryRow { .. }` construction.

- [ ] **Step 6: Compile again**

Run: `cargo build -p magpie-app`
Expected: compiles clean.

- [ ] **Step 7: Commit**

```bash
git add crates/magpie-app/ui/launcher.slint crates/magpie-app/src/runtime.rs
git commit -m "feat(app): Slint EntryRow.slot + detail-pane assign cells"
```

---

### Task 6: Runtime wiring — annotate slots, `on_assign_slot`, slot-first quick-paste

**Files:**
- Modify: `crates/magpie-app/src/runtime.rs`

**Interfaces:**
- Consumes: `Store::{slot_map, slot_entry, assign_slot, clear_slot}`, `resolve_slot_or_recent`, existing `refresh`, `to_rows`, `spawn_hotkeys`, `current_results`.
- Produces:
  - `refresh` builds `slot_map()` into a `HashMap<i64, i64>` (entry_id → slot) and passes it to `to_rows`, which sets `EntryRow.slot`.
  - `start()` wires `ui.on_assign_slot(index, n)`: resolve the entry from current results; if it already holds slot `n`, `clear_slot(n)`, else `assign_slot(n, entry.id)`; then `refresh`.
  - `spawn_hotkeys`' `QuickPaste(slot)` arm resolves `slot_entry(slot)` first and uses `resolve_slot_or_recent`.

- [ ] **Step 1: Change `to_rows` to take a slot map and set `EntryRow.slot`**

```rust
use magpie_app::paste_action::resolve_slot_or_recent;
use std::collections::HashMap;

fn to_rows(entries: &[Entry], slots: &HashMap<i64, i64>) -> Vec<EntryRow> {
    entries
        .iter()
        .map(|e| EntryRow {
            title: SharedString::from(preview_title(e)),
            subtitle: SharedString::from(format!("{} · copied {}×", e.kind.as_str(), e.copy_count)),
            kind: SharedString::from(e.kind.as_str()),
            slot: *slots.get(&e.id).unwrap_or(&0) as i32,
        })
        .collect()
}
```

(`HashMap` may already be imported; if so, don't duplicate the `use`.)

- [ ] **Step 2: Build the slot map in `refresh`**

```rust
fn refresh(ui: &LauncherWindow, state: &AppState) {
    let results = current_results(state, now_ms());
    let slots: HashMap<i64, i64> = {
        match state.store.lock() {
            Ok(store) => store.slot_map().unwrap_or_default().into_iter().map(|(slot, eid)| (eid, slot)).collect(),
            Err(_) => HashMap::new(),
        }
    };
    let detail = results.first().map(|e| e.full_text.clone()).unwrap_or_default();
    ui.set_entries(ModelRc::new(VecModel::from(to_rows(&results, &slots))));
    ui.set_detail_text(SharedString::from(detail));
}
```

- [ ] **Step 3: Wire `on_assign_slot` in `start()`** (near the other `ui.on_*` blocks)

```rust
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_assign_slot(move |index, n| {
            let recent = current_results(&s, now_ms());
            if let Some(entry) = recent.get(index as usize) {
                if let Ok(store) = s.store.lock() {
                    let already_here = store
                        .slot_entry(n as i64)
                        .ok()
                        .flatten()
                        .map(|e| e.id)
                        == Some(entry.id);
                    if already_here {
                        let _ = store.clear_slot(n as i64);
                    } else {
                        let _ = store.assign_slot(n as i64, entry.id);
                    }
                }
            }
            if let Some(ui) = w.upgrade() {
                refresh(&ui, &s);
            }
        });
    }
```

- [ ] **Step 4: Make `QuickPaste` slot-first in `spawn_hotkeys`**

Replace the `Some(HotAction::QuickPaste(slot))` arm body with:

```rust
                    Some(HotAction::QuickPaste(slot)) => {
                        let recent = current_results(&state, now_ms());
                        let slotted = state
                            .store
                            .lock()
                            .ok()
                            .and_then(|st| st.slot_entry(*slot as i64).ok().flatten());
                        if let Some(entry) = resolve_slot_or_recent(slotted, &recent, *slot) {
                            if let Ok(mut clip) = magpie_platform::platform_clipboard() {
                                let _ = perform_paste(&mut clip, &EnigoPaster, &entry, PasteKind::Formatted, auto);
                            }
                        }
                    }
```

- [ ] **Step 5: Compile**

Run: `cargo build -p magpie-app`
Expected: compiles. (`perform_paste` takes `&Entry`; `entry` here is an owned `Entry`, so pass `&entry`.)

- [ ] **Step 6: Manual verification**

Run `cargo run -p magpie-app`; copy a snippet; open the launcher; select it; click slot cell **2** (it highlights). Copy several newer things. Press `Super+Ctrl+2` in another app and confirm the *original* snippet pastes (not the 2nd-most-recent). Click cell 2 again to clear; confirm `Super+Ctrl+2` reverts to the 2nd-most-recent. Record in your report.

- [ ] **Step 7: Commit**

```bash
git add crates/magpie-app/src/runtime.rs
git commit -m "feat(app): wire slot annotation, assign cells, slot-first quick-paste"
```

---

## Self-Review

**Spec coverage:**
- `slots` table (additive, no migration) → Task 1. ✅
- `assign_slot`/`clear_slot`/`slot_map` + one-entry-one-slot + assign-pins + validation → Task 1. ✅
- `slot_entry` → Task 2. ✅
- Slot stability across newer copies + retention exemption → Task 3. ✅
- Slot-first, recency-fallback hotkey via pure helper → Tasks 4 (helper) + 6 (wiring). ✅
- `EntryRow.slot` + detail-pane assign cells + `assign-slot` callback → Tasks 5, 6. ✅
- Assign/clear toggle from a cell → Task 6. ✅
- Deferred (noted): two-digit in-launcher select — not in any task, by design.

**Placeholder scan:** Task 5/6 UI + wiring use compile + `## Manual verification` by necessity (no display in CI); core + helper logic (1–4) is fully TDD'd. Task 5's temporary `slot: 0` is finalized in Task 6 (explicit, not a leftover). No TODOs.

**Type consistency:** `assign_slot(slot: i64, entry_id: i64)`, `clear_slot(i64)`, `slot_entry(i64) -> Option<Entry>`, `slot_map() -> Vec<(i64,i64)>`, `resolve_slot_or_recent(Option<Entry>, &[Entry], usize) -> Option<Entry>`, `EntryRow.slot: int`, `assign-slot(int,int)`, and `to_rows(&[Entry], &HashMap<i64,i64>)` are consistent across tasks. Slot map is `entry_id → slot` in the app (built by inverting `slot_map`'s `(slot, entry_id)`).

## Notes

- `slot_map()` returns `(slot, entry_id)`; the app inverts it to `entry_id → slot` for row annotation. Keep the core order `(slot, entry_id)`.
- `slot_entry` reuses `search::{row_to_entry, ENTRY_COLUMNS}` (both `pub(crate)`), prefixing columns with `e.` exactly as `search` does.
