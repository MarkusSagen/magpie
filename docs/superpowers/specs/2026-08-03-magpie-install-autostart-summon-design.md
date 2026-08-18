# Magpie — Installable App, Autostart & Summon Hotkey (macOS-first)

**Date:** 2026-08-03
**Status:** Approved design → implementation plan next
**Inspiration:** espanso (Rust background agent: `.app` + Homebrew, LaunchAgent,
`LSUIElement` background process, self-registering service).

## Overview

Turn Magpie from "a binary you run" into an **installable, auto-starting,
reliably-summonable** desktop app — **macOS first**, with Windows/Linux designed as
documented follow-ups. The single most important behavior, built and verified
**first**, is the **global summon hotkey**: press it and Magpie's window comes to
the front over everything, takes keyboard focus, with the search field focused and
the top item selected — ready to type immediately, even though Magpie runs as a
hidden background agent.

## Decisions (from brainstorm, 2026-08-03)

- **Distribution:** `.app` + `.dmg` (drag-to-Applications) now; Homebrew cask is a
  documented later follow-up pointing at the same release artifact.
- **Signing:** **unsigned / ad-hoc** now, with a documented first-run Gatekeeper
  workaround. Developer-ID signing + notarization is a **separate future doc/plan**
  (explicitly not a priority now).
- **Summon hotkey default:** **`Cmd+Shift+Space`** on macOS (`Ctrl+Shift+Space` on
  Windows/Linux). Configurable.

## Current state (what already exists — reuse, don't rebuild)

- `magpie-platform` `MacAutostart` writes `~/Library/LaunchAgents/io.magpie.agent.plist`
  with `Label`, `ProgramArguments`, `RunAtLoad` (`os/autostart.rs`).
- CLI `magpie --install-autostart` / `--uninstall-autostart` → `platform_autostart`
  using `std::env::current_exe()` (`cli.rs`).
- `just package-macos` assembles an **unsigned** `target/Magpie.app` from
  `packaging/macos/Info.plist`.
- Global hotkey path: `parse_hotkey` (supports `shift`, `cmd/super`) →
  `to_global_hotkey` → `global-hotkey`. Default `launcher_hotkey = "super+ctrl+v"`.
- A `show_window(ui, state)` runtime helper (hotkey + tray paths call it).

## Goals / Non-goals

**Goals:** reliable summon (front + focus + selected, over fullscreen/spaces);
proper `LSUIElement` `.app` with icon + versioned bundle; `.dmg` installer; a
**first-run welcome/intro + help screen**; run-at-login as the installed app that
actually takes effect; docs for install + permissions.

**Non-goals (documented follow-ups):** Developer-ID signing/notarization; Homebrew
cask; auto-update; Windows installer (`.msi`) and Linux `.deb`/AppImage
*implementation* (this spec designs macOS and sketches the others).

---

## Task 1 — Summon hotkey: reliable front / focus / highlight (build & verify FIRST)

### 1a. Fix the key map (blocker for the chosen default)
`key_to_code` (`os/hotkeys.rs`) maps only A–Z/0–9, so `"SPACE"` returns
`Err("unsupported hotkey key: SPACE")` — `Cmd+Shift+Space` cannot register today.
Add `Space` (and, cheaply, the other common non-alphanumeric keys so future
bindings don't hit the same wall):

```rust
"SPACE" => Code::Space,
"ENTER" | "RETURN" => Code::Enter,
"TAB" => Code::Tab,
"ESC" | "ESCAPE" => Code::Escape,
"COMMA" => Code::Comma,
"PERIOD" | "DOT" => Code::Period,
"SLASH" => Code::Slash,
"BACKQUOTE" | "GRAVE" => Code::Backquote,
```
Unit-test: `to_global_hotkey(parse_hotkey("super+shift+space"))` is `Ok` with
`META|SHIFT` + `Code::Space`; an unknown key still errors.

### 1b. New default binding
`Config::default().launcher_hotkey = "super+shift+space"` (→ `Cmd+Shift+Space` on
mac, `Ctrl+Shift+Space` on win/linux via the winit meta/ctrl mapping). **Existing
config files keep their saved value** (Default only fills a missing/new config);
document how to rebind, and the future Settings panel exposes it. README/help
updated to the new default.

### 1c. Raise-to-front + focus (the core fix)
A background/agent app does **not** get keyboard focus just from showing a window;
it must activate the process and raise + key the window. Add a platform hook:

```rust
// magpie-platform: trait method or free fn
pub fn raise_to_front(win: &slint::Window);   // no-op default; macOS impl below
```

**macOS impl (objc2-app-kit):**
1. `NSApplication::sharedApplication().activate(ignoringOtherApps: true)` — brings
   Magpie to the foreground even as an `LSUIElement` agent. (Note: on macOS 14+
   this is soft-deprecated in favor of `NSApplication.activate()`; keep
   `ignoringOtherApps:true` for now — it's what Raycast/Alfred-style tools rely on —
   and leave a comment to revisit.)
2. Access the underlying `NSWindow` (via `i-slint-backend-winit`
   `WinitWindowAccessor::with_winit_window` → raw window handle, or objc2 on the
   key window) and:
   - `orderFrontRegardless` + `makeKeyAndOrderFront:nil` (raise + take key focus).
   - `setCollectionBehavior:` `canJoinAllSpaces | fullScreenAuxiliary` and a
     floating `level` so it appears **over fullscreen apps and on the active
     Space** (then restore normal level on hide, or leave floating — decide during
     impl; default: floating panel behavior like a launcher).

**Runtime `show_window` hardening:** call `raise_to_front(win)` **after** `ui.show()`,
then set the view to the search: focus the search input, select the top item
(`selected = 0`), reset preview scroll — so the user types immediately. On
Windows/Linux `raise_to_front` uses winit `focus_window()` + `set_visible(true)`
(the WM handles the rest; documented that Wayland may refuse programmatic focus).

### 1d. Verify FIRST
`timeout 8 ./target/debug/magpie` (exit 124 = alive), then **manual on this Mac**:
from another app, press `Cmd+Shift+Space` → Magpie window appears front-and-center,
search focused, top row selected, typing filters immediately; `Esc` hides it and
returns focus to the previous app. This gate must pass before Task 2.

## Task 2 — Proper macOS `.app` bundle

`packaging/macos/Info.plist` (regenerate/verify) contains:
- `CFBundleIdentifier = io.magpie` (matches the LaunchAgent label root).
- `CFBundleName = Magpie`, `CFBundleExecutable = magpie`.
- `CFBundleShortVersionString` / `CFBundleVersion` = crate version (injected by the
  packaging recipe from `CARGO_PKG_VERSION`, not hand-edited).
- **`LSUIElement = true`** — background agent, no Dock icon (tray + summon only).
- `CFBundleIconFile = Magpie.icns`, `LSMinimumSystemVersion`, copyright.
- No special TCC key needed for Accessibility; the OS prompts on first
  auto-paste (documented).

**Icon:** DONE — a single SVG source lives in `assets/` (`magpie.svg` colored app
icon, `magpie-mono.svg` menu-bar silhouette). `just icons` regenerates the derived
`packaging/macos/Magpie.icns` (all iconset sizes via `iconutil`) and the embedded
menu-bar template `crates/magpie-app/icons/tray-template.png`. Placed in
`Contents/Resources/`. Windows `.ico` / Linux PNGs derive from the same source when
those platforms land.

`just package-macos` (extended): copy binary + generated Info.plist + `Magpie.icns`;
apply **ad-hoc signature** `codesign -s - --force --deep target/Magpie.app` (helps
Accessibility/keychain grants persist across rebuilds and quiets some warnings) —
still "unsigned" for Gatekeeper purposes.

## Task 3 — `.dmg` installer (`just dmg`)

Recipe (pure `hdiutil`, no new dep):
1. `just package-macos` → `target/Magpie.app`.
2. Stage a temp dir with `Magpie.app` + a symlink to `/Applications`.
3. `hdiutil create -volname "Magpie" -srcfolder <stage> -ov -format UDZO
   target/Magpie-<version>.dmg`.
Result: a drag-to-Applications window. (Note `create-dmg` as a prettier optional
alternative with background art + layout — documented, not required.)

**Unsigned first-run:** README documents right-click → **Open** → Open once, or
`xattr -dr com.apple.quarantine /Applications/Magpie.app`.

## Task 4 — Run-at-login that actually takes effect

Reuse `MacAutostart` (LaunchAgent). Two refinements:
- When installed as an app, the LaunchAgent's `ProgramArguments[0]` must be
  `/Applications/Magpie.app/Contents/MacOS/magpie`. `--install-autostart` already
  uses `current_exe()`, which resolves correctly **when run from inside the .app**
  (document: run `Magpie.app`'s binary or `open -a Magpie` then enable, not the
  dev-tree binary).
- **Take effect immediately** (no logout): after writing the plist, `set_enabled`
  best-effort runs `launchctl bootstrap gui/$UID <plist>` (fallback
  `launchctl load -w <plist>`), and `bootout`/`unload` on disable. Failures are
  non-fatal (plist still governs next login). Keep `RunAtLoad=true`; do **not** set
  `KeepAlive` (respect user Quit).
- **First-run UX:** on first GUI launch with autostart not enabled, show a one-time,
  dismissible hint ("Start Magpie at login? — enable in Settings / `--install-autostart`").
  A "Start at login" toggle in the Settings panel is cross-referenced to the
  Settings & Theming spec (wired there, reading `MacAutostart::is_enabled`).

## Task 4b — Reliability: never fail silently (crash surfacing + auto-restart)

Magpie runs in the background, so a silent death is invisible until the user
notices paste/capture stopped. Make failures loud and recoverable.

**Logging.** A `diagnostics` module appends timestamped lines to a rotating log
(`<data_dir>/logs/magpie.log`): startup/shutdown, config/DB errors, captured-panic
details + backtrace. Bounded size (truncate/rotate at a cap) so it never grows
unbounded.

**Panic hook** (`std::panic::set_hook`, installed first thing in `start()`): writes
the panic (thread, location, message, `Backtrace::force_capture`) to the log, leaves
the session marked "running" (so the next launch also notices), and shows a native
alert — `⚠ Magpie hit an error … [View Log] [Quit]` — via `osascript display dialog`
(a separate process, safe to spawn mid-unwind from any thread). Fires for panics on
the main thread *and* background threads.

**Abnormal-exit detection (catches hard crashes / kills too).** On start, read a
session-state file **before** overwriting it: `mark_running()` writes `running` at
startup; `mark_clean()` writes `clean` on a real quit (tray Quit → before
`quit_event_loop`). If the previous value was `running`, the last run died
abnormally → record a crash timestamp and show a next-launch notice: *"Magpie quit
unexpectedly last time — [View Log] [Dismiss]"*.

**Auto-restart + crash-loop guard.** The LaunchAgent gets
`KeepAlive = { SuccessfulExit = false }` — launchd relaunches Magpie only when it
exits non-zero/crashes, **not** on a clean quit (so tray Quit stays quit). launchd's
`ThrottleInterval` (≥10s) rate-limits relaunches. On top of that, Magpie tracks
crash timestamps: **≥3 crashes within 60s = crash loop** → instead of the normal
notice, show *"Magpie is crashing repeatedly — [View Log] [Disable auto-start]
[Quit]"* ("Disable auto-start" calls `MacAutostart::set_enabled(false)` so launchd
stops relaunching a persistently broken build). On a normal recovery (single crash),
notify: *"Magpie recovered from a crash."*

**Background-thread resilience.** The watcher (capture) thread is wrapped so a panic
is logged + surfaced and the thread is **restarted** rather than silently ending
clipboard capture.

**Testing.** Pure/path-injected: session-state read/mark round-trip
(`running`→abnormal, `clean`→normal, missing→first-run); crash-loop detection
(`is_crash_loop(times, now, 60_000, 3)`); log append + rotation cap. OS glue
(osascript alert/notify, panic hook) is launch-verified, not unit-tested.

## Task 5 — First-run welcome / intro & help screen

The first time Magpie launches (freshly installed from the `.dmg`), show a
**welcome screen** that orients the user and gets the two permissions/settings
that make Magpie useful, instead of leaving them to discover the tray.

**When it shows:** first launch only, gated by a `#[serde(default)]`
`welcomed: bool` (default `false`) in `Config` — set `true` after the user
dismisses it (persist via `config::save`). A hidden `--show-welcome` flag / a
"Show intro" item in Help re-opens it for testing and later reference.

**Content (a `mode == "welcome"` overlay, same modal style as Help):**
1. **Hello / what Magpie is** — one line + the menu-bar/no-Dock note.
2. **Summon** — big, clear: “Press **⌘⇧Space** anywhere to open Magpie” (reads
   the live `launcher_hotkey`, so it stays correct if rebound).
3. **Accessibility** — “Auto-paste needs Accessibility permission” + an **Open
   System Settings** button (`open "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility"`)
   and a one-line why. Skippable (capture/search still work without it).
4. **Start at login** — a checkbox that calls the autostart enable path
   (Task 4), defaulting to on for an installed app.
5. **Quick help** — the essential shortcuts (↑/↓ navigate, ⏎ paste, ⌘K actions,
   Esc hide) — reuse the existing Help cheat-sheet rows so there's one source of
   truth; a “Full shortcuts (⌘/ or ?)” link opens the existing Help modal.
6. **Get started** button → sets `welcomed = true`, closes to the list.

**Reuse:** the Help modal (`mode=="help"`) already exists — the welcome screen
embeds/links its shortcut rows rather than duplicating them. No new capture or
storage; `welcomed` is the only state added.

**Testing:** `config.rs` — `welcomed` defaults `false`, round-trips, old configs
load with `false`. Manual: fresh data dir → launch → welcome shows once →
dismiss → relaunch → not shown; `--show-welcome` re-opens.

## Task 6 — Docs & permissions

README "Install" section (macOS): download `Magpie-<version>.dmg` → drag to
Applications → first-run right-click **Open** → grant **Accessibility** (System
Settings ▸ Privacy & Security ▸ Accessibility) for auto-paste → optionally enable
Start-at-login → summon with `Cmd+Shift+Space`. Document the quarantine command and
that Magpie has no Dock icon (it's a menu-bar/tray agent).

## Cross-platform follow-ups (designed, implemented later)

- **Windows:** ship the release `.exe`; installer = later (`.msi` via WiX or a
  simple NSIS/Inno script). Autostart = existing HKCU `...\Run` key. Summon default
  `Ctrl+Shift+Space`. `LSUIElement` equivalent = no console window (already a GUI
  subsystem bin) + tray.
  - **Autostart-at-login:** the HKCU `Run` key already starts it at login; there is
    no launchd-style KeepAlive, so **auto-restart-on-crash** needs either a
    Scheduled Task with "restart on failure", or an in-process supervisor. Deferred
    — for v1 Windows relies on the next-launch notice, not auto-restart.
  - **Crash surfacing:** reuse the shared `diagnostics` core (logging, session
    state, crash-loop); replace the macOS `osascript` alert/notify with a Win32
    `MessageBox` (alert) + toast/`Shell_NotifyIcon` balloon (notify).
- **Linux:** ship the binary + a `.desktop` file; packaging = later (AppImage for
  portability, or `.deb`). Autostart = existing XDG `~/.config/autostart/*.desktop`.
  Wayland may refuse programmatic focus on summon (documented limitation).
  - **Autostart-at-login:** the XDG autostart `.desktop` starts it at login. For a
    systemd-user setup, a `magpie.service` with `Restart=on-failure` +
    `StartLimitIntervalSec`/`StartLimitBurst` gives the crash-loop-guarded
    auto-restart equivalent of launchd KeepAlive (offered as an alternative to XDG
    autostart). Deferred to Linux packaging.
  - **Crash surfacing:** reuse the shared `diagnostics` core; alert via
    `zenity`/`kdialog` (fallback to a desktop notification via `notify-send`).

### Portability of the crash-surfacing design

`diagnostics` is split so the **logic is shared and cross-platform** (log
append/rotate, session-state read/mark, crash-timestamp tracking + crash-loop
detection, panic-hook installation). Only three thin functions are per-OS behind a
trait/`cfg`: `alert(title,msg,buttons)`, `notify(title,msg)`, and `reveal(path)` —
macOS `osascript`/`open`, Windows `MessageBox`/toast/Explorer, Linux
`zenity`/`notify-send`/`xdg-open`. So Windows/Linux inherit all the reliability
behavior for free once those three functions are implemented.

### Icons — one source, all platforms (extends `just icons`)

The single SVG source in `assets/` (`magpie.svg` colored, `magpie-mono.svg`
silhouette) already drives the macOS `.icns` + the embedded menu-bar template via
`just icons`. Extend the **same recipe** to emit the other platforms' formats so no
icon asset is ever hand-maintained per platform:

- **Windows `.ico`** — rasterize `assets/magpie.svg` at 16/24/32/48/64/128/256 and
  pack into `packaging/windows/Magpie.ico` (`magick <pngs> Magpie.ico`). Embedded in
  the `.exe` via a resource script / `winres`/`embed-resource` build step;
  `package-windows` references it. The tray on Windows can reuse the same colored
  small PNG (Windows tray icons are not template images).
- **Linux PNGs** — rasterize `assets/magpie.svg` to
  `packaging/linux/hicolor/<size>/apps/io.magpie.png` at the freedesktop sizes
  (32/48/64/128/256); the `.desktop` file's `Icon=io.magpie` resolves to them.
  `package-linux` installs them under the icon theme dir.

Keep the derived files committed (as macOS already does) so `cargo build` and
`just package-*` never require a rasterizer; `librsvg` is only needed to re-run
`just icons` when the logo changes. Each `package-<platform>` copies its
pre-generated icon into the bundle/installer.

## Out of scope (explicit)

- **Developer-ID signing + notarization** — own future doc/plan (user: "later… not
  priority"). Recipes leave clean hooks (a `SIGN_ID` env → `codesign`/`notarytool`/
  `stapler` step) so it slots in without restructuring.
- Homebrew cask; auto-update; Windows/Linux installer implementation.

## Testing

**Pure/unit (CI-safe):**
- `hotkeys.rs`: `Space`/`Enter`/… map correctly; `super+shift+space` round-trips;
  unknown key errors.
- `config.rs`: default `launcher_hotkey == "super+shift+space"`; existing configs
  keep their value (back-compat test).
- `autostart.rs`: plist for the `.app` program path contains the bundle binary +
  `RunAtLoad`; enable/disable writes/removes (existing tests extended).
- Info.plist generation: `LSUIElement`, `CFBundleIdentifier`, version injected.
- `config.rs`: `welcomed` defaults `false`, round-trips, old configs load `false`.

**Manual (host, can't automate):** Task 1d summon gate; `just dmg` → install → first
launch (Gatekeeper workaround) → Accessibility grant → enable autostart → **log out /
in** → Magpie running, `Cmd+Shift+Space` summons it.

**Gates:** `cargo test`, `clippy --all-targets -D warnings`, `fmt --check`, timeout
launch (124). macOS `NSApp`/`NSWindow` code compiles on this box; win/linux
`raise_to_front` is cfg-gated/compile-only.

## Security / privacy

Unchanged. The LaunchAgent runs the user's own installed binary. Accessibility is
required only for auto-paste (OS-prompted). No new network or data paths. Unsigned
distribution is a *trust/UX* tradeoff (documented), not a change to what Magpie
stores or sends (nothing).

## Implementation task order (one plan)

1. **Summon hotkey** — `Space`+keys map, new default, `raise_to_front` (macOS
   activate + window raise/key + space/fullscreen behavior), `show_window`
   hardening; **verify summon on host first**.
2. **`.app` bundle** — Info.plist (`LSUIElement`, versioned id/version), `.icns`
   icon, `just package-macos` + ad-hoc sign.
3. **`.dmg`** — `just dmg` (hdiutil, /Applications symlink), quarantine docs.
4. **Autostart** — LaunchAgent takes-effect-now (`launchctl bootstrap`),
   `KeepAlive={SuccessfulExit=false}` (auto-restart on crash, not on clean quit),
   app-path program args, first-run hint, Settings cross-ref.
4b. **Reliability / crash surfacing** — `diagnostics` module (log + rotate, panic
   hook + native alert, session-state abnormal-exit detection + next-launch notice,
   crash-loop guard + "Disable auto-start", watcher restart-on-panic).
5. **First-run welcome / help** — `mode=="welcome"` overlay (summon key,
   Accessibility button, start-at-login, quick help reusing the Help rows),
   gated by `welcomed` config flag; `--show-welcome` to re-open.
6. **Docs** — README install + permissions + summon key.
7. **Cross-platform icons** — extend `just icons` to also emit the Windows `.ico`
   (`packaging/windows/Magpie.ico`) and Linux hicolor PNGs
   (`packaging/linux/hicolor/<size>/apps/io.magpie.png`) from the same `assets/`
   SVG source; `package-windows`/`package-linux` reference them. (macOS `.icns` +
   embedded menu-bar template already done.)
