# Magpie Merge-Multiple — Design

**Date:** 2026-08-02 · **Status:** Approved (standing delegation) · **Phase:** 1

## Summary

Select several entries and paste their combined text, joined by a chosen
separator. Core provides `merged_text`; the launcher adds a lightweight "merge
tray" (toggle rows in/out, pick a separator, Merge & paste).

## Core

`crates/magpie-core/src/merge.rs`:
```rust
impl Store {
    /// Concatenate the `full_text` of the given entry ids, in order, joined by
    /// `separator`. Missing ids are skipped.
    pub fn merged_text(&self, ids: &[i64], separator: &str) -> Result<String>;
}
```
Reads each entry's `full_text` via `SELECT ... WHERE id = ?` (`.optional()`),
`join`s with the separator. No wall clock. No schema change.

## App

- Window state: `in property <[int]> merge-set;` (entry indices currently queued)
  and `in-out property <int> merge-sep;` (0 = newline, 1 = space, 2 = ", ").
- Each list row gets a small toggle cell: clicking it calls `toggle-merge(int)`
  (add/remove the index). Queued rows are visually marked.
- A merge bar (shown when `merge-set` is non-empty): a separator selector
  (Newline / Space / Comma), a **Merge & paste** button → `merge-paste()`, and a
  **Clear** button → `clear-merge()`.
- Runtime: `merge_set: Vec<i32>` lives in `AppState` (a `Mutex<Vec<i32>>`), mirrored
  to the `merge-set` property. `merge-paste` resolves indices → entry ids from the
  current results, calls `merged_text(ids, sep_str)`, sets the clipboard, and
  pastes (reuses `perform_paste` with a synthetic text entry, or sets clipboard +
  `EnigoPaster`). Then clears the set and refreshes.

Separator mapping: `0 -> "\n"`, `1 -> " "`, `2 -> ", "`.

## Testing

- **core:** `merged_text` joins in order with the separator; skips missing ids;
  empty ids → empty string; single id → that text.
- **app:** a pure `separator_str(i32) -> &'static str` helper (0/1/2/other→newline),
  unit-tested.
- **run-verify:** launch the binary (exit 124), toggle two rows, Merge & paste.

## Files
- Create: `crates/magpie-core/src/merge.rs`; modify `lib.rs`; create `tests/merge.rs`.
- Create: `crates/magpie-app/src/merge_view.rs` (`separator_str`).
- Modify: `crates/magpie-app/src/lib.rs`, `app_state.rs` (merge_set), `ui/launcher.slint`, `runtime.rs`.

## Risks
- Multi-select adds a per-row toggle; kept to one small cell to keep rows light.
- `AppState` gains `merge_set: Mutex<Vec<i32>>` — sits beside `store`/`ui`.
