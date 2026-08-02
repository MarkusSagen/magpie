# Magpie Icons + Favicons + ⌘K + Help + Polish Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Source app icons (per-OS) + link-site favicons shown per entry; a searchable ⌘K actions menu (with Delete + Paste-and-keep-open); a Help shortcut cheat sheet; and Raycast-style metadata/date-header/action-bar polish.

**Architecture:** App icons resolve in the platform layer (`ActiveWinSource` gets a cache dir; `frontmost()` fills `AppInfo.icon_path`), so `AppInfo` is unchanged. Favicons + icon-path selection + all UI live in the app layer. Pure helpers are unit-tested; OS icon extraction (macOS verified, Windows/Linux cfg-compiled) and network favicon fetch are manual/integration.

**Tech Stack:** Rust, Slint 1.17.1, `objc2-app-kit` (macOS), `windows` 0.58 (Windows), `freedesktop-icons` (Linux), `ureq` (favicon fetch), `rusqlite`.

## Global Constraints

- Toolchain pinned `rust-toolchain.toml` (1.97.1). Run cargo as `env -u RUSTUP_TOOLCHAIN ~/.cargo/bin/cargo …`.
- Gates: `cargo test` (all green), `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, `timeout 8 … run` (exit 124 = alive).
- Paste always uses the real `full_text`. Icons/favicons are display-only.
- Favicon fetch hits **only** the site's own `https://<domain>/favicon.ico` — never a third-party favicon aggregator (privacy). Gated by `Config.fetch_link_favicons`.
- Windows/Linux icon extractors are cfg-gated + compile-checked only (no runtime verify on this Mac), matching the existing per-OS adapter pattern.
- `EntryRow` field order must match between the Slint struct and every `to_rows` literal.

---

### Task 1: core — `app_icon_pairs` + `delete_entry`

**Files:**
- Modify: `crates/magpie-core/src/mask_support.rs` (add `app_icon_pairs`)
- Create: `crates/magpie-core/src/delete.rs` (add `Store::delete_entry`); `pub mod delete;` in `lib.rs`

**Interfaces:**
- Produces: `Store::app_icon_pairs(&self) -> Result<Vec<(i64, String)>>` (entry_id → apps.icon_path, non-null only); `Store::delete_entry(&self, id: i64) -> Result<Removed>`.

- [ ] **Step 1: Failing test for `app_icon_pairs`** — in `mask_support.rs` tests, add an ingest-with-icon helper and:

```rust
    #[test]
    fn icon_pairs_return_non_null_icon_paths() {
        let s = open_in_memory().unwrap();
        // app with an icon
        let a = s.ingest(&CaptureEvent {
            content: Content::Text("x".into()),
            source_app: Some(AppInfo { identifier: "Foo".into(), display_name: "Foo".into(), icon_path: Some("/i/foo.png".into()) }),
            copied_at_ms: 1,
        }, &Noop).unwrap().entry_id;
        // app without an icon
        let _b = s.ingest(&CaptureEvent {
            content: Content::Text("y".into()),
            source_app: Some(AppInfo { identifier: "Bar".into(), display_name: "Bar".into(), icon_path: None }),
            copied_at_ms: 2,
        }, &Noop).unwrap().entry_id;
        assert_eq!(s.app_icon_pairs().unwrap(), vec![(a, "/i/foo.png".to_string())]);
    }
```

- [ ] **Step 2: Run → fails.** `env -u RUSTUP_TOOLCHAIN ~/.cargo/bin/cargo test -p magpie-core icon_pairs`.

- [ ] **Step 3: Implement `app_icon_pairs`** in `mask_support.rs`:

```rust
    /// `(entry_id, app icon_path)` for entries whose source app has a non-null icon.
    pub fn app_icon_pairs(&self) -> Result<Vec<(i64, String)>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT e.id, a.icon_path
             FROM entries e JOIN apps a ON a.id = e.source_app_id
             WHERE a.icon_path IS NOT NULL
             ORDER BY e.id",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
        rows.collect()
    }
```

- [ ] **Step 4: Failing test for `delete_entry`** — create `crates/magpie-core/src/delete.rs`:

```rust
use crate::retention::Removed;
use crate::store::{Result, Store};
use std::collections::BTreeSet;

impl Store {
    /// Delete a single entry and its copy_events/tags/slots. Returns the image
    /// paths that the caller should remove from disk (core stays FS-free).
    pub fn delete_entry(&self, id: i64) -> Result<Removed> {
        let mut set = BTreeSet::new();
        set.insert(id);
        self.delete_entries(set)
    }
}

#[cfg(test)]
mod tests {
    use crate::model::{CaptureEvent, Content};
    use crate::store::{open_in_memory, ImageStore, Store};

    struct Noop;
    impl ImageStore for Noop {
        fn put(&self, h: &str, _b: &[u8]) -> std::io::Result<String> { Ok(h.to_string()) }
    }
    fn ingest(s: &Store, t: &str) -> i64 {
        s.ingest(&CaptureEvent { content: Content::Text(t.into()), source_app: None, copied_at_ms: 1 }, &Noop).unwrap().entry_id
    }

    #[test]
    fn delete_removes_only_the_target() {
        let s = open_in_memory().unwrap();
        let a = ingest(&s, "keep");
        let b = ingest(&s, "drop");
        s.delete_entry(b).unwrap();
        let remaining: Vec<i64> = s.recent(100).unwrap().into_iter().map(|e| e.id).collect();
        assert!(remaining.contains(&a));
        assert!(!remaining.contains(&b));
    }
}
```

Requirements to confirm first: `delete_entries` visibility (it is `fn delete_entries(&self, victims: BTreeSet<i64>)` in `retention.rs` — make it `pub(crate)` if not already: `grep -n "fn delete_entries" crates/magpie-core/src/retention.rs`). `Removed` is `pub` in `retention.rs`. `Store::recent` exists (used elsewhere). Adjust the test's listing call if `recent` differs.

- [ ] **Step 5: Add `pub mod delete;`** to `lib.rs` (after `pub mod delete;` alphabetical — before `detect`). Make `delete_entries` reachable (`pub(crate)`).

- [ ] **Step 6: Run → PASS.** `cargo test -p magpie-core delete_removes` and `icon_pairs`.

- [ ] **Step 7: Commit** `feat(core): app_icon_pairs + single-entry delete_entry`.

---

### Task 2: app — `format_time::abs_date`

**Files:**
- Modify: `crates/magpie-app/src/format_time.rs`

**Interfaces:**
- Produces: `pub fn abs_date(ms: i64) -> String` → `YYYY-MM-DD` (UTC), reusing `civil_from_days`.

- [ ] **Step 1: Failing test** — add to `format_time.rs` tests:

```rust
    #[test]
    fn abs_date_formats_epoch_day() {
        // 1000 days after epoch = 1972-09-27
        assert_eq!(super::abs_date(1_000 * DAY), "1972-09-27");
    }
```

- [ ] **Step 2: Run → fails.**

- [ ] **Step 3: Implement** (make `civil_from_days` reachable — it's already in the module):

```rust
/// Absolute UTC date `YYYY-MM-DD` for an epoch-millis timestamp.
pub fn abs_date(ms: i64) -> String {
    let (y, m, d) = civil_from_days(ms.div_euclid(86_400_000));
    format!("{y:04}-{m:02}-{d:02}")
}
```

- [ ] **Step 4: Run → PASS.**

- [ ] **Step 5: Commit** `feat(app): abs_date formatter`.

---

### Task 3: app — `favicon` module + config toggle + `ureq`

**Files:**
- Modify: `crates/magpie-app/Cargo.toml` (add `ureq`)
- Create: `crates/magpie-app/src/favicon.rs`; `pub mod favicon;` in `lib.rs`
- Modify: `crates/magpie-app/src/config.rs` (`fetch_link_favicons`)

**Interfaces:**
- Produces: `domain_of(&str) -> Option<String>`; `favicon_cache_path(&Path, &str) -> PathBuf`; `ensure_favicon(&Path, &str, impl Fn(&str)->Option<Vec<u8>>) -> Option<PathBuf>`; `fetch_favicon(&str) -> Option<Vec<u8>>`.

- [ ] **Step 1: Add `ureq`** to `crates/magpie-app/Cargo.toml` `[dependencies]`: `ureq = "2"` (confirm a 2.x resolves; it is a small blocking client).

- [ ] **Step 2: Write the module with failing-first tests** — create `favicon.rs`:

```rust
//! Best-effort site favicons for link entries. Fetches the site's OWN
//! /favicon.ico only — never a third-party favicon aggregator (privacy).

use std::path::{Path, PathBuf};

/// Lowercased host of an http(s) URL, without port. `None` for non-URLs.
pub fn domain_of(url: &str) -> Option<String> {
    let rest = url.strip_prefix("http://").or_else(|| url.strip_prefix("https://"))?;
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = host.split('@').last().unwrap_or(host); // drop userinfo
    let host = host.split(':').next().unwrap_or(host);  // drop port
    if host.is_empty() || !host.contains('.') {
        return None;
    }
    Some(host.to_lowercase())
}

/// `<dir>/<domain>.png`.
pub fn favicon_cache_path(dir: &Path, domain: &str) -> PathBuf {
    dir.join(format!("{domain}.png"))
}

/// Cached favicon path if present; else fetch + write. `fetch` is injected so the
/// logic is testable without network.
pub fn ensure_favicon(
    dir: &Path,
    domain: &str,
    fetch: impl Fn(&str) -> Option<Vec<u8>>,
) -> Option<PathBuf> {
    let path = favicon_cache_path(dir, domain);
    if path.exists() {
        return Some(path);
    }
    let bytes = fetch(domain)?;
    if bytes.is_empty() {
        return None;
    }
    std::fs::create_dir_all(dir).ok()?;
    std::fs::write(&path, &bytes).ok()?;
    Some(path)
}

/// Real fetch: the site's own /favicon.ico, short timeout, size-capped.
pub fn fetch_favicon(domain: &str) -> Option<Vec<u8>> {
    let url = format!("https://{domain}/favicon.ico");
    let resp = ureq::get(&url).timeout(std::time::Duration::from_secs(4)).call().ok()?;
    let mut buf = Vec::new();
    use std::io::Read;
    resp.into_reader().take(512 * 1024).read_to_end(&mut buf).ok()?;
    if buf.is_empty() { None } else { Some(buf) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn domain_parsing() {
        assert_eq!(domain_of("https://github.com/a/b?x=1"), Some("github.com".into()));
        assert_eq!(domain_of("http://Example.COM:8080/x"), Some("example.com".into()));
        assert_eq!(domain_of("not a url"), None);
        assert_eq!(domain_of("https://localhost/x"), None); // no dot
    }

    #[test]
    fn ensure_uses_cache_then_writes() {
        let dir = std::env::temp_dir().join(format!("magpie-fav-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        // first call fetches
        let p = ensure_favicon(&dir, "example.com", |_| Some(vec![1, 2, 3])).unwrap();
        assert!(p.exists());
        // second call must NOT call fetch (panics if it does) — file already there
        let p2 = ensure_favicon(&dir, "example.com", |_| panic!("should be cached")).unwrap();
        assert_eq!(p, p2);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn ensure_returns_none_when_fetch_fails() {
        let dir = std::env::temp_dir().join(format!("magpie-fav2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(ensure_favicon(&dir, "nope.example", |_| None).is_none());
        std::fs::remove_dir_all(&dir).ok();
    }
}
```

- [ ] **Step 3: `pub mod favicon;`** in `lib.rs` (after `config`, before `format_time`).

- [ ] **Step 4: Config toggle** — in `config.rs`, add field + default fn + Default value:
```rust
#[serde(default = "default_true")]
pub fetch_link_favicons: bool,
```
```rust
fn default_true() -> bool { true }
```
`Default::default()` sets `fetch_link_favicons: true`. Add a test that a config without the key loads `true`.

- [ ] **Step 5: Run → PASS.** `cargo test -p magpie-app favicon` and `config`.

- [ ] **Step 6: Commit** `feat(app): favicon module (domain/cache/fetch) + config toggle`.

---

### Task 4: platform — `app_icons` (macOS) + `ActiveWinSource` cache dir

**Files:**
- Create: `crates/magpie-platform/src/app_icons.rs`; `pub mod app_icons;` in `lib.rs`
- Modify: `crates/magpie-platform/src/os/source_app.rs`
- Modify: `crates/magpie-platform/src/runtime`? No — construction is in `magpie-app` `spawn_watcher`.

**Interfaces:**
- Produces: `app_icons::icon_cache_path(&Path, &str) -> PathBuf`; `app_icons::ensure_app_icon(&Path, &str, &Path) -> Option<PathBuf>`; `ActiveWinSource { pub cache_dir: PathBuf }`.

- [ ] **Step 1: Failing tests for the pure/cache logic** — create `app_icons.rs`:

```rust
//! Source-application icon resolution + on-disk cache. OS extraction is
//! platform-specific and best-effort; the cache/reuse logic is pure.

use std::path::{Path, PathBuf};

/// Deterministic cache path for an app identifier: `<dir>/<blake3(id)>.png`.
pub fn icon_cache_path(dir: &Path, app_identifier: &str) -> PathBuf {
    let hash = blake3::hash(app_identifier.as_bytes()).to_hex();
    dir.join(format!("{hash}.png"))
}

/// Cached icon path if present; else extract from `exe_path`, write, and return.
/// Returns `None` if extraction fails (caller falls back to a glyph).
pub fn ensure_app_icon(dir: &Path, app_identifier: &str, exe_path: &Path) -> Option<PathBuf> {
    let path = icon_cache_path(dir, app_identifier);
    if path.exists() {
        return Some(path);
    }
    let bytes = app_icon_png(exe_path)?;
    if bytes.is_empty() {
        return None;
    }
    std::fs::create_dir_all(dir).ok()?;
    std::fs::write(&path, &bytes).ok()?;
    Some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_path_is_deterministic_and_distinct() {
        let d = Path::new("/tmp/icons");
        assert_eq!(icon_cache_path(d, "Foo"), icon_cache_path(d, "Foo"));
        assert_ne!(icon_cache_path(d, "Foo"), icon_cache_path(d, "Bar"));
    }

    #[test]
    fn ensure_reuses_existing_file_without_extraction() {
        let dir = std::env::temp_dir().join(format!("magpie-icons-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = icon_cache_path(&dir, "Cached");
        std::fs::write(&p, b"x").unwrap();
        // exe path is bogus; extraction must NOT be needed because the file exists.
        let got = ensure_app_icon(&dir, "Cached", Path::new("/nonexistent/bin"));
        assert_eq!(got, Some(p));
        std::fs::remove_dir_all(&dir).ok();
    }
}
```

- [ ] **Step 2: Run → fails to compile** (`app_icon_png` missing). Add `pub mod app_icons;` to `lib.rs`.

- [ ] **Step 3: Implement `app_icon_png` per-OS.** Start with macOS + a non-macOS stub so it compiles everywhere; Windows/Linux real impls come in Task 9.

```rust
// --- macOS ---
#[cfg(target_os = "macos")]
fn app_icon_png(exe_path: &Path) -> Option<Vec<u8>> {
    use objc2_app_kit::NSWorkspace;
    use objc2_foundation::NSString;
    // …/Foo.app/Contents/MacOS/foo -> …/Foo.app
    let bundle = bundle_path_from_exe(exe_path)?;
    unsafe {
        let ws = NSWorkspace::sharedWorkspace();
        let ns = NSString::from_str(bundle.to_str()?);
        let image = ws.iconForFile(&ns); // NSImage
        png_from_nsimage(&image, 64.0)
    }
}

#[cfg(target_os = "macos")]
fn bundle_path_from_exe(exe: &Path) -> Option<std::path::PathBuf> {
    // Walk up to the directory ending in ".app"; fall back to the exe itself.
    let mut cur = exe;
    while let Some(parent) = cur.parent() {
        if parent.extension().and_then(|e| e.to_str()) == Some("app") {
            return Some(parent.to_path_buf());
        }
        cur = parent;
    }
    Some(exe.to_path_buf())
}
```

`png_from_nsimage`: lock the icon to a 64×64 `NSImage`, get `TIFFRepresentation` → `NSBitmapImageRep` → `representationUsingType: NSBitmapImageFileType::PNG` → bytes. **Confirm exact objc2-app-kit 0.3 method names during implementation** (`iconForFile`, `TIFFRepresentation`, `NSBitmapImageRep::imageRepWithData`, `representationUsingType_properties`). If a method is awkward in `objc2`, an acceptable fallback is `NSBitmapImageRep` from the image's `CGImage`. Wrap all `unsafe` tightly; on any `None`, the caller falls back to a glyph.

```rust
// --- non-macOS placeholder (real Windows/Linux in Task 9) ---
#[cfg(not(target_os = "macos"))]
fn app_icon_png(_exe_path: &Path) -> Option<Vec<u8>> {
    None
}
```

- [ ] **Step 4: `ActiveWinSource { cache_dir }`** — rewrite `os/source_app.rs`:

```rust
use crate::app_icons::ensure_app_icon;
use crate::traits::SourceApp;
use magpie_core::AppInfo;
use std::path::PathBuf;

pub struct ActiveWinSource {
    pub cache_dir: PathBuf,
}

impl SourceApp for ActiveWinSource {
    fn frontmost(&self) -> Option<AppInfo> {
        let w = active_win_pos_rs::get_active_window().ok()?;
        let ident = if w.app_name.is_empty() { format!("pid:{}", w.process_id) } else { w.app_name.clone() };
        let name = ident.clone();
        let icon_path = ensure_app_icon(&self.cache_dir, &ident, w.process_path.as_path())
            .and_then(|p| p.to_str().map(|s| s.to_string()));
        Some(AppInfo { identifier: ident, display_name: name, icon_path })
    }
}
```

Confirm `active_win_pos_rs::ActiveWindow` field `process_path` is a `PathBuf` (`grep` the crate); if it is a `String`, adapt `.as_path()`/`Path::new`.

- [ ] **Step 5: Update the one construction site** in `magpie-app` `runtime::spawn_watcher`:
```rust
        let mut watcher = Watcher::new(
            clip,
            ActiveWinSource { cache_dir: data_dir().join("app_icons") },
            policy,
        );
```

- [ ] **Step 6: Build workspace + clippy + fmt.** `cargo build`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`. Run the `app_icons` unit tests.

- [ ] **Step 7: Manual (macOS):** `timeout 8 … run`, copy from a couple of apps, then check the DB: `sqlite3 "$HOME/Library/Application Support/magpie/magpie.sqlite3" "SELECT display_name, icon_path FROM apps"` — `icon_path` should be populated and the PNG files exist in `…/magpie/app_icons/`.

- [ ] **Step 8: Commit** `feat(platform): source app icon resolution + cache (macOS)`.

---

### Task 5: app — render icons in rows + Source line; favicon wiring

**Files:**
- Modify: `crates/magpie-app/ui/launcher.slint` (EntryRow `icon`/`has_icon`; render)
- Modify: `crates/magpie-app/src/runtime.rs` (`to_rows` icon choice; maps; favicon in watcher)

**Interfaces:**
- Consumes: `app_icon_pairs`, `favicon::{domain_of, favicon_cache_path, ensure_favicon, fetch_favicon}`, `slint::Image`.
- Produces: `EntryRow { …, icon: image, has_icon: bool }`.

- [ ] **Step 1: EntryRow gains icon fields** — in `launcher.slint`:
```slint
    icon: image,
    has_icon: bool,
```

- [ ] **Step 2: `to_rows` picks + loads the icon.** Add params `app_icons: &HashMap<i64, String>` and `favicon_dir: &Path`, and inside the closure:
```rust
            let icon_path: Option<String> = if matches!(e.kind, Kind::Link) {
                favicon::domain_of(&e.full_text)
                    .map(|d| favicon::favicon_cache_path(favicon_dir, &d))
                    .filter(|p| p.exists())
                    .and_then(|p| p.to_str().map(|s| s.to_string()))
                    .or_else(|| app_icons.get(&e.id).cloned())
            } else {
                app_icons.get(&e.id).cloned()
            };
            let (icon_img, has_icon) = match icon_path.as_deref().map(std::path::Path::new) {
                Some(p) => match slint::Image::load_from_path(p) {
                    Ok(img) => (img, true),
                    Err(_) => (slint::Image::default(), false),
                },
                None => (slint::Image::default(), false),
            };
```
Add `icon: icon_img, has_icon,` to the `EntryRow` literal. Import `magpie_app::favicon;`. Thread `favicon_dir` (=`data_dir().join("favicons")`) and the `app_icons` map from `refresh`.

- [ ] **Step 3: Build the `app_icons` map in `refresh`** — in the `store.lock()` block, add `let app_icons: HashMap<i64,String> = store.app_icon_pairs().unwrap_or_default().into_iter().collect();` and widen the returned tuple; pass `&app_icons` and `&data_dir().join("favicons")` to `to_rows`.

- [ ] **Step 4: Render icon-or-glyph** — in the list row, replace the glyph `Text` with:
```slint
                                if row.has-icon: Image { source: row.icon; width: 20px; height: 20px; }
                                if !row.has-icon: Text { text: row.glyph; font-size: 18px; vertical-alignment: center; width: 24px; horizontal-alignment: center; }
```
And in the metadata **Source** row (Task 8 restyles it; for now) show the icon before `source` similarly.

- [ ] **Step 5: Favicon fetch in the watcher** — in `spawn_watcher`, after a successful ingest of a **link** entry, best-effort:
```rust
                    if cfg_fetch_favicons {
                        if let Content::Text(ref t) = ev.content {
                            if let Some(d) = magpie_app::favicon::domain_of(t) {
                                let dir = data_dir().join("favicons");
                                let _ = magpie_app::favicon::ensure_favicon(&dir, &d, magpie_app::favicon::fetch_favicon);
                            }
                        }
                    }
```
Only attempt for link-kind (check via `magpie_core::detect` or the ingested entry kind). Pass `cfg.fetch_link_favicons` into `spawn_watcher` (add a param). (Domain gate already limits it to real URLs.)

- [ ] **Step 6: Build + clippy + fmt + `timeout 8` run.** Manual (macOS): rows/Source show app icons; copy a URL, reopen → its site favicon shows.

- [ ] **Step 7: Commit** `feat(app): render app icons + link favicons per entry`.

---

### Task 6: Searchable ⌘K actions + Delete + Paste-and-keep-open

**Files:**
- Modify: `crates/magpie-app/ui/launcher.slint`
- Modify: `crates/magpie-app/src/runtime.rs`

**Interfaces:**
- Produces: callbacks `delete-entry(int)`, `activate-keep(int)`; property `actions-query: string`; a filtered action model.

- [ ] **Step 1: Slint — action model + search.** Replace the hard-coded ⌘K action rows with a model the FocusScope can filter. Add props:
```slint
    in-out property <string> actions-query: "";
```
Define the action list as a `[{icon, label, key, id}]` array; render only rows whose `label` contains `actions-query` (case-insensitive via `.to-lowercase()`); keep `action-selected` clamped to the filtered length. Add a bottom search field (custom-drawn like the main search, fed by the same key handler while `mode == "actions"`): printable keys append to `actions-query`, Backspace trims. Icons are emoji: 📋 Paste, 📄 Copy, 📌 Pin, ✏️ Edit, 🧩 Snippet, ➕ Merge, 🗑 Delete.

- [ ] **Step 2: Extend the action set + `run-selected-action`.** Map filtered index → action id; run the matching callback. Add **Delete** (`root.delete-entry(root.selected)`) and **Paste & keep open** (`root.activate-keep(root.selected)`). Keep the slot 1-9 row.

- [ ] **Step 3: Key routing while actions open** — in the FocusScope `mode == "actions"` branch: printable → `actions-query += text` (via a Rust callback `actions-key-char`/`actions-backspace`, mirroring the main query approach to avoid Slint string slicing), ↑/↓ move within the filtered set, ⏎ runs, digits still assign slots, Esc closes + clears `actions-query`.

- [ ] **Step 4: Runtime — `delete_entry` + `activate_keep`:**
```rust
    {
        let s = state.clone(); let w = ui.as_weak();
        ui.on_delete_entry(move |idx| {
            let results = current_results(&s, now_ms());
            if let Some(e) = results.get(idx as usize) {
                if let Ok(store) = s.store.lock() {
                    if let Ok(removed) = store.delete_entry(e.id) {
                        s.images.remove_paths(&removed.image_paths);
                    }
                }
            }
            if let Some(ui) = w.upgrade() { ui.set_mode("list".into()); refresh(&ui, &s); }
        });
    }
    {
        let s = state.clone(); let auto = cfg.paste_on_select;
        ui.on_activate_keep(move |idx| { activate_index(&s, idx as usize, auto); });
    }
```
`activate_keep` reuses `activate_index` but the window is NOT hidden (it never auto-hides today; `hide-window` is explicit), so keep-open is simply "paste without Esc". Wire the `actions-key-char`/`actions-backspace` callbacks in Rust to mutate `actions_query` via `ui.get/set_actions_query`.

- [ ] **Step 5: Build + clippy + fmt + `timeout 8` run.** Manual: ⌘K → type to filter actions; Delete removes the selected entry; Paste & keep open pastes without closing.

- [ ] **Step 6: Commit** `feat(app): searchable ⌘K actions with icons + Delete + keep-open`.

---

### Task 7: Help cheat sheet view

**Files:**
- Modify: `crates/magpie-app/ui/launcher.slint`

- [ ] **Step 1: Add a `?` button** in the filter row (next to Stats) and a `⌘/` handler in the FocusScope list-mode branch: both set `root.view = root.view == "help" ? "list" : "help"`.

- [ ] **Step 2: Help view** — `if root.view == "help": VerticalLayout { … }` with a Back button and grouped static shortcut rows (Navigation: ↑/↓, ⏎, Esc; Actions: ⌘C, ⌘E, ⌘N, ⌘P, ⌘M, ⌘K, ⌘1-9, ⌃X delete; Views: Stats, ⌘/ Help). Each row: `HorizontalLayout { Text(label) … Text(keys) }`. `Esc` returns to list (extend the FocusScope Escape branch: if `view == "help"` set `view = "list"`).

- [ ] **Step 3: Build + `timeout 8` run.** Manual: `?`/⌘/ opens Help; Esc closes.

- [ ] **Step 4: Commit** `feat(app): help / keyboard-shortcuts cheat sheet`.

---

### Task 8: UI polish — Information block, date headers, action-bar target app

**Files:**
- Modify: `crates/magpie-app/ui/launcher.slint`
- Modify: `crates/magpie-app/src/runtime.rs`

- [ ] **Step 1: EntryRow gains `words: int` and `section: string`.** In `to_rows`: `words: e.word_count as i32`, `section: SharedString::from(grouping::section_for(e.last_copied_at_ms, now).label())`. Import `magpie_app::grouping` (it is in the app lib) — confirm `Section::label` + `section_for` are `pub`.

- [ ] **Step 2: Metadata "Information" block** — restyle the metadata `VerticalLayout`: a header `Text { "Information"; color:#6a6a70; }`, then label/value rows where the **value is right-aligned** (value `Text` gets `horizontal-alignment: right;` and stretches). Add rows **Words** (`entries[selected].words`) and **Copied** (`entries[selected].copied-date`). Add `copied_date: SharedString::from(abs_date(e.last_copied_at_ms))` to EntryRow + the struct.

- [ ] **Step 3: Date group headers** — in the list `for row[i]`, before each row, show a header when the section changes:
```slint
                        if i == 0 || entries[i - 1].section != row.section: Text {
                            text: row.section;
                            color: #6a6a70;
                            font-size: 11px;
                        }
```
(Place it inside the `for` as a sibling above the row `Rectangle` by wrapping each iteration in a `VerticalLayout`.)

- [ ] **Step 4: Action-bar target app** — capture the frontmost app in `show_window` **before** `ui.show()` and set window props:
```rust
    let target = ActiveWinSource { cache_dir: data_dir().join("app_icons") }.frontmost();
    if let Some(app) = &target {
        ui.set_target_app(app.display_name.clone().into());
        // load its icon like to_rows
    }
```
Add Slint props `target-app: string`, `target-icon: image`, `target-has-icon: bool`; render "Paste to [icon] <target-app>" in the action bar (fallback to the plain hints when empty). Keep it best-effort.

- [ ] **Step 5: Build + clippy + fmt + `timeout 8` run.** Manual: metadata shows Information/Words/Copied right-aligned; list shows Today/Yesterday headers; action bar shows the target app + icon.

- [ ] **Step 6: Commit** `feat(app): Information metadata, date headers, action-bar target app`.

---

### Task 9: Windows + Linux app icon extraction (cfg-gated)

**Files:**
- Modify: `crates/magpie-platform/Cargo.toml` (win Shell/Gdi features; linux `freedesktop-icons`)
- Modify: `crates/magpie-platform/src/app_icons.rs`

- [ ] **Step 1: Windows deps** — add to the `[target.'cfg(target_os = "windows")'.dependencies] windows` feature list: `"Win32_UI_Shell"`, `"Win32_Graphics_Gdi"`, `"Win32_UI_WindowsAndMessaging"`.

- [ ] **Step 2: Windows `app_icon_png`** — `#[cfg(target_os = "windows")]`: `SHGetFileInfoW(exe, 0, &mut shfi, size, SHGFI_ICON | SHGFI_LARGEICON)` → `HICON`; `GetIconInfo` + `GetDIBits` into a 32-bit top-down BGRA buffer; encode PNG via the `image` crate (already a dep? it's on `magpie-app`, not platform — use a minimal PNG encoder or add `image` to platform, or hand-roll). Simplest: add `image = "0.25"` to the Windows target deps and use `image::RgbaImage` → `write_to(Cursor, ImageFormat::Png)`. `DestroyIcon` after. All `unsafe`; return `None` on any failure. **Compile-checked only.**

- [ ] **Step 3: Linux deps + impl** — add `[target.'cfg(target_os = "linux")'.dependencies] freedesktop-icons = "0.2"` (confirm the crate + API). `#[cfg(target_os = "linux")]`: derive an icon name from the exe basename (and/or `.desktop` lookup), `freedesktop_icons::lookup(name).with_size(64).find()` → a file path; read PNG bytes (if SVG, skip or note). Best-effort; `None` on miss. **Compile-checked only.**

- [ ] **Step 4: Replace the `#[cfg(not(target_os = "macos"))]` stub** so each OS has its arm and there is a final `#[cfg(not(any(macos, windows, linux)))] -> None`.

- [ ] **Step 5: Build for host (macOS unaffected) + clippy + fmt.** The Windows/Linux arms compile only on their targets/CI; verify the macOS build is still green and the stub is gone. (If cross-compiling isn't available, rely on CI; note it.)

- [ ] **Step 6: Commit** `feat(platform): Windows + Linux app icon extraction (cfg-gated)`.

---

### Task 10: Full gate + run-verify + memory

- [ ] **Step 1: Full gates**
```bash
env -u RUSTUP_TOOLCHAIN ~/.cargo/bin/cargo test
env -u RUSTUP_TOOLCHAIN ~/.cargo/bin/cargo clippy --all-targets -- -D warnings
env -u RUSTUP_TOOLCHAIN ~/.cargo/bin/cargo fmt --check
timeout 8 env -u RUSTUP_TOOLCHAIN ~/.cargo/bin/cargo run -p magpie-app
```
- [ ] **Step 2: Update memory** `magpie-clipboard-manager.md`; commit docs.

## Self-Review

- **Spec coverage:** app icons → T4 (macOS) + T9 (win/linux) + T5 (render); favicons → T3 + T5; ⌘K search/Delete/keep-open → T6 (+ core delete T1); Help → T7; polish (Information/Words/Copied/date headers/target app) → T2 + T8; `app_icon_pairs` → T1. All mapped.
- **Placeholder scan:** none — pure code concrete; OS extraction gives reference impls with explicit "confirm API" checks (objc2 method names, active-win field type, windows/linux crate APIs) rather than hidden TODOs.
- **Type consistency:** `to_rows` gains `app_icons: &HashMap<i64,String>`, `favicon_dir: &Path` (T5) — matches its `refresh` call. `EntryRow` grows `icon`/`has_icon` (T5), `words`/`section`/`copied_date` (T8); Slint struct + every `to_rows` literal updated together. `ensure_app_icon`/`ensure_favicon` signatures match their call sites. `delete_entry(id) -> Removed` (T1) matches `on_delete_entry` (T6). `ActiveWinSource { cache_dir }` (T4) matches `spawn_watcher` (T4 S5).
- **Risk flags:** (a) objc2-app-kit 0.3 exact method names for NSWorkspace icon + NSBitmapImageRep — verify in T4. (b) `active-win` `process_path` type — verify in T4. (c) Slint `Image` render + `load_from_path` — verify in T5. (d) `ureq` 2.x API (`.timeout`, `.into_reader`) — verify in T3. (e) `freedesktop-icons` crate name/API + windows GDI — verify in T9 (compile-only). (f) image decode per refresh — acceptable at 64px, cache later if needed.
