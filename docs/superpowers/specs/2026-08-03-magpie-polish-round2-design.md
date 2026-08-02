# Magpie — Polish Round 2 (fixes): hover, selection, caret, close, pin, empty-state

**Date:** 2026-08-03 · **Status:** Approved · **Phase:** 1 (Plan 1 of 2; theming +
settings follow in a second plan)

Hands-on fixes. Interactive/visual — agent launch-checks for no-panic; user verifies.

## 1. Hover states
Interactive elements brighten slightly on hover. Slint pattern:
`ta := TouchArea {}` inside the Rectangle, `background: ta.has-hover ? <hover> :
<base>`. Applied to: list rows (hover tint when not selected), filter/sort/Stats/
🔒/? chips, ⌘K action rows, ⌘F app rows + chips, slot chips, detail-pane buttons
(Edit/Add-tag/Save/Cancel), merge bar buttons. Hover color = one step lighter than
the element's base (rows: `#26262c`; chips: lighten `#2a2a30`→`#35353d`).

## 2. Selection → subtle bottom underline (not left border)
Remove the gray left-edge `Rectangle`. Selected row keeps a faint fill
(`#2f2f37`) **and** gains a thin bottom underline: a full-width 1px `Rectangle` at
the row's bottom, accent-colored, shown only when selected. Hover (`#26262c`) stays
visually distinct from selected.

## 3. Blinking caret
The search caret is a solid blue bar that reads as a stray line. Make it **blink**
via a `Timer { interval: 530ms; running: true; triggered => { blink = !blink; } }`
toggling a `blink` bool; caret `opacity: root.blink ? 1.0 : 0.0`. (The caret's
position after the text is already correct.)

## 4. Red close button → hide, don't quit
Closing the window terminates Magpie. In `start()`, before the event loop:
`ui.window().on_close_requested(|| slint::CloseRequestResponse::HideWindow)` so the
red button hides the window; the tray/hotkey reopen it and `Quit Magpie` (tray)
still exits.

## 5. Pin = sticky-on-top (not a filter)
The `📌 Pin` chip filtered to only-pinned and blanked the list. Replace with:
- **Pinned entries sort to the TOP always.** `order_clause` gains a leading
  `e.pinned DESC,` so pinned rows lead every sort/search. Tested.
- **`EntryRow.pinned: bool`** (from `entry.pinned`); pinned rows show a small `📌`
  marker.
- **Remove the `📌 Pin` filter chip** + its `set-pinned-only` wiring from the UI
  (core `pinned_only` support stays, now unused by the UI).
- `⌘P` / ⌘K "Pin" toggles pin as today (already wired).

## 6. Empty-state message
When `entries.length == 0`, the list pane shows a centered message instead of
blank: `root.query != "" || root.filtered` → "No matching items" + hint; else
"No clipboard history yet — copy something to get started."

## Files
- `crates/magpie-core/src/search.rs` — `order_clause` pinned-first + test.
- `crates/magpie-app/src/runtime.rs` — `EntryRow.pinned`; drop `set-pinned-only`
  handler; `on_close_requested` in `start`; remove pinned-chip refs.
- `crates/magpie-app/ui/launcher.slint` — hover states; selection underline; caret
  Timer; `EntryRow.pinned` field + 📌 marker; remove Pin chip; empty-state.

## Testing
- Core: `order_clause`-driven — a pinned entry sorts before a newer unpinned one
  (ingest two, pin the older, `search(default)` → pinned first).
- Manual: hover brightens rows/buttons; selection shows a bottom underline (no left
  bar); caret blinks; red button hides (reopen via hotkey/tray); pinning promotes to
  top with 📌; empty/filtered list shows the message.

## Risks
- `order_clause` pinned-first changes global result ordering (intended). Fuzzy/regex
  candidates inherit it — fine.
- Blink Timer runs continuously (~2Hz) — negligible CPU; only while the window shows.
