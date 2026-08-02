# Magpie Retention Caps Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add opt-in retention caps (max entry count, max age, max image-cache size) that prune the database and image cache on startup and after each capture, keeping pinned entries always exempt.

**Architecture:** A new `magpie_core::retention` module runs one transactional deletion (`Store::enforce_retention`) that unions the three caps' victim sets, cascades to `copy_events`, lets the existing FTS trigger clean the index, and returns the deleted entries' image paths. The app builds a `RetentionPolicy` from config, deletes the returned image files, and calls the sweep on startup + after each capture.

**Tech Stack:** Rust, `rusqlite` (existing). No new dependencies.

## Global Constraints

- **No schema changes** — deletes over existing `entries`/`copy_events`; the existing `entries_ad` FTS trigger keeps `entries_fts` consistent.
- **Off by default:** all-`None` `RetentionPolicy` is a no-op.
- **Pinned entries are never deleted** by any cap and do not count toward `max_entries`.
- **Core stays filesystem-free:** `enforce_retention` returns `image_paths`; the app deletes the files.
- **Determinism:** `now_ms: i64` is injected; core never calls the wall clock.
- **Deletion order:** delete `copy_events` for victims first, then `entries` (foreign_keys pragma is ON).
- **Constants:** day = `86_400_000` ms; MB = `1_048_576` bytes.
- **Commit style:** conventional commits, one per task.

---

### Task 1: `retention` module — policy, `Removed`, age + count enforcement

**Files:**
- Create: `crates/magpie-core/src/retention.rs`
- Modify: `crates/magpie-core/src/lib.rs` (`pub mod retention;` + re-exports)
- Test: `retention.rs` tests module.

**Interfaces:**
- Consumes: `Store`, `Result`; (tests) `open_in_memory`, `CaptureEvent`, `Content`, `ImageStore`.
- Produces:
  - `#[derive(Debug, Clone, Copy)] pub struct RetentionPolicy { pub max_entries: Option<i64>, pub max_age_ms: Option<i64>, pub max_image_bytes: Option<i64> }` with `pub fn is_noop(&self) -> bool`.
  - `pub struct Removed { pub entries_deleted: usize, pub image_paths: Vec<String> }`
  - `impl Store { pub fn enforce_retention(&self, policy: &RetentionPolicy, now_ms: i64) -> Result<Removed> }` implementing **age** + **count** rules (image-bytes added in Task 2) + the shared cascade delete. Pinned (`pinned = 1`) rows are never victims.
  - `lib.rs` re-exports: `pub use retention::{RetentionPolicy, Removed};`

- [ ] **Step 1: Write failing tests in `retention.rs`**

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
    fn text(s: &str, ms: i64) -> CaptureEvent {
        CaptureEvent { content: Content::Text(s.into()), source_app: None, copied_at_ms: ms }
    }
    fn seed(store: &Store, evs: &[CaptureEvent]) {
        for e in evs { store.ingest(e, &Noop).unwrap(); }
    }
    fn texts(store: &Store) -> Vec<String> {
        store.recent(100).unwrap().into_iter().map(|e| e.full_text).collect()
    }

    #[test]
    fn noop_policy_deletes_nothing() {
        let s = open_in_memory().unwrap();
        seed(&s, &[text("a", 1), text("b", 2)]);
        let r = s.enforce_retention(&RetentionPolicy { max_entries: None, max_age_ms: None, max_image_bytes: None }, 1000).unwrap();
        assert_eq!(r.entries_deleted, 0);
        assert_eq!(texts(&s).len(), 2);
    }

    #[test]
    fn age_deletes_old_but_keeps_pinned_and_recent() {
        let s = open_in_memory().unwrap();
        let old = s.ingest(&text("old", 100), &Noop).unwrap();
        let old_pinned = s.ingest(&text("old-pinned", 200), &Noop).unwrap();
        s.ingest(&text("recent", 5_000), &Noop).unwrap();
        s.set_pinned(old_pinned.entry_id, true).unwrap();

        // cutoff = now(10_000) - age(1_000) = 9_000 -> "old" (100) and "old-pinned" (200) are old
        let policy = RetentionPolicy { max_entries: None, max_age_ms: Some(1_000), max_image_bytes: None };
        let r = s.enforce_retention(&policy, 10_000).unwrap();
        assert_eq!(r.entries_deleted, 1); // only unpinned "old"
        let remaining = texts(&s);
        assert!(remaining.contains(&"recent".to_string()));
        assert!(remaining.contains(&"old-pinned".to_string()));
        assert!(!remaining.contains(&"old".to_string()));
        let _ = old;
    }

    #[test]
    fn count_keeps_newest_n_unpinned_plus_pinned() {
        let s = open_in_memory().unwrap();
        let a = s.ingest(&text("a", 1), &Noop).unwrap(); // oldest
        s.ingest(&text("b", 2), &Noop).unwrap();
        s.ingest(&text("c", 3), &Noop).unwrap();
        s.ingest(&text("d", 4), &Noop).unwrap(); // newest
        s.set_pinned(a.entry_id, true).unwrap(); // pin the oldest

        // keep newest 2 unpinned (c,d) + pinned a; delete b
        let policy = RetentionPolicy { max_entries: Some(2), max_age_ms: None, max_image_bytes: None };
        let r = s.enforce_retention(&policy, 100).unwrap();
        assert_eq!(r.entries_deleted, 1);
        let remaining = texts(&s);
        assert_eq!(remaining.len(), 3);
        assert!(remaining.contains(&"a".to_string())); // pinned
        assert!(remaining.contains(&"c".to_string()));
        assert!(remaining.contains(&"d".to_string()));
        assert!(!remaining.contains(&"b".to_string()));
    }
}
```

- [ ] **Step 2: Add `pub mod retention;` to `lib.rs`; run to verify fail**

Run: `cargo test -p magpie-core --lib retention::`
Expected: FAIL — items not found.

- [ ] **Step 3: Implement `retention.rs`**

```rust
use crate::store::{Result, Store};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy)]
pub struct RetentionPolicy {
    pub max_entries: Option<i64>,
    pub max_age_ms: Option<i64>,
    pub max_image_bytes: Option<i64>,
}

impl RetentionPolicy {
    pub fn is_noop(&self) -> bool {
        self.max_entries.is_none() && self.max_age_ms.is_none() && self.max_image_bytes.is_none()
    }
}

pub struct Removed {
    pub entries_deleted: usize,
    pub image_paths: Vec<String>,
}

impl Store {
    pub fn enforce_retention(&self, policy: &RetentionPolicy, now_ms: i64) -> Result<Removed> {
        if policy.is_noop() {
            return Ok(Removed { entries_deleted: 0, image_paths: Vec::new() });
        }
        let mut victims: BTreeSet<i64> = BTreeSet::new();

        if let Some(age) = policy.max_age_ms {
            let cutoff = now_ms - age;
            let mut stmt = self
                .conn()
                .prepare("SELECT id FROM entries WHERE pinned = 0 AND last_copied_at_ms < ?1")?;
            let ids = stmt.query_map([cutoff], |r| r.get::<_, i64>(0))?;
            for id in ids {
                victims.insert(id?);
            }
        }

        if let Some(n) = policy.max_entries {
            let mut stmt = self.conn().prepare(
                "SELECT id FROM entries WHERE pinned = 0 AND id NOT IN
                   (SELECT id FROM entries WHERE pinned = 0
                    ORDER BY last_copied_at_ms DESC LIMIT ?1)",
            )?;
            let ids = stmt.query_map([n], |r| r.get::<_, i64>(0))?;
            for id in ids {
                victims.insert(id?);
            }
        }

        self.delete_entries(victims)
    }

    /// Delete a victim set: collect image paths, remove copy_events then entries
    /// (FTS trigger cleans the index), all in one transaction.
    fn delete_entries(&self, victims: BTreeSet<i64>) -> Result<Removed> {
        if victims.is_empty() {
            return Ok(Removed { entries_deleted: 0, image_paths: Vec::new() });
        }
        let ids: Vec<i64> = victims.into_iter().collect();
        let placeholders = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");

        let mut stmt = self.conn().prepare(&format!(
            "SELECT image_path FROM entries WHERE id IN ({placeholders}) AND image_path IS NOT NULL"
        ))?;
        let image_paths: Vec<String> = stmt
            .query_map(rusqlite::params_from_iter(ids.iter()), |r| r.get::<_, String>(0))?
            .collect::<Result<Vec<String>>>()?;

        let tx = self.conn().unchecked_transaction()?;
        tx.execute(
            &format!("DELETE FROM copy_events WHERE entry_id IN ({placeholders})"),
            rusqlite::params_from_iter(ids.iter()),
        )?;
        let deleted = tx.execute(
            &format!("DELETE FROM entries WHERE id IN ({placeholders})"),
            rusqlite::params_from_iter(ids.iter()),
        )?;
        tx.commit()?;

        Ok(Removed { entries_deleted: deleted, image_paths })
    }
}
```

- [ ] **Step 4: Add re-exports to `lib.rs`; run to verify pass**

Run: `cargo test -p magpie-core --lib retention::`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/magpie-core/src/retention.rs crates/magpie-core/src/lib.rs
git commit -m "feat(core): retention age + count caps with transactional cascade"
```

---

### Task 2: image-bytes cap

**Files:**
- Modify: `crates/magpie-core/src/retention.rs`
- Test: `retention.rs` tests module.

**Interfaces:**
- Consumes: everything from Task 1.
- Produces: `enforce_retention` also honors `max_image_bytes` — walk non-pinned `kind = 'image'` entries newest-first, accumulating `byte_size`; every entry whose *running total exceeds* the budget is a victim (keep the newest images that fit).

- [ ] **Step 1: Add failing test to `retention.rs` tests module**

```rust
    fn image(bytes: Vec<u8>, ms: i64) -> CaptureEvent {
        CaptureEvent { content: Content::Image { bytes }, source_app: None, copied_at_ms: ms }
    }

    #[test]
    fn image_bytes_evicts_oldest_images_over_budget() {
        let s = open_in_memory().unwrap();
        // three 100-byte images; budget 250 -> keep newest 2 (200 <= 250), evict oldest
        s.ingest(&image(vec![0u8; 100], 1), &Noop).unwrap(); // oldest
        s.ingest(&image(vec![1u8; 100], 2), &Noop).unwrap();
        let pinned_old = s.ingest(&image(vec![2u8; 100], 3), &Noop).unwrap();
        s.ingest(&image(vec![3u8; 100], 4), &Noop).unwrap(); // newest

        // pin the ms=3 image: it must survive regardless of budget
        s.set_pinned(pinned_old.entry_id, true).unwrap();

        let policy = RetentionPolicy { max_entries: None, max_age_ms: None, max_image_bytes: Some(150) };
        let r = s.enforce_retention(&policy, 1000).unwrap();
        // non-pinned images newest-first: ms4(100 ok), ms2(200 > 150 evict), ms1(evict)
        assert_eq!(r.entries_deleted, 2);
        assert_eq!(r.image_paths.len(), 2); // both had image_path
        let remaining = s.recent(100).unwrap().len();
        assert_eq!(remaining, 2); // ms4 (fits) + pinned ms3
    }
```

- [ ] **Step 2: Run to verify fail**

Run: `cargo test -p magpie-core --lib retention::tests::image_bytes`
Expected: FAIL — image cap not yet applied.

- [ ] **Step 3: Add the image-bytes block to `enforce_retention`** (after the count block, before `self.delete_entries(victims)`)

```rust
        if let Some(budget) = policy.max_image_bytes {
            let mut stmt = self.conn().prepare(
                "SELECT id, byte_size FROM entries
                 WHERE pinned = 0 AND kind = 'image'
                 ORDER BY last_copied_at_ms DESC",
            )?;
            let rows: Vec<(i64, i64)> = stmt
                .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))?
                .collect::<Result<Vec<(i64, i64)>>>()?;
            let mut running: i64 = 0;
            for (id, size) in rows {
                running += size;
                if running > budget {
                    victims.insert(id);
                }
            }
        }
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p magpie-core --lib retention::`
Expected: PASS (4 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/magpie-core/src/retention.rs
git commit -m "feat(core): retention max-image-bytes cap"
```

---

### Task 3: Core integration tests (cascade + search + combined)

**Files:**
- Create: `crates/magpie-core/tests/retention.rs`

**Interfaces:**
- Consumes: public `Store::enforce_retention`, `RetentionPolicy`, `open_in_memory`, `default_query`.
- Produces: integration coverage that deletion truly cascades (search/recent + copy_events) and combined policies union correctly.

- [ ] **Step 1: Write the integration test file**

```rust
//! Integration tests for retention enforcement.

use magpie_core::{
    default_query, open_in_memory, CaptureEvent, Content, ImageStore, RetentionPolicy,
};

struct Noop;
impl ImageStore for Noop {
    fn put(&self, h: &str, _b: &[u8]) -> std::io::Result<String> { Ok(h.to_string()) }
}
fn text(s: &str, ms: i64) -> CaptureEvent {
    CaptureEvent { content: Content::Text(s.into()), source_app: None, copied_at_ms: ms }
}

#[test]
fn deletion_cascades_to_copy_events_and_search() {
    let s = open_in_memory().unwrap();
    // "gone" copied twice (2 copy_events), "kept" once
    s.ingest(&text("gone", 1), &Noop).unwrap();
    s.ingest(&text("gone", 2), &Noop).unwrap();
    s.ingest(&text("kept", 9_000), &Noop).unwrap();

    let policy = RetentionPolicy { max_entries: None, max_age_ms: Some(1_000), max_image_bytes: None };
    let r = s.enforce_retention(&policy, 10_000).unwrap();
    assert_eq!(r.entries_deleted, 1);

    // no longer in recent
    let remaining: Vec<String> = s.recent(100).unwrap().into_iter().map(|e| e.full_text).collect();
    assert_eq!(remaining, vec!["kept".to_string()]);

    // FTS search no longer finds it
    let mut q = default_query();
    q.text = "gone".into();
    assert_eq!(s.search(&q).unwrap().len(), 0);

    // and searching "kept" still works (index consistent)
    q.text = "kept".into();
    assert_eq!(s.search(&q).unwrap().len(), 1);
}

#[test]
fn combined_policy_unions_victims() {
    let s = open_in_memory().unwrap();
    // old (age victim), and many recent (count victims)
    s.ingest(&text("old", 1), &Noop).unwrap();
    for i in 0..5 {
        s.ingest(&text(&format!("r{i}"), 10_000 + i), &Noop).unwrap();
    }
    // age deletes "old"; count keeps newest 2 of the recents -> deletes 3 recents
    let policy = RetentionPolicy { max_entries: Some(2), max_age_ms: Some(1_000), max_image_bytes: None };
    let r = s.enforce_retention(&policy, 20_000).unwrap();
    assert_eq!(r.entries_deleted, 4); // old + 3 recents
    assert_eq!(s.recent(100).unwrap().len(), 2);
}
```

- [ ] **Step 2: Run to verify it passes**

Run: `cargo test -p magpie-core --test retention`
Expected: PASS (2 tests).

- [ ] **Step 3: Commit**

```bash
git add crates/magpie-core/tests/retention.rs
git commit -m "test(core): retention cascade + search consistency + combined policy"
```

---

### Task 4: App config fields + `policy_from_config`

**Files:**
- Modify: `crates/magpie-app/src/config.rs`
- Create: `crates/magpie-app/src/retention.rs`
- Modify: `crates/magpie-app/src/lib.rs` (`pub mod retention;`)
- Test: `crates/magpie-app/tests/retention.rs`

**Interfaces:**
- Consumes: `Config`, `magpie_core::RetentionPolicy`.
- Produces:
  - `Config` gains `#[serde(default)] pub max_entries: Option<i64>`, `#[serde(default)] pub max_age_days: Option<i64>`, `#[serde(default)] pub max_image_mb: Option<i64>` (all default `None`; `Default` impl sets them `None`).
  - `pub fn policy_from_config(cfg: &Config) -> RetentionPolicy` — `max_age_ms = days * 86_400_000`, `max_image_bytes = mb * 1_048_576`, `None` passthrough.

- [ ] **Step 1: Write failing tests in `crates/magpie-app/tests/retention.rs`**

```rust
use magpie_app::config::Config;
use magpie_app::retention::policy_from_config;

#[test]
fn defaults_are_all_none() {
    let p = policy_from_config(&Config::default());
    assert!(p.max_entries.is_none() && p.max_age_ms.is_none() && p.max_image_bytes.is_none());
    assert!(p.is_noop());
}

#[test]
fn conversions_days_and_mb() {
    let cfg = Config {
        max_entries: Some(500),
        max_age_days: Some(30),
        max_image_mb: Some(200),
        ..Config::default()
    };
    let p = policy_from_config(&cfg);
    assert_eq!(p.max_entries, Some(500));
    assert_eq!(p.max_age_ms, Some(30 * 86_400_000));
    assert_eq!(p.max_image_bytes, Some(200 * 1_048_576));
}

#[test]
fn partial_config_toml_loads_with_retention_defaults() {
    // An older config.toml without retention fields still parses.
    let dir = std::env::temp_dir().join(format!("magpie-ret-cfg-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("config.toml");
    std::fs::write(&path, "launcher_hotkey = \"super+ctrl+v\"\nquick_paste_hotkeys = []\npaste_on_select = true\napp_denylist = []\n").unwrap();
    let cfg = magpie_app::config::load_or_default(&path);
    assert!(cfg.max_entries.is_none());
    std::fs::remove_dir_all(&dir).ok();
}
```

- [ ] **Step 2: Add the fields + module; run to verify fail**

Run: `cargo test -p magpie-app --test retention`
Expected: FAIL — fields/module not found.

- [ ] **Step 3: Add fields to `Config`** (in `config.rs`, both the struct and `Default`)

```rust
// add to the Config struct:
    #[serde(default)]
    pub max_entries: Option<i64>,
    #[serde(default)]
    pub max_age_days: Option<i64>,
    #[serde(default)]
    pub max_image_mb: Option<i64>,
```

```rust
// add to impl Default for Config { fn default() -> Self { Config { ... } } }:
            max_entries: None,
            max_age_days: None,
            max_image_mb: None,
```

- [ ] **Step 4: Implement `crates/magpie-app/src/retention.rs`**

```rust
use crate::config::Config;
use magpie_core::RetentionPolicy;

const DAY_MS: i64 = 86_400_000;
const MB: i64 = 1_048_576;

pub fn policy_from_config(cfg: &Config) -> RetentionPolicy {
    RetentionPolicy {
        max_entries: cfg.max_entries,
        max_age_ms: cfg.max_age_days.map(|d| d * DAY_MS),
        max_image_bytes: cfg.max_image_mb.map(|m| m * MB),
    }
}
```

Add `pub mod retention;` to `crates/magpie-app/src/lib.rs`.

- [ ] **Step 5: Run to verify pass**

Run: `cargo test -p magpie-app --test retention`
Expected: PASS (3 tests).

- [ ] **Step 6: Commit**

```bash
git add crates/magpie-app/src/config.rs crates/magpie-app/src/retention.rs crates/magpie-app/src/lib.rs crates/magpie-app/tests/retention.rs
git commit -m "feat(app): retention config fields + policy_from_config"
```

---

### Task 5: `FsImageStore::remove_paths`

**Files:**
- Modify: `crates/magpie-app/src/image_cache.rs`
- Test: `image_cache.rs` tests module.

**Interfaces:**
- Consumes: `FsImageStore`.
- Produces: `impl FsImageStore { pub fn remove_paths(&self, paths: &[String]) }` — best-effort delete of each `<hash>.bin` plus its `<hash>.thumb.png` sibling; missing files ignored.

- [ ] **Step 1: Add failing test to `image_cache.rs` tests module**

```rust
    #[test]
    fn remove_paths_deletes_bin_and_thumbnail_and_ignores_missing() {
        let dir = std::env::temp_dir().join(format!("magpie-rm-{}", std::process::id()));
        let store = FsImageStore { dir: dir.clone() };
        let bin = store.put("abcd", &[1, 2, 3]).unwrap();
        let rgba = vec![255, 0, 0, 255];
        let thumb = store.write_thumbnail("abcd", 1, 1, &rgba, 8).unwrap();
        assert!(std::path::Path::new(&bin).exists());
        assert!(std::path::Path::new(&thumb).exists());

        store.remove_paths(&[bin.clone(), "/no/such/file.bin".to_string()]);
        assert!(!std::path::Path::new(&bin).exists());
        assert!(!std::path::Path::new(&thumb).exists()); // sibling removed too
        std::fs::remove_dir_all(&dir).ok();
    }
```

- [ ] **Step 2: Run to verify fail**

Run: `cargo test -p magpie-app --lib image_cache::tests::remove_paths`
Expected: FAIL — `remove_paths` not found.

- [ ] **Step 3: Implement `remove_paths` in `impl FsImageStore`**

```rust
    /// Best-effort removal of each image file plus its thumbnail sibling.
    pub fn remove_paths(&self, paths: &[String]) {
        for p in paths {
            let _ = std::fs::remove_file(p);
            if let Some(stem) = p.strip_suffix(".bin") {
                let _ = std::fs::remove_file(format!("{stem}.thumb.png"));
            }
        }
    }
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p magpie-app --lib image_cache::tests::remove_paths`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/magpie-app/src/image_cache.rs
git commit -m "feat(app): FsImageStore::remove_paths (bin + thumbnail)"
```

---

### Task 6: Runtime wiring — `sweep_retention` on startup + after capture

**Files:**
- Modify: `crates/magpie-app/src/runtime.rs`

**Interfaces:**
- Consumes: `magpie_core::RetentionPolicy`, `Store::enforce_retention`, `retention::policy_from_config`, `FsImageStore::remove_paths`, existing `AppState`/`now_ms`/`spawn_watcher`.
- Produces:
  - `fn sweep_retention(state: &AppState, policy: &RetentionPolicy)` — no-op when `policy.is_noop()`; else lock the store, `enforce_retention(policy, now_ms())`, drop the lock, then `state.images.remove_paths(&removed.image_paths)`. Errors logged and swallowed.
  - `start()` builds `let policy = policy_from_config(&cfg);`, calls `sweep_retention(&state, &policy)` once after `build_state`/before `ui.run()`, and passes `policy` into `spawn_watcher`.
  - `spawn_watcher(state, denylist, policy, weak)` gains a `policy: RetentionPolicy` param and calls `sweep_retention(&state, &policy)` after a successful `ingest_event`, before the UI refresh.

> Runtime touches the live event loop / filesystem; this task is **manual-verify** for the end-to-end behavior. The `sweep_retention` helper logic is simple and covered indirectly by the core tests.

- [ ] **Step 1: Add `sweep_retention` to `runtime.rs`**

```rust
use magpie_app::retention::policy_from_config;
use magpie_core::RetentionPolicy;

fn sweep_retention(state: &AppState, policy: &RetentionPolicy) {
    if policy.is_noop() {
        return;
    }
    let removed = {
        let store = match state.store.lock() {
            Ok(s) => s,
            Err(_) => return,
        };
        match store.enforce_retention(policy, now_ms()) {
            Ok(r) => r,
            Err(_) => return,
        }
    };
    state.images.remove_paths(&removed.image_paths);
}
```

- [ ] **Step 2: Thread `policy` through `spawn_watcher`**

Change the signature and the post-ingest block:

```rust
fn spawn_watcher(
    state: Arc<AppState>,
    denylist: Vec<String>,
    policy: RetentionPolicy,
    weak: slint::Weak<LauncherWindow>,
) {
    std::thread::spawn(move || {
        // ... existing setup ...
        loop {
            if let Some(ev) = watcher.poll_once(now_ms()) {
                if ingest_event(&state, &ev).is_ok() {
                    sweep_retention(&state, &policy);
                    let w = weak.clone();
                    let s = state.clone();
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(ui) = w.upgrade() {
                            refresh(&ui, &s);
                        }
                    });
                }
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    });
}
```

- [ ] **Step 3: Wire `start()`** — build the policy, sweep on startup, pass to watcher

```rust
    // after `let state = build_state();` and `let cfg = ...` are available:
    let policy = policy_from_config(&cfg);
    sweep_retention(&state, &policy);
    // ... update the spawn_watcher call:
    spawn_watcher(state.clone(), denylist, policy, weak.clone());
```

- [ ] **Step 4: Compile**

Run: `cargo build -p magpie-app`
Expected: compiles. `RetentionPolicy` is `Copy`, so passing `policy` into `spawn_watcher` and reusing it for the startup sweep both work.

- [ ] **Step 5: Manual verification**

Set `~/Library/Application Support/magpie/config.toml` to `max_entries = 3`, run `cargo run -p magpie-app`, copy 5+ distinct things, open the launcher, and confirm only the newest 3 (plus any pinned) remain. Record in your report.

- [ ] **Step 6: Commit**

```bash
git add crates/magpie-app/src/runtime.rs
git commit -m "feat(app): wire retention sweep on startup + after capture"
```

---

## Self-Review

**Spec coverage:**
- `RetentionPolicy` (3 optional caps) + `is_noop` → Task 1. ✅
- Age + count enforcement, pinned exempt, cascade to copy_events, FTS trigger → Task 1. ✅
- Image-bytes cap → Task 2. ✅
- Core returns `image_paths`, FS-free → Tasks 1–2 (`Removed`). ✅
- Cascade verified via search/recent + combined policy → Task 3. ✅
- Config fields + `policy_from_config` conversions + partial-config load → Task 4. ✅
- Image file + thumbnail deletion → Task 5. ✅
- Sweep on startup + after each capture, errors swallowed → Task 6. ✅
- All-`None` no-op → Task 1 test + Task 6 guard. ✅
- `Some(0)` edge (documented): age Some(0) deletes all non-pinned; count Some(0) keeps only pinned; image Some(0) evicts all non-pinned images — all fall out of the implemented rules naturally. ✅

**Placeholder scan:** Task 6 uses a `## Manual verification` block by necessity (live loop + FS); all core + helper logic (1–5) is fully TDD'd. No TODOs.

**Type consistency:** `RetentionPolicy { max_entries, max_age_ms, max_image_bytes }`, `Removed { entries_deleted, image_paths }`, `Store::enforce_retention`, `delete_entries`, `policy_from_config`, `FsImageStore::remove_paths`, `sweep_retention`, and the `spawn_watcher(state, denylist, policy, weak)` signature are consistent across tasks. `Config` field names (`max_entries`/`max_age_days`/`max_image_mb`) match Task 4 usage.

## Notes

- `unchecked_transaction()` lets `delete_entries` run a transaction from the `pub(crate) fn conn(&self) -> &Connection` accessor without needing `&mut self`.
- Deleting `copy_events` before `entries` respects the `foreign_keys = ON` pragma.
