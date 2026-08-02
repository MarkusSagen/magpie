# Magpie UI Round 2 Redesign Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rebuild the launcher as a keyboard-first, Raycast-style command bar: type-to-filter search, two-pane type-icon list + full preview + metadata block, type filter, bottom action bar, and a ⌘K actions palette.

**Architecture:** Pure tested helpers (`relative_time`, `type_glyph`, `type_filter_from_index`, `sort_from_index`), one tested core helper (`app_name_pairs`), an expanded `EntryRow` so the row/preview/metadata/action-bar all bind to the selected row without Rust round-trips, and a rebuilt `launcher.slint` with a root `FocusScope` keyboard model + a ⌘K overlay.

**Tech Stack:** Rust, Slint 1.17.1, `rusqlite`.

## Global Constraints

- Toolchain pinned `rust-toolchain.toml` (1.97.1). Run cargo as `env -u RUSTUP_TOOLCHAIN ~/.cargo/bin/cargo …` to reproduce the user's build (the harness sets `RUSTUP_TOOLCHAIN=1.96.0`).
- Gates: `cargo test` (all green), `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`.
- Slint facts (verified against installed 1.17.1): `ListView inherits ScrollView` → `viewport-y` is settable, `visible-height` readable. `FocusScope { key-pressed(event) => { … return accept|reject; } }`. Named keys: `Key.UpArrow/DownArrow/Return/Escape/Backspace/Tab/Delete/Home/End/PageUp/PageDown/Shift/Control/Alt/Meta/LeftArrow/RightArrow/Space`. Modifiers: `event.modifiers.{meta,control,shift,alt}` (Cmd == `meta` on macOS). `event.text` is the character; `event.text.character-count` gives length. Elements expose `.focus()`.
- Paste always uses the real `full_text`; glyphs/previews/metadata are display only.
- App builds and runs after every task.

---

### Task 1: `relative_time` pure helper

**Files:**
- Create: `crates/magpie-app/src/format_time.rs`
- Modify: `crates/magpie-app/src/lib.rs` (add `pub mod format_time;`)

**Interfaces:**
- Produces: `fn relative_time(then_ms: i64, now_ms: i64) -> String`.

- [ ] **Step 1: Write the failing test** — create `format_time.rs` with tests:

```rust
/// Human relative time: "just now", "5m", "3h", "yesterday", "5d", else date.
pub fn relative_time(then_ms: i64, now_ms: i64) -> String {
    unimplemented!()
}

#[cfg(test)]
mod tests {
    use super::relative_time;
    const S: i64 = 1000;
    const MIN: i64 = 60 * S;
    const HOUR: i64 = 60 * MIN;
    const DAY: i64 = 24 * HOUR;

    #[test]
    fn buckets() {
        let now = 1_000 * DAY; // far from epoch
        assert_eq!(relative_time(now, now), "just now");
        assert_eq!(relative_time(now - 30 * S, now), "just now"); // <60s
        assert_eq!(relative_time(now - 5 * MIN, now), "5m");
        assert_eq!(relative_time(now - 3 * HOUR, now), "3h");
        assert_eq!(relative_time(now - 26 * HOUR, now), "yesterday");
        assert_eq!(relative_time(now - 5 * DAY, now), "5d");
    }

    #[test]
    fn future_clock_skew_is_just_now() {
        let now = 1_000 * DAY;
        assert_eq!(relative_time(now + 10 * S, now), "just now");
    }

    #[test]
    fn old_shows_date_yyyy_mm_dd() {
        let now = 1_000 * DAY; // 1970 + 1000 days = 1972-09-27
        let out = relative_time(now - 40 * DAY, now);
        assert_eq!(out.len(), 10); // YYYY-MM-DD
        assert_eq!(out.as_bytes()[4], b'-');
        assert_eq!(out.as_bytes()[7], b'-');
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `env -u RUSTUP_TOOLCHAIN ~/.cargo/bin/cargo test -p magpie-app relative_time` — expect panic (`unimplemented`) / fail. (Add `pub mod format_time;` to `lib.rs` first so it compiles.)

- [ ] **Step 3: Implement** — replace the body. Date branch uses a pure civil-date formatter (no chrono):

```rust
pub fn relative_time(then_ms: i64, now_ms: i64) -> String {
    let diff = now_ms - then_ms;
    const S: i64 = 1000;
    const MIN: i64 = 60 * S;
    const HOUR: i64 = 60 * MIN;
    const DAY: i64 = 24 * HOUR;
    if diff < MIN {
        "just now".to_string()
    } else if diff < HOUR {
        format!("{}m", diff / MIN)
    } else if diff < DAY {
        format!("{}h", diff / HOUR)
    } else if diff < 2 * DAY {
        "yesterday".to_string()
    } else if diff < 30 * DAY {
        format!("{}d", diff / DAY)
    } else {
        let (y, m, d) = civil_from_days(then_ms.div_euclid(DAY));
        format!("{y:04}-{m:02}-{d:02}")
    }
}

/// Howard Hinnant's days->civil date algorithm (proleptic Gregorian, UTC).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}
```

- [ ] **Step 4: Run tests** — `env -u RUSTUP_TOOLCHAIN ~/.cargo/bin/cargo test -p magpie-app relative_time` → PASS. Also `cargo test -p magpie-app format_time` covers the module.

- [ ] **Step 5: Commit**

```bash
git add crates/magpie-app/src/format_time.rs crates/magpie-app/src/lib.rs
git commit -m "feat(app): relative_time formatter (no chrono dep)"
```

---

### Task 2: `type_glyph` pure helper

**Files:**
- Modify: `crates/magpie-app/src/runtime.rs`

**Interfaces:**
- Consumes: `magpie_core::Kind`.
- Produces: `fn type_glyph(kind: Kind) -> &'static str`.

- [ ] **Step 1: Failing test** — append to `round1_tests` (or a new `mod`) in `runtime.rs`:

```rust
#[cfg(test)]
mod glyph_tests {
    use super::type_glyph;
    use magpie_core::Kind;
    #[test]
    fn every_kind_has_a_glyph() {
        assert_eq!(type_glyph(Kind::Link), "🔗");
        assert_eq!(type_glyph(Kind::Color), "🎨");
        assert_eq!(type_glyph(Kind::Email), "✉️");
        assert_eq!(type_glyph(Kind::Image), "🖼️");
        assert_eq!(type_glyph(Kind::File), "📁");
        assert_eq!(type_glyph(Kind::Text), "📄");
        assert_eq!(type_glyph(Kind::Rtf), "📄");
        assert_eq!(type_glyph(Kind::Html), "📄");
    }
}
```

- [ ] **Step 2: Run → fails** (`type_glyph` not found). Run: `env -u RUSTUP_TOOLCHAIN ~/.cargo/bin/cargo test -p magpie-app --bin magpie type_glyph`.

- [ ] **Step 3: Implement** near `line_badge` in `runtime.rs`. Confirm the exact `Kind` variants first: `grep -n "pub enum Kind" -A14 crates/magpie-core/src/model.rs` (the test above assumes Text/Rtf/Html/Link/Color/Email/Image/File — adjust arms to the real variant names):

```rust
fn type_glyph(kind: Kind) -> &'static str {
    match kind {
        Kind::Link => "🔗",
        Kind::Color => "🎨",
        Kind::Email => "✉️",
        Kind::Image => "🖼️",
        Kind::File => "📁",
        Kind::Text | Kind::Rtf | Kind::Html => "📄",
    }
}
```

Add `Kind` to the `magpie_core` import in `runtime.rs` if not already present.

- [ ] **Step 4: Run → PASS.**

- [ ] **Step 5: Commit**

```bash
git add crates/magpie-app/src/runtime.rs
git commit -m "feat(app): type_glyph mapping for list rows"
```

---

### Task 3: `type_filter_from_index` + `sort_from_index`

**Files:**
- Modify: `crates/magpie-app/src/viewmodel.rs`

**Interfaces:**
- Produces: `fn type_filter_from_index(i: i32) -> TypeFilter`; `fn sort_from_index(i: i32) -> Sort`.

- [ ] **Step 1: Failing test** — add to `viewmodel.rs` tests:

```rust
    #[test]
    fn type_filter_index_mapping() {
        assert_eq!(type_filter_from_index(0), TypeFilter::All);
        assert_eq!(type_filter_from_index(1), TypeFilter::Text);
        assert_eq!(type_filter_from_index(2), TypeFilter::Link);
        assert_eq!(type_filter_from_index(3), TypeFilter::Color);
        assert_eq!(type_filter_from_index(4), TypeFilter::Image);
        assert_eq!(type_filter_from_index(5), TypeFilter::File);
        assert_eq!(type_filter_from_index(99), TypeFilter::All); // out of range
    }

    #[test]
    fn sort_index_mapping() {
        assert_eq!(sort_from_index(0), Sort::Recency);
        assert_eq!(sort_from_index(1), Sort::MostCopied);
        assert_eq!(sort_from_index(99), Sort::Recency);
    }
```

(`TypeFilter` needs `#[derive(PartialEq)]` — it already has `Eq, PartialEq`. `Sort` comes from `magpie_core`; import it in the test `use` if needed and ensure it derives `PartialEq` — verify with `grep -n "pub enum Sort" -A6 crates/magpie-core/src/search.rs`; if it lacks `PartialEq`, compare via `matches!` instead.)

- [ ] **Step 2: Run → fails.** `env -u RUSTUP_TOOLCHAIN ~/.cargo/bin/cargo test -p magpie-app --lib type_filter_index_mapping`.

- [ ] **Step 3: Implement** in `viewmodel.rs`:

```rust
pub fn type_filter_from_index(i: i32) -> TypeFilter {
    match i {
        1 => TypeFilter::Text,
        2 => TypeFilter::Link,
        3 => TypeFilter::Color,
        4 => TypeFilter::Image,
        5 => TypeFilter::File,
        _ => TypeFilter::All,
    }
}

pub fn sort_from_index(i: i32) -> Sort {
    match i {
        1 => Sort::MostCopied,
        _ => Sort::Recency,
    }
}
```

Ensure `Sort` is imported at the top (`use magpie_core::{… Sort …}` — already present).

- [ ] **Step 4: Run → PASS.**

- [ ] **Step 5: Commit**

```bash
git add crates/magpie-app/src/viewmodel.rs
git commit -m "feat(app): type-filter/sort index mappings for the UI"
```

---

### Task 4: core `app_name_pairs`

**Files:**
- Create: `crates/magpie-core/src/mask_support.rs`
- Modify: `crates/magpie-core/src/lib.rs` (add `pub mod mask_support;`)

**Interfaces:**
- Produces: `impl Store { pub fn app_name_pairs(&self) -> Result<Vec<(i64, String)>> }` — `(entry_id, apps.display_name)` for entries whose `source_app_id` resolves; NULL-app entries omitted.

- [ ] **Step 1: Failing test** — first confirm the schema column names: `grep -n "display_name\|CREATE TABLE apps\|source_app_id" crates/magpie-core/src/schema.sql`. Then create `mask_support.rs`:

```rust
use crate::store::{Result, Store};

impl Store {
    /// `(entry_id, source app display_name)` for entries with a resolved app.
    /// Entries with a NULL `source_app_id` are omitted.
    pub fn app_name_pairs(&self) -> Result<Vec<(i64, String)>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT e.id, a.display_name
             FROM entries e JOIN apps a ON a.id = e.source_app_id
             ORDER BY e.id",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
        rows.collect()
    }
}

#[cfg(test)]
mod tests {
    use crate::model::{AppInfo, CaptureEvent, Content};
    use crate::store::{open_in_memory, ImageStore, Store};

    struct Noop;
    impl ImageStore for Noop {
        fn put(&self, h: &str, _b: &[u8]) -> std::io::Result<String> { Ok(h.to_string()) }
    }

    fn ingest(s: &Store, t: &str, app: Option<&str>) -> i64 {
        s.ingest(
            &CaptureEvent {
                content: Content::Text(t.into()),
                source_app: app.map(|name| AppInfo {
                    identifier: name.to_string(),
                    display_name: name.to_string(),
                    icon_path: None,
                }),
                copied_at_ms: 1,
            },
            &Noop,
        ).unwrap().entry_id
    }

    #[test]
    fn pairs_include_apps_and_omit_null() {
        let s = open_in_memory().unwrap();
        let a = ingest(&s, "with app", Some("Terminal"));
        let _b = ingest(&s, "no app", None);
        let pairs = s.app_name_pairs().unwrap();
        assert_eq!(pairs, vec![(a, "Terminal".to_string())]);
    }
}
```

Verify `AppInfo` field names against `model.rs` (`grep -n "pub struct AppInfo" -A6 crates/magpie-core/src/model.rs`) and `Store::conn` visibility (used elsewhere in core, e.g. `merge.rs` uses `self.conn()`), and adjust if needed.

- [ ] **Step 2: Run → fails to compile** (module/method missing). Add `pub mod mask_support;` to `lib.rs`. Run: `env -u RUSTUP_TOOLCHAIN ~/.cargo/bin/cargo test -p magpie-core app_name_pairs`.

- [ ] **Step 3: Implement** — (already written above; make it compile).

- [ ] **Step 4: Run → PASS.**

- [ ] **Step 5: Commit**

```bash
git add crates/magpie-core/src/mask_support.rs crates/magpie-core/src/lib.rs
git commit -m "feat(core): app_name_pairs (entry -> source app name)"
```

---

### Task 5: EntryRow fields + `to_rows` populate

**Files:**
- Modify: `crates/magpie-app/ui/launcher.slint` (EntryRow struct only)
- Modify: `crates/magpie-app/src/runtime.rs` (`to_rows` signature + body; `refresh` builds app-name map, passes `now_ms`)

**Interfaces:**
- Consumes: `type_glyph`, `relative_time`, `app_name_pairs`, `line_badge`.
- Produces: `EntryRow { title, subtitle, kind, slot, merged, full, badge, glyph, source, when, copied: int, size }`; `to_rows(entries, slots, tags, merge_set, app_names: &HashMap<i64,String>, now_ms: i64)`.

- [ ] **Step 1: Extend the Slint `EntryRow` struct** — add fields:

```slint
struct EntryRow {
    title: string,
    subtitle: string,
    kind: string,
    slot: int,
    merged: bool,
    full: string,
    badge: string,
    glyph: string,
    source: string,
    when: string,
    copied: int,
    size: string,
}
```

- [ ] **Step 2: Rework `to_rows`** in `runtime.rs`:

```rust
fn to_rows(
    entries: &[Entry],
    slots: &HashMap<i64, i64>,
    tags: &HashMap<i64, Vec<String>>,
    merge_set: &[i32],
    app_names: &HashMap<i64, String>,
    now_ms: i64,
) -> Vec<EntryRow> {
    entries
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let source = app_names
                .get(&e.id)
                .cloned()
                .unwrap_or_else(|| "—".to_string());
            let when = relative_time(e.last_copied_at_ms, now_ms);
            let tagline = tags
                .get(&e.id)
                .map(|ts| ts.iter().map(|t| format!("#{t}")).collect::<Vec<_>>().join(" "))
                .unwrap_or_default();
            let subtitle = if tagline.is_empty() {
                format!("{source} · {when}")
            } else {
                format!("{source} · {when} · {tagline}")
            };
            let size = format!("{} chars · {} lines", e.char_count, e.line_count);
            EntryRow {
                title: SharedString::from(preview_title(e)),
                subtitle: SharedString::from(subtitle),
                kind: SharedString::from(e.kind.as_str()),
                slot: *slots.get(&e.id).unwrap_or(&0) as i32,
                merged: merge_set.contains(&(i as i32)),
                full: SharedString::from(e.full_text.clone()),
                badge: SharedString::from(line_badge(&e.full_text)),
                glyph: SharedString::from(type_glyph(e.kind)),
                source: SharedString::from(source),
                when: SharedString::from(when),
                copied: e.copy_count as i32,
                size: SharedString::from(size),
            }
        })
        .collect()
}
```

Import `relative_time`: add `use magpie_app::format_time::relative_time;` to `runtime.rs`. (`e.kind` is `Copy`? If `Kind` isn't `Copy`, pass `e.kind` by value where it derives `Copy` — verify; `kind.as_str()` already takes `&self`, and `type_glyph(e.kind)` needs `Kind: Copy`. If not `Copy`, change `type_glyph` to take `&Kind`.)

- [ ] **Step 3: Update `refresh`** to build the app-name map and pass `now_ms`:

In the `store.lock()` block that already builds `slots`/`tag_map`/`all_tags`, also build:
```rust
            let app_names: HashMap<i64, String> =
                store.app_name_pairs().unwrap_or_default().into_iter().collect();
```
Return it in the tuple (widen the match to a 4-tuple / restructure). Then:
```rust
    ui.set_entries(ModelRc::new(VecModel::from(to_rows(
        &results, &slots, &tag_map, &merge_set, &app_names, now_ms(),
    ))));
```
(Watch clippy `type_complexity` on the tuple — annotate the inner `let`s, not the tuple, as noted in memory.)

- [ ] **Step 4: Build + clippy + fmt**

```bash
env -u RUSTUP_TOOLCHAIN ~/.cargo/bin/cargo build -p magpie-app
env -u RUSTUP_TOOLCHAIN ~/.cargo/bin/cargo clippy --all-targets -- -D warnings
env -u RUSTUP_TOOLCHAIN ~/.cargo/bin/cargo fmt --check
```
Expected: clean. (The UI won't show the new fields yet — that's Tasks 6–8. Existing behavior intact.)

- [ ] **Step 5: Commit**

```bash
git add crates/magpie-app/ui/launcher.slint crates/magpie-app/src/runtime.rs
git commit -m "feat(app): enrich EntryRow (glyph/source/when/copied/size)"
```

---

### Task 6: Layout shell — search row, type-filter + sort row, wiring

**Files:**
- Modify: `crates/magpie-app/ui/launcher.slint`
- Modify: `crates/magpie-app/src/runtime.rs` (callbacks `set-type-filter`, `set-sort`)

**Interfaces:**
- Produces callbacks: `set-type-filter(int)`, `set-sort(int)`; properties `type-index: int`, `sort-index: int`.

This task restructures the top of `LauncherWindow`'s list view. Keep the Stats view block unchanged. Replace the header + search area with:

- [ ] **Step 1: Add properties + callbacks** to `LauncherWindow`:

```slint
    in-out property <int> type-index: 0;
    in-out property <int> sort-index: 0;
    callback set-type-filter(int);
    callback set-sort(int);
    callback hide-window();
    callback toggle-pin(int);
    callback paste-into-search();
    callback activate-nth(int);
```

- [ ] **Step 2: Build the search row** (replace the old `search := LineEdit {…}` with a custom command-bar field; the real key handling arrives in Task 9, so for now keep it read-only display bound to `query`):

```slint
            // Search row (custom command bar; keys handled by the root FocusScope)
            Rectangle {
                height: 44px;
                background: #161618;
                border-radius: 8px;
                HorizontalLayout {
                    padding-left: 12px; padding-right: 12px; spacing: 8px;
                    Text { text: "🔍"; font-size: 16px; vertical-alignment: center; }
                    Text {
                        text: root.query == "" ? "Search clipboard…" : root.query;
                        color: root.query == "" ? #6a6a70 : white;
                        font-size: 15px;
                        vertical-alignment: center;
                        horizontal-stretch: 1;
                        overflow: elide;
                    }
                }
            }
```

- [ ] **Step 3: Build the type-filter + sort row**:

```slint
            HorizontalLayout {
                spacing: 6px;
                alignment: start;
                for label[i] in ["All", "Text", "Link", "Color", "Image", "File"]: Rectangle {
                    height: 26px;
                    width: 58px;
                    background: i == root.type-index ? #6a8cff : #2a2a30;
                    border-radius: 6px;
                    TouchArea { clicked => { root.type-index = i; root.set-type-filter(i); } }
                    Text { text: label; color: white; font-size: 12px; horizontal-alignment: center; vertical-alignment: center; }
                }
                Rectangle { horizontal-stretch: 1; }
                for label[i] in ["Recent", "Most"]: Rectangle {
                    height: 26px;
                    width: 60px;
                    background: i == root.sort-index ? #3a3a40 : #202024;
                    border-radius: 6px;
                    TouchArea { clicked => { root.sort-index = i; root.set-sort(i); } }
                    Text { text: label; color: white; font-size: 12px; horizontal-alignment: center; vertical-alignment: center; }
                }
            }
```

- [ ] **Step 4: Wire the callbacks** in `runtime.rs` (near the other `ui.on_*` handlers in `start`):

```rust
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_set_type_filter(move |idx| {
            if let Ok(mut u) = s.ui.lock() {
                u.type_filter = magpie_app::viewmodel::type_filter_from_index(idx);
            }
            if let Some(ui) = w.upgrade() { refresh(&ui, &s); }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_set_sort(move |idx| {
            if let Ok(mut u) = s.ui.lock() {
                u.sort = magpie_app::viewmodel::sort_from_index(idx);
            }
            if let Some(ui) = w.upgrade() { refresh(&ui, &s); }
        });
    }
```

(`current_results`/`refresh` already read `UiState` via `to_query`. Confirm `to_query` uses `type_filter` + `sort` — it does.)

- [ ] **Step 5: Build + clippy + fmt + run** — `just`-style gates, then `timeout 8` launch to confirm no panic. Manual: filter chips change the list; search still filters (via existing `search-changed`, still wired to the old callback until Task 9 — keep the `search-changed` callback declaration).

- [ ] **Step 6: Commit** `feat(app): command-bar search row + type/sort filter row`.

---

### Task 7: Redesigned list rows

**Files:**
- Modify: `crates/magpie-app/ui/launcher.slint`

- [ ] **Step 1: Rebuild the row** inside `list := ListView { for row[i] … }` — fixed 52px height, glyph, title+badge line, `source · when` subtitle, selection accent:

```slint
                list := ListView {
                    min-width: 340px;
                    for row[i] in entries: Rectangle {
                        height: 52px;
                        background: i == root.selected ? #2f2f37 : transparent;
                        border-radius: 8px;
                        TouchArea {
                            clicked => { root.selected = i; }
                            double-clicked => { root.activate(i); }
                        }
                        HorizontalLayout {
                            padding-left: 8px; padding-right: 8px; spacing: 8px;
                            // selection accent bar
                            Rectangle { width: 3px; y: 12px; height: 28px; border-radius: 2px;
                                background: i == root.selected ? #6a8cff : transparent; }
                            Text { text: row.glyph; font-size: 18px; vertical-alignment: center; width: 24px; }
                            VerticalLayout {
                                horizontal-stretch: 1;
                                padding-top: 8px; padding-bottom: 8px;
                                HorizontalLayout {
                                    spacing: 6px;
                                    Text { text: row.title; color: white; font-size: 14px; overflow: elide; horizontal-stretch: 1; }
                                    if row.badge != "": Text { text: row.badge; color: #9a9aa0; font-size: 10px; vertical-alignment: center; }
                                }
                                Text { text: row.subtitle; color: #9a9aa0; font-size: 11px; overflow: elide; }
                            }
                            if row.slot > 0: Rectangle { width: 22px; y: 15px; height: 22px; border-radius: 6px; background: #3a3a40;
                                Text { text: "⌘" + row.slot; color: #d0d0d4; font-size: 10px; horizontal-alignment: center; vertical-alignment: center; } }
                            Rectangle {
                                width: 24px; y: 14px; height: 24px;
                                background: row.merged ? #6a8cff : transparent;
                                border-radius: 6px;
                                TouchArea { clicked => { root.toggle-merge(i); } }
                                Text { text: row.merged ? "✓" : "+"; color: row.merged ? white : #6a6a70; font-size: 14px; horizontal-alignment: center; vertical-alignment: center; }
                            }
                        }
                    }
                }
```

- [ ] **Step 2: Build + fmt + clippy + `timeout 8` launch.** Manual: rows show glyphs, `source · when`, slot pill, multiline badge; selection highlights.

- [ ] **Step 3: Commit** `feat(app): redesigned type-icon list rows`.

---

### Task 8: Metadata block + bottom action bar

**Files:**
- Modify: `crates/magpie-app/ui/launcher.slint`

The right pane currently holds preview + slot cells + tags + edit controls. Keep those controls for now (they move to ⌘K in Task 10) but add the metadata block under the preview, and add the bottom action bar.

- [ ] **Step 1: Metadata block** — insert after the preview `ScrollView`, guarded on selection, binding to `entries[root.selected]`:

```slint
                        if entries.length > 0 && root.selected < entries.length: VerticalLayout {
                            spacing: 4px;
                            Rectangle { height: 1px; background: #2a2a30; }
                            for pair in [
                                { k: "Application", v: entries[root.selected].source },
                                { k: "Type", v: entries[root.selected].kind },
                                { k: "Copied", v: entries[root.selected].copied + "×" },
                                { k: "Last used", v: entries[root.selected].when },
                                { k: "Size", v: entries[root.selected].size },
                            ]: HorizontalLayout {
                                Text { text: pair.k; color: #6a6a70; font-size: 11px; width: 110px; }
                                Text { text: pair.v; color: #d0d0d4; font-size: 11px; overflow: elide; }
                            }
                        }
```

- [ ] **Step 2: Bottom action bar** — add as the last child of the outer `VerticalLayout` (inside the list-view branch, after the two-pane `HorizontalLayout`):

```slint
            Rectangle {
                height: 34px;
                background: #161618;
                border-radius: 8px;
                HorizontalLayout {
                    padding-left: 12px; padding-right: 12px; spacing: 12px;
                    Text {
                        text: (entries.length > 0 && root.selected < entries.length) ? entries[root.selected].source : "";
                        color: #9a9aa0; font-size: 12px; vertical-alignment: center; horizontal-stretch: 1;
                    }
                    Text { text: "⏎ Paste   ⌘C Copy   ⌘K Actions"; color: #9a9aa0; font-size: 12px; vertical-alignment: center; }
                }
            }
```

- [ ] **Step 3: Build + fmt + clippy + `timeout 8` launch.** Manual: metadata + action bar reflect the selected row.

- [ ] **Step 4: Commit** `feat(app): metadata block + bottom action bar`.

---

### Task 9: Command-bar keyboard model

**Files:**
- Modify: `crates/magpie-app/ui/launcher.slint` (root `FocusScope`, scroll-follow, key handling)
- Modify: `crates/magpie-app/src/runtime.rs` (callbacks `hide-window`, `toggle-pin`, `paste-into-search`, `activate-nth`; focus on show)

**Interfaces:**
- Consumes callbacks declared in Task 6. Produces their Rust handlers.

- [ ] **Step 1: Wrap the window body in a root `FocusScope`** named `fs`, and set `forward-focus: fs;` on the `LauncherWindow`. Add a `ROW_H` constant (`52px`) and give the `ListView` the id `list` (done in Task 7). Constrain: the FocusScope wraps the existing `VerticalLayout` (make the FocusScope the single top-level child, the layout its child).

- [ ] **Step 2: Implement `key-pressed`** on `fs` (list-mode logic; `mode` property gates edit/actions handled in Task 10 — declare `in-out property <string> mode: "list";` now and guard):

```slint
    fs := FocusScope {
        key-pressed(event) => {
            if root.mode == "edit" { return reject; }
            // Navigation
            if event.text == Key.UpArrow {
                if root.selected > 0 { root.selected -= 1; }
                return accept;
            } else if event.text == Key.DownArrow {
                if root.selected < entries.length - 1 { root.selected += 1; }
                return accept;
            } else if event.text == Key.Return {
                if root.mode == "actions" { root.run-selected-action(); }
                else { root.activate(root.selected); }
                return accept;
            } else if event.text == Key.Escape {
                if root.mode == "actions" { root.mode = "list"; return accept; }
                if root.query != "" { root.query = ""; root.search-changed(root.query); return accept; }
                root.hide-window();
                return accept;
            } else if event.text == Key.Backspace {
                if root.query != "" {
                    root.query = root.query.substring(0, root.query.character-count - 1);
                    root.search-changed(root.query);
                }
                return accept;
            }
            // Cmd shortcuts
            if event.modifiers.meta {
                if event.text == "c" { root.copy-only(root.selected); return accept; }
                if event.text == "k" { root.mode = root.mode == "actions" ? "list" : "actions"; return accept; }
                if event.text == "e" { root.start-edit(root.selected); return accept; }
                if event.text == "n" { root.new-snippet(); return accept; }
                if event.text == "m" { root.toggle-merge(root.selected); return accept; }
                if event.text == "p" { root.toggle-pin(root.selected); return accept; }
                if event.text == "v" { root.paste-into-search(); return accept; }
                // Cmd+1..9 -> paste Nth (list) / assign slot (actions, Task 10)
                if event.text == "1" || event.text == "2" || event.text == "3"
                   || event.text == "4" || event.text == "5" || event.text == "6"
                   || event.text == "7" || event.text == "8" || event.text == "9" {
                    root.activate-nth(event.text.to-float());
                    return accept;
                }
                return reject;
            }
            // Typed character -> query (exclude named/non-printable keys)
            if event.text == Key.Tab || event.text == Key.Delete || event.text == Key.Home
               || event.text == Key.End || event.text == Key.PageUp || event.text == Key.PageDown
               || event.text == Key.LeftArrow || event.text == Key.RightArrow
               || event.text == Key.Shift || event.text == Key.Control
               || event.text == Key.Alt || event.text == Key.Meta {
                return reject;
            }
            if event.text.character-count == 1 {
                root.query += event.text;
                root.search-changed(root.query);
                return accept;
            }
            reject
        }
        // body (the existing VerticalLayout) goes here
    }
```

- [ ] **Step 3: Add a callback `run-selected-action()`** declaration (no-op stub until Task 10): `callback run-selected-action();`. In `runtime.rs`, wire it to a no-op closure for now (or omit wiring — an unconnected callback is fine in Slint; it just does nothing).

- [ ] **Step 4: Scroll-follow** — add to the `LauncherWindow` a handler so the selected row stays visible:

```slint
    changed selected => {
        if root.selected * 52px < -list.viewport-y {
            list.viewport-y = -root.selected * 52px;
        } else if (root.selected + 1) * 52px > -list.viewport-y + list.visible-height {
            list.viewport-y = -((root.selected + 1) * 52px - list.visible-height);
        }
    }
```

(`list` must be reachable from this scope; since `list` is nested, if it isn't reachable, move this logic into the Up/DownArrow branches referencing `list` directly, which are in the same component.)

- [ ] **Step 5: Rust handlers** in `runtime.rs` `start`:

```rust
    {
        let w = ui.as_weak();
        ui.on_hide_window(move || { if let Some(ui) = w.upgrade() { let _ = ui.hide(); } });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_toggle_pin(move |i| {
            let results = current_results(&s, now_ms());
            if let Some(e) = results.get(i as usize) {
                if let Ok(store) = s.store.lock() { let _ = store.set_pinned(e.id, !e.pinned); }
            }
            if let Some(ui) = w.upgrade() { refresh(&ui, &s); }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_activate_nth(move |n| {
            // Reuse the same paste path as `activate`, on the Nth visible result.
            if let Some(ui) = w.upgrade() { activate_index(&ui, &s, (n - 1) as usize); }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_paste_into_search(move || {
            if let Ok(mut clip) = magpie_platform::platform_clipboard() {
                if let Some(text) = clip.read_text().ok().flatten() {
                    if let Some(ui) = w.upgrade() {
                        let q = format!("{}{}", ui.get_query(), text);
                        ui.set_query(q.clone().into());
                        // trigger the existing search path
                        if let Ok(mut u) = s.ui.lock() { u.text = q; }
                        refresh(&ui, &s);
                    }
                }
            }
        });
    }
```

Notes for the implementer:
- `activate` is currently a closure; factor its body into a free `fn activate_index(ui: &LauncherWindow, state: &AppState, idx: usize)` and call it from both `on_activate` and `on_activate_nth`.
- Confirm the `Clipboard` trait's read method name (`grep -n "fn read" crates/magpie-platform/src/**/*.rs` / the trait def). Adjust `read_text()` to the real signature (it may return `Result<Option<String>>` or a `Content`). If reading text is awkward, implement `paste-into-search` as a no-op stub and note it — it's a nice-to-have.
- Re-focus `fs` when the window shows: in `show_window`, after `ui.show()`, call `ui.invoke_focus_search()` — add a tiny Slint `callback focus-search();` handled as `fs.focus()`, or set `forward-focus: fs` (usually enough). Prefer `forward-focus`.

- [ ] **Step 6: Build + clippy + fmt + `timeout 8` launch.** Manual: type to filter, ↑/↓ navigate with scroll-follow, ⏎ paste, Esc clears→hides, ⌘C copy, ⌘1-9 paste Nth.

- [ ] **Step 7: Commit** `feat(app): command-bar keyboard navigation`.

---

### Task 10: ⌘K actions palette + edit/tag focus hop

**Files:**
- Modify: `crates/magpie-app/ui/launcher.slint`
- Modify: `crates/magpie-app/src/runtime.rs`

- [ ] **Step 1: Actions palette overlay** — a full-window overlay shown `if root.mode == "actions"`, above everything (place as the last child of the root `FocusScope`, after the body, so it paints on top). Dim backdrop + centered panel with a keyboard-navigable list. Track `in-out property <int> action-selected: 0;` and an action list:

```slint
        if root.mode == "actions": Rectangle {
            background: #000000aa;
            TouchArea { clicked => { root.mode = "list"; } } // click-away closes
            Rectangle {
                width: 420px;
                height: 300px;
                background: #1e1e21;
                border-radius: 12px;
                border-width: 1px;
                border-color: #3a3a40;
                VerticalLayout {
                    padding: 10px; spacing: 4px;
                    Text { text: "Actions"; color: #9a9aa0; font-size: 12px; }
                    for a[i] in [
                        { label: "Edit", key: "⌘E" },
                        { label: "New snippet", key: "⌘N" },
                        { label: "Pin / Unpin", key: "⌘P" },
                        { label: "Add to merge", key: "⌘M" },
                    ]: Rectangle {
                        height: 32px;
                        background: i == root.action-selected ? #2f2f37 : transparent;
                        border-radius: 6px;
                        TouchArea {
                            clicked => { root.action-selected = i; root.run-selected-action(); }
                        }
                        HorizontalLayout {
                            padding-left: 10px; padding-right: 10px;
                            Text { text: a.label; color: white; font-size: 13px; vertical-alignment: center; horizontal-stretch: 1; }
                            Text { text: a.key; color: #6a6a70; font-size: 12px; vertical-alignment: center; }
                        }
                    }
                    // Assign to slot: press 1..9 or click
                    Text { text: "Assign to slot"; color: #9a9aa0; font-size: 11px; }
                    HorizontalLayout {
                        spacing: 4px;
                        for n in [1,2,3,4,5,6,7,8,9]: Rectangle {
                            width: 30px; height: 28px; border-radius: 6px;
                            background: (entries.length > 0 && entries[root.selected].slot == n) ? #6a8cff : #2a2a30;
                            TouchArea { clicked => { root.assign-slot(root.selected, n); root.mode = "list"; } }
                            Text { text: n; color: white; font-size: 12px; horizontal-alignment: center; vertical-alignment: center; }
                        }
                    }
                }
            }
        }
```

- [ ] **Step 2: Palette key handling** — extend the `fs` `key-pressed`: when `root.mode == "actions"`, ↑/↓ move `action-selected` (0..3), digits 1-9 call `assign-slot` + close, ⏎ runs the selected action, Esc closes. Add near the top of `key-pressed`:

```slint
            if root.mode == "actions" {
                if event.text == Key.UpArrow { if root.action-selected > 0 { root.action-selected -= 1; } return accept; }
                if event.text == Key.DownArrow { if root.action-selected < 3 { root.action-selected += 1; } return accept; }
                if event.text == Key.Escape { root.mode = "list"; return accept; }
                if event.text == Key.Return { root.run-selected-action(); return accept; }
                if event.text == "1" || event.text == "2" || event.text == "3"
                   || event.text == "4" || event.text == "5" || event.text == "6"
                   || event.text == "7" || event.text == "8" || event.text == "9" {
                    root.assign-slot(root.selected, event.text.to-float());
                    root.mode = "list";
                    return accept;
                }
                return accept; // swallow other keys while open
            }
```

- [ ] **Step 3: `run-selected-action` in Slint** — map `action-selected` to the callback + close:

```slint
    callback run-selected-action();
    run-selected-action() => {
        if root.action-selected == 0 { root.start-edit(root.selected); }
        else if root.action-selected == 1 { root.new-snippet(); }
        else if root.action-selected == 2 { root.toggle-pin(root.selected); }
        else if root.action-selected == 3 { root.toggle-merge(root.selected); }
        root.mode = "list";
    }
```

(A callback with an inline handler in the same component is allowed; if the compiler objects to self-invocation, inline the body into the Return/click handlers instead.)

- [ ] **Step 4: Edit focus hop** — when `start-edit` fires, set `mode = "edit"` and focus the editor; on save/cancel set `mode = "list"`. In the existing edit-mode `TextEdit`, give it id `editor` and add `init => { editor.focus(); }` guarded by the edit branch; set `root.mode = "edit"` in the Slint `start-edit` path (or in the Rust handler via a property). Simplest: in `on_start_edit`/`on_new_snippet` Rust handlers, after setting edit state, `ui.set_mode("edit".into())`; in `on_save_edit`/`on_cancel_edit`, `ui.set_mode("list".into())`.

- [ ] **Step 5: Prune the now-duplicated inline controls** — remove the old slot-cell row, the standalone Edit/New-snippet buttons, and the merge bar's redundant bits from the right pane that are now in ⌘K (keep the tag chips + add-tag field for now, and keep the merge separator bar since merge-paste needs the separator selector). Verify nothing referenced only-there breaks.

- [ ] **Step 6: Build + clippy + fmt + `timeout 8` launch.** Manual: ⌘K opens palette; ↑/↓/⏎ run actions; 1-9 assign slot; Esc/click-away closes; Edit hops to native editor and back.

- [ ] **Step 7: Commit** `feat(app): ⌘K actions palette + edit focus hop`.

---

### Task 11: Full gate + run-verify + memory

- [ ] **Step 1: Full gates**

```bash
env -u RUSTUP_TOOLCHAIN ~/.cargo/bin/cargo test
env -u RUSTUP_TOOLCHAIN ~/.cargo/bin/cargo clippy --all-targets -- -D warnings
env -u RUSTUP_TOOLCHAIN ~/.cargo/bin/cargo fmt --check
timeout 8 env -u RUSTUP_TOOLCHAIN ~/.cargo/bin/cargo run -p magpie-app  # exit 124 = alive
```

- [ ] **Step 2: Update memory** `magpie-clipboard-manager.md` with Round 2 completion; commit any docs.

## Self-Review

- **Spec coverage:** search command-bar → T6/T9; type-icon rows → T2/T7; metadata block → T4/T5/T8; type filter + sort → T3/T6; action bar → T8; keyboard model → T9; ⌘K palette → T10; relative time → T1; app names → T4. All spec sections mapped.
- **Placeholder scan:** none — testable code is concrete; UI steps give full Slint. Verification asides (Kind variants, AppInfo fields, Clipboard read signature, Sort PartialEq) are explicit "confirm then adjust" checks, not deferred work.
- **Type consistency:** `to_rows(entries, slots, tags, merge_set, app_names, now_ms)` signature matches its two call sites (definition T5S2, call T5S3). `EntryRow` field set consistent between the Slint struct (T5S1) and `to_rows` (T5S2) and the metadata/action-bar bindings (T8) and rows (T7). Callbacks declared in T6 (`set-type-filter`, `set-sort`, `hide-window`, `toggle-pin`, `paste-into-search`, `activate-nth`) are all wired in T6/T9. `run-selected-action` declared T9S3/T10.
- **Risk flags for the implementer:** (a) Slint `changed selected` reaching `list.viewport-y` — if `list` is out of scope, inline scroll math into the arrow branches. (b) `Kind: Copy` for `type_glyph(e.kind)` — else take `&Kind`. (c) Clipboard read signature for `paste-into-search` — stub if awkward. (d) self-invoked Slint callback `run-selected-action` — inline if the compiler objects.
