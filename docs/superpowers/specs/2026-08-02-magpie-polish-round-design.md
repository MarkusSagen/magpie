# Magpie — Polish & UX Round

**Date:** 2026-08-02 · **Status:** Approved · **Phase:** 1 · **One combined plan**

Fixes and features from hands-on feedback. Grouped by area. Many items are
interactive/visual and need user verification (the agent can't drive the GUI);
the app must build + launch (exit 124) after each task.

## Root-cause fixes

### 1. ⌘ shortcuts (⌘K, ⌘C, ⌘E, ⌘V, ⌘1-9) — the Cmd↔Ctrl swap
Slint's winit backend **swaps Command and Control on Apple platforms** (verified in
`i-slint-backend-winit/event_loop.rs`: "mapping command to control and control to
meta"). So macOS **⌘ arrives as `event.modifiers.control`**, not `.meta`. Every
`event.modifiers.meta` in the key handler must become `event.modifiers.control`.
This fixes ⌘K etc. on macOS AND makes them Ctrl-shortcuts on Windows/Linux (Slint
maps the primary modifier to `control` on every platform). Same for the `⌘/` help
check.

### 2. Stray blue caret in the top-right + search "active" affordance
The search caret `Rectangle` is placed *after* the horizontally-stretched query
`Text`, so it's flung to the far-right corner (the "blue bar top-right"). Fix:
place the caret **immediately after the query text** (text no longer stretches; a
spacer fills the rest). Give the search box a subtle focus ring so it reads as
active — it always is (the root FocusScope owns input, focused by default).

## Visual polish

### 3. Row icon vertical alignment
List-row icons render top-aligned. Wrap the icon/glyph in a full-row-height
`Rectangle` (Slint centers a Rectangle's children), so the 20px icon sits centered
against the two-line title+subtitle.

### 4. Selection treatment
Remove the blue 3px accent bar. Selected row = the gray rounded fill (`#2f2f37`)
plus a **subtle gray left edge** (a 2px `#5a5a62` bar inset within the rounded
rectangle), not blue.

### 5. Stats view — scroll + the lone giant bar
Wrap the stats content in a `ScrollView` (it currently overflows with no
scrollbar). The "Copies over time" chart renders one full-height bar when there's a
single day bucket; widen/space the bars and only draw the chart when there are ≥2
buckets (else a muted "Not enough data yet." line). Remove any full-height artifact.

### 6. Help as a modal
Convert the full-page `view == "help"` into a **centered overlay modal** (dim
backdrop, rounded panel, scrollable list) like the ⌘K palette. Opened by `⌘/` or
the `?` button; closed by `Esc`, `?`, or click-away. Track via `mode == "help"`
(consistent with `actions`), not a separate full view.

## Behavior

### 7. Paste flow (⏎ / ⌘⏎)
Magpie is a normal focused window, so pasting into the previous app requires
yielding focus first.
- **⏎ (activate):** copy the entry to the clipboard → `hide()` Magpie → after a
  short delay (focus returns to the previous app) send the paste keystroke
  (`EnigoPaster`). 
- **⌘⏎ (activate-keep):** same, then re-`show()` Magpie after the paste (paste lands
  in the previous app; Magpie reappears — a brief focus flicker is expected).
- Split the current `perform_paste` usage: copy synchronously, then a
  `spawn_paste(keep_open)` thread sleeps ~120ms, sends the keystroke, and (if
  `keep_open`) `invoke_from_event_loop`→`show()`. Enigo runs off the UI thread.

### 8. Restore list position on reopen
Do not reset `selected` on `hide`/`show`; clamp it into range in `refresh`. On
`show_window`, after refresh, nudge the selected row back into view (reuse the
scroll-follow). Typing a query still resets selection to 0 (fresh results).

## Power features

### 9. ⌘F advanced search
A centered overlay (`mode == "search"`) opened by `⌘F` (→ `event.modifiers.control`
+ `f`) to filter by **source app**, **time range**, and **sort**:
- Time: All · Today · Last 7 days · Last 30 days (extend `TimeFilter` with
  `Last30Days`).
- App: pick from the distinct apps that have entries (new core
  `Store::apps_in_use() -> Vec<(i64, String)>` — id + display_name for apps
  referenced by entries; tested). Sets `UiState.app_filter`.
- Sort: Recency · Most copied (existing).
- Apply writes `UiState` and refreshes; a "Clear filters" resets. A small indicator
  in the filter row shows when an app/time filter is active.

### 10. 1–9 number badges that light up with ⌘/Ctrl
Show a subtle `1..9` badge on the first nine rows. Track a `cmd-held: bool` on the
FocusScope (`key-pressed` sets it from `event.modifiers.control`; `key-released`
clears/updates it). While held, brighten the badges (they map to ⌘1-9 quick-paste).

### 11. Pinned section + slots strip; remove the per-row "+"
- **Remove** the per-row "+" merge toggle (it confused users). Merge now lives only
  in ⌘K ("Add to merge") + the existing merge bar; ⌘M toggles the selected entry.
- **Slots strip** (speed dial): when any slot 1-9 is assigned, a compact strip above
  the list shows each assigned slot as `① <short title>` (or `—`), clickable to
  paste that slot. Built from `slot_map` + entry lookups.
- **Pinned filter:** add a `Pinned` chip to the filter row that shows only pinned
  entries. New `SearchQuery.pinned_only: bool` (default false) → clause `AND e.pinned
  = 1`; `UiState.pinned_only` → `to_query`. Tested in core.

## Architecture / Files

- **core:** `search.rs` (`SearchQuery.pinned_only` + clause), new
  `Store::apps_in_use()` (in `mask_support.rs`), tests.
- **app:** `viewmodel.rs` (`UiState.pinned_only`, `TimeFilter::Last30Days`,
  `time_filter_from_index`, `app_filter` setter helpers), `runtime.rs` (paste
  flow split + `spawn_paste`, ⌘F/pinned/slots wiring, `cmd-held`, apps list,
  selection clamp), `ui/launcher.slint` (modifier fix, caret, icon centering,
  selection, stats scroll, help modal, ⌘F overlay, number badges, slots strip,
  pinned chip, remove per-row "+").

## Error Handling
- Paste keystroke best-effort (needs Accessibility); failures are silent.
- Empty apps list / no slots → strip/section hidden. `apps_in_use` lock error → empty.
- All overlays dismiss on Esc; `mode` returns to "list" and re-focuses the FocusScope.

## Testing
**Unit/core (verifiable):** `pinned_only` filters to pinned entries;
`apps_in_use` returns distinct `(id, name)` for apps with entries;
`time_filter_from_index` incl. Last30Days; `TimeFilter::Last30Days` sets the right
`since_ms`.
**Manual run-verify (macOS):** ⌘K opens actions (not "k"); search caret sits after
text + no top-right bar; row icons centered; selection is gray w/ gray left edge;
⏎ pastes into the previous app + closes, ⌘⏎ pastes + reopens; reopening keeps
position; Help is a modal; Stats scrolls + no giant bar; ⌘F filters by app/time;
holding ⌘ lights the 1-9 badges; slots strip shows assignments; no per-row "+".

## Risks / Notes
- **Interactive-only verification** for most items — agent launch-checks for
  no-panic; user confirms behavior/visuals.
- **Paste timing** (hide→delay→keystroke) may need tuning per machine; 120ms default.
- `key-released` availability on Slint `FocusScope` — confirm during build; if
  absent, derive `cmd-held` from each `key-pressed`'s modifiers (clears when the
  next event has no control) — acceptable approximation.
- Windows/Linux icon extraction still deferred (unchanged).
