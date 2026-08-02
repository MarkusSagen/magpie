# Magpie Analytics View — Design

**Date:** 2026-08-02
**Status:** Approved (brainstorm complete)
**Phase:** 1 (first sub-project)

## Summary

A **Stats** mode inside the Magpie launcher that visualizes clipboard usage from
data Magpie already records — the `entries` and `copy_events` tables. It answers
"what do I copy most, from where, of what type, and how has my copying trended
over time?" — the "track & visualize how much I copy" goal from the original
brief. No schema changes; this is a new read-only aggregate layer plus a
dependency-free bar-chart UI.

## Goals & Non-Goals

**Goals**
- Headline totals: total copies, unique entries, distinct source apps (in range).
- Most-copied entries (top-N).
- Copies over time (per-day buckets), with a selectable range: 7 / 30 / 90 days / All.
- Per-app breakdown (top-N).
- By-content-type breakdown (text/link/color/email/rtf/html/image/file).
- Live in the existing launcher window as a toggled "Stats" view.

**Non-Goals (v1)**
- Line/area/curve charts (bars only — a `Path` line chart is a possible follow-up).
- Interactive drill-down (clicking a bar to filter the list).
- Hourly granularity, custom date pickers, CSV export.
- Any new dependency (no charting crate).

## Chart Rendering Approach

**Hand-drawn Slint primitives** (chosen over `plotters`→PNG and Slint `Path`
line charts). The Rust side computes normalized values (`0.0..=1.0` against each
series' own max) plus labels/display strings; Slint renders bars as `Rectangle`s
from a model. Horizontal bars for most-copied / per-app / by-type; a row of
vertical bars for copies-over-time. Rationale: dependency-free, crisp at any
DPI/size, Slint-native, minimal — matching the project's footprint ethos. Curves
and tooltips are deferred.

## Architecture

### Core: new `stats` module (pure, testable)

`crates/magpie-core/src/stats.rs`, exposed as `Store::stats(range) -> Stats`.
All queries are read-only over existing tables; time is injected.

Types:

```rust
pub struct StatsRange {
    pub since_ms: Option<i64>,  // None = all-time
    pub now_ms: i64,            // upper bound + bucketing reference (injected)
}

pub struct Totals {
    pub copies: i64,        // count of copy_events in range
    pub unique_entries: i64,// distinct entries copied in range
    pub distinct_apps: i64, // distinct source apps in range
}

pub struct MostCopied {
    pub entry_id: i64,
    pub preview: String,
    pub kind: Kind,
    pub count: i64,
}

pub struct DayBucket {
    pub start_ms: i64,  // UTC midnight of the day
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
    pub most_copied: Vec<MostCopied>,   // top-N by count desc
    pub over_time: Vec<DayBucket>,      // one per day across range, zero-filled
    pub per_app: Vec<AppCount>,         // top-N by count desc
    pub by_type: Vec<KindCount>,        // all present kinds, count desc
}
```

Query notes:
- **Range window:** `copy_events.copied_at_ms >= since_ms` (when `Some`) and
  `<= now_ms`. For `entries`-derived series in range, count via joins to
  `copy_events` so "in range" is consistent everywhere.
- **totals.copies** = `COUNT(*)` of `copy_events` in range. **unique_entries** =
  `COUNT(DISTINCT entry_id)`. **distinct_apps** = `COUNT(DISTINCT source_app_id)`
  (NULL app excluded).
- **most_copied**: `SELECT entry_id, COUNT(*) c FROM copy_events [range] GROUP BY
  entry_id ORDER BY c DESC LIMIT N`, then join `entries` for preview/kind.
- **over_time**: `SELECT copied_at_ms/DAY AS day, COUNT(*) FROM copy_events
  [range] GROUP BY day`, then **zero-fill** every day from the first day of the
  range (`since_ms` or the earliest event) through `now_ms`'s day so gaps render.
  Cap the number of buckets defensively (e.g. all-time collapses to a sane max —
  see Risks).
- **per_app**: `copy_events JOIN apps GROUP BY source_app_id ORDER BY count DESC
  LIMIT N`.
- **by_type**: `copy_events JOIN entries GROUP BY entries.kind ORDER BY count DESC`.

`N` (top-N) is a constant (e.g. 10) — a parameter on the query call, not config.

### App/UI: Stats view in the launcher

- `LauncherWindow` gains `in property <string> view` (`"list"` | `"stats"`),
  toggled by a header button + hotkey `Cmd/Ctrl+T`; `Escape`/toggle returns to
  list.
- A pure Rust helper `to_bars(labels_values, max) -> Vec<Bar>` where
  `Bar { label: string, display: string, value_norm: f32 }` normalizes a series;
  **unit-tested**. `runtime::refresh_stats(range)` locks the store, calls
  `Store::stats`, builds four `Bar` models + a totals struct, sets them on the
  window.
- Slint `stats` view: a range selector (7/30/90/All → `stats-range-changed(int)`),
  a totals row, and four labeled bar-chart blocks. Bars are `Rectangle`s sized by
  `value_norm`. Kept thin; all math is in Rust.

## Data Flow

```
user toggles Stats / picks range
  -> stats-range-changed(range_idx)  (Slint callback)
  -> runtime::refresh_stats: StatsRange{since_ms, now_ms=now()} 
       -> Store::stats(range, N)               [core, read-only SQL]
       -> to_bars(...) per series              [pure, tested]
       -> set totals + 4 bar models on window  [Slint]
```

No new threads, no schema change, no new dependency.

## Error Handling

- `Store::stats` returns `Result` (rusqlite errors propagate); the UI shows an
  empty stats view on error (never crashes) — consistent with `unwrap_or_default`
  used elsewhere in `runtime`.
- Empty database → all series empty, totals zero; the view renders "no data yet".
- `to_bars` with `max == 0` yields `value_norm = 0.0` for all bars (no divide-by-zero).

## Testing

**core (unit + integration):**
- totals (copies / unique / distinct apps) over a seeded session.
- most_copied ordering + top-N cap + preview/kind correctness.
- over_time day-bucketing + zero-fill (empty days present with count 0) + range
  filtering (events outside `since_ms` excluded).
- per_app counts + ordering; NULL-app events excluded from per_app but counted in
  totals.copies.
- by_type counts across kinds.
- empty-DB edge (all zero/empty).
- **property:** Σ `over_time[i].count` == `totals.copies` for any seeded set +
  range (the buckets partition the in-range events).

**app:**
- `to_bars` normalization: correct `value_norm` ratios, `max == 0` safe, empty
  input → empty output, labels/displays preserved.

**UI:** Slint stats view is thin/manual (no display in CI); logic lives in tested
Rust helpers.

## File Structure

- Create: `crates/magpie-core/src/stats.rs` (module + `Store::stats`).
- Modify: `crates/magpie-core/src/lib.rs` (`pub mod stats;` + re-exports).
- Create: `crates/magpie-core/tests/stats.rs` (integration).
- Create: `crates/magpie-app/src/stats_view.rs` (`Bar`, `to_bars`, range mapping).
- Modify: `crates/magpie-app/src/lib.rs` (`pub mod stats_view;`).
- Modify: `crates/magpie-app/ui/launcher.slint` (stats view + range selector + view toggle).
- Modify: `crates/magpie-app/src/runtime.rs` (`refresh_stats`, view toggle, hotkey/callback wiring).
- Create: `crates/magpie-app/tests/stats_view.rs` (unit tests for `to_bars`).

## Risks / Open Details

- **All-time over_time bucket explosion:** an old DB could span thousands of days.
  Mitigation: when range is All, bucket from the earliest event but cap the series
  (e.g. if > 90 buckets, widen to weekly buckets, or just cap to the last 90 days
  of activity for the chart). v1: **All-time uses weekly buckets when day count
  exceeds 90**; totals/most-copied/per-app/by-type remain true all-time. Documented
  so the chart stays readable.
- **Preview length in most-copied:** reuse the stored `preview_text` (already ≤200
  chars); the bar label truncates to ~40 for display.
