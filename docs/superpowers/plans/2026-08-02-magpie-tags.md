# Magpie Tags Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Freeform string tags on entries — add/remove, see chips on the selected entry, and filter the list to a tag (via the existing search), with tag rows cleaned up on retention.

**Architecture:** An additive `entry_tags` join table + a `magpie_core::tags` module. Filtering reuses `search` via a new `SearchQuery.tag`; `UiState.tag` carries it from a Slint `tag-filter` property. The Slint detail pane renders the selected entry's tag chips + an add-tag input, and a tag-filter strip sits above the list; row subtitles show a compact `#tag` caption.

**Tech Stack:** Rust, `rusqlite` (existing), Slint + `std-widgets` (existing). No new dependencies.

## Global Constraints

- **Additive schema only** (`CREATE TABLE IF NOT EXISTS`); no migration.
- **Tags normalized** to `tag.trim().to_lowercase()`; empty after normalization → no-op.
- **Filtering reuses search:** `SearchQuery.tag: Option<String>` → clause `e.id IN (SELECT entry_id FROM entry_tags WHERE tag = ?)`.
- **Retention cleans tags:** `delete_entries` also removes `entry_tags` for victims.
- **Determinism:** core takes no wall clock.
- **Verify by running:** after UI wiring, launch the binary (`timeout 8 ./target/debug/magpie`; exit 124 = alive, no panic).
- **Commit style:** conventional commits, one per task.

---

### Task 1: schema `entry_tags` + `tags` module

**Files:**
- Modify: `crates/magpie-core/src/schema.sql`
- Create: `crates/magpie-core/src/tags.rs`
- Modify: `crates/magpie-core/src/lib.rs` (`pub mod tags;`)
- Test: `tags.rs` tests module.

**Interfaces:**
- Consumes: `Store`, `Result`; (tests) `open_in_memory`, `CaptureEvent`, `Content`, `ImageStore`.
- Produces: `impl Store { pub fn add_tag(&self, entry_id: i64, tag: &str) -> Result<()>; pub fn remove_tag(&self, entry_id: i64, tag: &str) -> Result<()>; pub fn tags_of(&self, entry_id: i64) -> Result<Vec<String>>; pub fn all_tags(&self) -> Result<Vec<String>> }`.

- [ ] **Step 1: Add the table to `schema.sql`** (after the `slots` table)

```sql
CREATE TABLE IF NOT EXISTS entry_tags (
  entry_id INTEGER NOT NULL,
  tag      TEXT NOT NULL,
  PRIMARY KEY (entry_id, tag)
);
CREATE INDEX IF NOT EXISTS idx_entry_tags_tag ON entry_tags(tag);
```

- [ ] **Step 2: Write failing tests in `tags.rs`**

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
    fn ingest(s: &Store, t: &str) -> i64 {
        s.ingest(&CaptureEvent { content: Content::Text(t.into()), source_app: None, copied_at_ms: 1 }, &Noop)
            .unwrap()
            .entry_id
    }

    #[test]
    fn add_normalizes_and_is_idempotent() {
        let s = open_in_memory().unwrap();
        let id = ingest(&s, "x");
        s.add_tag(id, "  Work ").unwrap();
        s.add_tag(id, "work").unwrap(); // dup after normalize
        s.add_tag(id, "  ").unwrap(); // empty -> no-op
        assert_eq!(s.tags_of(id).unwrap(), vec!["work".to_string()]);
    }

    #[test]
    fn remove_and_listing() {
        let s = open_in_memory().unwrap();
        let a = ingest(&s, "a");
        let b = ingest(&s, "b");
        s.add_tag(a, "red").unwrap();
        s.add_tag(a, "blue").unwrap();
        s.add_tag(b, "red").unwrap();
        assert_eq!(s.tags_of(a).unwrap(), vec!["blue".to_string(), "red".to_string()]); // sorted
        assert_eq!(s.all_tags().unwrap(), vec!["blue".to_string(), "red".to_string()]); // distinct+sorted
        s.remove_tag(a, "RED").unwrap(); // normalized on remove
        assert_eq!(s.tags_of(a).unwrap(), vec!["blue".to_string()]);
    }
}
```

- [ ] **Step 3: Add `pub mod tags;` to `lib.rs`; run to verify fail**

Run: `cargo test -p magpie-core --lib tags::`
Expected: FAIL — items not found.

- [ ] **Step 4: Implement `tags.rs`**

```rust
use crate::store::{Result, Store};

fn norm(tag: &str) -> String {
    tag.trim().to_lowercase()
}

impl Store {
    pub fn add_tag(&self, entry_id: i64, tag: &str) -> Result<()> {
        let t = norm(tag);
        if t.is_empty() {
            return Ok(());
        }
        self.conn().execute(
            "INSERT OR IGNORE INTO entry_tags (entry_id, tag) VALUES (?1, ?2)",
            rusqlite::params![entry_id, t],
        )?;
        Ok(())
    }

    pub fn remove_tag(&self, entry_id: i64, tag: &str) -> Result<()> {
        let t = norm(tag);
        self.conn().execute(
            "DELETE FROM entry_tags WHERE entry_id = ?1 AND tag = ?2",
            rusqlite::params![entry_id, t],
        )?;
        Ok(())
    }

    pub fn tags_of(&self, entry_id: i64) -> Result<Vec<String>> {
        let mut stmt = self
            .conn()
            .prepare("SELECT tag FROM entry_tags WHERE entry_id = ?1 ORDER BY tag")?;
        stmt.query_map([entry_id], |r| r.get::<_, String>(0))?.collect()
    }

    pub fn all_tags(&self) -> Result<Vec<String>> {
        let mut stmt = self
            .conn()
            .prepare("SELECT DISTINCT tag FROM entry_tags ORDER BY tag")?;
        stmt.query_map([], |r| r.get::<_, String>(0))?.collect()
    }
}
```

- [ ] **Step 5: Run to verify pass**

Run: `cargo test -p magpie-core --lib tags::`
Expected: PASS (2 tests).

- [ ] **Step 6: Commit**

```bash
git add crates/magpie-core/src/schema.sql crates/magpie-core/src/tags.rs crates/magpie-core/src/lib.rs
git commit -m "feat(core): entry_tags table + add/remove/tags_of/all_tags"
```

---

### Task 2: `SearchQuery.tag` filter

**Files:**
- Modify: `crates/magpie-core/src/search.rs`
- Test: `search.rs` tests module.

**Interfaces:**
- Consumes: `SearchQuery`, `default_query`, the existing `search` clause/param builder.
- Produces: `SearchQuery` gains `pub tag: Option<String>`; `default_query` sets `tag: None`; `search` adds the tag clause when `Some` and non-empty (normalized).

- [ ] **Step 1: Add a failing test to `search.rs` tests module**

```rust
    #[test]
    fn filter_by_tag() {
        let s = open_in_memory().unwrap();
        let a = s.ingest(&text_ev("tagged one", 1), &FakeImages).unwrap().entry_id;
        s.ingest(&text_ev("untagged", 2), &FakeImages).unwrap();
        s.add_tag(a, "keep").unwrap();

        let mut q = default_query();
        q.tag = Some("KEEP".into()); // normalized in search
        let rows = s.search(&q).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].full_text, "tagged one");
    }
```

- [ ] **Step 2: Run to verify fail**

Run: `cargo test -p magpie-core --lib search::tests::filter_by_tag`
Expected: FAIL — no `tag` field.

- [ ] **Step 3: Add the field + default + clause**

In `search.rs`, add to the `SearchQuery` struct:

```rust
    pub tag: Option<String>,
```

In `default_query`, add:

```rust
        tag: None,
```

In `search`, after the `q.time.until_ms` clause block, add:

```rust
        if let Some(tag) = &q.tag {
            let tag = tag.trim().to_lowercase();
            if !tag.is_empty() {
                clauses.push("e.id IN (SELECT entry_id FROM entry_tags WHERE tag = ?)".to_string());
                params.push(Value::Text(tag));
            }
        }
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p magpie-core --lib search::`
Expected: PASS (all search tests, including `filter_by_tag`).

- [ ] **Step 5: Commit**

```bash
git add crates/magpie-core/src/search.rs
git commit -m "feat(core): SearchQuery.tag filter"
```

---

### Task 3: Retention cleans up `entry_tags`

**Files:**
- Modify: `crates/magpie-core/src/retention.rs`
- Test: `retention.rs` tests module.

**Interfaces:**
- Consumes: `delete_entries`'s victim id list.
- Produces: `delete_entries` also runs `DELETE FROM entry_tags WHERE entry_id IN (victims)` inside its transaction.

- [ ] **Step 1: Add a failing test to `retention.rs` tests module**

```rust
    #[test]
    fn retention_removes_tags_of_deleted_entries() {
        let s = open_in_memory().unwrap();
        let old = s.ingest(&text("old", 100), &Noop).unwrap().entry_id;
        s.add_tag(old, "gone").unwrap();
        // age cap deletes the unpinned "old"
        let policy = RetentionPolicy { max_entries: None, max_age_ms: Some(1_000), max_image_bytes: None };
        s.enforce_retention(&policy, 10_000).unwrap();
        // its tag row is gone -> not in all_tags
        assert!(s.all_tags().unwrap().is_empty());
    }
```

Note: the existing `text()` helper returns a `CaptureEvent`; capture the ingest's `entry_id` via `.entry_id` as shown.

- [ ] **Step 2: Run to verify fail**

Run: `cargo test -p magpie-core --lib retention::tests::retention_removes_tags`
Expected: FAIL — tag row survives (all_tags not empty).

- [ ] **Step 3: Add the delete to `delete_entries`** (in `retention.rs`, inside the transaction, before `DELETE FROM entries`)

```rust
        tx.execute(
            &format!("DELETE FROM entry_tags WHERE entry_id IN ({placeholders})"),
            rusqlite::params_from_iter(ids.iter()),
        )?;
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p magpie-core --lib retention::`
Expected: PASS (all retention tests).

- [ ] **Step 5: Commit**

```bash
git add crates/magpie-core/src/retention.rs
git commit -m "feat(core): retention prunes entry_tags of deleted entries"
```

---

### Task 4: Core integration test

**Files:**
- Create: `crates/magpie-core/tests/tags.rs`

**Interfaces:**
- Consumes: public `add_tag`/`remove_tag`/`tags_of`/`all_tags`, `search` with `tag`, `open_in_memory`, `default_query`.

- [ ] **Step 1: Write the integration test file**

```rust
//! Integration tests for tags.

use magpie_core::{default_query, open_in_memory, CaptureEvent, Content, ImageStore, Store};

struct Noop;
impl ImageStore for Noop {
    fn put(&self, h: &str, _b: &[u8]) -> std::io::Result<String> { Ok(h.to_string()) }
}
fn ingest(s: &Store, t: &str) -> i64 {
    s.ingest(&CaptureEvent { content: Content::Text(t.into()), source_app: None, copied_at_ms: 1 }, &Noop)
        .unwrap()
        .entry_id
}

#[test]
fn tag_filter_combines_with_text_search() {
    let s = open_in_memory().unwrap();
    let a = ingest(&s, "alpha note");
    let b = ingest(&s, "alpha memo");
    ingest(&s, "beta note");
    s.add_tag(a, "work").unwrap();
    s.add_tag(b, "home").unwrap();

    // text "alpha" + tag "work" -> only entry a
    let mut q = default_query();
    q.text = "alpha".into();
    q.tag = Some("work".into());
    let rows = s.search(&q).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, a);
}

#[test]
fn all_tags_reflects_adds_and_removes() {
    let s = open_in_memory().unwrap();
    let id = ingest(&s, "x");
    s.add_tag(id, "one").unwrap();
    s.add_tag(id, "two").unwrap();
    assert_eq!(s.all_tags().unwrap(), vec!["one".to_string(), "two".to_string()]);
    s.remove_tag(id, "one").unwrap();
    assert_eq!(s.all_tags().unwrap(), vec!["two".to_string()]);
}
```

- [ ] **Step 2: Run to verify it passes**

Run: `cargo test -p magpie-core --test tags`
Expected: PASS (2 tests).

- [ ] **Step 3: Commit**

```bash
git add crates/magpie-core/tests/tags.rs
git commit -m "test(core): tag filter + all_tags lifecycle"
```

---

### Task 5: App `UiState.tag` → `to_query`

**Files:**
- Modify: `crates/magpie-app/src/viewmodel.rs`
- Test: `viewmodel.rs` tests module.

**Interfaces:**
- Consumes: `UiState`, `to_query`, `magpie_core::SearchQuery`.
- Produces: `UiState` gains `pub tag: Option<String>` (default `None`); `to_query` maps it to `SearchQuery.tag`.

- [ ] **Step 1: Add a failing test to `viewmodel.rs` tests module**

```rust
    #[test]
    fn tag_flows_into_query() {
        let mut ui = UiState::new();
        assert!(to_query(&ui, 0).tag.is_none());
        ui.tag = Some("work".into());
        assert_eq!(to_query(&ui, 0).tag, Some("work".to_string()));
    }
```

- [ ] **Step 2: Run to verify fail**

Run: `cargo test -p magpie-app --lib viewmodel::tests::tag_flows`
Expected: FAIL — no `tag` field.

- [ ] **Step 3: Add the field + mapping**

In `viewmodel.rs`, add to the `UiState` struct:

```rust
    pub tag: Option<String>,
```

In `UiState::new`, add:

```rust
            tag: None,
```

In `to_query`, before `q` is returned, add:

```rust
    q.tag = ui.tag.clone();
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p magpie-app --lib viewmodel::`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/magpie-app/src/viewmodel.rs
git commit -m "feat(app): UiState.tag -> SearchQuery.tag"
```

---

### Task 6: Slint tag UI + runtime wiring + run verification

**Files:**
- Modify: `crates/magpie-app/ui/launcher.slint`
- Modify: `crates/magpie-app/src/runtime.rs`

**Interfaces:**
- Consumes: `Store::{add_tag, remove_tag, tags_of, all_tags}`, `UiState.tag`, existing `refresh`/`to_rows`/`current_results`.
- Produces:
  - `LauncherWindow` props: `in property <[string]> selected-tags;` `in property <[string]> all-tags;` `in-out property <string> add-tag-text;` `in-out property <string> tag-filter;` and callbacks `add-tag(int, string)`, `remove-tag(int, string)`, `set-tag-filter(string)`.
  - Detail pane: selected-tags chips (click → `remove-tag`), an add input (`LineEdit` + Add → `add-tag`). Above the list: an `all-tags` filter strip (click → `set-tag-filter`).
  - `runtime::refresh`: sets `state.ui.tag` from the `tag-filter` property (before querying), sets `all-tags`/`selected-tags`, and appends a `#tag` caption to each row subtitle via a tag map. Callbacks wired.

> UI + runtime; gate is compile + `timeout` run. Interactive check is manual.

- [ ] **Step 1: Add props + callbacks to `LauncherWindow`** (next to the other callbacks)

```slint
    in property <[string]> selected-tags;
    in property <[string]> all-tags;
    in-out property <string> add-tag-text;
    in-out property <string> tag-filter;
    callback add-tag(int, string);
    callback remove-tag(int, string);
    callback set-tag-filter(string);
```

- [ ] **Step 2: Add the tag-filter strip in the list view** (inside `if root.view == "list": VerticalLayout { ... }`, right after the `search := LineEdit { ... }`)

```slint
            if root.all-tags.length > 0: HorizontalLayout {
                spacing: 4px;
                alignment: start;
                for t in root.all-tags: Rectangle {
                    height: 22px;
                    width: 64px;
                    background: root.tag-filter == t ? #6a8cff : #2a2a30;
                    border-radius: 11px;
                    TouchArea { clicked => { root.set-tag-filter(t); } }
                    Text { text: "#" + t; color: white; font-size: 11px; horizontal-alignment: center; vertical-alignment: center; overflow: elide; }
                }
            }
```

- [ ] **Step 3: Add the tag chips + add-input to the detail pane** (in view mode, after the slot-cells `if` block and before the view-mode action buttons)

```slint
                        if root.selected-tags.length > 0: HorizontalLayout {
                            spacing: 4px;
                            alignment: start;
                            for t in root.selected-tags: Rectangle {
                                height: 22px;
                                width: 70px;
                                background: #3a3a40;
                                border-radius: 11px;
                                TouchArea { clicked => { root.remove-tag(root.selected, t); } }
                                Text { text: "#" + t + " ✕"; color: #d0d0d4; font-size: 11px; horizontal-alignment: center; vertical-alignment: center; overflow: elide; }
                            }
                        }
                        if entries.length > 0 && root.selected < entries.length: HorizontalLayout {
                            spacing: 6px;
                            alignment: start;
                            tagfield := LineEdit {
                                placeholder-text: "add tag…";
                                width: 140px;
                                text <=> root.add-tag-text;
                            }
                            Rectangle {
                                width: 48px;
                                height: 26px;
                                background: #2a2a30;
                                border-radius: 6px;
                                TouchArea { clicked => { root.add-tag(root.selected, root.add-tag-text); } }
                                Text { text: "Add"; color: white; font-size: 12px; horizontal-alignment: center; vertical-alignment: center; }
                            }
                        }
```

- [ ] **Step 4: Update `runtime::to_rows` to append tag captions** and take a tag map

```rust
fn to_rows(
    entries: &[Entry],
    slots: &HashMap<i64, i64>,
    tags: &HashMap<i64, Vec<String>>,
) -> Vec<EntryRow> {
    entries
        .iter()
        .map(|e| {
            let tagline = tags
                .get(&e.id)
                .map(|ts| ts.iter().map(|t| format!("#{t}")).collect::<Vec<_>>().join(" "))
                .unwrap_or_default();
            let subtitle = if tagline.is_empty() {
                format!("{} · copied {}×", e.kind.as_str(), e.copy_count)
            } else {
                format!("{} · copied {}× · {}", e.kind.as_str(), e.copy_count, tagline)
            };
            EntryRow {
                title: SharedString::from(preview_title(e)),
                subtitle: SharedString::from(subtitle),
                kind: SharedString::from(e.kind.as_str()),
                slot: *slots.get(&e.id).unwrap_or(&0) as i32,
            }
        })
        .collect()
}
```

- [ ] **Step 5: Update `runtime::refresh`** to set the tag filter, build the tag map, and set the tag props

```rust
fn refresh(ui: &LauncherWindow, state: &AppState) {
    // Push the window's tag-filter into UiState before querying.
    {
        let t = ui.get_tag_filter().to_string();
        if let Ok(mut u) = state.ui.lock() {
            u.tag = if t.is_empty() { None } else { Some(t) };
        }
    }
    let results = current_results(state, now_ms());

    // slot map (entry_id -> slot) + tag map (entry_id -> tags) + all_tags
    let (slots, tag_map, all_tags): (HashMap<i64, i64>, HashMap<i64, Vec<String>>, Vec<String>) =
        match state.store.lock() {
            Ok(store) => {
                let slots = store
                    .slot_map()
                    .unwrap_or_default()
                    .into_iter()
                    .map(|(slot, eid)| (eid, slot))
                    .collect();
                let mut tag_map: HashMap<i64, Vec<String>> = HashMap::new();
                if let Ok(mut stmt) = store.conn_public().prepare("SELECT entry_id, tag FROM entry_tags ORDER BY tag") {
                    // conn is pub(crate); see note below — use a core helper instead.
                }
                let all_tags = store.all_tags().unwrap_or_default();
                (slots, tag_map, all_tags)
            }
            Err(_) => (HashMap::new(), HashMap::new(), Vec::new()),
        };
    // ... set props (see Step 6) ...
}
```

> The `conn` accessor is `pub(crate)` — the app cannot read `entry_tags` directly.
> Add a core helper in Task 6a below and use it instead of touching `conn`.

- [ ] **Step 6a: Add a core `tag_map()` helper** (in `crates/magpie-core/src/tags.rs`, `impl Store`)

```rust
    /// (entry_id, tag) pairs for annotating a list. Ordered by entry then tag.
    pub fn tag_pairs(&self) -> Result<Vec<(i64, String)>> {
        let mut stmt = self
            .conn()
            .prepare("SELECT entry_id, tag FROM entry_tags ORDER BY entry_id, tag")?;
        stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?
            .collect()
    }
```

Add a unit test for it in `tags.rs`:

```rust
    #[test]
    fn tag_pairs_lists_all() {
        let s = open_in_memory().unwrap();
        let id = ingest(&s, "x");
        s.add_tag(id, "a").unwrap();
        s.add_tag(id, "b").unwrap();
        assert_eq!(s.tag_pairs().unwrap(), vec![(id, "a".to_string()), (id, "b".to_string())]);
    }
```

- [ ] **Step 6: Finish `refresh`** using `tag_pairs()` and set props (replace the Step-5 sketch)

```rust
fn refresh(ui: &LauncherWindow, state: &AppState) {
    {
        let t = ui.get_tag_filter().to_string();
        if let Ok(mut u) = state.ui.lock() {
            u.tag = if t.is_empty() { None } else { Some(t) };
        }
    }
    let results = current_results(state, now_ms());

    let (slots, tag_map, all_tags): (HashMap<i64, i64>, HashMap<i64, Vec<String>>, Vec<String>) =
        match state.store.lock() {
            Ok(store) => {
                let slots = store.slot_map().unwrap_or_default().into_iter().map(|(s, e)| (e, s)).collect();
                let mut tag_map: HashMap<i64, Vec<String>> = HashMap::new();
                for (eid, tag) in store.tag_pairs().unwrap_or_default() {
                    tag_map.entry(eid).or_default().push(tag);
                }
                let all_tags = store.all_tags().unwrap_or_default();
                (slots, tag_map, all_tags)
            }
            Err(_) => (HashMap::new(), HashMap::new(), Vec::new()),
        };

    let sel = ui.get_selected() as usize;
    let selected_tags: Vec<slint::SharedString> = results
        .get(sel)
        .and_then(|e| tag_map.get(&e.id))
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(slint::SharedString::from)
        .collect();
    ui.set_selected_tags(ModelRc::new(VecModel::from(selected_tags)));
    ui.set_all_tags(ModelRc::new(VecModel::from(
        all_tags.into_iter().map(slint::SharedString::from).collect::<Vec<_>>(),
    )));

    let detail = results.first().map(|e| e.full_text.clone()).unwrap_or_default();
    ui.set_entries(ModelRc::new(VecModel::from(to_rows(&results, &slots, &tag_map))));
    ui.set_detail_text(SharedString::from(detail));
}
```

- [ ] **Step 7: Wire the callbacks in `start()`** (near the other `ui.on_*` blocks)

```rust
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_add_tag(move |index, tag| {
            let recent = current_results(&s, now_ms());
            if let Some(e) = recent.get(index as usize) {
                if let Ok(store) = s.store.lock() {
                    let _ = store.add_tag(e.id, &tag);
                }
            }
            if let Some(ui) = w.upgrade() {
                ui.set_add_tag_text(SharedString::from(""));
                refresh(&ui, &s);
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_remove_tag(move |index, tag| {
            let recent = current_results(&s, now_ms());
            if let Some(e) = recent.get(index as usize) {
                if let Ok(store) = s.store.lock() {
                    let _ = store.remove_tag(e.id, &tag);
                }
            }
            if let Some(ui) = w.upgrade() {
                refresh(&ui, &s);
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_set_tag_filter(move |tag| {
            if let Some(ui) = w.upgrade() {
                let current = ui.get_tag_filter().to_string();
                let next = if current == tag.as_str() { SharedString::from("") } else { tag };
                ui.set_tag_filter(next);
                refresh(&ui, &s);
            }
        });
    }
```

- [ ] **Step 8: Compile + run**

Run: `cargo build -p magpie-app`
Expected: compiles clean.

Run: `timeout 8 ./target/debug/magpie; echo "exit=$?"`
Expected: `exit=124` (alive, no panic).

- [ ] **Step 9: Manual verification** (record in report)

`cargo run -p magpie-app`; select an entry; type a tag + **Add** → the `#tag` chip appears and the subtitle shows it. Click the tag in the filter strip → the list narrows to that tag; click again → clears. Click a chip's ✕ → tag removed.

- [ ] **Step 10: Commit**

```bash
git add crates/magpie-app/ui/launcher.slint crates/magpie-app/src/runtime.rs crates/magpie-core/src/tags.rs crates/magpie-core/src/lib.rs
git commit -m "feat(app): tag chips, add input, tag-filter strip + wiring"
```

---

## Self-Review

**Spec coverage:**
- `entry_tags` table + `add_tag`/`remove_tag`/`tags_of`/`all_tags` → Task 1. ✅
- `SearchQuery.tag` filter → Task 2. ✅
- Retention cleans `entry_tags` → Task 3. ✅
- `UiState.tag` → `to_query` → Task 5. ✅
- Detail-pane chips + add input + tag-filter strip + row captions → Task 6. ✅
- `tag_pairs` list-annotation helper → Task 6a. ✅
- Normalization + idempotence + empty no-op → Tasks 1, 2, 5. ✅
- Run-verification → Task 6 Step 8. ✅

**Placeholder scan:** Task 6 UI/runtime use compile + `timeout` run + manual; core (1–5, 6a) fully TDD'd. The Step-5 sketch is explicitly superseded by Step 6 (marked). No TODOs.

**Type consistency:** `add_tag`/`remove_tag`/`tags_of`/`all_tags`/`tag_pairs` signatures; `SearchQuery.tag: Option<String>`; `UiState.tag: Option<String>`; Slint `selected-tags`/`all-tags`/`add-tag-text`/`tag-filter` + `add-tag(int,string)`/`remove-tag(int,string)`/`set-tag-filter(string)`; `to_rows(&[Entry], &HashMap<i64,i64>, &HashMap<i64,Vec<String>>)` are consistent across tasks.

## Notes

- `tag_pairs()` is the app's read path for list annotation (the `conn` accessor is `pub(crate)`, so the app can't query `entry_tags` directly).
- Adding `tag` to `SearchQuery` only requires updating `default_query` (all sites use it); existing tests keep compiling.
