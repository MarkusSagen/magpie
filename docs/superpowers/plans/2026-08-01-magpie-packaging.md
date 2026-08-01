# magpie Packaging & Autostart Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Produce small, self-contained release binaries for all three OSes and give the app a CLI to install/remove autostart, plus a `justfile` and docs so the whole thing is buildable and shippable with one command per platform.

**Architecture:** A release Cargo profile (LTO + strip + single codegen unit) shrinks the binary. A tiny hand-rolled CLI-arg parser (no `clap` — keep deps minimal) adds `magpie --install-autostart` / `--uninstall-autostart` / `--help`, wired to `magpie_platform::platform_autostart`. A `justfile` (matching the Lumen setup) drives build/test/run/release/package. Per-OS bundling is documented recipes: macOS `.app`, Windows `.exe`, Linux binary + `.desktop`.

**Tech Stack:** Rust (release profile), `just`, `mise` (already used across these hobby repos). No new library dependencies.

## Global Constraints

- **No new dependencies.** CLI parsing is hand-rolled (~30 lines), tested.
- **Autostart uses `platform_autostart`** from Plan 4 — never reimplement per-OS autostart here.
- **Personal-first:** no code signing / notarization / auto-update in v1. The release profile + recipes must leave room to add them without restructuring.
- **Binary size matters:** the release profile must set `lto = true`, `strip = true`, `codegen-units = 1`, `opt-level = "z"` (or `"s"` — pick `"z"` and note the tradeoff).
- **Commit style:** conventional commits, one per task.

---

### Task 1: Release profile + minimal CLI (install/uninstall autostart)

**Files:**
- Modify: `Cargo.toml` (root) — add `[profile.release]`
- Create: `crates/magpie-app/src/cli.rs`
- Modify: `crates/magpie-app/src/main.rs` (`mod cli;` + dispatch)
- Test: `cli.rs` tests module.

**Interfaces:**
- Produces:
  - `pub enum Command { Run, InstallAutostart, UninstallAutostart, Help }`
  - `pub fn parse_args(args: &[String]) -> Command` — maps `--install-autostart`→`InstallAutostart`, `--uninstall-autostart`→`UninstallAutostart`, `--help`/`-h`→`Help`, anything else (incl. empty)→`Run`.
  - `pub fn run_command(cmd: Command) -> i32` — `Help` prints usage and returns 0; `InstallAutostart`/`UninstallAutostart` resolve the current exe via `std::env::current_exe()`, call `platform_autostart(exe).set_enabled(true/false)`, print the result, return 0/1; `Run` returns a sentinel (e.g. `-1`) telling `main` to start the GUI. (The GUI launch stays in `main`, not here, so this is testable without a display.)

- [ ] **Step 1: Add the release profile to root `Cargo.toml`**

```toml
[profile.release]
opt-level = "z"    # smallest binary; use "s" if runtime speed regresses noticeably
lto = true
strip = true
codegen-units = 1
panic = "abort"
```

- [ ] **Step 2: Write failing tests in `cli.rs`**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn v(items: &[&str]) -> Vec<String> { items.iter().map(|s| s.to_string()).collect() }

    #[test]
    fn parses_known_flags() {
        assert!(matches!(parse_args(&v(&["--install-autostart"])), Command::InstallAutostart));
        assert!(matches!(parse_args(&v(&["--uninstall-autostart"])), Command::UninstallAutostart));
        assert!(matches!(parse_args(&v(&["--help"])), Command::Help));
        assert!(matches!(parse_args(&v(&["-h"])), Command::Help));
    }

    #[test]
    fn empty_or_unknown_is_run() {
        assert!(matches!(parse_args(&v(&[])), Command::Run));
        assert!(matches!(parse_args(&v(&["--nonsense"])), Command::Run));
    }

    #[test]
    fn help_command_returns_zero() {
        assert_eq!(run_command(Command::Help), 0);
    }
}
```

- [ ] **Step 3: Add `mod cli;` to `main.rs`; run to verify fail**

Run: `cargo test -p magpie-app cli`
Expected: FAIL.

- [ ] **Step 4: Implement `cli.rs`**

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command { Run, InstallAutostart, UninstallAutostart, Help }

pub fn parse_args(args: &[String]) -> Command {
    for a in args {
        match a.as_str() {
            "--install-autostart" => return Command::InstallAutostart,
            "--uninstall-autostart" => return Command::UninstallAutostart,
            "--help" | "-h" => return Command::Help,
            _ => {}
        }
    }
    Command::Run
}

const USAGE: &str = "\
Magpie — cross-platform clipboard manager

USAGE:
    magpie [FLAGS]

FLAGS:
    (no flags)              Run the tray app
    --install-autostart     Start Magpie automatically at login
    --uninstall-autostart   Remove login autostart
    -h, --help              Show this help
";

/// Returns a process exit code, or -1 to signal `main` to launch the GUI.
pub fn run_command(cmd: Command) -> i32 {
    match cmd {
        Command::Help => {
            println!("{USAGE}");
            0
        }
        Command::InstallAutostart => set_autostart(true),
        Command::UninstallAutostart => set_autostart(false),
        Command::Run => -1,
    }
}

fn set_autostart(on: bool) -> i32 {
    let exe = match std::env::current_exe() {
        Ok(p) => p.to_string_lossy().into_owned(),
        Err(e) => { eprintln!("cannot resolve executable path: {e}"); return 1; }
    };
    let auto = magpie_platform::platform_autostart(&exe);
    match auto.set_enabled(on) {
        Ok(()) => {
            println!("autostart {}", if on { "enabled" } else { "disabled" });
            0
        }
        Err(e) => { eprintln!("autostart change failed: {e}"); 1 }
    }
}
```

- [ ] **Step 5: Wire `main.rs`** to parse args first:

```rust
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = cli::run_command(cli::parse_args(&args));
    if code >= 0 {
        std::process::exit(code);
    }
    // code == -1: launch the GUI (existing runtime entry point)
    runtime::start();
}
```

(Extract the Task-8 runtime body into `pub fn start()` in `runtime.rs` if not already.)

- [ ] **Step 6: Run tests + build**

Run: `cargo test -p magpie-app cli && cargo build -p magpie-app`
Expected: PASS + compiles.

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml crates/magpie-app/src/cli.rs crates/magpie-app/src/main.rs crates/magpie-app/src/runtime.rs
git commit -m "feat(app): release profile + autostart install/uninstall CLI"
```

---

### Task 2: justfile + mise task runner

**Files:**
- Create: `justfile`
- Create: `mise.toml`

**Interfaces:**
- Produces `just` recipes: `test`, `run`, `build`, `release`, `size`, `install-autostart`, `fmt`, `clippy`, and per-OS `package-macos` / `package-windows` / `package-linux`. `mise.toml` pins the Rust tool.

> Recipes that only run tooling are verified by invoking them, not by unit tests.

- [ ] **Step 1: Create `mise.toml`**

```toml
[tools]
rust = "stable"
just = "latest"
```

- [ ] **Step 2: Create `justfile`**

```make
set shell := ["bash", "-uc"]

# Run the full test suite
test:
    cargo test --workspace

# Run the app (debug)
run:
    cargo run -p magpie-app

# Debug build of everything
build:
    cargo build --workspace

# Optimized release binary
release:
    cargo build --release -p magpie-app

# Print the release binary size
size: release
    ls -lh target/release/magpie* | awk '{print $5, $9}'

fmt:
    cargo fmt --all

clippy:
    cargo clippy --workspace --all-targets -- -D warnings

# Install login autostart using the built release binary
install-autostart: release
    ./target/release/magpie --install-autostart

# --- packaging ---

# macOS: assemble a minimal .app bundle around the release binary
package-macos: release
    #!/usr/bin/env bash
    set -euo pipefail
    APP="target/Magpie.app"
    rm -rf "$APP"
    mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
    cp target/release/magpie "$APP/Contents/MacOS/magpie"
    cp packaging/macos/Info.plist "$APP/Contents/Info.plist"
    echo "built $APP (unsigned)"

# Windows: the release .exe is the deliverable
package-windows: release
    @echo "deliverable: target/release/magpie.exe"

# Linux: binary + autostart .desktop template
package-linux: release
    @echo "deliverable: target/release/magpie (+ ~/.config/autostart via --install-autostart)"
```

- [ ] **Step 3: Verify recipes run**

Run: `just test` and `just release` and `just size`.
Expected: tests pass; release binary builds; `size` prints a single-digit-to-low-double-digit MB size. Record the observed size in your report.

- [ ] **Step 4: Commit**

```bash
git add justfile mise.toml
git commit -m "chore: justfile + mise task runner"
```

---

### Task 3: macOS Info.plist + packaging assets

**Files:**
- Create: `packaging/macos/Info.plist`

**Interfaces:**
- Produces a minimal `Info.plist` for the `.app` bundle: bundle id `io.magpie`, `LSUIElement=true` (agent/tray app — no Dock icon), executable `magpie`, a version string. This makes `package-macos` produce a launchable agent app.

- [ ] **Step 1: Create `packaging/macos/Info.plist`**

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key>
    <string>Magpie</string>
    <key>CFBundleIdentifier</key>
    <string>io.magpie</string>
    <key>CFBundleExecutable</key>
    <string>magpie</string>
    <key>CFBundleVersion</key>
    <string>0.1.0</string>
    <key>CFBundleShortVersionString</key>
    <string>0.1.0</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>LSMinimumSystemVersion</key>
    <string>11.0</string>
    <key>LSUIElement</key>
    <true/>
</dict>
</plist>
```

- [ ] **Step 2: Manual verification**

Run `just package-macos`, then `open target/Magpie.app`. Confirm it launches as a tray/agent app (no Dock icon, tray icon present). Grant Accessibility permission when prompted (needed for auto-paste). Record the result in your report.

- [ ] **Step 3: Commit**

```bash
git add packaging/macos/Info.plist
git commit -m "chore(packaging): macOS Info.plist for agent .app bundle"
```

---

### Task 4: README with build/install/permissions docs

**Files:**
- Create: `README.md`

**Interfaces:**
- Produces the project README: what Magpie is, supported platforms + honest caveats (Wayland source-app = "Unknown"; macOS auto-paste needs Accessibility), build (`just release`), install autostart (`magpie --install-autostart`), config file location + keys, and the privacy model (local-only, secret-marker matrix, denylist, regex ignore). Links the design spec and the plan set.

- [ ] **Step 1: Write `README.md`** covering these sections: Overview · Features (the Raycast gaps fixed + search) · Platforms & caveats · Build (`just release`, expected binary size) · Run & autostart (`--install-autostart`) · Config (`~/Library/Application Support/magpie/config.toml` etc., keys: `launcher_hotkey`, `quick_paste_hotkeys`, `paste_on_select`, `app_denylist`) · Privacy · Architecture (core / platform / app crate split) · Roadmap (Phase 1 items) · Links to `docs/superpowers/specs` and `docs/superpowers/plans`.

- [ ] **Step 2: Verify links resolve**

Confirm the referenced spec/plan paths exist in the repo. (No code test.)

- [ ] **Step 3: Commit**

```bash
git add README.md
git commit -m "docs: project README (build, install, permissions, privacy)"
```

---

### Task 5: Workspace-wide verification gate

**Files:**
- None (verification only) — optionally Create: `.github/workflows/ci.yml` if CI is wanted (host-OS matrix, `cargo test --workspace` + `cargo clippy -- -D warnings`; OS-glue tasks remain manual per their notes).

**Interfaces:**
- Produces a green whole-workspace check: `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`. This is the merge gate for the whole project.

- [ ] **Step 1: Run the gate**

Run:
```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```
Expected: all clean/green on the host OS. Fix any fmt/clippy issues surfaced (they should be minor). Record the final test count in your report.

- [ ] **Step 2 (optional): Add `.github/workflows/ci.yml`**

```yaml
name: ci
on: [push, pull_request]
jobs:
  check:
    strategy:
      matrix:
        os: [macos-latest, ubuntu-latest, windows-latest]
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4
      - run: cargo test --workspace
      - run: cargo clippy --workspace --all-targets -- -D warnings
```

- [ ] **Step 3: Commit**

```bash
git add .github/workflows/ci.yml 2>/dev/null || true
git commit -m "chore: workspace verification gate (+ optional CI matrix)" --allow-empty
```

---

## Self-Review

**Spec coverage:**
- Release build (LTO/strip/small) → Task 1. ✅
- Autostart install/uninstall (all OSes via `platform_autostart`) → Task 1. ✅
- Task runner + packaging recipes → Task 2. ✅
- macOS agent `.app` (LSUIElement, no Dock icon) → Tasks 2, 3. ✅
- Docs incl. honest caveats + privacy + config → Task 4. ✅
- Whole-workspace verification gate → Task 5. ✅

**Placeholder scan:** packaging/doc tasks are inherently recipe/manual; CLI parsing (Task 1) is fully TDD'd. No TODOs.

**Type consistency:** `Command`, `parse_args`, `run_command` consistent; `platform_autostart` matches Plan 4. `runtime::start()` is the agreed GUI entry point (extracted from Plan 3 Task 8).

## Done

With Plans 1–5 complete, Magpie is a shippable v1: a tested pure core, OS adapters behind traits for all three platforms, a Slint tray launcher with first-class search + quick-paste, and one-command build/install. Phase 1 (analytics view, pinned slots, editable items, encryption, event-driven watchers, OCR/QR) builds on this foundation.
