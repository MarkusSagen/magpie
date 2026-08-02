# Magpie UI Round 2 — Raycast-style Redesign

**Date:** 2026-08-02 · **Status:** Approved · **Phase:** 1

## Summary

Full visual + interaction redesign of the launcher, built on Round 1. A
keyboard-first "command bar": type anywhere to filter, arrow keys navigate,
Enter pastes; a two-pane body (type-icon list left, full preview + metadata
right); a type-filter row; a bottom action bar with keyboard hints; and a ⌘K
actions palette holding the secondary actions. Built as one combined plan.

## Window & Layout

Keep the native macOS window (title bar, movable). Inside, a vertical stack:
1. **Search row** — magnifier glyph + the query text (custom-drawn, see Keyboard).
2. **Type-filter row** — `All · Text · Link · Color · Image · File` + a Sort control.
3. **Body** — two panes: list (left, min 340px) · preview + metadata (right).
4. **Action bar** — full width, bottom: source app (left) + key hints (right).

The existing **Stats** view stays reachable (a small control in the type-filter
row); its internals are unchanged.

## List Rows

Fixed height (52px) so keyboard scroll-follow is deterministic. Each row:
- **Type glyph** (left): 🔗 link · 🎨 color · ✉️ email · 🖼️ image · 📁 file ·
  📄 text/rtf/html. Pure `type_glyph(kind) -> &'static str`.
- **Title**: first non-empty line, elided (existing `preview_title`).
- **Badge**: `N lines` for multiline (Round 1 `line_badge`).
- **Subtitle**: `source · relative-time` (e.g. `Terminal · 2m`).
- **Selected** row: rounded accent bar + highlighted background. `merged` rows
  keep their distinct tint.

## Right Pane

- **Preview** (top, stretches): the Round 1 scrollable, word-wrapped, full text
  bound to `entries[selected].full`.
- **Metadata block** (below): label/value rows — Application, Type, Copied N
  times, Last used (relative), Size (`N chars · N lines`), Slot (`⌘N` or `—`).
  Bound to the selected row's fields so it updates on keyboard nav with no Rust
  round-trip.

## Type Filter + Sort

`UiState.type_filter` and `UiState.sort` already exist and map into the query;
this only surfaces them.
- `type_filter_from_index(i32) -> TypeFilter`: 0 All · 1 Text · 2 Link · 3 Color
  · 4 Image · 5 File (Email folds under All). Pure, tested.
- `sort_from_index(i32) -> Sort`: 0 Recency · 1 Most copied. Pure, tested.
- New callbacks `set-type-filter(int)` / `set-sort(int)` update `UiState` and
  `refresh`.

## Bottom Action Bar

Left: the selected entry's source app. Right: static hints
`⏎ Paste · ⌘C Copy · ⌘K Actions`.

## Keyboard Model (command bar)

A root `FocusScope` (`fs`) owns the keyboard; the window `forward-focus: fs`, and
`fs.focus()` is re-asserted when the window shows and when edit/palette modes
close. A `mode` property gates routing: `"list"` (default), `"edit"`, `"actions"`.

**List mode** (`key-pressed`):
- printable char (single char, no meta/ctrl) → `query += text`; `search-changed`.
- `Key.Backspace` → drop last char; `search-changed`.
- `Key.UpArrow` / `Key.DownArrow` → move `selected` (clamped); scroll-follow.
- `Key.Return` → `activate(selected)` (paste).
- `Key.Escape` → if `query != ""` clear it (+`search-changed`), else `hide-window()`.
- meta + `c` → `copy-only(selected)`; meta + `k` → open actions (`mode = "actions"`);
  meta + `e` → `start-edit`; meta + `n` → `new-snippet`; meta + `m` →
  `toggle-merge(selected)`; meta + `v` → `paste-into-search()` (Rust reads the
  clipboard, appends to query); meta + digit `1..9` → `activate-nth(n)` (paste the
  Nth visible result).

**Scroll-follow**: rows are fixed height `H=52px`. In a `changed selected`
handler, adjust `list.viewport-y` so the selected row stays visible:
`if selected*H < -vy: vy = -selected*H;` `else if (selected+1)*H > -vy +
list.height: vy = -((selected+1)*H - list.height)`.

**Edit mode**: entering edit focuses the native `TextEdit` (so typing/caret are
native); `mode = "edit"` so `fs` ignores text keys even if it holds focus. Save /
Cancel set `mode = "list"` and re-focus `fs`.

**Search field**: custom-drawn — magnifier glyph + `query` `Text` + a thin caret
rectangle + placeholder when empty. No native caret/selection (accepted
trade-off); `⌘V` handled explicitly.

## ⌘K Actions Palette

An overlay (`mode == "actions"`): a centered panel over a dimmed backdrop with a
keyboard-navigable action list. `↑/↓` move `action-selected`, `⏎` runs it, `Esc`
closes (`mode = "list"`). Actions (each shows its shortcut):
- **Edit** (⌘E) → `start-edit(selected)`
- **New snippet** (⌘N) → `new-snippet()`
- **Pin / Unpin** (⌘P) → `toggle-pin(selected)` (core `set_pinned`)
- **Add to merge / Remove** (⌘M) → `toggle-merge(selected)`
- **Assign to slot** → while the palette is open, pressing a digit `1..9` calls
  `assign-slot(selected, n)` and closes; a visible `1 2 3 … 9` row affords clicks.
- **Clear slot** → `assign-slot(selected, currentSlot)` toggles it off (existing
  toggle semantics) when slotted.
- **Add tag** → focuses a small native tag `LineEdit` in the palette (`mode`
  stays "actions" but text goes to the field); Enter calls `add-tag(selected,
  text)`. **Remove tag** → the selected entry's tag chips render in the palette;
  clicking one calls `remove-tag(selected, tag)`.

Tag filter chips (`set-tag-filter`) stay in the main view under the type row.

## Architecture

- **Pure, tested helpers** (app): `relative_time(then_ms, now_ms) -> String`
  ("just now", "2m", "3h", "yesterday", "5d", else `YYYY-MM-DD`); `type_glyph`;
  `type_filter_from_index`; `sort_from_index`.
- **Core helper** (tested): `Store::app_name_pairs() -> Result<Vec<(i64,
  String)>>` — entry id → source app `display_name` (JOIN entries→apps; entries
  with NULL `source_app_id` omitted). Reused later by masking.
- **`EntryRow`** (Slint struct) gains: `full`, `badge` (Round 1), plus `glyph`,
  `source`, `when`, `copied: int`, `size` — so the row, preview, metadata, and
  action bar all bind to the selected row without Rust round-trips on nav.
- **`to_rows`** populates the new fields (needs the app-name map + `now_ms`).
- **Runtime** adds callbacks: `set-type-filter`, `set-sort`, `toggle-pin`,
  `paste-into-search`, `activate-nth`, `hide-window`; and pushes `app_name_pairs`
  into the row build.

## Data Flow

```
refresh(now): results + slots + tags + app_names ->
  to_rows sets glyph/source/when/copied/size/full/badge/slot per row
list mode key -> mutates query/selected in Slint; query change -> search-changed -> refresh
select change -> preview + metadata + action-bar rebind to entries[selected] (no Rust call)
⌘K -> mode=actions overlay; action -> existing callback; digit -> assign-slot
paste (⏎ / ⌘1-9) -> activate/activate-nth -> real full_text (never the glyph/preview)
```

## Error Handling

- Empty list / out-of-range `selected` → preview + metadata show blank; nav clamps.
- `app_name_pairs` store/lock error → rows show `—` for Application (nav/paste
  unaffected).
- `relative_time` with `then > now` (clock skew) → "just now".
- `paste-into-search` clipboard read failure → no-op.
- Unknown key → ignored (event rejected).

## Testing

**Unit (pure):**
- `relative_time`: `now` → "just now"; +90s → "1m"; +2h → "2h"; +26h → "yesterday";
  +5d → "5d"; +40d → date string. `then>now` → "just now".
- `type_glyph`: each Kind → expected glyph; total mapping covered.
- `type_filter_from_index` / `sort_from_index`: index → variant, out-of-range →
  default (All / Recency).
- `app_name_pairs` (core): entries with an app → `(id, name)`; NULL-app omitted;
  ordering stable.

**Manual run-verify (`just run`):** type to filter; ↑/↓ navigate + list scrolls to
keep selection visible; ⏎ pastes; Esc clears then hides; type-filter row filters;
metadata + action bar track selection; ⌘K opens the palette, ↑/↓/⏎ run actions,
digits assign slots, Add/Remove tag work; Edit hops to native editor and back.

## File Structure

- Create: `crates/magpie-app/src/format_time.rs` (`relative_time`) — `pub mod` in lib.
- Create: `crates/magpie-core/src/mask_support.rs` (`impl Store { app_name_pairs }`)
  — `pub mod` in core lib. (Named for its future masking reuse.)
- Modify: `crates/magpie-app/src/viewmodel.rs` — `type_filter_from_index`,
  `sort_from_index`.
- Modify: `crates/magpie-app/src/runtime.rs` — `type_glyph`; `to_rows` new fields;
  new callbacks; app-name map in `refresh`; `now_ms` threaded into `to_rows`.
- Modify: `crates/magpie-app/ui/launcher.slint` — full layout rebuild: search row,
  type-filter + sort row, redesigned rows, metadata block, action bar, root
  `FocusScope` keyboard model, ⌘K palette overlay.
- Modify: `crates/magpie-app/Cargo.toml` — (no new deps expected).

## Risks / Notes

- **Slint scroll-follow**: `ListView` has no scroll-to-item; we drive
  `viewport-y` by fixed row height. Verify `viewport-y` is settable on the chosen
  `ListView` during implementation; if not, wrap rows in a `Flickable` we control.
- **Focus exclusivity**: only one focus target at a time — hence the `mode` gate
  and explicit re-focus of `fs` when leaving edit/tag input.
- **Custom search field** loses native caret/selection; accepted for the
  command-bar feel. `⌘V`-into-search is handled explicitly.
- Large surface; tasks are ordered so the app compiles and runs after each.
