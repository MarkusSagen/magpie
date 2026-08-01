# Magpie

A local-first, cross-platform (macOS · Linux · Windows) clipboard manager. Magpie
captures everything you copy — text, links, rich text, images, and file paths —
and remembers **where** each copy came from (source app, time) and **how much**
you reach for it (a running copy count). It's a single ~10 MB native binary with
no runtime dependencies: Rust core + [Slint](https://slint.dev) UI, no webview.

## Why (the Raycast gaps it fixes)

Raycast's clipboard history is the visual inspiration, but Magpie closes its gaps:

- **First-class search** — live search-as-you-type, word-mode by default, and an
  **exact / fuzzy / regex** toggle. Fuzzy ranks exact-substring matches first.
- **Real filtering & sorting** — by **source app**, **time range** (today / 7
  days), and **content type**; sort by recency / most-copied / alphabetical.
- **Nothing expires** — your history stays until you clear it (no Pro paywall).
- **Quick-paste shortcuts** — `Super+Ctrl+1..9` paste the Nth most-recent entry
  straight into the focused app.
- **Usage tracking** — every copy is logged, so "most-copied" and (Phase 1)
  over-time charts are real.

## Platforms & honest caveats

| | Clipboard capture | Source app | Auto-paste | Autostart |
|---|---|---|---|---|
| **macOS** | ✅ (+ concealed-type detection) | ✅ | ✅ *(needs Accessibility permission)* | LaunchAgent |
| **Windows** | ✅ | ✅ | ✅ | Registry Run key |
| **Linux (X11)** | ✅ | ✅ | ✅ | XDG autostart |
| **Linux (Wayland)** | ✅ | ⚠️ often "Unknown" (compositor security) | ✅ | XDG autostart |

- **Wayland** hides other apps' identity, so source attribution falls back to
  "Unknown" there; capture still works.
- **macOS auto-paste** needs a one-time Accessibility grant (System Settings →
  Privacy & Security → Accessibility).

## Build

Requires Rust (stable) — `mise install` sets up Rust + `just`.

```bash
just release          # optimized ~10 MB binary at target/release/magpie
just test             # run the full test suite
just package-macos    # assemble target/Magpie.app (unsigned)
```

## Run & autostart

```bash
./target/release/magpie              # run the tray app
./target/release/magpie --install-autostart     # start at login
./target/release/magpie --uninstall-autostart   # stop starting at login
```

Trigger the launcher with `Super+Ctrl+V` (configurable). In the launcher: `Enter`
pastes into the focused app, `Cmd/Ctrl+Enter` copies without pasting.

## Config

`~/Library/Application Support/magpie/config.toml` on macOS (Linux
`~/.local/share/magpie/`, Windows `%APPDATA%\magpie\`):

```toml
launcher_hotkey = "super+ctrl+v"
quick_paste_hotkeys = ["super+ctrl+1", "super+ctrl+2", "…", "super+ctrl+9"]
paste_on_select = true          # Enter also fires a synthetic paste
app_denylist = []               # app names to never capture from
```

## Privacy

Magpie is **fully local** — no network, no telemetry. Data lives in the app-data
dir. Sensitive copies are skipped at the capture layer:

- **OS/app secret markers** — macOS `org.nspasteboard.ConcealedType` /
  `TransientType`, Windows `ExcludeClipboardContentFromMonitorProcessing` /
  `CanIncludeInClipboardHistory`, Linux `x-kde-passwordManagerHint`.
- **App denylist** — password managers (1Password, Bitwarden, KeePassXC) by
  default, plus your own additions.
- **Content-regex ignore** — built-in patterns for obvious secrets (AWS keys,
  private keys, JWT-shaped strings).
- **Pause capture** any time.

## Architecture

A Cargo workspace of three crates:

- **`magpie-core`** — pure Rust: SQLite storage (dedup + copy-count + event log),
  content-type detection, and the search engine. No OS or UI; fully unit-tested.
- **`magpie-platform`** — OS clipboard/window/hotkey/paste/autostart behind
  traits, with a cfg-selected factory. Decision logic is pure & tested; adapters
  use `arboard` / `active-win-pos-rs` / `global-hotkey` / `enigo` to minimize FFI.
- **`magpie-app`** — Slint launcher + tray + runtime wiring.

## Roadmap (Phase 1)

Analytics/charts view · pinned quick-paste slots · editable items · snippets ·
link previews / code syntax / QR · at-rest encryption · opt-in retention caps.

## Docs

- Design spec: [`docs/superpowers/specs/2026-08-01-magpie-clipboard-manager-design.md`](docs/superpowers/specs/2026-08-01-magpie-clipboard-manager-design.md)
- Implementation plans: [`docs/superpowers/plans/`](docs/superpowers/plans/)
