# Magpie — App Icons, Link Favicons, ⌘K Actions, Help Menu & Polish

**Date:** 2026-08-02 · **Status:** Approved · **Phase:** 1 · **One combined plan**

## Summary

Five Raycast-inspired improvements in one plan:
1. **Source app icons** — capture + store the source application's icon per OS and
   show it (Source metadata line + list rows), falling back to the type glyph.
2. **Link-site favicons** — for URL entries, show the site's own favicon (fetched
   directly from the site, cached; privacy-first, no third-party service).
3. **Searchable ⌘K actions** — a filterable actions menu with per-action icons +
   shortcuts, including a new **Delete**.
4. **Help cheat sheet** — a shortcuts view (⌘/ or `?`).
5. **UI polish** — "Information" metadata header with right-aligned values (+ Words,
   Copied date), date group headers, and "Paste to [icon] <target app>" in the bar.

**Verification split:** macOS icons + favicons (loopback-testable logic) + ⌘K +
Help + polish are buildable and verifiable here; Windows/Linux icon extraction is
cfg-gated + compile-checked only (the project's established per-OS pattern).

## 1. Source App Icons

**Resolve in the platform layer** (where `active-win`'s `process_path` lives), so
`AppInfo` and its 18 constructors stay unchanged.

- New platform module `app_icons`:
  - `icon_cache_path(dir: &Path, app_identifier: &str) -> PathBuf` — `<dir>/<blake3(id)>.png`. Pure, tested.
  - `ensure_app_icon(dir, app_identifier, exe_path: &Path) -> Option<PathBuf>` — return the cached path if the file exists; else `app_icon_png(exe_path)` → write → return. The "reuse if present" branch is tested (pre-create the file); OS extraction is manual/integration.
  - `fn app_icon_png(exe_path: &Path) -> Option<Vec<u8>>` — cfg per-OS:
    - **macOS** (`objc2-app-kit`): derive the `.app` bundle from `exe_path`
      (`…/Foo.app/Contents/MacOS/foo` → `…/Foo.app`); `NSWorkspace::sharedWorkspace().iconForFile(bundle)` → `NSImage` → 64px PNG via `NSBitmapImageRep`.
    - **Windows** (cfg): `SHGetFileInfoW(exe_path, SHGFI_ICON)` → `HICON` → 32-bit
      DIB → PNG; `DestroyIcon`. Needs `windows` features `Win32_UI_Shell`,
      `Win32_Graphics_Gdi`, `Win32_UI_WindowsAndMessaging`.
    - **Linux** (cfg): basename/app-name → `.desktop` lookup → `Icon=` → resolve in
      the freedesktop icon theme (`freedesktop-icons` crate) → load → PNG. Best-effort.
- **`ActiveWinSource` gains `cache_dir: PathBuf`.** `frontmost()` computes the
  cache path from the app identifier; if present, sets `icon_path`; else resolves
  via `ensure_app_icon(cache_dir, id, process_path)` and sets `icon_path`. Resolves
  **once per app** (file-existence cache). Only construction site is `spawn_watcher`
  (`ActiveWinSource { cache_dir: data_dir().join("app_icons") }`).
- **Store:** `apps.icon_path` already exists and is written by `upsert_app`; no
  schema change. New core helper `Store::app_icon_pairs() -> Vec<(i64, String)>`
  (entry_id → app icon_path, non-null only), alongside `app_name_pairs`.

## 2. Link-site Favicons

- New app module `favicon`:
  - `domain_of(url: &str) -> Option<String>` — host from an `http(s)` URL, lowercased, no port. Pure, tested.
  - `favicon_cache_path(dir, domain) -> PathBuf` — `<dir>/<domain>.png`. Pure, tested.
  - `ensure_favicon(dir, domain, fetch: impl Fn(&str) -> Option<Vec<u8>>) -> Option<PathBuf>` — cached path if present, else `fetch(domain)` → write. Tested with an injected fetcher (no network in tests).
  - `fetch_favicon(domain) -> Option<Vec<u8>>` — `ureq` GET `https://<domain>/favicon.ico`, short timeout, size cap; **the site directly, never a third-party favicon service** (privacy). Real network; not unit-tested.
- **Wire:** the `spawn_watcher` loop, after ingesting a **link** entry, best-effort
  `domain_of` + `ensure_favicon` under `…/magpie/favicons/`. Config
  `fetch_link_favicons: bool` (`#[serde(default = "…true")]`) gates it.
- **New dep:** `ureq` (blocking, tiny) on `magpie-app`.

## 3. Icon rendering (shared by 1 & 2)

- `EntryRow` gains `icon: image` and `has_icon: bool`.
- `to_rows` chooses the icon path per entry: **link + cached favicon → favicon**;
  else **app icon** (`app_icon_pairs`); else none. Loads via
  `slint::Image::load_from_path`; sets `has_icon` on success.
- Slint: where a row/Source line shows the glyph, render
  `if row.has-icon: Image { source: row.icon; } else: Text { text: row.glyph; }`.
- Decoding per refresh is acceptable for 64px icons (noted; cache later if needed).

## 4. Searchable ⌘K Actions

- The palette gains a **search field** (`actions-query`) that filters the action
  list; `↑/↓` move within the filtered set, `⏎` runs, `Esc` closes.
- Each action row shows an **icon (emoji) + label + shortcut**. Actions: Paste ⏎,
  Copy ⌘C, Paste & keep open ⌥⏎, Edit ⌘E, New snippet ⌘N, Pin ⌘P, Add to merge ⌘M,
  Delete ⌃X, plus the slot 1-9 row.
- **Paste & keep open** = paste without hiding the window (new `activate-keep(int)`
  runtime path; existing paste path but skip hide — the window is already shown, so
  it just doesn't dismiss).
- **Delete**: new core `Store::delete_entry(id) -> Result<Removed>` (reuse the
  retention `delete_entries` transaction with a single id) returning image paths;
  runtime `on_delete_entry` removes images + refreshes. Tested in core.

## 5. Help Cheat Sheet

- A `view == "help"` screen (toggled by `⌘/` in the FocusScope and a `?` button in
  the filter row; `Esc`/`?` returns). Static, grouped shortcut list (Navigation /
  Actions / Views), dependency-free `Text` rows.

## 6. UI Polish

- **Metadata "Information" block:** a header label, right-aligned values, add
  **Words** (`EntryRow.words`) and **Copied** (absolute date via a new
  `format_time::abs_date(ms)`), matching Raycast.
- **Date group headers:** each `EntryRow` gets a `section: string` (from
  `grouping::section_for(last_copied, now)`), and the list renders a small header
  above a row when its `section` differs from the previous row's. In Slint the
  previous row is `entries[i - 1]`; the first row (`i == 0`) always shows its
  header. No parallel model — one `section` field drives it.
- **Action bar target app:** `show_window` captures the frontmost app (before
  showing) via `ActiveWinSource.frontmost()`; the bar shows "Paste to [icon]
  <name>". Stored as window properties `target-app`, `target-icon`, `target-has-icon`.

## Architecture / Files

- **core:** `mask_support.rs` gains `app_icon_pairs`; new `Store::delete_entry` (in
  `retention.rs` or a small `delete.rs`); tests.
- **platform:** new `app_icons.rs` (`icon_cache_path`, `ensure_app_icon`,
  cfg `app_icon_png`); `os/source_app.rs` (`ActiveWinSource { cache_dir }`); Cargo
  target deps (win Shell/Gdi features; linux `freedesktop-icons`).
- **app:** new `favicon.rs`; `format_time::abs_date`; `config.rs`
  (`fetch_link_favicons`); `runtime.rs` (icon/favicon wiring, `to_rows` icon choice
  + `words`/`section`, delete/keep-open/help callbacks, target-app capture); Cargo
  `ureq`; `ui/launcher.slint` (icons, searchable ⌘K, Help view, metadata/polish).

## Data Flow

```
capture (watcher):
  frontmost() -> resolves+caches app icon -> AppInfo.icon_path -> upsert_app
  if link entry & fetch_link_favicons: ensure_favicon(domain) -> favicons/<domain>.png
refresh -> to_rows:
  icon_path = (link && favicon cached) ? favicon : app_icon_pairs[id]
  EntryRow.icon = Image::load_from_path(icon_path); has_icon = ok
paste: unchanged (real full_text). Delete/keep-open are new runtime paths.
show_window: capture target app (frontmost) + its icon for the action bar.
```

## Error Handling

- Icon/favicon resolution failures → `None` → row shows the type glyph. Never fatal.
- Favicon fetch: short timeout + size cap; network/DNS failure → skip silently.
- `slint::Image::load_from_path` failure → `has_icon = false`.
- Windows/Linux extractors unavailable/None → glyph fallback.
- `domain_of` on non-URL/opaque → `None`.

## Testing

**Pure/unit (verifiable here):**
- `icon_cache_path` deterministic + collision-safe; `ensure_app_icon` reuses an
  existing file without calling the extractor.
- `domain_of`: `https://github.com/x` → `github.com`; strips `www.`? (no — keep host
  as-is except lowercase + drop port); non-URL → None. `favicon_cache_path` shape.
  `ensure_favicon` with an injected fetcher writes + reuses.
- `Store::delete_entry` removes the entry (+ copy_events/tags/slots), returns image
  paths, leaves others intact.
- `format_time::abs_date(ms)` → `YYYY-MM-DD` (reuses the civil-date helper).
- core `app_icon_pairs` returns `(id, icon_path)` for apps with a non-null icon.

**Manual run-verify (macOS):** copy from several apps → Source line shows each app's
icon; copy a URL → its site favicon appears (network); ⌘K → searchable actions with
icons, Delete works, Paste & keep open pastes without closing; ⌘/ → Help sheet;
metadata shows Information/Words/Copied; date headers group the list; action bar
shows "Paste to [icon] <target>".

## Risks / Notes

- **Windows/Linux icons are compile-checked only** here (no runtime verify), like the
  existing per-OS clipboard/autostart adapters. Linux icon theming is best-effort.
- **Favicon fetch = network + privacy:** we hit only the site's own `/favicon.ico`,
  never a third-party aggregator; gated by `fetch_link_favicons` (default on).
- **Image decode per refresh** is fine at 64px; add an in-memory `path -> Image`
  cache only if it shows up in profiling.
- Large surface: the plan orders tasks so the app builds + runs after each, with the
  verifiable macOS/logic pieces first and cfg-gated OS code last.
