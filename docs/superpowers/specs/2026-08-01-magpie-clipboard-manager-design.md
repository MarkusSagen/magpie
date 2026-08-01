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
closes (all confirmed against user complaints — see *Prior Art* below):

- **Weak search.** Raycast filters by content type only. Magpie has first-class
  search: live search-as-you-type, word-mode by default, an exact/fuzzy/regex
  toggle, ranked + highlighted results, plus filter chips for source app, time
  range, and content type. (See the dedicated *Search* section.)
- **No filtering/sorting by source app, time range, or type.** Magpie filters by
  source app, time range (today / 7 days / custom), and content type, and sorts
  by recency / most-copied / alphabetical.
- **History expires / results disappear (unlimited gated behind Pro).** Magpie
  never expires entries; retention caps are opt-in only.
- **No quick-paste shortcuts.** Magpie binds `Super+Ctrl+1..9` to paste the Nth
  most-recent entry directly.
- **Formatting friction.** Raycast preserves source formatting with no easy
  plain-text default. Magpie makes **paste-as-plain-text** a first-class shortcut.
- **No usage analytics.** Magpie tracks how often each value is copied and (Phase
  1) visualizes copy frequency over time and per app.

### Prior Art (competitive survey)

Design informed by a survey of CopyQ, Maccy, Ditto, Pano/GPaste, Paste, ClipBook,
EcoPaste, Flycut/Clipy, and Windows `Win+V`. What we borrow:

- **Search:** Ditto's *mode toggle* (contains/wildcard/regex), Maccy's *fuzzy +
  match highlighting* (learning from its bugs — exact ranked above fuzzy,
  debounced, case-insensitive), CopyQ's *word-mode* (space-separated terms = AND,
  order-independent), Paste's *filter-by-app/type/date*.
- **Content intelligence:** Pano's *typed previews* (color swatch, image
  thumbnail) and *Tab-through-type-categories*.
- **Power features:** pinned favorites (universal), paste-as-plain-text (Maccy/
  ClipBook), paste-on-select toggle (Maccy).
- **Privacy:** CopyQ's *cross-platform secret-marker matrix* applied at the watch
  layer; Maccy's *content-regex ignore* + *ignore-next-copy* toggle.

Deferred power features (kept out of v1, data model left open for them): CopyQ-style
scripting/custom actions, tabs/collections, tags & notes, snippets/templates,
editable items, merge-multiple, OCR/QR, and encrypted P2P sync.

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
  (`text|link|color|email|rtf|html|image|file`), `preview_text`, `full_text`,
  `image_path`, `byte_size`, `char_count`, `word_count`, `line_count`,
  `first_copied_at`, `last_copied_at`, `copy_count`, `pinned`, `source_app_id`
  (FK → `apps`). `kind` is assigned by the content-type detector at capture
  (see *Content-Type Intelligence*); `color`/`email`/`link` are text entries with
  a recognized shape, so search and paste still treat their `full_text` as text.
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

Secret detection happens at the **watch-daemon layer**, where the clipboard's MIME
metadata is still intact (one-shot paste calls lose it, especially on Wayland).
The cross-platform secret-marker matrix (CopyQ's model) is honored **by default**:

- **macOS:** skip `org.nspasteboard.ConcealedType`, `TransientType`,
  `AutoGeneratedType`; plus a default source-app ignore set for known password
  managers (e.g. `com.agilebits.onepassword`) — Maccy's default list is the seed.
- **Windows:** honor `ExcludeClipboardContentFromMonitorProcessing`,
  `CanIncludeInClipboardHistory` (local history), and `CanUploadToCloudClipboard`
  independently; plus legacy `Clipboard Viewer Ignore`.
- **Linux:** skip `x-kde-passwordManagerHint = secret` on CLIPBOARD and PRIMARY.

Because markers depend on the source app (some password managers have shipped
without them; XWayland can drop them), Magpie also ships its own defenses:

- **App/window ignore list** with optional title regex — set in settings.
  Especially important on Windows, which has no OS-level per-app exclusion.
- **Content-regex ignore** — auto-skip anything matching user patterns (and a
  built-in default set for obvious secrets: keys/tokens/JWT-shaped strings).
- **Pause capture / ignore-next-copy** toggle (tray menu + hotkey).
- **Fully local, no network, no telemetry.** Data stays in the app-data dir.
- **Retention:** unlimited by default (your requirement); optional opt-in caps
  (count and/or time-based) and at-rest encryption (SQLCipher) are Phase 1.

## Search (first-class)

Search is the headline feature. It runs live (search-as-you-type, debounced) over
an FTS5 index of `preview_text` / `full_text`, and is fully keyboard-driven.

- **Default matching = word mode** (CopyQ style): space-separated terms are ANDed,
  order-independent (`foo bar` matches items containing both). Backed by FTS5.
- **Mode toggle in the search field** (Ditto/Maccy): **exact** · **fuzzy** ·
  **regex**. Exact/word results always rank **above** fuzzy matches. Fuzzy is
  non-contiguous, case-insensitive, and debounced (explicitly fixing Maccy's
  documented fuzzy-pollution bugs). Mode persists in settings.
- **Match highlighting** on results (Maccy-style) — cheap, high perceived quality.
- **Filter chips**, combinable with the text query and each other:
  - **Type** — text / link / color / email / image / file (the "All Types" chip).
  - **Source app** — pick an app (with icon) to see only its copies.
  - **Time range** — today / last 7 days / custom.
- **Type tabs:** `Tab` / `Shift+Tab` cycle the type filter without leaving the
  search box (Pano); `Backspace` on an empty query clears the active type filter.
- **Sort:** recency / most-copied / alphabetical.
- **Results never expire.**

**Performance:** the result list is **virtualized/lazy-loaded** from day one (only
visible rows are realized) — avoiding Maccy's load-everything cap and matching
Lumen's virtualized grid approach. FTS5 keeps queries fast at large history sizes.

## Content-Type Intelligence

At capture, a lightweight detector classifies each item (cheap regex/heuristics)
and sets `kind`, enabling typed previews and the type filter:

- **Detected types (v1):** `link` (URL), `color` (hex / `rgb()` / `hsl()`),
  `email`, `image`, `file` (path), else `text` (`rtf`/`html` retained when the
  clipboard offers rich variants).
- **Typed previews:** color → **swatch** with the value; image → **thumbnail**;
  link → the URL (favicon/social-card preview is Phase 1); everything else → text
  preview with char/word/line counts.

Deferred (Phase 1+): code detection + syntax highlighting, number detection, QR
generate/extract, OCR-inside-images.

## UI (Slint) — Raycast-inspired

- **Top:** search field (with the mode toggle) + type filter chip ("All Types") +
  source-app and time-range chips.
- **Left:** date-grouped list (Today / Yesterday / older), virtualized.
- **Right:** detail pane + an **Information** panel showing source app (+ icon),
  content type, characters, words, lines, copy count, and first/last-copied times.
- **Bottom bar:** "Paste to \<app\>" + Actions.
- **Keyboard actions:** `Enter` = set clipboard + auto-paste into the focused app;
  `Cmd/Ctrl+Enter` = copy to clipboard only; **paste-as-plain-text** shortcut
  strips formatting; `1..9` (in-window) selects by position. A global
  **paste-on-select vs paste-key** preference toggles the default `Enter`
  behavior.
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
- Content-type detection (link / color / email / image / file / text) with typed
  previews (color swatch, image thumbnail).
- SQLite storage: dedup + count, `copy_events` log, content-addressed image cache,
  FTS5 index.
- Slint launcher: **first-class search** (live, word-mode default, exact/fuzzy/
  regex toggle, highlighted, virtualized) + filter chips (type / app / time) +
  sort + type tabs + list/detail.
- Global hotkeys: launcher toggle + quick-paste `1..9` with set-clipboard +
  auto-paste.
- Paste actions: auto-paste, copy-only, **paste-as-plain-text**, paste-on-select
  toggle. Pinned favorites.
- Privacy: cross-platform secret-marker matrix + app/window ignore list +
  content-regex ignore + pause/ignore-next toggle.
- Autostart install recipes for all three platforms.

### Phase 1 (future)

- Analytics/charts view (most-copied, over-time, per-app).
- Pinned quick-paste slots (persistent `1..9`) + two-digit select for large lists.
- Editable items; merge-multiple with separator; snippets/templates.
- Link social-card previews; code detection + syntax highlighting; QR; OCR search.
- Event-driven watchers on every platform.
- At-rest encryption (SQLCipher); opt-in retention caps (count/time/cache size).
- Kept-open-for-later: tabs/collections, tags & notes, scripting/custom actions,
  encrypted P2P/LAN sync.

## Open Questions / Risks

- **Slint polish vs Raycast:** achieving a close-to-Raycast aesthetic in Slint is
  achievable but requires manual styling effort; accepted as "close, not
  identical."
- **Launcher show latency:** the Slint window must be pre-created and hidden (not
  recreated per invocation) to feel instant on hotkey.
- **Wayland attribution** and **macOS Accessibility permission** are the two known
  platform limitations, both documented above.
