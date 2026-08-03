# Handling large text without grinding to a halt

A reference on why Magpie stalled on huge clipboard entries, what we changed, and
how battle-tested tools (Zed, Ghostty, Yazi, Helix, terminals, editors) stay
buttery on text that is thousands — or millions — of lines. Written to learn from.

---

## 1. The symptom we hit

Copy ~2,500 lines (130k chars). Selecting that entry froze the UI for a few
seconds; the preview couldn't scroll.

## 2. The actual root cause (measured against the code, not guessed)

Three candidate causes — only one was real:

| Suspect | Verdict |
|---|---|
| Recomputing stats (chars/words/lines) on display | **Not it.** They're computed **once at ingest** (`metrics::text_metrics`) and stored in the DB columns `char_count/word_count/line_count`. Display just reads them. |
| DB write / capture being slow | **Not it.** Capture runs on a **background watcher thread**; a 130k insert + blake3 hash + FTS index is milliseconds. |
| Rendering the text | **This was it.** `to_rows` put the **entire** `full_text` into every list row's model (`full: SharedString::from(e.full_text.clone())`), and the detail pane bound a single Slint `Text { wrap: word-wrap }` to `entries[selected].full`. On selection, Slint **shaped and word-wrapped ~130k glyphs / 2,495 lines in one element** — an O(n) layout with a big per-glyph constant. That's the multi-second stall. |

The golden rule this violates: **never do O(content) work to show O(viewport) of
it.** A 40-line pane should cost ~40 lines of work, not 2,495.

## 3. What we changed (and why it's enough for now)

`preview_display(full, char_cap=20k, line_cap=400)` returns a **bounded** copy of
the text (stops early, so it's O(cap) not O(len)) with a "… preview truncated —
press ⏎ to paste the full text" note. The model now carries that bounded string, so
the preview `Text` never shapes more than ~20k chars.

Crucially, **paste is unaffected**: `activate → paste_and_close` reads the full text
from the DB (`current_results`), not from the display model. So the full content is
always pasted; only the *rendered preview* is capped.

This is the same first move every tool below makes: **bound what you render.**

## 4. What Magpie already does right

- **Metrics once, at ingest** (your "calculate stats once" idea — already true).
- **FTS5 index** for search (`entries_fts`), plus indexes on `source_app_id`, etc.
- **Row virtualization**: Slint's `ListView` only instantiates the *visible* rows,
  so a 10,000-entry history doesn't build 10,000 row widgets.
- **Dedup + `copy_count`**: identical copies are one row, not N.

## 5. How the pros stay smooth

The common thread: **(a) an efficient in-memory representation, (b) render/parse
only the visible viewport, (c) do heavy work off the UI thread, (d) cache layout.**

### Zed (Rust editor) — rope + `SumTree` + GPU + viewport
- Text is a **rope**: a balanced tree (`sum_tree::SumTree`, a copy-on-write B-tree)
  of small chunks. Indexing by char/byte/line and edits are **O(log n)**; you never
  shift a giant buffer to insert a character.
- The renderer (**gpui**, GPU) lays out and paints **only the visible lines** each
  frame; scrolling changes which slice is shaped, not how much.
- Syntax highlighting is **incremental** (Tree-sitter) and runs in the background.
- Takeaway: separate the *data structure* (rope, cheap random access + edits) from
  the *view* (shape only what's on screen).
- Refs: Zed blog "Rope & SumTree" (https://zed.dev/blog/rope), `zed` repo
  `crates/rope`, `crates/sum_tree`.

### Ghostty (terminal, Zig) — paged screen + GPU grid + viewport
- The terminal screen + scrollback live in a **paginated structure** (`PageList`: a
  linked list of fixed-size pages) rather than one giant array — so millions of
  scrollback lines don't need one contiguous allocation, and trimming/adding is
  cheap.
- A **GPU renderer** draws only the **visible grid** (viewport) every frame;
  scrollback off-screen costs nothing to render.
- Takeaway: **page/chunk** huge content; render the window, not the whole.
- Refs: `ghostty` repo `src/terminal/PageList.zig`, `page.zig`.

### Yazi (Rust TUI file manager) — async, bounded previews
- Previews (text/image/etc.) run as **background tasks** on an async scheduler, and
  are **bounded**: it previews only the first screenful/N lines/bytes of a file, not
  the whole thing, and **caches** the result.
- Rendering is **ratatui** — only the visible viewport is drawn.
- Takeaway: previews are **best-effort, bounded, and async** — exactly our
  `preview_display` idea, plus "don't block the UI while producing it."
- Refs: `yazi` repo `yazi-core` (preview/plugin system).

### Helix (Rust editor) — `ropey`
- Uses the **`ropey`** rope crate for text; O(log n) line/char indexing; renders the
  viewport only. A drop-in lesson if we ever make previews editable/scrollable over
  full content.
- Refs: `helix` repo; `ropey` crate (https://docs.rs/ropey).

### Terminals generally (Alacritty, WezTerm, kitty)
- Scrollback in a **ring buffer / grid**; GPU renders only the viewport. Same
  pattern as Ghostty.

### Pagers (`less`, `bat`)
- `less` builds a **line index** (byte offsets of line starts) so it can jump to any
  line in O(1) and only read/format the visible page — it never loads the whole file
  to show a screen. `bat` streams + highlights the visible region.

### Raycast (closed source)
- Not inspectable, but a clipboard manager at that polish almost certainly: stores a
  **short preview** + lazy-loads the full item, renders only what's visible, and
  keeps history in SQLite with indexes. The visible behavior matches "bounded
  preview + lazy full."

## 6. Data-structure cheat sheet

| Structure | Good for | Used by |
|---|---|---|
| **Rope** (balanced tree of chunks) | Large *editable* text; O(log n) edit + line/char index | Zed (`SumTree`), Helix/`ropey`, `crop`, `jumprope` |
| **Piece table** | Editable text; cheap undo; append-mostly | VS Code |
| **Gap buffer** | Localized edits around a cursor | Emacs |
| **Ring buffer / paged list** | Append-and-scroll (logs, scrollback) | Terminals, Ghostty `PageList` |
| **Line index** (offsets of line starts) | Read-only random line access | `less`, log viewers |

For Magpie's **read-only** previews, we don't need a rope — a **line index +
viewport slicing** is the right tool if/when we want unbounded smooth scrolling.

## 7. Slint-specific constraints (what bit us)

- `ListView` virtualizes **rows** (great) — but a single `Text` / `TextEdit` /
  `TextInput` **shapes its entire string**; there is **no intra-`Text`
  virtualization**. So a huge string in one `Text` is always O(n).
- Therefore, in Slint you either **bound the string** (what we did) or **build a
  viewport-windowed text view**: keep the full text in Rust, expose only the visible
  slice as a model, and update the slice as the user scrolls (render N lines around
  the scroll offset). That's the Slint-native equivalent of what Zed/less do.

## 8. Magpie roadmap for "buttery at any size"

Ordered by value/effort. (1) is shipped.

1. **[done] Bound the preview render** (`preview_display`). Removes the stall.
2. **Lazy full-text load.** `search()` currently `SELECT`s `full_text` for *every*
   result (up to 200), so opening the window pulls every big blob into memory. Split
   into a light list query (id, preview_text, metrics, app) + a `Store::full_text(id)`
   fetched only for paste/selected. Saves memory and list-build time.
3. **Store a dedicated bounded preview blob at ingest** so display never touches
   `full_text` at all (trade a little disk for guaranteed O(cap) display).
4. **Viewport-windowed preview** (the real fix for millions of lines): precompute a
   line index for the selected entry; expose only the visible lines to Slint and
   re-slice on scroll. Then scrolling a 1M-line entry costs one screen.
5. **Spill very large items to a file + memory-map**; keep only a preview in SQLite;
   load slices on demand (what pagers/editors do for giant files).
6. **Keep the model lean**: avoid duplicating strings across `EntryRow` fields;
   compute display strings lazily for the selected row only.
7. **Verify DB indexes** cover every sort/filter path (recency, most-copied, pinned,
   app, kind) so search stays O(log n) as history grows.

## 9. References

- Zed rope/SumTree: https://zed.dev/blog/rope · repo https://github.com/zed-industries/zed (`crates/rope`, `crates/sum_tree`)
- gpui (Zed GPU UI): https://github.com/zed-industries/zed (`crates/gpui`)
- Ghostty: https://github.com/ghostty-org/ghostty (`src/terminal/PageList.zig`)
- Yazi: https://github.com/sxyazi/yazi
- Helix: https://github.com/helix-editor/helix · ropey: https://docs.rs/ropey
- crop rope: https://docs.rs/crop · jumprope: https://docs.rs/jumprope
- ratatui: https://ratatui.rs
- Rope science (xi-editor): https://xi-editor.io/docs/rope_science_00.html
- VS Code piece table: https://code.visualstudio.com/blogs/2018/03/23/text-buffer-reimplementation
