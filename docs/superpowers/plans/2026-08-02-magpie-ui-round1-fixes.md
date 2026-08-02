# Magpie UI Round 1 Fixes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix three launcher problems — tray-icon click behavior, multiline row display, and the preview not following selection — without a visual redesign.

**Architecture:** Small changes to `crates/magpie-app`: one pure tested helper (`line_badge`), two new `EntryRow` fields (`full`, `badge`) populated in `to_rows`, a reactive preview binding + `ScrollView` in `launcher.slint`, and a reworked `build_tray` that opens the window on left-click and shows a `Show Magpie` / `Quit Magpie` menu on right-click via a shared `show_window` helper.

**Tech Stack:** Rust, Slint 1.x, `tray-icon` 0.24.2 (`with_menu_on_left_click`, `TrayIconEvent`), `rusqlite`.

## Global Constraints

- Toolchain pinned via `rust-toolchain.toml` (channel `1.97.1`). The agent harness may set `RUSTUP_TOOLCHAIN=1.96.0`; to reproduce the real build run cargo as `env -u RUSTUP_TOOLCHAIN ~/.cargo/bin/cargo …`.
- Gates that must stay green: `cargo test` (all), `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`.
- `tray-icon` 0.24.2 API (verified against installed source): `TrayIconBuilder::with_menu_on_left_click(bool)`; `TrayIconEvent::receiver() -> &TrayIconEventReceiver`; `TrayIconEvent::Click { id, position, rect, button: MouseButton, button_state: MouseButtonState }`; `MouseButton::{Left,Right,Middle}`; `MouseButtonState::{Up,Down}`. Linux does not emit `TrayIconEvent` — the right-click `Show Magpie` menu item is the portable fallback and must exist.
- Preview must always paste/act on the real selected entry; masking/redesign are out of scope.

---

### Task 1: `line_badge` pure helper

**Files:**
- Modify: `crates/magpie-app/src/runtime.rs` (add `line_badge` fn + `#[cfg(test)]` tests)

**Interfaces:**
- Produces: `fn line_badge(full_text: &str) -> String` — returns `"<n> lines"` when the text has more than one non-empty line, else `""`. `n` counts non-empty (after `trim`) lines.

- [ ] **Step 1: Write the failing test**

Add near the bottom of `runtime.rs`:

```rust
#[cfg(test)]
mod round1_tests {
    use super::line_badge;

    #[test]
    fn badge_counts_nonempty_lines() {
        assert_eq!(line_badge("a\nb\nc"), "3 lines");
        assert_eq!(line_badge("a\n\nb"), "2 lines"); // blank interior line ignored
    }

    #[test]
    fn badge_empty_for_single_or_no_line() {
        assert_eq!(line_badge("one line"), "");
        assert_eq!(line_badge(""), "");
        assert_eq!(line_badge("   \n  "), ""); // only blank lines
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `env -u RUSTUP_TOOLCHAIN ~/.cargo/bin/cargo test -p magpie-app line_badge -- --nocapture`
Expected: FAIL to compile — `line_badge` not found.

- [ ] **Step 3: Write minimal implementation**

Add above `preview_title` in `runtime.rs`:

```rust
/// A row badge like "3 lines" when the text has more than one non-empty line.
fn line_badge(full_text: &str) -> String {
    let n = full_text.lines().filter(|l| !l.trim().is_empty()).count();
    if n > 1 {
        format!("{n} lines")
    } else {
        String::new()
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `env -u RUSTUP_TOOLCHAIN ~/.cargo/bin/cargo test -p magpie-app line_badge`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/magpie-app/src/runtime.rs
git commit -m "feat(app): line_badge helper for multiline row hint"
```

---

### Task 2: EntryRow `full` + `badge`, reactive scrollable preview

**Files:**
- Modify: `crates/magpie-app/ui/launcher.slint` (EntryRow struct; row badge pill; preview binding + ScrollView)
- Modify: `crates/magpie-app/src/runtime.rs` (`to_rows` populates `full` + `badge`; drop the `results.first()` detail push)

**Interfaces:**
- Consumes: `line_badge` (Task 1); existing `Entry.full_text`, `preview_title`.
- Produces: `EntryRow { title, subtitle, kind, slot, merged, full, badge }`.

- [ ] **Step 1: Add the two fields to the Slint `EntryRow` struct**

In `launcher.slint`, extend the struct:

```slint
struct EntryRow {
    title: string,
    subtitle: string,
    kind: string,
    slot: int,
    merged: bool,
    full: string,
    badge: string,
}
```

- [ ] **Step 2: Populate them in `to_rows`**

In `runtime.rs` `to_rows`, inside the returned `EntryRow { … }`, add:

```rust
            EntryRow {
                title: SharedString::from(preview_title(e)),
                subtitle: SharedString::from(subtitle),
                kind: SharedString::from(e.kind.as_str()),
                slot: *slots.get(&e.id).unwrap_or(&0) as i32,
                merged: merge_set.contains(&(i as i32)),
                full: SharedString::from(e.full_text.clone()),
                badge: SharedString::from(line_badge(&e.full_text)),
            }
```

- [ ] **Step 3: Render the badge pill in the row**

In `launcher.slint`, in the row's `HorizontalLayout` (the one with the title `VerticalLayout` and the merge button), insert a badge between the title column and the merge button, shown only when non-empty:

```slint
                        if row.badge != "": Rectangle {
                            y: 11px;
                            height: 18px;
                            width: 58px;
                            background: #2a2a30;
                            border-radius: 9px;
                            Text {
                                text: row.badge;
                                color: #9a9aa0;
                                font-size: 10px;
                                horizontal-alignment: center;
                                vertical-alignment: center;
                            }
                        }
```

- [ ] **Step 4: Make the preview follow selection and scroll**

In `launcher.slint`, replace the preview `Text { text: root.detail-text; … }` (first child of the detail `VerticalLayout`) with a guarded, scrollable, selection-bound preview:

```slint
                        ScrollView {
                            min-height: 120px;
                            vertical-stretch: 1;
                            Text {
                                width: parent.visible-width;
                                text: (entries.length > 0 && root.selected < entries.length) ? entries[root.selected].full : "";
                                color: #d0d0d4;
                                wrap: word-wrap;
                            }
                        }
```

- [ ] **Step 5: Stop pushing `results.first()` into `detail-text`**

In `runtime.rs` `refresh`, remove the `detail` local and the `ui.set_detail_text(...)` call (the preview no longer reads `detail-text`). Leave the `detail-text` property declaration in the `.slint` file only if something else still binds it; grep first:

Run: `grep -rn "detail-text\|detail_text\|set_detail_text\|get_detail_text" crates/magpie-app`
If the only remaining references are the property declaration + the removed push, delete the property declaration from `launcher.slint` too.

- [ ] **Step 6: Build, lint, format, and manually verify**

Run:
```bash
env -u RUSTUP_TOOLCHAIN ~/.cargo/bin/cargo build -p magpie-app
env -u RUSTUP_TOOLCHAIN ~/.cargo/bin/cargo clippy --all-targets -- -D warnings
env -u RUSTUP_TOOLCHAIN ~/.cargo/bin/cargo fmt --check
```
Expected: all clean.

Manual (`just run`): copy a multiline snippet → its row shows `N lines`; click different rows → the preview updates to each selected entry's full text and scrolls for long content.

- [ ] **Step 7: Commit**

```bash
git add crates/magpie-app/ui/launcher.slint crates/magpie-app/src/runtime.rs
git commit -m "feat(app): multiline row badge + selection-driven scrollable preview"
```

---

### Task 3: Tray left-click opens window, right-click menu

**Files:**
- Modify: `crates/magpie-app/src/runtime.rs` (`show_window` helper; `build_tray` signature + wiring; call site in `start`)

**Interfaces:**
- Consumes: `refresh(&LauncherWindow, &AppState)`; `slint::Weak<LauncherWindow>`; `Arc<AppState>`; `tray-icon` 0.24 API from Global Constraints.
- Produces: `fn show_window(ui: &LauncherWindow, state: &AppState)`; `fn build_tray(weak: slint::Weak<LauncherWindow>, state: Arc<AppState>) -> Option<tray_icon::TrayIcon>`.

- [ ] **Step 1: Add the shared `show_window` helper**

In `runtime.rs`, add:

```rust
/// Refresh results and show/raise the launcher window. Shared by the launcher
/// hotkey and the tray (left-click + "Show Magpie").
fn show_window(ui: &LauncherWindow, state: &AppState) {
    refresh(ui, state);
    let _ = ui.show();
}
```

Then in `spawn_hotkeys`, change the `HotAction::Launcher` arm body to use it:

```rust
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(ui) = w.upgrade() {
                                show_window(&ui, &s);
                            }
                        });
```

- [ ] **Step 2: Rewrite `build_tray` to take the window + state and wire clicks**

Replace `build_tray` with:

```rust
/// Build the tray icon: right-click shows a menu (Show / Quit); left-click opens
/// the window. On Linux, tray click events are not emitted, so "Show Magpie" in
/// the menu is the portable way to open the window.
fn build_tray(
    weak: slint::Weak<LauncherWindow>,
    state: Arc<AppState>,
) -> Option<tray_icon::TrayIcon> {
    use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
    use tray_icon::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

    let menu = Menu::new();
    let show = MenuItem::new("Show Magpie", true, None);
    let quit = MenuItem::new("Quit Magpie", true, None);
    menu.append(&show).ok()?;
    menu.append(&PredefinedMenuItem::separator()).ok()?;
    menu.append(&quit).ok()?;
    let show_id = show.id().clone();
    let quit_id = quit.id().clone();

    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .with_tooltip("Magpie")
        .with_title("🐦")
        .build()
        .ok()?;

    // Menu clicks (right-click menu): Show / Quit.
    {
        let w = weak.clone();
        let s = state.clone();
        std::thread::spawn(move || {
            let rx = MenuEvent::receiver();
            while let Ok(ev) = rx.recv() {
                if ev.id == quit_id {
                    let _ = slint::invoke_from_event_loop(|| {
                        let _ = slint::quit_event_loop();
                    });
                } else if ev.id == show_id {
                    let w = w.clone();
                    let s = s.clone();
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(ui) = w.upgrade() {
                            show_window(&ui, &s);
                        }
                    });
                }
            }
        });
    }

    // Left-click on the icon opens the window (macOS/Windows; Linux no-op).
    {
        let w = weak.clone();
        let s = state.clone();
        std::thread::spawn(move || {
            let rx = TrayIconEvent::receiver();
            while let Ok(ev) = rx.recv() {
                if let TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } = ev
                {
                    let w = w.clone();
                    let s = s.clone();
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(ui) = w.upgrade() {
                            show_window(&ui, &s);
                        }
                    });
                }
            }
        });
    }

    Some(tray)
}
```

- [ ] **Step 3: Update the call site in `start`**

In `start`, change `let _tray = build_tray();` to:

```rust
    let _tray = build_tray(weak.clone(), state.clone());
```

(`weak` and `state` are already in scope there — `weak` is `ui.as_weak()`, `state` is the `Arc<AppState>`.)

- [ ] **Step 4: Build, lint, format**

Run:
```bash
env -u RUSTUP_TOOLCHAIN ~/.cargo/bin/cargo build -p magpie-app
env -u RUSTUP_TOOLCHAIN ~/.cargo/bin/cargo clippy --all-targets -- -D warnings
env -u RUSTUP_TOOLCHAIN ~/.cargo/bin/cargo fmt --check
```
Expected: all clean. (If `PredefinedMenuItem` path differs in muda re-export, adjust the `use`; confirm with `grep -rn "PredefinedMenuItem" ~/.cargo/registry/src/*/tray-icon-0.24.2`.)

- [ ] **Step 5: Manual run-verify**

`just run`, then:
- Left-click the 🐦 menu-bar icon → window opens.
- Right-click → menu shows `Show Magpie` + `Quit Magpie`. `Show Magpie` opens the window; `Quit Magpie` exits.

- [ ] **Step 6: Commit**

```bash
git add crates/magpie-app/src/runtime.rs
git commit -m "feat(app): tray left-click opens window, right-click Show/Quit menu"
```

---

### Task 4: Full gate + memory update

**Files:**
- Modify: memory `magpie-clipboard-manager.md` (status note)

- [ ] **Step 1: Run the full test suite + gates**

Run:
```bash
env -u RUSTUP_TOOLCHAIN ~/.cargo/bin/cargo test
env -u RUSTUP_TOOLCHAIN ~/.cargo/bin/cargo clippy --all-targets -- -D warnings
env -u RUSTUP_TOOLCHAIN ~/.cargo/bin/cargo fmt --check
```
Expected: all pass, zero warnings.

- [ ] **Step 2: Update the project memory** with Round 1 completion + Round 2 (redesign) and masking still pending. Commit any doc/memory changes.

## Self-Review

- **Spec coverage:** Tray click behavior → Task 3. Multiline row badge → Tasks 1–2. Preview follows selection + scroll → Task 2. `show_window` shared helper → Task 3. All spec sections covered.
- **Placeholder scan:** none — every code step has concrete code.
- **Type consistency:** `line_badge(&str) -> String` defined in Task 1, used in Task 2. `EntryRow` field set (`full`, `badge`) consistent between the Slint struct (Task 2 Step 1) and `to_rows` (Task 2 Step 2). `build_tray(weak, state)` signature consistent between Task 3 Step 2 (definition) and Step 3 (call site).
