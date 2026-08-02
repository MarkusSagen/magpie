# Magpie Polish & UX Round Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix the ⌘-modifier bug + visual/behavior issues and add advanced search, number badges, and a pinned/slots rework — per hands-on feedback.

**Architecture:** Core gains a `pinned_only` search filter + an `apps_in_use` list. The viewmodel adds a 30-day range + pinned flag. The runtime splits the paste flow (copy → hide → delayed keystroke) and wires ⌘F/pins/slots/badges. `launcher.slint` fixes the Cmd↔Ctrl swap, caret, icon centering, selection, stats scroll, and turns Help into a modal.

**Tech Stack:** Rust, Slint 1.17.1, `rusqlite`, `enigo`.

## Global Constraints

- Toolchain pinned `rust-toolchain.toml` (1.97.1). Run cargo as `env -u RUSTUP_TOOLCHAIN ~/.cargo/bin/cargo …`.
- Gates: `cargo test` (all green), `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, `timeout 8 … run` (exit 124 = alive).
- **Slint swaps Command↔Control on Apple.** Use `event.modifiers.control` for the primary shortcut modifier everywhere (= Cmd on macOS, Ctrl on Windows/Linux). Never `.meta`.
- Paste always uses the real `full_text`.
- `EntryRow` field order must match between the Slint struct and every `to_rows` literal.

---

### Task 1: core — `pinned_only` filter + `apps_in_use`

**Files:** `crates/magpie-core/src/search.rs`, `crates/magpie-core/src/mask_support.rs`

**Interfaces:** `SearchQuery.pinned_only: bool` (default false, adds `AND e.pinned = 1`); `Store::apps_in_use(&self) -> Result<Vec<(i64, String)>>` (distinct app id+display_name referenced by entries, ordered by name).

- [ ] **Step 1: Failing tests.** In `search.rs` tests add a pinned filter test (ingest two entries, `set_pinned` one, query with `pinned_only=true` returns only it). In `mask_support.rs` tests add `apps_in_use` returns distinct apps. Run to confirm they fail.

- [ ] **Step 2: Implement `pinned_only`.** Add the field to `SearchQuery` + `default_query()` (`pinned_only: false`). In the query builder, when `pinned_only`, append `AND e.pinned = 1` (match the existing clause style; confirm the entries alias is `e`).

- [ ] **Step 3: Implement `apps_in_use`** in `mask_support.rs`:
```rust
    /// Distinct source apps that have at least one entry: `(app_id, display_name)`.
    pub fn apps_in_use(&self) -> Result<Vec<(i64, String)>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT DISTINCT a.id, a.display_name
             FROM apps a JOIN entries e ON e.source_app_id = a.id
             ORDER BY a.display_name",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
        rows.collect()
    }
```

- [ ] **Step 4: Run → PASS.** `cargo test -p magpie-core`.

- [ ] **Step 5: Commit** `feat(core): pinned_only search filter + apps_in_use`.

---

### Task 2: viewmodel — 30-day range, pinned flag, app filter

**Files:** `crates/magpie-app/src/viewmodel.rs`

**Interfaces:** `TimeFilter::Last30Days`; `UiState.pinned_only: bool`; `time_filter_from_index` maps 0 All·1 Today·2 Last7Days·3 Last30Days; `to_query` copies `pinned_only`.

- [ ] **Step 1: Failing tests.** Add: `time_filter_from_index(3) == TimeFilter::Last30Days`; a `Last30Days` `to_query` sets `since_ms == now - 30*DAY`; `pinned_only` flows into the query. Run → fail.

- [ ] **Step 2: Implement.** Add `Last30Days` to `TimeFilter`; extend `time_filter_from_index` (currently only maps type — add a `time_filter_from_index(i) -> TimeFilter`: `1=>Today, 2=>Last7Days, 3=>Last30Days, _=>All`); add `pub pinned_only: bool` to `UiState` (+ `new()` = false); in `to_query` set `q.pinned_only = ui.pinned_only` and add the `Last30Days` arm (`since_ms: Some(now - 30*DAY_MS)`).

- [ ] **Step 3: Run → PASS.** `cargo test -p magpie-app --lib viewmodel`.

- [ ] **Step 4: Commit** `feat(app): 30-day range + pinned flag in the view-model`.

---

### Task 3: ⌘ modifier fix (Cmd↔Ctrl swap)

**Files:** `crates/magpie-app/ui/launcher.slint`

- [ ] **Step 1: Replace every `event.modifiers.meta` with `event.modifiers.control`** in the FocusScope key handler (the ⌘ shortcut block + the `⌘/` help check + the actions-mode branch if any). Grep to confirm none remain: `grep -n "modifiers.meta" crates/magpie-app/ui/launcher.slint` → empty.

- [ ] **Step 2: Build + `timeout 8` run.** Manual: ⌘K now opens the actions palette (does not type "k"); ⌘C/⌘E/⌘V/⌘1-9 work.

- [ ] **Step 3: Commit** `fix(app): use control modifier for ⌘ shortcuts (Slint swaps Cmd↔Ctrl on macOS)`.

---

### Task 4: Search caret placement + active affordance

**Files:** `crates/magpie-app/ui/launcher.slint`

- [ ] **Step 1: Rewrite the search field** so the caret follows the text:
```slint
                Rectangle {
                    height: 44px;
                    background: #161618;
                    border-radius: 10px;
                    border-width: 1px;
                    border-color: #2f2f37;
                    HorizontalLayout {
                        padding-left: 12px; padding-right: 12px; spacing: 8px;
                        Text { text: "🔍"; font-size: 16px; vertical-alignment: center; }
                        Text {
                            text: root.query == "" ? "Search clipboard…" : root.query;
                            color: root.query == "" ? #6a6a70 : white;
                            font-size: 15px; vertical-alignment: center;
                        }
                        Rectangle { width: 2px; height: 20px; y: 12px; background: #6a8cff; }
                        Rectangle { horizontal-stretch: 1; }
                    }
                }
```
(The caret is now right after the text; the trailing spacer fills the rest, so no far-right bar.)

- [ ] **Step 2: Build + `timeout 8` run.** Manual: caret sits after the query text; no blue bar in the corner; typing shows immediately.

- [ ] **Step 3: Commit** `fix(app): search caret follows text; remove stray corner bar`.

---

### Task 5: Row icon centering + selection treatment

**Files:** `crates/magpie-app/ui/launcher.slint`

- [ ] **Step 1: Remove the blue accent bar** (the `Rectangle { width: 3px; … background: … #6a8cff … }` at the row's left) and replace with a subtle gray left edge shown only when selected:
```slint
                                Rectangle {
                                    width: 2px; y: 8px; height: 36px; border-radius: 1px;
                                    background: i == root.selected ? #5a5a62 : transparent;
                                }
```

- [ ] **Step 2: Center the icon/glyph** by wrapping in a full-height Rectangle:
```slint
                                Rectangle {
                                    width: 28px;
                                    if row.has-icon: Image { source: row.icon; width: 20px; height: 20px; }
                                    if !row.has-icon: Text { text: row.glyph; font-size: 18px; }
                                }
```
(A Rectangle centers its children, so the icon is vertically centered against the row.)

- [ ] **Step 3: Build + `timeout 8` run.** Manual: icons centered; selection is gray fill + gray left edge, no blue.

- [ ] **Step 4: Commit** `fix(app): center row icons; gray selection edge (no blue bar)`.

---

### Task 6: Help as a modal overlay

**Files:** `crates/magpie-app/ui/launcher.slint`

- [ ] **Step 1: Convert Help to `mode == "help"`.** Change the `?`/`⌘/` handlers to set `root.mode = root.mode == "help" ? "list" : "help"` (not `root.view`). In the FocusScope, handle `mode == "help"` like the actions overlay: `Esc`/`?`/`⌘/` → `mode = "list"`; swallow other keys.

- [ ] **Step 2: Render Help as a centered overlay** (sibling of the ⌘K overlay, `if root.mode == "help"`): dim backdrop + rounded panel + a `ListView`/`ScrollView` of the grouped shortcut rows (reuse the existing content). Click-away closes.

- [ ] **Step 3: Remove the old full-page `if root.view == "help"` block** and the `?` button's `view` toggle (now sets `mode`).

- [ ] **Step 4: Build + `timeout 8` run.** Manual: `?`/⌘/ opens a centered modal; Esc/click-away closes.

- [ ] **Step 5: Commit** `feat(app): help as a centered modal`.

---

### Task 7: Stats — scroll + fix the lone bar

**Files:** `crates/magpie-app/ui/launcher.slint`

- [ ] **Step 1: Wrap the stats content in a `ScrollView`** (the `if root.view == "stats": VerticalLayout { … }` body becomes `ScrollView { VerticalLayout { … } }`; keep the range selector row outside/inside as fits, ensuring the long lists scroll).

- [ ] **Step 2: Fix "Copies over time".** Give the bars a min width (`width: max(8px, …)`), spacing, and only draw the chart when `root.over-time-bars.length >= 2`; otherwise a muted `Text { "Not enough data yet."; }`. Ensure no full-height artifact remains (the chart Rectangle keeps `height: 64px`).

- [ ] **Step 3: Build + `timeout 8` run.** Manual: Stats scrolls; no giant vertical bar; range buttons still work.

- [ ] **Step 4: Commit** `fix(app): scrollable stats; tidy the over-time chart`.

---

### Task 8: Paste flow — ⏎ paste+close, ⌘⏎ paste+reopen

**Files:** `crates/magpie-app/src/runtime.rs`, `crates/magpie-app/ui/launcher.slint`

**Interfaces:** `activate(int)` = copy+hide+paste; `activate-keep(int)` = copy+hide+paste+reopen. Helper `spawn_paste(weak, keep_open)`.

- [ ] **Step 1: Add `spawn_paste`** in `runtime.rs`:
```rust
/// After hiding, wait for focus to return to the previous app, send the paste
/// keystroke, and optionally re-show the window.
fn spawn_paste(weak: slint::Weak<LauncherWindow>, keep_open: bool) {
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(120));
        let _ = magpie_platform::Paster::paste(&EnigoPaster);
        if keep_open {
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = weak.upgrade() {
                    let _ = ui.show();
                }
            });
        }
    });
}
```

- [ ] **Step 2: Rewrite `on_activate`** to copy → hide → paste (no immediate enigo):
```rust
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_activate(move |idx| {
            let recent = current_results(&s, now_ms());
            if let Some(entry) = recent.get(idx as usize) {
                if let Ok(mut clip) = magpie_platform::platform_clipboard() {
                    let _ = perform_paste(&mut clip, &EnigoPaster, entry, PasteKind::Formatted, false);
                }
            }
            if let Some(ui) = w.upgrade() {
                let _ = ui.hide();
                spawn_paste(w.clone(), false);
            }
        });
    }
```
(Replace the old `activate_index`-based `on_activate`. Keep `activate_index` for `on_activate_nth`, but also route ⌘1-9 through the same copy→hide→paste? For now leave `activate_nth` as the immediate quick-paste it already is, OR update it to the same flow — update it: copy the Nth, hide, paste.)

- [ ] **Step 3: `on_activate_keep`** = copy + hide + paste + reopen:
```rust
    {
        let s = state.clone();
        let w = ui.as_weak();
        ui.on_activate_keep(move |idx| {
            let recent = current_results(&s, now_ms());
            if let Some(entry) = recent.get(idx as usize) {
                if let Ok(mut clip) = magpie_platform::platform_clipboard() {
                    let _ = perform_paste(&mut clip, &EnigoPaster, entry, PasteKind::Formatted, false);
                }
            }
            if let Some(ui) = w.upgrade() {
                let _ = ui.hide();
                spawn_paste(w.clone(), true);
            }
        });
    }
```

- [ ] **Step 4: Bind ⌘⏎ in Slint.** In the FocusScope list-mode `Key.Return` handling, branch on the modifier: `if event.modifiers.control { root.activate-keep(root.selected); } else { root.activate(root.selected); }`.

- [ ] **Step 5: Build + clippy + fmt + `timeout 8` run.** Manual (needs Accessibility): ⏎ pastes into the previous app + closes; ⌘⏎ pastes + reopens Magpie.

- [ ] **Step 6: Commit** `feat(app): ⏎ paste+close, ⌘⏎ paste+reopen (hide→paste flow)`.

---

### Task 9: Restore list position on reopen

**Files:** `crates/magpie-app/src/runtime.rs`

- [ ] **Step 1: Clamp (don't reset) `selected` in `refresh`.** After building results, clamp: `let sel = ui.get_selected(); if sel < 0 || sel as usize >= results.len() { ui.set_selected(0); }` — only reset when out of range (keeps position across hide/show; results order is stable unless a new item arrives).

- [ ] **Step 2: On `show_window`, nudge scroll to the selected row.** After `refresh`, re-assert selection so the scroll-follow fires: read `let s = ui.get_selected(); ui.set_selected(-1); ui.set_selected(s);` — the change triggers the `changed sel-follow` handler. (Confirm this doesn't visibly flicker; if it does, add a Slint `public function scroll-to-selected()` calling the same math and invoke it.)

- [ ] **Step 3: Build + `timeout 8` run.** Manual: open, move down several, hide (Esc), reopen → same position + scrolled into view.

- [ ] **Step 4: Commit** `feat(app): keep list position across hide/show`.

---

### Task 10: 1–9 number badges that light up with ⌘/Ctrl

**Files:** `crates/magpie-app/ui/launcher.slint`

- [ ] **Step 1: Track `cmd-held`.** Add `in-out property <bool> cmd-held: false;`. In the FocusScope: at the top of `key-pressed` set `root.cmd-held = event.modifiers.control;`; add `key-released(event) => { root.cmd-held = event.modifiers.control; return accept; }`.

- [ ] **Step 2: Render badges** on the first nine rows (inside the row, before the merge area): `if i < 9: Text { text: i + 1; color: root.cmd-held ? #6a8cff : #4a4a52; font-size: 11px; }`. Bright when held, dim otherwise.

- [ ] **Step 3: Build + `timeout 8` run.** Manual: rows 1-9 show dim numbers; holding ⌘ brightens them.

- [ ] **Step 4: Commit** `feat(app): 1-9 quick-paste badges that light up with ⌘`.

---

### Task 11: Remove per-row "+"; slots strip; pinned chip

**Files:** `crates/magpie-app/ui/launcher.slint`, `crates/magpie-app/src/runtime.rs`

- [ ] **Step 1: Remove the per-row "+" merge `Rectangle`** (the `TouchArea { clicked => { root.toggle-merge(i); } }` control) from the list row. Merge stays reachable via ⌘M / ⌘K + the merge bar.

- [ ] **Step 2: Pinned chip.** Add a `Pinned` chip after `All/Text/Link/Color/Image/File` bound to a new `in-out property <bool> pinned-only` + callback `set-pinned-only(bool)`. Runtime `on_set_pinned_only` sets `UiState.pinned_only` + refresh.

- [ ] **Step 3: Slots strip.** Push a `[SlotItem { n: int, title: string, filled: bool }]` model (`in property <[SlotItem]> slots;`) built in `refresh` from `store.slot_map()` + entry titles (slot 1-9; empty → `filled=false`). Render a compact strip above the list, shown only when any slot is filled; each chip `① <title>` clickable → a new `paste-slot(int)` callback that pastes that slot (reuse the quick-paste path + the hide→paste flow).

- [ ] **Step 4: Build + clippy + fmt + `timeout 8` run.** Manual: no per-row "+"; Pinned chip filters to pinned; assign a slot via ⌘K then see it in the strip; clicking a slot pastes it.

- [ ] **Step 5: Commit** `feat(app): pinned chip + slots speed-dial strip; remove per-row merge "+"`.

---

### Task 12: ⌘F advanced search overlay

**Files:** `crates/magpie-app/ui/launcher.slint`, `crates/magpie-app/src/runtime.rs`

- [ ] **Step 1: Open on ⌘F.** In the FocusScope ⌘ block add `if event.text == "f" { root.open-search(); return accept; }`. `mode == "search"` gates an overlay; Esc closes.

- [ ] **Step 2: Overlay UI** (`if root.mode == "search"`): dim backdrop + panel with:
  - **Time**: chips All · Today · Last 7 days · Last 30 days bound to `time-index` + `set-time-filter(int)`.
  - **App**: a `ListView` of `in property <[AppItem]> apps` (`{id:int, name:string}`), each clickable → `set-app-filter(int)`; an "Any app" row → `set-app-filter(-1)`.
  - **Sort**: Recency · Most copied (reuse `set-sort`).
  - A "Clear filters" button → `clear-filters()`.

- [ ] **Step 3: Runtime wiring.** `on_open_search` pushes the apps model (`store.apps_in_use()` → `[AppItem]`) + sets `mode="search"`. `on_set_time_filter(i)` → `UiState.time_filter = time_filter_from_index(i)` + refresh. `on_set_app_filter(id)` → `UiState.app_filter = (id >= 0).then_some(id as i64)` + refresh + close. `on_clear_filters` resets time/app/pinned + refresh. Show a small "filtered" indicator in the filter row when any of app/time/pinned is active (a property set in `refresh`).

- [ ] **Step 4: Build + clippy + fmt + `timeout 8` run.** Manual: ⌘F opens; filter by Ghostty / Last 7 days narrows the list; Clear resets.

- [ ] **Step 5: Commit** `feat(app): ⌘F advanced search (app + time range + sort)`.

---

### Task 13: Full gate + run-verify + memory

- [ ] **Step 1: Full gates** — `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, `timeout 8 … run`.
- [ ] **Step 2: Update memory** `magpie-clipboard-manager.md`; commit docs.

## Self-Review

- **Spec coverage:** ⌘ fix → T3; caret → T4; icon/selection → T5; help modal → T6; stats → T7; paste flow → T8; reopen position → T9; number badges → T10; pins/slots/remove-"+" → T11; ⌘F search → T12; `pinned_only`/`apps_in_use` → T1; 30-day/pinned viewmodel → T2. All mapped.
- **Placeholder scan:** none — core/viewmodel steps are concrete; UI steps give full Slint or precise instructions.
- **Type consistency:** `SearchQuery.pinned_only` (T1) ↔ `UiState.pinned_only`/`to_query` (T2). `apps_in_use -> Vec<(i64,String)>` (T1) ↔ `AppItem` model (T12). `TimeFilter::Last30Days` + `time_filter_from_index` (T2) ↔ ⌘F time chips (T12). New callbacks (`activate-keep` already exists; `set-pinned-only`, `paste-slot`, `open-search`, `set-time-filter`, `set-app-filter`, `clear-filters`, `open-actions` existing) declared in Slint + wired in runtime. New Slint structs `SlotItem`, `AppItem`.
- **Risks:** paste timing (120ms) may need tuning; `key-released` confirmed to exist on FocusScope; the reopen scroll nudge (`set-selected(-1)` then back) may need a dedicated `public function` if it flickers.
