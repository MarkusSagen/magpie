# Magpie — Cross-Platform Clipboard Manager — Design

**Date:** 2026-08-01
**Status:** Approved (brainstorm complete)
**Author:** Markus + Claude

## Summary

**Magpie** is a local-first, cross-platform (macOS / Linux / Windows) clipboard
manager. It captures everything the user copies — text, links, rich text, images,
and file paths — and records rich provenance for each copy: the source
application, timestamp, and content metrics (characters, words, lines, bytes).
Identical content is deduplicated into a single entry with a running **copy
count**, and every individual copy is also logged so usage can be visualized over
time.

It is built as a single self-contained Rust binary running as a background tray
process (the "daemon"), with a Slint native GUI launcher. There is **no webview
and no runtime dependency** — the goal is minimal memory, minimal bundle size,
minimal disk, and dead-simple cross-platform shipping.

### Why this exists (the Raycast gaps we fix)

The Raycast clipboard manager is the visual inspiration, but it has gaps Magpie
closes:

- **No filtering/sorting by source app, time range, or type.** Magpie filters by
  source app, time range (today / 7 days / custom), and content type, and sorts
  by recency / most-copied / alphabetical.
- **History expires / results disappear.** Magpie never expires entries.
- **No quick-paste shortcuts.** Magpie binds `Super+Ctrl+1..9` to paste the Nth
  most-recent entry directly.
- **No usage analytics.** Magpie tracks how often each value is copied and (Phase
  1) visualizes copy frequency over time and per app.

## Locked Decisions

- **Language:** Rust. Chosen over Zig/Odin because every hard OS-integration task
  (clipboard incl. images, global hotkeys, synthetic paste, tray, source-app
  detection, SQLite) already exists as a mature crate — giving minimal footprint
  *and* minimal effort. Static builds (LTO + strip) keep binaries small (~5–15 MB)
  with no runtime deps.
- **GUI:** Slint — declarative, minimal footprint, software-or-GPU renderer, no
  webview. Gets close to the Raycast look without WebKitGTK (the main Linux
  cross-platform-shipping headache in a Tauri stack).
- **Storage:** SQLite via `rusqlite` with the `bundled` feature (SQLite compiled
  statically into the binary — zero system dependency). WAL mode + FTS5 search.
- **Capture:** text, links, rich text (RTF/HTML), images, and file paths.
- **Dedup model:** identical content collapses to one `entries` row with a
  `copy_count`; a lightweight append-only `copy_events` log records each copy for
  over-time analytics.
- **Quick-paste behavior:** set the clipboard **and** synthesize a paste
  keystroke into the focused app (auto-paste).
- **Audience:** personal-first, but architected clean enough to sign / notarize /
  distribute later.
- **Retention:** no expiry in v1. Optional retention caps are Phase 1.

## Architecture

### One process, one binary

A background tray application owns a single `winit` event loop shared by the tray
icon, global hotkeys, and the Slint window. A dedicated clipboard-watcher thread
feeds capture events into the core over a channel. The "daemon" is simply this
process running in the background/tray — no separate OS service is required
(cleanest across all three platforms).

### Crate layout (mirrors Lumen's core/platform/app split for testability)

- **`magpie-core`** (lib): domain logic only — models, dedup + count, search,
  filters, sorting, SQLite access. No OS or UI calls, so it is fully unit-testable
  with injected fake capture events.
- **`magpie-platform`** (lib): all OS integration behind traits —
  `ClipboardWatcher`, `SourceApp`, `Hotkeys`, `Paster`, `Autostart`. Real
  per-OS implementations; fakes injected in tests (same pattern as Lumen's
  `ImageEmbedder`/`FaceEngine` traits).
- **`magpie-app`** (bin): wires platform + core + Slint UI + tray + event loop.

### Data flow

```
watcher thread → CaptureEvent{content, kind, timestamp} + source-app metadata
   → core: hash content (blake3) → dedup
       ├ hash exists  → copy_count++, last_copied_at = now, insert copy_event
       └ new          → insert entry + first copy_event (+ image bytes → cache)
   → SQLite (WAL)
UI ← core queries (filtered / sorted / FTS-searched)
```

## Data Model (SQLite)

- **`entries`**: `id`, `content_hash` (unique), `kind`
  (`text|link|rtf|html|image|file`), `preview_text`, `full_text`, `image_path`,
  `byte_size`, `char_count`, `word_count`, `line_count`, `first_copied_at`,
  `last_copied_at`, `copy_count`, `pinned`, `source_app_id` (FK → `apps`).
- **`apps`**: `id`, `bundle_id_or_exe`, `display_name`, `icon_path`.
- **`copy_events`**: `id`, `entry_id` (FK), `copied_at`, `source_app_id` (FK).
  Append-only log powering over-time analytics.
- **FTS5** virtual table over `preview_text` / `full_text` for instant search.
- **Image storage:** content-addressed cache directory keyed by `content_hash`,
  storing the image bytes plus a small list thumbnail (mirrors Lumen's thumbnail
  cache). Dedup falls out of the hash naturally.

The DB and cache live in the platform app-data dir (e.g. macOS
`~/Library/Application Support/magpie`, Linux `~/.local/share/magpie`, Windows
`%APPDATA%\magpie`).

## OS Integration & Per-Platform Caveats

| Capability | macOS | Windows | Linux |
|---|---|---|---|
| Clipboard read (incl. images) | `arboard` | `arboard` | `arboard` |
| Change detection | poll `NSPasteboard.changeCount` (~250 ms, cheap) | `AddClipboardFormatListener` event (`WM_CLIPBOARDUPDATE`) | X11 XFIXES selection-notify / Wayland `wlr-data-control` |
| **Source app** | `NSWorkspace.frontmostApplication` (name + bundle id + icon) ✅ | foreground window → PID → exe name + icon ✅ | X11 `_NET_ACTIVE_WINDOW` → `_NET_WM_PID`/`WM_CLASS` ✅ / **Wayland: best-effort, often "Unknown"** ⚠️ |
| Global hotkeys | `global-hotkey` | `global-hotkey` | `global-hotkey` |
| Auto-paste | `enigo` Cmd+V (needs **Accessibility** permission) | `enigo` Ctrl+V | `enigo` Ctrl+V |
| Autostart | LaunchAgent plist | registry `Run` key | systemd user unit / XDG autostart `.desktop` |

**Honest caveats:**

- **Wayland source attribution** is limited: compositor security hides other
  apps' identity, so those entries fall back to `source_app = Unknown`. Clipboard
  content itself is still captured via `wlr-data-control`.
- **macOS auto-paste** requires the user to grant Accessibility permission (a
  one-time system prompt on first paste).
- **Change detection** may start as a uniform ~250 ms poll for v1 simplicity;
  the event-driven paths (Windows listener, X11 XFIXES) are an optimization that
  can land per-platform behind the `ClipboardWatcher` trait.

## Global Hotkeys

- **Launcher toggle:** configurable (default e.g. `Super+Ctrl+V`) → show/hide the
  Slint window.
- **Quick-paste `Super+Ctrl+1..9`:** paste the Nth most-recent entry. Sequence:
  set clipboard → hide our window (focus returns to the previously-active app) →
  brief delay → `enigo` fires the paste keystroke.
- Pinned "slots" assigned to `1..9` (persistent, independent of recency) are a
  Phase-1 upgrade.

## Privacy & Security

- **Skip sensitive copies:** honor `org.nspasteboard.ConcealedType` /
  `org.nspasteboard.TransientType` (macOS) and
  `ExcludeClipboardContentFromMonitorProcessing` (Windows) so password managers
  and other transient copies are never logged.
- **App denylist** in settings (skip capture from named apps).
- **Fully local, no network.** Data stays in the app-data dir. Optional at-rest
  encryption is Phase 1.

## UI (Slint) — Raycast-inspired

- **Top:** search field + type filter dropdown ("All Types").
- **Left:** date-grouped list (Today / Yesterday / older).
- **Right:** detail pane + an **Information** panel showing source app (+ icon),
  content type, characters, words, lines, copy count, and first/last-copied times.
- **Bottom bar:** "Paste to \<app\>" + Actions.
- **Filters (the Raycast gaps):** by source app, by time range (today / 7 days /
  custom), by type; sort by recency / most-copied / alphabetical; results never
  expire.
- **Analytics view (Phase 1):** most-copied entries, copies-over-time chart, and
  per-app breakdown — powered by `copy_events`.

Renderer: Slint's `winit` backend with the FemtoVG/GL renderer, with the software
renderer as a fallback for maximum compatibility and minimal GPU-driver risk.

## Testing (Lumen-style)

- **core:** unit tests via injected fake `CaptureEvent`s — dedup/count, search,
  filters, sorting. No OS or display required.
- **platform:** trait fakes in CI; real per-OS implementations behind `cfg` with
  documented manual smoke tests (require a display / permissions — not
  CI-verified, exactly like Lumen's optional `ai` feature).
- **UI:** view-model logic in Rust unit-tested; `.slint` markup kept thin.

## Packaging & Distribution

- `cargo build --release` per target with LTO + strip; static SQLite via
  `rusqlite` bundled. Consider `musl` on Linux for a fully static binary.
- Autostart install helpers per OS: macOS LaunchAgent plist, Windows registry
  `Run` key / Startup, Linux systemd user unit or XDG autostart `.desktop`.
- Task runner: `just` + `mise` (matches the Lumen setup).
- Personal-first: no signing/notarization/auto-update in v1, but the crate split
  and packaging structure leave room to add them without rework.

## Scope & Phasing

### Phase 0 (v1 — this spec)

- Tray daemon + shared `winit` event loop.
- Clipboard watcher for text / link / image / file with source-app attribution
  (macOS + Windows + Linux X11 fully; Wayland best-effort).
- SQLite storage: dedup + count, `copy_events` log, content-addressed image cache.
- Slint launcher: search + filter (type / app / time) + sort + list/detail.
- Global hotkeys: launcher toggle + quick-paste `1..9` with set-clipboard +
  auto-paste.
- Privacy: honor concealed/transient markers + app denylist.
- Autostart install recipes for all three platforms.

### Phase 1 (future)

- Analytics/charts view (most-copied, over-time, per-app).
- Pinned quick-paste slots.
- Event-driven watchers on every platform.
- At-rest encryption.
- Retention caps (e.g. max image-cache size).

## Open Questions / Risks

- **Slint polish vs Raycast:** achieving a close-to-Raycast aesthetic in Slint is
  achievable but requires manual styling effort; accepted as "close, not
  identical."
- **Launcher show latency:** the Slint window must be pre-created and hidden (not
  recreated per invocation) to feel instant on hotkey.
- **Wayland attribution** and **macOS Accessibility permission** are the two known
  platform limitations, both documented above.
