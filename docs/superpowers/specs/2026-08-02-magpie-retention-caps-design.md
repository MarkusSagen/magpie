# Magpie Retention Caps — Design

**Date:** 2026-08-02
**Status:** Approved (brainstorm complete)
**Phase:** 1 (second sub-project)

## Summary

Opt-in retention caps that keep Magpie's database and image cache from growing
forever. Three independent, individually-optional caps — **max entry count**,
**max age**, and **max image-cache size** — enforced on startup and after each
capture. All caps are **off by default** (Magpie's promise is "nothing expires
unless you ask"), and **pinned entries are always exempt**.

## Goals & Non-Goals

**Goals**
- `RetentionPolicy` with three optional caps; any combination active at once.
- Enforcement deletes the right entries, cascades to `copy_events`, keeps the FTS
  index consistent, and reports which image files to remove.
- Enforcement runs on startup and after every successful capture (single cheap
  transaction).
- Config fields to set the caps; conversions (days→ms, MB→bytes) in the app.

**Non-Goals (v1)**
- A settings UI (config file only for now).
- Per-type or per-app caps.
- Compaction/VACUUM scheduling (SQLite auto-manages; not needed at these sizes).

## Policy

```rust
pub struct RetentionPolicy {
    pub max_entries: Option<i64>,     // keep newest N *non-pinned* entries
    pub max_age_ms: Option<i64>,      // delete non-pinned older than now - this
    pub max_image_bytes: Option<i64>, // cap total non-pinned image byte_size
}
```

- **All `None` → no-op.** Off by default.
- **Pinned entries are never deleted** by any cap, and do not count against
  `max_entries` (reads as "keep your last N clips, plus anything pinned").

## Core: `Store::enforce_retention`

`crates/magpie-core/src/retention.rs`:

```rust
pub struct Removed {
    pub entries_deleted: usize,
    pub image_paths: Vec<String>, // image_path of each deleted entry (non-null)
}

impl Store {
    pub fn enforce_retention(&self, policy: &RetentionPolicy, now_ms: i64) -> Result<Removed>;
}
```

Algorithm (single transaction):
1. **Collect victim entry ids** (a `BTreeSet<i64>`, union of the active rules):
   - **Age:** `SELECT id FROM entries WHERE pinned = 0 AND last_copied_at_ms < (now_ms - max_age_ms)`.
   - **Count:** `SELECT id FROM entries WHERE pinned = 0 AND id NOT IN (SELECT id FROM entries WHERE pinned = 0 ORDER BY last_copied_at_ms DESC LIMIT max_entries)`.
   - **Image bytes:** walk non-pinned image entries newest-first, accumulating
     `byte_size`; every entry whose running total *exceeds* `max_image_bytes` is a
     victim (i.e. keep the newest images that fit the budget, evict the rest).
2. If the victim set is empty → return `Removed { 0, [] }` (still commits a no-op
   transaction cheaply, or short-circuits).
3. **Collect `image_path`s** of victims that have one (non-null).
4. `DELETE FROM copy_events WHERE entry_id IN (victims)`.
5. `DELETE FROM entries WHERE id IN (victims)` — the existing `entries_ad` trigger
   removes each row from `entries_fts`, so search stays consistent.
6. Commit. Return `Removed { entries_deleted, image_paths }`.

Core never touches the filesystem — it returns `image_paths` for the app to
delete. Deleting an entry necessarily deletes its `copy_events` (so those copies
leave the analytics history too — an accepted consequence of opt-in retention).

## App: config → policy, file cleanup, wiring

- **Config** (`crates/magpie-app/src/config.rs`) gains three optional fields
  (all default `None`): `max_entries: Option<i64>`, `max_age_days: Option<i64>`,
  `max_image_mb: Option<i64>`.
- **`retention::policy_from_config(&Config) -> RetentionPolicy`** (new
  `crates/magpie-app/src/retention.rs`, pure + tested): `max_age_ms = days *
  86_400_000`, `max_image_bytes = mb * 1_048_576`, `None` passes through.
- **`FsImageStore::remove_paths(&[String])`** (new method): best-effort delete of
  each file plus its `<hash>.thumb.png` sibling; ignores missing files.
- **Wiring** (`runtime.rs`): build the policy once from config; a helper
  `sweep_retention(state, policy)` calls `enforce_retention(policy, now_ms())`
  and `images.remove_paths(removed.image_paths)`. Call it on **startup** (after
  `build_state`) and in **`spawn_watcher`** after each successful `ingest_event`
  (before the UI refresh). Errors are logged and swallowed — never crash capture.

## Data Flow

```
startup:           build_state -> sweep_retention -> (UI)
after each copy:   ingest_event -> sweep_retention -> refresh(UI)
sweep_retention:   enforce_retention(policy, now)  [core, one txn]
                     -> Removed{ entries_deleted, image_paths }
                   images.remove_paths(image_paths) [app, fs]
```

## Error Handling

- `enforce_retention` runs in a transaction; any SQL error rolls back and
  propagates a `Result::Err` — the caller logs and continues.
- `remove_paths` is best-effort: missing files are ignored; other IO errors are
  logged, not fatal.
- A policy with a cap set to `Some(0)` is valid: `max_entries: Some(0)` keeps only
  pinned; `max_age_ms: Some(0)` deletes everything non-pinned; `max_image_bytes:
  Some(0)` evicts all non-pinned images. (Documented, not special-cased.)

## Testing

**core (`retention.rs` unit + `tests/retention.rs` integration):**
- Age: deletes non-pinned older than cutoff; keeps pinned old + recent non-pinned.
- Count: keeps newest N non-pinned + all pinned; deletes the rest.
- Image bytes: evicts oldest non-pinned image entries until total ≤ budget; keeps
  pinned images regardless.
- Cascade: a deleted entry's `copy_events` are gone; the entry no longer appears
  in `search`/`recent` (FTS trigger fired); returned `image_paths` match victims'
  paths (and exclude non-image/null-path entries).
- Combined policy (all three) unions victims correctly.
- All-`None` policy is a no-op (nothing deleted).

**app (`tests/retention.rs`):**
- `policy_from_config`: days→ms, MB→bytes, `None` passthrough, and mixed.

**UI/wiring:** manual (no display in CI) — startup + after-capture sweep verified
by running the app with a tiny cap and confirming old entries disappear.

## File Structure

- Create: `crates/magpie-core/src/retention.rs` (+ `RetentionPolicy`, `Removed`, `Store::enforce_retention`).
- Modify: `crates/magpie-core/src/lib.rs` (`pub mod retention;` + re-exports).
- Create: `crates/magpie-core/tests/retention.rs`.
- Create: `crates/magpie-app/src/retention.rs` (`policy_from_config`).
- Modify: `crates/magpie-app/src/config.rs` (three optional fields).
- Modify: `crates/magpie-app/src/image_cache.rs` (`remove_paths`).
- Modify: `crates/magpie-app/src/lib.rs` (`pub mod retention;`).
- Modify: `crates/magpie-app/src/runtime.rs` (build policy, `sweep_retention`, wire startup + watcher).
- Create: `crates/magpie-app/tests/retention.rs`.

## Risks / Notes

- **Analytics interaction:** retention deletes `copy_events` for pruned entries, so
  over-time/most-copied lose that history. This is expected for opt-in retention
  and documented; users who want full analytics leave retention off.
- **Count query cost:** the `NOT IN (… LIMIT N)` subquery is fine at these sizes;
  `idx_entries_last_copied` supports the ordering.
