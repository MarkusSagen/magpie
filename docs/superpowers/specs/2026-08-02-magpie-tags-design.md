# Magpie Tags — Design

**Date:** 2026-08-02
**Status:** Approved (brainstorm complete)
**Phase:** 1 (fifth sub-project)

## Summary

Freeform string tags on entries: add/remove tags, see them as chips on the
selected entry, and filter the list to a tag. Tags are normalized (trim +
lowercase) strings in a join table; filtering reuses the existing search via a new
`SearchQuery.tag` field. Additive schema (no migration); tag rows are cleaned up
when their entry is pruned by retention.

## Goals & Non-Goals

**Goals**
- `entry_tags(entry_id, tag)` join table.
- Core: `add_tag`, `remove_tag`, `tags_of`, `all_tags`, and a `SearchQuery.tag`
  filter.
- UI: tag chips on the selected entry (click a chip to remove), an add-tag input,
  and a tag-filter strip (click a tag to filter the list; click again to clear).
- Retention deletes an entry's `entry_tags` when the entry is deleted.

**Non-Goals (v1)**
- A managed tag entity (rename/color/merge tags) — tags are plain strings.
- Multi-tag AND/OR filtering — the filter is a single tag at a time.
- Tag autocomplete.

## Data Model

Add to `crates/magpie-core/src/schema.sql` (runs via `CREATE TABLE IF NOT EXISTS`
on open → **no migration**):

```sql
CREATE TABLE IF NOT EXISTS entry_tags (
  entry_id INTEGER NOT NULL,
  tag      TEXT NOT NULL,
  PRIMARY KEY (entry_id, tag)
);
CREATE INDEX IF NOT EXISTS idx_entry_tags_tag ON entry_tags(tag);
```

Tags are normalized to `tag.trim().to_lowercase()` before storage/query. Empty
after normalization → no-op.

## Core: `magpie_core::tags` (methods on `Store`)

```rust
impl Store {
    pub fn add_tag(&self, entry_id: i64, tag: &str) -> Result<()>;      // INSERT OR IGNORE
    pub fn remove_tag(&self, entry_id: i64, tag: &str) -> Result<()>;
    pub fn tags_of(&self, entry_id: i64) -> Result<Vec<String>>;        // sorted
    pub fn all_tags(&self) -> Result<Vec<String>>;                      // distinct, sorted
}
```

- `add_tag`/`remove_tag` normalize the tag; empty → `Ok(())` no-op.
- **Search filter:** `SearchQuery` gains `pub tag: Option<String>`. `default_query`
  sets it `None`. When `Some(t)` (normalized), `search` adds the clause
  `e.id IN (SELECT entry_id FROM entry_tags WHERE tag = ?)`, composed with the
  other filters in the existing clause/param builder. Fuzzy/regex paths honor it
  too because they run through `candidates()` (which calls `search` with text
  blanked but filters intact).
- **Retention cleanup:** `retention::delete_entries` also runs
  `DELETE FROM entry_tags WHERE entry_id IN (victims)` inside its transaction, so
  no dangling tag rows.

## UI (Slint)

- `EntryRow` gains `tags: string` — a comma-joined list of the entry's tags (kept
  simple: rendered as a small caption; per-chip removal happens against the
  *selected* entry via its own controls).
- Detail pane (view mode), for the selected entry:
  - A **tag chips row**: each of the selected entry's tags as a chip; clicking a
    chip calls `remove-tag(selected, tag)`.
  - An **add-tag input**: a `LineEdit` (`add-tag-text`) + **Add** button →
    `add-tag(selected, add-tag-text)`.
- A **tag-filter strip** above the list (list view): `all-tags` rendered as
  clickable chips; the active one highlighted. Clicking sets `tag-filter`; clicking
  the active one clears it. Bound to `in-out property <string> tag-filter`.

Properties/callbacks on `LauncherWindow`:
`in property <[string]> selected-tags;` `in property <[string]> all-tags;`
`in-out property <string> add-tag-text;` `in-out property <string> tag-filter;`
`callback add-tag(int, string);` `callback remove-tag(int, string);`
`callback set-tag-filter(string);`

## Runtime

- `refresh`: after computing results, fetch `all_tags()` → set `all-tags`; fetch
  `tags_of(selected_entry_id)` → set `selected-tags`; annotate each `EntryRow.tags`
  from a `entry_id → Vec<tag>` map (one `SELECT entry_id, tag FROM entry_tags`).
- `to_query`/`current_results`: thread the UI `tag-filter` (empty → `None`) into
  `SearchQuery.tag`.
- `on_add_tag(i, tag)`: resolve entry from results, `store.add_tag(id, &tag)`,
  clear `add-tag-text`, refresh.
- `on_remove_tag(i, tag)`: `store.remove_tag(id, &tag)`, refresh.
- `on_set_tag_filter(tag)`: if `tag == current tag-filter`, clear; else set; then
  refresh. Guard re-entrant store locks (compute results before locking).

## Data Flow

```
add:    add-tag(i, t)  -> store.add_tag(id, t)      -> refresh (chips update)
remove: remove-tag(i,t)-> store.remove_tag(id, t)   -> refresh
filter: set-tag-filter(t) -> tag-filter=t -> to_query(tag=Some(t)) -> search -> list
```

## Error Handling

- Store lock / SQL errors in callbacks are swallowed (logged), never crash.
- Empty/whitespace tag → no-op (guarded in core normalization).
- Filtering by a tag no entry has → empty list (expected).

## Testing

**core (`tags.rs` unit + `tests/tags.rs`):**
- `add_tag` normalizes (`"  Work "` → `work`) and is idempotent (`INSERT OR IGNORE`).
- `remove_tag` removes; `tags_of` sorted; `all_tags` distinct + sorted.
- `SearchQuery.tag` returns only entries with that tag; combines with text/type.
- retention prunes `entry_tags` for deleted entries (no dangling).

**app (`tests/tags.rs`):**
- `to_query` maps a non-empty `tag-filter` to `SearchQuery.tag = Some(normalized)`,
  empty → `None`.

**run-verify:** launch the binary (`timeout 8 ./target/debug/magpie`, exit 124),
add a tag to an entry, filter by it.

## File Structure

- Modify: `crates/magpie-core/src/schema.sql` (add `entry_tags`).
- Create: `crates/magpie-core/src/tags.rs` (`add_tag`/`remove_tag`/`tags_of`/`all_tags`).
- Modify: `crates/magpie-core/src/lib.rs` (`pub mod tags;`).
- Modify: `crates/magpie-core/src/search.rs` (`SearchQuery.tag` + clause).
- Modify: `crates/magpie-core/src/retention.rs` (`delete_entries` also deletes `entry_tags`).
- Create: `crates/magpie-core/tests/tags.rs`.
- Modify: `crates/magpie-app/src/viewmodel.rs` (`UiState.tag`? no — tag comes from a Slint prop; `to_query` gains a tag param) — see note.
- Modify: `crates/magpie-app/ui/launcher.slint` (chips, add input, filter strip, props/callbacks).
- Modify: `crates/magpie-app/src/runtime.rs` (annotate tags, wire callbacks, thread tag filter).

Note on `to_query`: keep `to_query(ui, now)` unchanged and set `q.tag` from the
Slint `tag-filter` in `current_results`/runtime (the tag filter lives in the
window state, not `UiState`), OR add `tag: Option<String>` to `UiState`. **Chosen:**
add `pub tag: Option<String>` to `UiState` (default `None`); `to_query` maps it to
`SearchQuery.tag`; runtime sets `state.ui.tag` from the `tag-filter` property before
querying. This keeps all query inputs in `UiState` (consistent with text/type/time).

## Risks / Notes

- **`SearchQuery` gains a field** — every construction site uses `default_query()`
  or `..default_query()`, so adding `tag: None` to `default_query` is the only
  required change; existing tests keep compiling.
- Tag chips per-row in the list are shown only as a compact caption
  (`EntryRow.tags`); full chip interaction is on the selected entry in the detail
  pane, keeping the list rows light.

## Addendum — 2026-08-03: tag row layout

The detail-pane **tag chips row** is a **single row** (never wraps to multiple
lines). When the chips exceed the available width, the row scrolls **horizontally**
rather than growing taller:

- Implement as a Slint horizontal `ListView`/`Flickable` of chips with a fixed
  height, `viewport-width` = sum of chip widths, clipped to the pane width.
- Show a thin horizontal scrollbar only when overflowing (`ScrollBarPolicy.as-needed`,
  matching the flush, minimal scrollbar styling used elsewhere in the app).
- The **add-tag input** stays fixed (does not scroll with the chips) so it's always
  reachable — placed adjacent to the scrolling chip row, not inside it.
- Keyboard access to individual chips (focus/select tag #N) is deferred to the
  forthcoming **keyboard-navigation spec**, not this feature.
