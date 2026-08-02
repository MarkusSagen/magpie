# Magpie Pinned Quick-Paste Slots — Design

**Date:** 2026-08-02
**Status:** Approved (brainstorm complete)
**Phase:** 1 (third sub-project)

## Summary

Persistent numbered slots (1–9): assign a specific clipboard entry to a slot and
`Super+Ctrl+N` always pastes it, regardless of recency. When a slot is empty, the
hotkey falls back to the Nth most-recent entry (today's behavior), so it's
backwards-compatible. Assignment is a click on a numbered cell in the launcher's
detail pane. Slots are a numbered kind of favorite: assigning one keeps the entry
(pins it, so retention never prunes it).

## Goals & Non-Goals

**Goals**
- A `slots` table mapping slot 1–9 → entry (one entry per slot, one slot per entry).
- `Super+Ctrl+N` pastes the slot-N entry if assigned, else the Nth most-recent.
- Assign / clear a slot from the launcher by clicking a numbered cell.
- Assigning a slot pins the entry (retention-exempt).

**Non-Goals (v1)**
- Two-digit in-launcher select for histories > 9 (separate nav feature; deferred).
- Keyboard-chord assignment (click-to-assign only in v1).
- Reordering slots by drag; slot labels/names.

## Data Model

New table, added to `crates/magpie-core/src/schema.sql`. Because the schema runs
via `CREATE TABLE IF NOT EXISTS` on every `Store::open`, existing databases pick
it up with **no migration**:

```sql
CREATE TABLE IF NOT EXISTS slots (
  slot     INTEGER PRIMARY KEY CHECK(slot BETWEEN 1 AND 9),
  entry_id INTEGER NOT NULL
);
```

Invariants:
- **One entry per slot** — `slot` is the primary key.
- **One slot per entry** — `assign_slot` removes the entry from any prior slot
  before setting the new one.
- **Assigning pins** — `assign_slot` sets the entry's `pinned = 1`, so retention
  (which exempts pinned) never deletes a slotted entry. `clear_slot` leaves
  `pinned` as-is (the user can unpin separately).

No foreign key: since slotted entries are pinned, retention never deletes them, so
there are no dangling rows in practice; and `slot_entry` uses an INNER JOIN, so a
slot whose entry is somehow gone reads as empty rather than erroring.

## Core API (`magpie_core::slots`, methods on `Store`)

```rust
impl Store {
    /// Assign `entry_id` to `slot` (1..=9): move it out of any prior slot, upsert
    /// slot->entry, and pin the entry. Err on out-of-range slot (no write).
    pub fn assign_slot(&self, slot: i64, entry_id: i64) -> Result<()>;

    /// Remove any assignment for `slot`. No-op if empty. Err on out-of-range slot.
    pub fn clear_slot(&self, slot: i64) -> Result<()>;

    /// The entry currently in `slot`, or None if empty / entry missing.
    pub fn slot_entry(&self, slot: i64) -> Result<Option<Entry>>;

    /// All assignments as (slot, entry_id) pairs (for annotating the list).
    pub fn slot_map(&self) -> Result<Vec<(i64, i64)>>;
}
```

- Out-of-range validation: a guard `slot < 1 || slot > 9` returns an `Err` before
  any SQL runs (constructed as `rusqlite::Error::ToSqlConversionFailure` wrapping a
  small message error). Tests assert `is_err()` and that no row was written.
- `assign_slot` and `clear_slot` run in a single `unchecked_transaction`.

## Hotkey Behavior (slot-first, recency fallback)

`Super+Ctrl+N` quick-paste, in `runtime::spawn_hotkeys`:
1. Look up `slot_entry(N)`.
2. Pure helper `resolve_slot_or_recent(slot_entry, &recent, N) -> Option<&Entry>`:
   returns the slot entry when present; otherwise `resolve_quick_paste(recent, N)`;
   `None` when N is out of range and no slot entry.
3. Paste the resolved entry (set clipboard + auto-paste as today).

The helper is pure and unit-tested; the store lookups stay in `runtime`.

## UI (Slint)

- `EntryRow` gains `slot: int` (0 = unassigned, 1–9 = its slot). `runtime::refresh`
  builds `slot_map()` once into a `HashMap<entry_id, i64>` and annotates each row.
- The detail pane gains a **row of 9 numbered cells**. The cell whose number
  equals the selected entry's `slot` is highlighted (shows where this entry is
  pinned). Clicking cell N fires `assign-slot(entry_index, slot_number)`.
- `runtime` `on_assign_slot(index, n)`: resolve the entry from the current
  results; if that entry's current slot is already `n`, `clear_slot(n)`, else
  `assign_slot(n, entry_id)`; then `refresh` (re-annotates slots, re-pins).

## Data Flow

```
assign (click cell N on selected entry):
  assign-slot(index, N) -> resolve entry
    -> entry.slot == N ? clear_slot(N) : assign_slot(N, entry_id)  [core, 1 txn]
    -> refresh (slot_map -> EntryRow.slot; entry now pinned)

quick-paste (Super+Ctrl+N):
  slot_entry(N)  [core]
  resolve_slot_or_recent(slot_entry, recent, N)  [pure]
  perform_paste(entry)  [existing]
```

## Error Handling

- Out-of-range slot → `Err`, no write (guarded before any SQL).
- `assign_slot`/`clear_slot` transactional; SQL errors roll back and propagate.
- In `runtime`, slot lookups that error are treated as "no slot" (fall back to
  recency / skip), never crashing capture or paste.

## Testing

**core (`slots.rs` unit + `tests/slots.rs`):**
- `assign_slot` inserts the mapping and pins the entry.
- Reassigning an entry to a new slot moves it (leaves no stale slot for it).
- Assigning a different entry to an occupied slot replaces the occupant.
- `clear_slot` removes the mapping (idempotent when empty).
- `slot_entry`: Some for an assigned slot, None for empty, None when the entry was
  deleted out from under a (hypothetical) unpinned slot.
- `slot_map` returns all pairs.
- Invalid slot (0, 10) → `Err`, and no row written.

**app (`tests/slots.rs`):**
- `resolve_slot_or_recent`: slot present → slot entry; slot empty + valid N →
  Nth recent; out-of-range N + no slot → None.

**UI/wiring:** manual — assign via a cell, confirm the number highlights, and the
hotkey pastes the slot entry even after copying newer things.

## File Structure

- Modify: `crates/magpie-core/src/schema.sql` (add `slots` table).
- Create: `crates/magpie-core/src/slots.rs` (`assign_slot`/`clear_slot`/`slot_entry`/`slot_map`).
- Modify: `crates/magpie-core/src/lib.rs` (`pub mod slots;`).
- Create: `crates/magpie-core/tests/slots.rs`.
- Modify: `crates/magpie-app/src/paste_action.rs` (add `resolve_slot_or_recent`).
- Modify: `crates/magpie-app/tests/` (new `slots.rs` or extend) for the helper.
- Modify: `crates/magpie-app/ui/launcher.slint` (`EntryRow.slot`, detail-pane cells, `assign-slot` callback).
- Modify: `crates/magpie-app/src/runtime.rs` (annotate rows, `on_assign_slot`, slot-first quick-paste).

## Risks / Notes

- **First schema addition:** additive `CREATE TABLE IF NOT EXISTS`, no migration
  framework needed; existing DBs get the table on next open.
- **Selected-entry slot in UI:** derived from `EntryRow.slot` of the selected row
  (no separate selection→slot query), keeping the UI reactive without extra
  round-trips.
