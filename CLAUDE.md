# Magpie — developer & debugging guide

Local-first, cross-platform clipboard manager. Rust workspace: **`magpie-core`**
(pure domain logic, fully testable), **`magpie-platform`** (OS adapters behind
traits + `cfg`-selected factory), **`magpie-app`** (Slint GUI bin + a `lib.rs`
split so `tests/` can reach the modules). SQLite via bundled `rusqlite` (WAL +
FTS5). Single static binary.

## Toolchain — read this first

The repo pins Rust `1.97.1` via `rust-toolchain.toml`, and the user's shell +
`just` use it through rustup. **But an agent/CI harness may export
`RUSTUP_TOOLCHAIN=1.96.0`, which *masks* `rust-toolchain.toml`.** To reproduce the
user's toolchain exactly, strip that env var:

```bash
env -u RUSTUP_TOOLCHAIN ~/.cargo/bin/cargo <cmd>     # matches `just`
rustup run 1.97.1 cargo <cmd>                         # equivalent
```

`just run` = `cargo run -p magpie-app` (fresh **debug** build — you always get
current source, never a stale release binary).

## The gate (run before every commit)

```bash
env -u RUSTUP_TOOLCHAIN ~/.cargo/bin/cargo test --workspace
env -u RUSTUP_TOOLCHAIN ~/.cargo/bin/cargo clippy --all-targets -- -D warnings
env -u RUSTUP_TOOLCHAIN ~/.cargo/bin/cargo fmt --check
```

`clippy` is `-D warnings` (deny). **Always `cargo fmt` before committing** — edits
that add multi-line expressions frequently trip `fmt --check` (e.g. arrays,
`Command` chains, long `use` lists reorder alphabetically).

## Running headless / launch-verify

Compile-clean ≠ runs. After **any** UI or dependency change, launch-verify that the
binary doesn't panic on startup:

```bash
timeout 6 env -u RUSTUP_TOOLCHAIN ~/.cargo/bin/cargo run -p magpie-app 2>/tmp/m.log
echo "exit=$?"     # 124 = still alive after 6s (good); 101/134 = panicked/aborted
grep -iE "panic|error" /tmp/m.log
```

**`exit 124` (timeout killed a living process) is the pass signal.** The agent host
has no screen-recording permission, so GUI *visuals* and real key events can't be
driven from here — those need the user. But you can still prove "launches without
panicking."

### Exercising GUI callbacks headlessly (main-thread paths)

To reproduce a crash/behavior in a Slint callback **without** sending real keys,
add a temporary self-test gated by an env var that fires the callback logic on the
UI thread via `slint::invoke_from_event_loop`, then quits:

```rust
if std::env::var("MAGPIE_SELFTEST").is_ok() {
    let (w, s) = (weak.clone(), state.clone());
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(800));
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = w.upgrade() { show_window(&ui, &s); }
            paste_and_close(&s, &w, 0, false);   // the path under test
            let _ = slint::quit_event_loop();
        });
    });
}
```
Run with `MAGPIE_SELFTEST=1 RUST_BACKTRACE=1 … cargo run -p magpie-app`. Remove the
block before committing. **Caveat:** this runs on the event loop as a *user event*,
which is NOT the same context as a synchronous `keyDown` dispatch — see the gotcha
below.

### Probing OS behavior with throwaway examples

`magpie-platform` is a lib, so drop a one-off probe in `crates/magpie-platform/examples/`
and `cargo run -p magpie-platform --example <name>` to test native behavior in
isolation (no GUI). Used to confirm enigo paste returns `Ok` vs aborts, and that
`accessibility_trusted()` reports `false`. Delete the example after.

### macOS crash reports

A hard crash writes `~/Library/Logs/DiagnosticReports/magpie-app-*.ips` (JSON:
header line + payload line). `EXC_CRASH / SIGABRT` on **thread 0** = a Rust panic
inside a Slint callback unwinding through winit's Obj-C event loop. **macOS
throttles duplicate reports** — an identical repeat crash may write no new file, so
check timestamps and don't assume "no new report" = "not crashing."

## Hard-won gotchas

- **`ui.hide()` (Slint `Window::hide`) QUITS `run_event_loop`** once a window has
  been shown — the whole tray daemon exits (clean exit, *no* crash report, so it
  looks like a silent "death"). To hide the launcher but keep the process alive,
  hide at the OS level: **macOS → `NSApp.hide` (`magpie_platform::hide_and_yield_focus`)**,
  which also returns focus to the app underneath. See `hide_launcher()`. Isolate
  this class of bug with a headless self-test that shows→hides→checks "still alive."
- **Never hide/mutate the window synchronously inside a Slint key handler.**
  `activate`/`hide-window` fire inside winit's `keyDown`; hiding there aborts
  (SIGABRT, thread 0). **Defer via `slint::invoke_from_event_loop`.**
- **Slint's winit backend swaps Command↔Control on Apple.** Use
  `event.modifiers.control` for the primary shortcut modifier (= ⌘ on mac, Ctrl on
  win/linux). `.meta` is wrong on mac.
- **A single Slint `Text` shapes its whole string (no intra-`Text` virtualization).**
  Huge entries must be rendered as a `ListView` of line rows (only visible rows are
  shaped). `ListView` virtualizes rows; a lone `Text` does not.
- **`ListView` allows exactly one `for` child** — put fixed rows outside it.
- **A conditional `id` (declared inside `if`) can't be referenced from a root
  handler.** Use a local `property` + a `changed` handler placed inside that subtree.
- **Poisoned mutex = crash risk.** A panic while holding a lock poisons it; a later
  `.expect()`/`.unwrap()` lock on the UI thread then panics → abort. Recover with
  `.lock().unwrap_or_else(|e| e.into_inner())` on the UI hot path.
- **enigo's ⌘V synthesis MUST run on the main thread (macOS).** It calls
  TIS/HIToolbox input-source APIs that `dispatch_assert_queue` — from a background
  thread you get `EXC_BREAKPOINT`/`SIGTRAP` (`TSMGetInputSourceProperty` in the
  stack), and this only reliably fires in the packaged `.app`, not always in `just
  run`. Paste via `slint::Timer::single_shot` (fires on the event loop) or
  `invoke_from_event_loop`, never `std::thread::spawn`.
- **macOS auto-paste needs Accessibility.** `enigo`'s ⌘V silently no-ops without it.
  Check `magpie_platform::accessibility_trusted()`; guide via
  `open_accessibility_settings()` / `prompt_accessibility()` (the latter *registers*
  the app in the list — a bare trust check never does).
- **An unsigned `.app` can't reliably hold a TCC grant.** The Accessibility toggle
  shows on, yet `AXIsProcessTrusted()` still returns false (and grants usually only
  take effect after an **app relaunch**). Ad-hoc sign the bundle
  (`codesign --force --deep -s - --identifier io.magpie`) so TCC binds to a stable
  identity, and after granting **quit + reopen** the app. Never let the permission
  modal become un-clearable — gate it at most once per session (`A11Y_DISMISSED`)
  and always allow dismiss (button / Esc), then paste best-effort.
- **TCC attributes a `cargo run` binary to the launching terminal**, not to Magpie.
  So `AXIsProcessTrusted()` is always false for the dev binary even when the paste
  works (delivered via the terminal's own grant), and prompting just nags for the
  *terminal* forever. Only gate/prompt when running as a real `.app`
  (`running_as_app_bundle()` = exe path contains `/Contents/MacOS/`); in dev, skip
  the modal and paste best-effort. Also: an unsigned `.app`/binary loses its grant
  on every rebuild (TCC keys on the code hash) — use the bundle for a stable grant.
- **The global summon hotkey needs no Accessibility** and works identically from
  `just run` or an installed `.app` (Carbon `RegisterEventHotKey`, system-wide).
- **`target/debug` can balloon to tens of GB** (Slint generates one enormous Rust
  file × incremental artifacts). On `ENOSPC`: `rm -rf target/debug/incremental` or
  `cargo clean`. **Symptom of a disk-full release build:** `rust-objcopy`
  (`strip = true`) dies mid-write — `LLVM ERROR: IO failure on output stream: No
  space left on device` — leaving a **truncated ~2 KB `data` binary**, which macOS
  then rejects with *"the application cannot be opened because it has an incorrect
  executable format."* Free space and rebuild; verify with
  `file target/release/magpie` (should say `Mach-O …executable`, not `data`).
- **Real-clipboard / real-pasteboard tests must be ONE sequential test per file** —
  parallel `NSPasteboard` access SIGTRAPs on macOS.

## Data & config locations (macOS)

`~/Library/Application Support/magpie/`:
- `config.toml` — only written once settings exist; until then defaults apply
  (so changing a `Config::default()` value takes effect with no migration).
- `magpie.sqlite3` (+ `-wal`, `-shm`) — history DB. Delete the dir to reset state.
- `app_icons/`, `favicons/` — cached PNGs.

Inspect the DB: `sqlite3 ~/Library/Application\ Support/magpie/magpie.sqlite3
'select id,kind,substr(full_text,1,40) from entries order by last_copied_at_ms desc limit 10;'`

## Packaging (see the spec)

`just package-macos` → unsigned `target/Magpie.app`. Install/autostart/summon design
lives in `docs/superpowers/specs/2026-08-03-magpie-install-autostart-summon-design.md`.
Specs live in `docs/superpowers/specs/`, plans in `docs/superpowers/plans/`.
