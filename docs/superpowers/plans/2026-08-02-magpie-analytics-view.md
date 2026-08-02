# Magpie Analytics View Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a "Stats" mode to the Magpie launcher that visualizes clipboard usage (totals, most-copied, copies-over-time, per-app, by-type) from the already-recorded `entries`/`copy_events` tables, with a selectable time range and dependency-free hand-drawn bar charts.

**Architecture:** A new read-only `magpie_core::stats` module runs aggregate SQL over existing tables (no schema change) and returns plain data via `Store::stats(range, top_n)`. A pure app-side `stats_view` helper normalizes each series into `Bar { label, display, value_norm }`. The Slint launcher gains a `view` toggle and a stats layout that renders bars as `Rectangle`s from models; `runtime::refresh_stats` wires range changes to the core query.

**Tech Stack:** Rust, `rusqlite` (existing), Slint (existing). No new dependencies.

## Global Constraints

- **No schema changes** — read-only aggregates over existing `entries`, `copy_events`, `apps`.
- **No new dependencies** — bars are hand-drawn Slint `Rectangle`s; charts computed in Rust.
- **Determinism:** all time enters as `now_ms: i64`; core never calls the wall clock.
- **"In range" is uniform:** every series filters `copy_events.copied_at_ms BETWEEN lo AND hi`, where `lo = range.since_ms.unwrap_or(i64::MIN)` and `hi = range.now_ms`.
- **top-N is a call parameter** (default 10 from the UI), not config or schema.
- **All-time over-time bucketing:** day buckets, but switch to **weekly** buckets when the span exceeds 90 days, so the chart stays readable. Totals/most-copied/per-app/by-type are always true all-time.
- **DAY constant:** `86_400_000` ms. Bucketing uses integer division.
- **Commit style:** conventional commits, one per task.

---

### Task 1: `stats` module scaffold + types + `totals`

**Files:**
- Create: `crates/magpie-core/src/stats.rs`
- Modify: `crates/magpie-core/src/lib.rs` (add `pub mod stats;`)
- Test: `stats.rs` tests module.

**Interfaces:**
- Consumes: `Store`, `Kind`, `Result`, and (in tests) `CaptureEvent`/`Content`/`AppInfo`/`ImageStore`/`open_in_memory`.
- Produces:
  - `pub struct StatsRange { pub since_ms: Option<i64>, pub now_ms: i64 }`
  - `pub struct Totals { pub copies: i64, pub unique_entries: i64, pub distinct_apps: i64 }`
  - `pub struct MostCopied { pub entry_id: i64, pub preview: String, pub kind: Kind, pub count: i64 }`
  - `pub struct DayBucket { pub start_ms: i64, pub count: i64 }`
  - `pub struct AppCount { pub name: String, pub count: i64 }`
  - `pub struct KindCount { pub kind: Kind, pub count: i64 }`
  - `pub struct Stats { pub totals: Totals, pub most_copied: Vec<MostCopied>, pub over_time: Vec<DayBucket>, pub per_app: Vec<AppCount>, pub by_type: Vec<KindCount> }`
  - `pub(crate) const DAY_MS: i64 = 86_400_000;`
  - `impl Store { pub(crate) fn totals(&self, lo: i64, hi: i64) -> Result<Totals> }` — `COUNT(*)`, `COUNT(DISTINCT entry_id)`, `COUNT(DISTINCT source_app_id)` over `copy_events` in `[lo,hi]`.

- [ ] **Step 1: Write failing tests in `stats.rs`**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AppInfo, CaptureEvent, Content};
    use crate::store::{open_in_memory, ImageStore, Store};

    struct Noop;
    impl ImageStore for Noop {
        fn put(&self, h: &str, _b: &[u8]) -> std::io::Result<String> { Ok(h.to_string()) }
    }
    fn ev(text: &str, ms: i64, app: Option<&str>) -> CaptureEvent {
        CaptureEvent {
            content: Content::Text(text.into()),
            source_app: app.map(|a| AppInfo { identifier: a.into(), display_name: a.into(), icon_path: None }),
            copied_at_ms: ms,
        }
    }
    fn seed(store: &Store, evs: &[CaptureEvent]) {
        for e in evs { store.ingest(e, &Noop).unwrap(); }
    }

    #[test]
    fn totals_counts_copies_unique_and_apps_in_range() {
        let s = open_in_memory().unwrap();
        seed(&s, &[
            ev("a", 100, Some("Ghostty")),
            ev("a", 200, Some("Ghostty")), // dup content, same app
            ev("b", 300, Some("Safari")),
            ev("c", 50, None),             // no app
            ev("old", 5, Some("Ghostty")), // will be out of range below
        ]);
        // range [10, 1000]: excludes the ms=5 event
        let t = s.totals(10, 1000).unwrap();
        assert_eq!(t.copies, 4);
        assert_eq!(t.unique_entries, 3); // a, b, c
        assert_eq!(t.distinct_apps, 2);  // Ghostty, Safari (NULL excluded)
    }
}
```

- [ ] **Step 2: Add `pub mod stats;` to `lib.rs`; run to verify fail**

Run: `cargo test -p magpie-core stats::tests::totals`
Expected: FAIL — items not found.

- [ ] **Step 3: Implement types + `totals` in `stats.rs`**

```rust
use crate::detect::Kind;
use crate::store::{Result, Store};

pub(crate) const DAY_MS: i64 = 86_400_000;

pub struct StatsRange {
    pub since_ms: Option<i64>,
    pub now_ms: i64,
}

pub struct Totals {
    pub copies: i64,
    pub unique_entries: i64,
    pub distinct_apps: i64,
}

pub struct MostCopied {
    pub entry_id: i64,
    pub preview: String,
    pub kind: Kind,
    pub count: i64,
}

pub struct DayBucket {
    pub start_ms: i64,
    pub count: i64,
}

pub struct AppCount {
    pub name: String,
    pub count: i64,
}

pub struct KindCount {
    pub kind: Kind,
    pub count: i64,
}

pub struct Stats {
    pub totals: Totals,
    pub most_copied: Vec<MostCopied>,
    pub over_time: Vec<DayBucket>,
    pub per_app: Vec<AppCount>,
    pub by_type: Vec<KindCount>,
}

impl Store {
    pub(crate) fn totals(&self, lo: i64, hi: i64) -> Result<Totals> {
        self.conn().query_row(
            "SELECT COUNT(*), COUNT(DISTINCT entry_id), COUNT(DISTINCT source_app_id)
             FROM copy_events WHERE copied_at_ms BETWEEN ?1 AND ?2",
            [lo, hi],
            |r| Ok(Totals { copies: r.get(0)?, unique_entries: r.get(1)?, distinct_apps: r.get(2)? }),
        )
    }
}
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p magpie-core stats`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/magpie-core/src/stats.rs crates/magpie-core/src/lib.rs
git commit -m "feat(core): stats module scaffold + totals aggregate"
```

---

### Task 2: `most_copied`

**Files:**
- Modify: `crates/magpie-core/src/stats.rs`
- Test: `stats.rs` tests module.

**Interfaces:**
- Consumes: `Totals` task's types, `Kind::from_str`.
- Produces: `impl Store { pub(crate) fn most_copied(&self, lo: i64, hi: i64, top_n: i64) -> Result<Vec<MostCopied>> }` — copies per entry in range, joined to `entries` for `preview_text`/`kind`, ordered by count desc then entry_id, limited to `top_n`.

- [ ] **Step 1: Add failing test to `stats.rs` tests module**

```rust
    #[test]
    fn most_copied_orders_by_count_and_limits() {
        let s = open_in_memory().unwrap();
        seed(&s, &[
            ev("thrice", 1, None), ev("thrice", 2, None), ev("thrice", 3, None),
            ev("once", 4, None),
            ev("twice", 5, None), ev("twice", 6, None),
        ]);
        let rows = s.most_copied(i64::MIN, 1000, 2).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].preview, "thrice");
        assert_eq!(rows[0].count, 3);
        assert_eq!(rows[0].kind.as_str(), "text");
        assert_eq!(rows[1].preview, "twice");
        assert_eq!(rows[1].count, 2);
    }
```

- [ ] **Step 2: Run to verify fail**

Run: `cargo test -p magpie-core most_copied`
Expected: FAIL.

- [ ] **Step 3: Implement `most_copied`**

```rust
impl Store {
    pub(crate) fn most_copied(&self, lo: i64, hi: i64, top_n: i64) -> Result<Vec<MostCopied>> {
        let mut stmt = self.conn().prepare(
            "SELECT ce.entry_id, COUNT(*) AS c, e.preview_text, e.kind
             FROM copy_events ce JOIN entries e ON e.id = ce.entry_id
             WHERE ce.copied_at_ms BETWEEN ?1 AND ?2
             GROUP BY ce.entry_id
             ORDER BY c DESC, ce.entry_id
             LIMIT ?3",
        )?;
        let rows = stmt.query_map([lo, hi, top_n], |r| {
            let kind_str: String = r.get(3)?;
            Ok(MostCopied {
                entry_id: r.get(0)?,
                count: r.get(1)?,
                preview: r.get(2)?,
                kind: Kind::from_str(&kind_str).unwrap_or(Kind::Text),
            })
        })?;
        rows.collect()
    }
}
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p magpie-core most_copied`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/magpie-core/src/stats.rs
git commit -m "feat(core): most_copied aggregate"
```

---

### Task 3: `over_time` (day bucketing + zero-fill + weekly fallback)

**Files:**
- Modify: `crates/magpie-core/src/stats.rs`
- Test: `stats.rs` tests module.

**Interfaces:**
- Consumes: `StatsRange`, `DayBucket`, `DAY_MS`.
- Produces:
  - `pub(crate) fn bucket_ms(range: &StatsRange, earliest_ms: i64) -> i64` — `DAY_MS`, or `7*DAY_MS` when `range.since_ms.is_none()` AND the span `(now_ms/DAY - earliest/DAY) > 90`.
  - `impl Store { pub(crate) fn over_time(&self, lo: i64, hi: i64, range: &StatsRange) -> Result<Vec<DayBucket>> }` — groups `copy_events` in `[lo,hi]` by `copied_at_ms / bucket`, then zero-fills every bucket from the start bucket (`since_ms` when set, else earliest event, else `now_ms`) through `now_ms`'s bucket. `start_ms` of each `DayBucket` is `bucket_index * bucket`.

- [ ] **Step 1: Add failing tests to `stats.rs` tests module**

```rust
    const DAY: i64 = 86_400_000;

    #[test]
    fn over_time_day_buckets_are_zero_filled() {
        let s = open_in_memory().unwrap();
        // events on day 100 (x2) and day 102 (x1); day 101 empty
        seed(&s, &[
            ev("a", 100 * DAY + 1, None),
            ev("b", 100 * DAY + 2, None),
            ev("c", 102 * DAY + 5, None),
        ]);
        let range = StatsRange { since_ms: Some(100 * DAY), now_ms: 102 * DAY + 10 };
        let buckets = s.over_time(range.since_ms.unwrap(), range.now_ms, &range).unwrap();
        // days 100,101,102 present
        assert_eq!(buckets.len(), 3);
        assert_eq!(buckets[0].start_ms, 100 * DAY);
        assert_eq!(buckets[0].count, 2);
        assert_eq!(buckets[1].start_ms, 101 * DAY);
        assert_eq!(buckets[1].count, 0);
        assert_eq!(buckets[2].count, 1);
    }

    #[test]
    fn over_time_all_time_switches_to_weekly_beyond_90_days() {
        let s = open_in_memory().unwrap();
        seed(&s, &[ev("x", 0, None), ev("y", 200 * DAY, None)]);
        let range = StatsRange { since_ms: None, now_ms: 200 * DAY };
        let buckets = s.over_time(i64::MIN, range.now_ms, &range).unwrap();
        // weekly buckets over ~200 days => ~29 buckets, far fewer than 200
        assert!(buckets.len() < 40, "weekly bucketing keeps the series short: {}", buckets.len());
        let total: i64 = buckets.iter().map(|b| b.count).sum();
        assert_eq!(total, 2);
    }
```

- [ ] **Step 2: Run to verify fail**

Run: `cargo test -p magpie-core over_time`
Expected: FAIL.

- [ ] **Step 3: Implement `bucket_ms` + `over_time`**

```rust
use std::collections::HashMap;

pub(crate) fn bucket_ms(range: &StatsRange, earliest_ms: i64) -> i64 {
    if range.since_ms.is_none() {
        let span_days = range.now_ms / DAY_MS - earliest_ms / DAY_MS;
        if span_days > 90 {
            return 7 * DAY_MS;
        }
    }
    DAY_MS
}

impl Store {
    pub(crate) fn over_time(&self, lo: i64, hi: i64, range: &StatsRange) -> Result<Vec<DayBucket>> {
        // earliest in-range event (for all-time start + weekly decision)
        let earliest: i64 = self
            .conn()
            .query_row(
                "SELECT MIN(copied_at_ms) FROM copy_events WHERE copied_at_ms BETWEEN ?1 AND ?2",
                [lo, hi],
                |r| r.get::<_, Option<i64>>(0),
            )?
            .unwrap_or(range.now_ms);

        let start_ms = range.since_ms.unwrap_or(earliest);
        let bucket = bucket_ms(range, earliest);

        let mut stmt = self.conn().prepare(
            "SELECT copied_at_ms / ?3 AS b, COUNT(*)
             FROM copy_events WHERE copied_at_ms BETWEEN ?1 AND ?2
             GROUP BY b",
        )?;
        let counts: HashMap<i64, i64> = stmt
            .query_map([lo, hi, bucket], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))?
            .collect::<Result<HashMap<i64, i64>>>()?;

        let start_b = start_ms.div_euclid(bucket);
        let end_b = range.now_ms.div_euclid(bucket);
        let mut out = Vec::new();
        for b in start_b..=end_b {
            out.push(DayBucket { start_ms: b * bucket, count: *counts.get(&b).unwrap_or(&0) });
        }
        Ok(out)
    }
}
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p magpie-core over_time`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/magpie-core/src/stats.rs
git commit -m "feat(core): over_time day/weekly buckets with zero-fill"
```

---

### Task 4: `per_app`, `by_type`, and `Store::stats` + public API

**Files:**
- Modify: `crates/magpie-core/src/stats.rs`
- Modify: `crates/magpie-core/src/lib.rs` (re-exports)
- Test: `stats.rs` tests module.

**Interfaces:**
- Consumes: all prior stats types + `Kind::from_str`.
- Produces:
  - `impl Store { pub(crate) fn per_app(&self, lo: i64, hi: i64, top_n: i64) -> Result<Vec<AppCount>> }` — `copy_events JOIN apps` (NULL apps excluded), grouped, count desc, limited.
  - `impl Store { pub(crate) fn by_type(&self, lo: i64, hi: i64) -> Result<Vec<KindCount>> }` — `copy_events JOIN entries` grouped by kind, count desc.
  - `impl Store { pub fn stats(&self, range: &StatsRange, top_n: i64) -> Result<Stats> }` — composes all five with `lo = range.since_ms.unwrap_or(i64::MIN)`, `hi = range.now_ms`.
  - `lib.rs` re-exports: `pub use stats::{AppCount, DayBucket, KindCount, MostCopied, Stats, StatsRange, Totals};`

- [ ] **Step 1: Add failing tests to `stats.rs` tests module**

```rust
    #[test]
    fn per_app_counts_exclude_null_apps() {
        let s = open_in_memory().unwrap();
        seed(&s, &[
            ev("a", 1, Some("Ghostty")),
            ev("b", 2, Some("Ghostty")),
            ev("c", 3, Some("Safari")),
            ev("d", 4, None), // excluded from per_app
        ]);
        let rows = s.per_app(i64::MIN, 1000, 10).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].name, "Ghostty");
        assert_eq!(rows[0].count, 2);
        assert_eq!(rows[1].name, "Safari");
    }

    #[test]
    fn by_type_counts_per_kind() {
        let s = open_in_memory().unwrap();
        seed(&s, &[
            ev("plain words", 1, None),
            ev("https://a.io", 2, None),
            ev("https://a.io", 3, None), // same link, 2 copies
        ]);
        let rows = s.by_type(i64::MIN, 1000).unwrap();
        // link has 2 copies, text has 1 -> link first
        assert_eq!(rows[0].kind.as_str(), "link");
        assert_eq!(rows[0].count, 2);
        assert_eq!(rows[1].kind.as_str(), "text");
    }

    #[test]
    fn stats_composes_all_series() {
        let s = open_in_memory().unwrap();
        seed(&s, &[ev("a", 1, Some("Ghostty")), ev("a", 2, Some("Ghostty")), ev("b", 3, None)]);
        let range = StatsRange { since_ms: None, now_ms: 1000 };
        let st = s.stats(&range, 10).unwrap();
        assert_eq!(st.totals.copies, 3);
        assert_eq!(st.most_copied[0].count, 2);
        assert_eq!(st.per_app[0].name, "Ghostty");
        assert!(!st.by_type.is_empty());
        assert!(!st.over_time.is_empty());
    }
```

- [ ] **Step 2: Run to verify fail**

Run: `cargo test -p magpie-core stats`
Expected: FAIL — `per_app`/`by_type`/`stats` not found.

- [ ] **Step 3: Implement `per_app`, `by_type`, `stats`**

```rust
impl Store {
    pub(crate) fn per_app(&self, lo: i64, hi: i64, top_n: i64) -> Result<Vec<AppCount>> {
        let mut stmt = self.conn().prepare(
            "SELECT a.display_name, COUNT(*) AS c
             FROM copy_events ce JOIN apps a ON a.id = ce.source_app_id
             WHERE ce.copied_at_ms BETWEEN ?1 AND ?2
             GROUP BY ce.source_app_id
             ORDER BY c DESC, a.display_name
             LIMIT ?3",
        )?;
        let rows = stmt.query_map([lo, hi, top_n], |r| {
            Ok(AppCount { name: r.get(0)?, count: r.get(1)? })
        })?;
        rows.collect()
    }

    pub(crate) fn by_type(&self, lo: i64, hi: i64) -> Result<Vec<KindCount>> {
        let mut stmt = self.conn().prepare(
            "SELECT e.kind, COUNT(*) AS c
             FROM copy_events ce JOIN entries e ON e.id = ce.entry_id
             WHERE ce.copied_at_ms BETWEEN ?1 AND ?2
             GROUP BY e.kind
             ORDER BY c DESC",
        )?;
        let rows = stmt.query_map([lo, hi], |r| {
            let kind_str: String = r.get(0)?;
            Ok(KindCount { kind: Kind::from_str(&kind_str).unwrap_or(Kind::Text), count: r.get(1)? })
        })?;
        rows.collect()
    }

    pub fn stats(&self, range: &StatsRange, top_n: i64) -> Result<Stats> {
        let lo = range.since_ms.unwrap_or(i64::MIN);
        let hi = range.now_ms;
        Ok(Stats {
            totals: self.totals(lo, hi)?,
            most_copied: self.most_copied(lo, hi, top_n)?,
            over_time: self.over_time(lo, hi, range)?,
            per_app: self.per_app(lo, hi, top_n)?,
            by_type: self.by_type(lo, hi)?,
        })
    }
}
```

- [ ] **Step 4: Add re-exports to `lib.rs`; run to verify pass**

Run: `cargo test -p magpie-core stats`
Expected: PASS (all stats unit tests).

- [ ] **Step 5: Commit**

```bash
git add crates/magpie-core/src/stats.rs crates/magpie-core/src/lib.rs
git commit -m "feat(core): per_app + by_type + Store::stats composition"
```

---

### Task 5: Core integration + property test for stats

**Files:**
- Create: `crates/magpie-core/tests/stats.rs`

**Interfaces:**
- Consumes: public `Store::stats`, `StatsRange`, and the stat structs re-exported from the crate root.
- Produces: integration coverage for a realistic seeded session + the partition invariant.

- [ ] **Step 1: Write the integration test file**

```rust
//! Integration tests for Store::stats over a seeded session.

use magpie_core::{open_in_memory, AppInfo, CaptureEvent, Content, ImageStore, StatsRange};

struct Noop;
impl ImageStore for Noop {
    fn put(&self, h: &str, _b: &[u8]) -> std::io::Result<String> { Ok(h.to_string()) }
}
fn ev(text: &str, ms: i64, app: Option<&str>) -> CaptureEvent {
    CaptureEvent {
        content: Content::Text(text.into()),
        source_app: app.map(|a| AppInfo { identifier: a.into(), display_name: a.into(), icon_path: None }),
        copied_at_ms: ms,
    }
}
const DAY: i64 = 86_400_000;

#[test]
fn realistic_session_stats() {
    let s = open_in_memory().unwrap();
    for e in [
        ev("docs/design.md", 100 * DAY + 1, Some("Ghostty")),
        ev("docs/design.md", 100 * DAY + 2, Some("Ghostty")), // re-copy
        ev("https://asana.com/x", 100 * DAY + 3, Some("Vivaldi")),
        ev("#1a2b3c", 101 * DAY + 4, Some("Figma")),
        ev("some note", 101 * DAY + 5, None),
    ] {
        s.ingest(&e, &Noop).unwrap();
    }
    let range = StatsRange { since_ms: Some(100 * DAY), now_ms: 101 * DAY + 100 };
    let st = s.stats(&range, 10).unwrap();

    assert_eq!(st.totals.copies, 5);
    assert_eq!(st.totals.unique_entries, 4);
    assert_eq!(st.totals.distinct_apps, 3);

    assert_eq!(st.most_copied[0].preview, "docs/design.md");
    assert_eq!(st.most_copied[0].count, 2);

    assert_eq!(st.per_app[0].name, "Ghostty"); // 2 copies
    // by_type includes link + color + text
    assert!(st.by_type.iter().any(|k| k.kind.as_str() == "link"));
    assert!(st.by_type.iter().any(|k| k.kind.as_str() == "color"));

    // over_time spans day 100 and 101
    assert_eq!(st.over_time.len(), 2);
}

#[test]
fn over_time_buckets_partition_the_copies() {
    // Sum of bucket counts == totals.copies for the same range.
    let s = open_in_memory().unwrap();
    for i in 0..40 {
        s.ingest(&ev(&format!("c{}", i % 5), (10 * DAY) + i * 3_600_000, None), &Noop).unwrap();
    }
    let range = StatsRange { since_ms: Some(9 * DAY), now_ms: 12 * DAY };
    let st = s.stats(&range, 10).unwrap();
    let bucket_sum: i64 = st.over_time.iter().map(|b| b.count).sum();
    assert_eq!(bucket_sum, st.totals.copies);
}

#[test]
fn empty_db_yields_zeroes() {
    let s = open_in_memory().unwrap();
    let st = s.stats(&StatsRange { since_ms: None, now_ms: 1000 }, 10).unwrap();
    assert_eq!(st.totals.copies, 0);
    assert!(st.most_copied.is_empty());
    assert!(st.per_app.is_empty());
    assert!(st.by_type.is_empty());
}
```

- [ ] **Step 2: Run to verify it compiles + passes**

Run: `cargo test -p magpie-core --test stats`
Expected: PASS (3 tests). If `realistic_session_stats` over_time length differs, confirm the seeded timestamps land on exactly two UTC days (they do: `100*DAY+..` and `101*DAY+..`).

- [ ] **Step 3: Commit**

```bash
git add crates/magpie-core/tests/stats.rs
git commit -m "test(core): stats integration + bucket-partition invariant"
```

---

### Task 6: App `stats_view` — `Bar`, `to_bars`, range mapping

**Files:**
- Create: `crates/magpie-app/src/stats_view.rs`
- Modify: `crates/magpie-app/src/lib.rs` (`pub mod stats_view;`)
- Test: `crates/magpie-app/tests/stats_view.rs`

**Interfaces:**
- Consumes: `magpie_core::StatsRange`.
- Produces:
  - `pub struct Bar { pub label: String, pub display: String, pub value_norm: f32 }`
  - `pub fn to_bars(items: &[(String, String, i64)]) -> Vec<Bar>` — normalizes `value` against the series max; `max == 0` → all `value_norm = 0.0`; preserves order, label, display.
  - `pub fn range_from_index(idx: i32, now_ms: i64) -> StatsRange` — `0 -> last 7 days`, `1 -> 30`, `2 -> 90`, anything else -> All (`since_ms: None`). Days map to `since_ms = Some(now_ms - days * 86_400_000)`.

- [ ] **Step 1: Write failing tests in `crates/magpie-app/tests/stats_view.rs`**

```rust
use magpie_app::stats_view::{range_from_index, to_bars, Bar};

fn item(label: &str, display: &str, v: i64) -> (String, String, i64) {
    (label.into(), display.into(), v)
}

#[test]
fn to_bars_normalizes_against_series_max() {
    let bars = to_bars(&[item("a", "A", 10), item("b", "B", 5), item("c", "C", 0)]);
    assert_eq!(bars.len(), 3);
    assert!((bars[0].value_norm - 1.0).abs() < 1e-6);
    assert!((bars[1].value_norm - 0.5).abs() < 1e-6);
    assert!((bars[2].value_norm - 0.0).abs() < 1e-6);
    assert_eq!(bars[0].label, "a");
    assert_eq!(bars[0].display, "A");
}

#[test]
fn to_bars_all_zero_is_safe() {
    let bars = to_bars(&[item("a", "A", 0), item("b", "B", 0)]);
    assert!(bars.iter().all(|b| b.value_norm == 0.0));
}

#[test]
fn to_bars_empty_is_empty() {
    let bars: Vec<Bar> = to_bars(&[]);
    assert!(bars.is_empty());
}

#[test]
fn range_from_index_maps_windows() {
    let now = 100 * 86_400_000i64;
    assert_eq!(range_from_index(0, now).since_ms, Some(now - 7 * 86_400_000));
    assert_eq!(range_from_index(1, now).since_ms, Some(now - 30 * 86_400_000));
    assert_eq!(range_from_index(2, now).since_ms, Some(now - 90 * 86_400_000));
    assert_eq!(range_from_index(3, now).since_ms, None); // All
    assert_eq!(range_from_index(3, now).now_ms, now);
}
```

- [ ] **Step 2: Add `pub mod stats_view;` to `crates/magpie-app/src/lib.rs`; run to verify fail**

Run: `cargo test -p magpie-app --test stats_view`
Expected: FAIL — module not found.

- [ ] **Step 3: Implement `stats_view.rs`**

```rust
use magpie_core::StatsRange;

pub struct Bar {
    pub label: String,
    pub display: String,
    pub value_norm: f32,
}

pub fn to_bars(items: &[(String, String, i64)]) -> Vec<Bar> {
    let max = items.iter().map(|(_, _, v)| *v).max().unwrap_or(0);
    items
        .iter()
        .map(|(label, display, v)| Bar {
            label: label.clone(),
            display: display.clone(),
            value_norm: if max > 0 { *v as f32 / max as f32 } else { 0.0 },
        })
        .collect()
}

const DAY_MS: i64 = 86_400_000;

pub fn range_from_index(idx: i32, now_ms: i64) -> StatsRange {
    let since_ms = match idx {
        0 => Some(now_ms - 7 * DAY_MS),
        1 => Some(now_ms - 30 * DAY_MS),
        2 => Some(now_ms - 90 * DAY_MS),
        _ => None,
    };
    StatsRange { since_ms, now_ms }
}
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p magpie-app --test stats_view`
Expected: PASS (4 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/magpie-app/src/stats_view.rs crates/magpie-app/src/lib.rs crates/magpie-app/tests/stats_view.rs
git commit -m "feat(app): stats_view Bar/to_bars + range mapping"
```

---

### Task 7: Slint stats view markup + view toggle

**Files:**
- Modify: `crates/magpie-app/ui/launcher.slint`

**Interfaces:**
- Produces additions to `LauncherWindow`:
  - `struct Bar { label: string, display: string, value_norm: float }`
  - `in-out property <string> view: "list";` (`"list"` | `"stats"`)
  - `in property <[Bar]> over-time-bars;` `in property <[Bar]> most-copied-bars;` `in property <[Bar]> per-app-bars;` `in property <[Bar]> by-type-bars;`
  - `in property <string> totals-line;` (e.g. "1,234 copies · 210 entries · 5 apps")
  - `in-out property <int> range-index: 1;` (default 30 days)
  - `callback stats-range-changed(int);` `callback toggle-view();`
  - Existing list UI is shown when `view == "list"`, the new stats layout when `view == "stats"`. A header button calls `toggle-view()`. The stats layout: a range selector (four buttons setting `range-index` + calling `stats-range-changed`), the totals line, and four labeled bar blocks. Horizontal bars: `Rectangle { width: track.width * bar.value-norm; }`. Vertical bars for over-time: `Rectangle { height: track.height * bar.value-norm; }`.

> UI markup renders only with a display; this task's gate is that it **compiles** via `slint-build`. Visual check is manual.

- [ ] **Step 1: Add the `Bar` struct and properties/callbacks to `LauncherWindow`**

Insert near the top (after the existing `EntryRow` struct):

```slint
struct Bar {
    label: string,
    display: string,
    value_norm: float,
}
```

Add these to the `LauncherWindow` property block (alongside existing `entries`/`query`):

```slint
    in-out property <string> view: "list";
    in property <[Bar]> over-time-bars;
    in property <[Bar]> most-copied-bars;
    in property <[Bar]> per-app-bars;
    in property <[Bar]> by-type-bars;
    in property <string> totals-line;
    in-out property <int> range-index: 1;
    callback stats-range-changed(int);
    callback toggle-view();
```

- [ ] **Step 2: Wrap the existing list content and add the stats view**

Wrap the current `VerticalLayout { ... list + detail ... }` body so it only shows in list mode, and add a sibling stats view. A reusable horizontal-bar row component (define at file top, before `LauncherWindow`):

```slint
component HBar inherits Rectangle {
    in property <string> label;
    in property <string> display;
    in property <float> value_norm;
    height: 22px;
    HorizontalLayout {
        spacing: 8px;
        Text { text: label; color: #d0d0d4; font-size: 11px; width: 180px; overflow: elide; }
        track := Rectangle {
            background: #2a2a30;
            border-radius: 3px;
            Rectangle {
                x: 0; y: 0; height: parent.height;
                width: track.width * value_norm;
                background: #6a8cff;
                border-radius: 3px;
            }
        }
        Text { text: display; color: #9a9aa0; font-size: 11px; width: 48px; }
    }
}
```

Then inside `LauncherWindow`, structure the top-level as:

```slint
    // header with a view toggle
    // (place a small button that calls toggle-view; label shows the *other* mode)
    // list view
    if root.view == "list" : /* existing VerticalLayout with search + list + detail */ {}
    // stats view
    if root.view == "stats" : VerticalLayout {
        padding: 12px;
        spacing: 10px;
        HorizontalLayout {
            spacing: 6px;
            for label[i] in ["7d", "30d", "90d", "All"] : Rectangle {
                background: i == root.range-index ? #3a3a40 : #202024;
                border-radius: 6px;
                width: 56px; height: 26px;
                TouchArea { clicked => { root.range-index = i; root.stats-range-changed(i); } }
                Text { text: label; color: white; horizontal-alignment: center; vertical-alignment: center; }
            }
        }
        Text { text: root.totals-line; color: white; font-size: 13px; }

        Text { text: "Copies over time"; color: #9a9aa0; font-size: 11px; }
        HorizontalLayout {
            alignment: start;
            spacing: 2px;
            height: 60px;
            for bar in root.over-time-bars : Rectangle {
                width: 8px;
                background: transparent;
                Rectangle {
                    y: parent.height * (1.0 - bar.value_norm);
                    height: parent.height * bar.value_norm;
                    width: parent.width;
                    background: #6a8cff;
                    border-radius: 2px;
                }
            }
        }

        Text { text: "Most copied"; color: #9a9aa0; font-size: 11px; }
        for bar in root.most-copied-bars : HBar { label: bar.label; display: bar.display; value_norm: bar.value_norm; }

        Text { text: "By app"; color: #9a9aa0; font-size: 11px; }
        for bar in root.per-app-bars : HBar { label: bar.label; display: bar.display; value_norm: bar.value_norm; }

        Text { text: "By type"; color: #9a9aa0; font-size: 11px; }
        for bar in root.by-type-bars : HBar { label: bar.label; display: bar.display; value_norm: bar.value_norm; }
    }
```

Add a small toggle affordance in both views' headers (a `Rectangle` + `TouchArea { clicked => { root.toggle-view(); } }` with text "Stats" in list mode and "← Back" in stats mode). Keep existing list callbacks intact.

- [ ] **Step 3: Compile (Slint codegen)**

Run: `cargo build -p magpie-app`
Expected: compiles. Fix any Slint syntax errors the compiler reports (the generated `Bar` type name and `set_over_time_bars` etc. become available to Rust).

- [ ] **Step 4: Manual verification**

Temporarily set `main` to open the window with `view = "stats"` and a couple of hand-built `Bar` models; run `cargo run -p magpie-app`; confirm the range buttons, totals line, and four bar blocks render. Revert. Record in your report.

- [ ] **Step 5: Commit**

```bash
git add crates/magpie-app/ui/launcher.slint
git commit -m "feat(app): Slint stats view (bars + range selector + toggle)"
```

---

### Task 8: Runtime wiring — refresh_stats, toggle, range callback

**Files:**
- Modify: `crates/magpie-app/src/runtime.rs`

**Interfaces:**
- Consumes: `magpie_core::{StatsRange, Stats}`, `Store::stats`, `stats_view::{to_bars, range_from_index, Bar}`, generated Slint `Bar` + setters (`set_over_time_bars`, `set_most_copied_bars`, `set_per_app_bars`, `set_by_type_bars`, `set_totals_line`, `set_view`, `on_stats_range_changed`, `on_toggle_view`).
- Produces:
  - `fn stats_bars(...)` conversions from core `Stats` into Slint `Bar` models via `stats_view::to_bars`.
  - `fn refresh_stats(ui: &LauncherWindow, state: &AppState, range_index: i32)` — build `StatsRange` via `range_from_index(range_index, now_ms())`, call `state.store.lock().stats(&range, 10)`, convert each series to bars, set totals line + 4 models.
  - In `start()`: wire `ui.on_toggle_view` (flip `view` between "list"/"stats"; on entering stats, call `refresh_stats` with the current `range-index`), and `ui.on_stats_range_changed` (call `refresh_stats`).

- [ ] **Step 1: Add the conversion + refresh_stats to `runtime.rs`**

```rust
use magpie_app::stats_view::{range_from_index, to_bars};
use magpie_core::Stats;

fn to_slint_bars(items: &[(String, String, i64)]) -> slint::ModelRc<Bar> {
    let bars: Vec<Bar> = to_bars(items)
        .into_iter()
        .map(|b| Bar {
            label: b.label.into(),
            display: b.display.into(),
            value_norm: b.value_norm,
        })
        .collect();
    slint::ModelRc::new(slint::VecModel::from(bars))
}

fn refresh_stats(ui: &LauncherWindow, state: &AppState, range_index: i32) {
    let range = range_from_index(range_index, now_ms());
    let stats: Stats = match state.store.lock() {
        Ok(store) => store.stats(&range, 10).unwrap_or_else(|_| empty_stats(&range)),
        Err(_) => empty_stats(&range),
    };

    let ot: Vec<(String, String, i64)> = stats
        .over_time
        .iter()
        .map(|b| (String::new(), String::new(), b.count))
        .collect();
    let mc: Vec<(String, String, i64)> = stats
        .most_copied
        .iter()
        .map(|m| (truncate(&m.preview, 40), m.count.to_string(), m.count))
        .collect();
    let pa: Vec<(String, String, i64)> = stats
        .per_app
        .iter()
        .map(|a| (a.name.clone(), a.count.to_string(), a.count))
        .collect();
    let bt: Vec<(String, String, i64)> = stats
        .by_type
        .iter()
        .map(|k| (k.kind.as_str().to_string(), k.count.to_string(), k.count))
        .collect();

    ui.set_over_time_bars(to_slint_bars(&ot));
    ui.set_most_copied_bars(to_slint_bars(&mc));
    ui.set_per_app_bars(to_slint_bars(&pa));
    ui.set_by_type_bars(to_slint_bars(&bt));
    ui.set_totals_line(SharedString::from(format!(
        "{} copies · {} entries · {} apps",
        stats.totals.copies, stats.totals.unique_entries, stats.totals.distinct_apps
    )));
}

fn truncate(s: &str, n: usize) -> String {
    let one_line = s.lines().next().unwrap_or("");
    one_line.chars().take(n).collect()
}

fn empty_stats(range: &magpie_core::StatsRange) -> Stats {
    let _ = range;
    Stats {
        totals: magpie_core::Totals { copies: 0, unique_entries: 0, distinct_apps: 0 },
        most_copied: Vec::new(),
        over_time: Vec::new(),
        per_app: Vec::new(),
        by_type: Vec::new(),
    }
}
```

- [ ] **Step 2: Wire the callbacks in `start()`** (after the existing `on_search_changed`/`on_activate` blocks)

```rust
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_toggle_view(move || {
            if let Some(ui) = w.upgrade() {
                let to_stats = ui.get_view() != "stats";
                ui.set_view(SharedString::from(if to_stats { "stats" } else { "list" }));
                if to_stats {
                    refresh_stats(&ui, &s, ui.get_range_index());
                }
            }
        });
    }
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_stats_range_changed(move |idx| {
            if let Some(ui) = w.upgrade() {
                refresh_stats(&ui, &s, idx);
            }
        });
    }
```

- [ ] **Step 3: Compile**

Run: `cargo build -p magpie-app`
Expected: compiles. Adjust generated setter/getter names to match Slint's codegen (kebab `over-time-bars` → `set_over_time_bars`, `range-index` → `get_range_index`).

- [ ] **Step 4: Manual verification**

Run `cargo run -p magpie-app`; copy several things from different apps; open the launcher; click the **Stats** toggle; confirm totals + four bar blocks populate; switch ranges (7/30/90/All) and confirm the charts update; toggle back to the list. Record in your report.

- [ ] **Step 5: Commit**

```bash
git add crates/magpie-app/src/runtime.rs
git commit -m "feat(app): wire stats view — refresh_stats, toggle, range callback"
```

---

## Self-Review

**Spec coverage:**
- Totals (copies/unique/apps) → Task 1. ✅
- Most-copied → Task 2. ✅
- Over-time + selectable range + zero-fill + weekly all-time fallback → Tasks 3, 6 (range), 7/8 (selector). ✅
- Per-app → Task 4. By-type + totals → Tasks 4, 8. ✅
- Stats mode in launcher (toggle + range selector) → Tasks 7, 8. ✅
- Hand-drawn bars, no dependency → Task 7. ✅
- Error handling (empty view on error, empty DB) → Task 8 (`unwrap_or_else(empty_stats)`), Task 5 (empty-DB test). ✅
- `to_bars` max==0 safe / empty → Task 6. ✅
- Property: Σ over_time == totals.copies → Task 5. ✅
- Note: the spec mentioned an optional `Cmd/Ctrl+T` hotkey; v1 ships the **header-button toggle** (Task 7/8). A key shortcut is a thin follow-up and is intentionally omitted to avoid unverified Slint key-event handling.

**Placeholder scan:** UI task (7) uses a `## Manual verification` block by necessity (no display in CI) with concrete markup; all core + helper logic (1–6) is fully TDD'd. No TODOs.

**Type consistency:** `StatsRange`, `Totals`, `MostCopied`, `DayBucket`, `AppCount`, `KindCount`, `Stats`, `Store::{totals,most_copied,over_time,per_app,by_type,stats}`, `bucket_ms`, `Bar`, `to_bars`, `range_from_index`, and the Slint `Bar` + setters are named consistently across tasks. `top_n` is an `i64` call param throughout. `lo/hi` derived identically in `stats` and tests.

## Notes

- `Store::conn()` is `pub(crate)` (added in the core plan) — all stats queries use it.
- The generated Slint `Bar` struct lives in the bin crate (`crate::Bar`); the app `stats_view::Bar` is the pure/tested twin. `runtime.rs` converts between them.
