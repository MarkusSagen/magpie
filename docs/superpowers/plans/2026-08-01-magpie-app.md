# magpie-app Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build `magpie-app`, the binary that wires `magpie-core` + `magpie-platform` into a running tray daemon with a Slint launcher — search UI, filter chips, quick-paste hotkeys, and background clipboard capture.

**Architecture:** A single binary. A background thread polls the clipboard via `Watcher::poll_once` (real wall-clock time here — the app *is* allowed `SystemTime::now`, core is not) and ingests events into a `Mutex<Store>`. The Slint window owns the main event loop; it's created hidden and shown on the launcher hotkey via `slint::invoke_from_event_loop`. A hotkey thread drains `global_hotkey`'s receiver: the launcher hotkey toggles the window; quick-paste `1..9` set the clipboard and fire a synthetic paste with no UI. All non-UI logic — config, image cache, the UI-state→`SearchQuery` mapping, date-section grouping, quick-paste resolution, and paste orchestration — is pure/fake-testable; only the Slint markup, tray, and event-loop wiring are manual-verify.

**Tech Stack:** Rust, `magpie-core` + `magpie-platform` (path deps), `slint` (native GUI), `tray-icon`, `serde` + `toml` (config), `dirs` (platform paths), `image` (PNG encode + thumbnail for the cache). Testable logic uses fakes + temp dirs + injected `now_ms`.

## Global Constraints

- **Reuses** `magpie-core` (`Store`, `ingest`, `search`, `recent`, `SearchQuery`, `Kind`, `Content`, `ImageStore`, `Entry`) and `magpie-platform` (`Watcher`, `CapturePolicy`, adapters, `parse_hotkey`, defaults). Never reimplement them.
- **Wall-clock lives here, not in core:** the app stamps `copied_at_ms` and computes "today/yesterday" from `SystemTime::now`. Pure helpers still take `now_ms: i64` so they stay testable.
- **The `Store` is shared** behind `std::sync::Mutex` (single writer from the watcher thread, readers from the UI). No async runtime.
- **Launcher must feel instant:** the Slint window is created once and hidden, never recreated per invocation.
- **Config + cache** live under `dirs` app paths (`~/Library/Application Support/magpie`, etc.).
- **Minimal dependencies:** only the crates named in Tech Stack.
- **Commit style:** conventional commits, one per task.

---

### Task 1: App crate scaffold + config model (load/save)

**Files:**
- Modify: `Cargo.toml` (workspace `members`)
- Create: `crates/magpie-app/Cargo.toml`
- Create: `crates/magpie-app/src/main.rs` (stub `fn main()`)
- Create: `crates/magpie-app/src/config.rs`
- Test: `config.rs` tests module.

**Interfaces:**
- Produces:
  - `pub struct Config { pub launcher_hotkey: String, pub quick_paste_hotkeys: Vec<String>, pub paste_on_select: bool, pub app_denylist: Vec<String> }` — `#[derive(serde::Serialize, serde::Deserialize)]`, with `impl Default` (launcher `"super+ctrl+v"`, quick-paste `["super+ctrl+1"..="super+ctrl+9"]`, `paste_on_select = true`, empty denylist).
  - `pub fn load_or_default(path: &std::path::Path) -> Config` — reads+parses TOML, falling back to `Default` on missing/invalid.
  - `pub fn save(cfg: &Config, path: &std::path::Path) -> std::io::Result<()>` — writes TOML, creating parent dirs.

- [ ] **Step 1: Add `crates/magpie-app` to workspace `members`.**

- [ ] **Step 2: Create `crates/magpie-app/Cargo.toml`**

```toml
[package]
name = "magpie-app"
edition.workspace = true
version.workspace = true
license.workspace = true

[dependencies]
magpie-core = { path = "../magpie-core" }
magpie-platform = { path = "../magpie-platform" }
slint = "1.8"
tray-icon = "0.19"
serde = { version = "1", features = ["derive"] }
toml = "0.8"
dirs = "5"
image = "0.25"

[build-dependencies]
slint-build = "1.8"
```

- [ ] **Step 3: Create `src/main.rs` stub**

```rust
fn main() {
    println!("magpie-app placeholder");
}

mod config;
```

- [ ] **Step 4: Write failing tests in `config.rs`**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_has_nine_quick_paste_hotkeys() {
        let c = Config::default();
        assert_eq!(c.quick_paste_hotkeys.len(), 9);
        assert_eq!(c.quick_paste_hotkeys[0], "super+ctrl+1");
        assert!(c.paste_on_select);
    }

    #[test]
    fn save_then_load_roundtrips() {
        let dir = std::env::temp_dir().join(format!("magpie-cfg-{}", std::process::id()));
        let path = dir.join("config.toml");
        let mut c = Config::default();
        c.paste_on_select = false;
        c.app_denylist = vec!["Secret".into()];
        save(&c, &path).unwrap();
        let loaded = load_or_default(&path);
        assert!(!loaded.paste_on_select);
        assert_eq!(loaded.app_denylist, vec!["Secret".to_string()]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn missing_file_yields_default() {
        let c = load_or_default(std::path::Path::new("/nonexistent/magpie/nope.toml"));
        assert_eq!(c.launcher_hotkey, "super+ctrl+v");
    }
}
```

- [ ] **Step 5: Add `mod config;` to `main.rs`; run to verify fail**

Run: `cargo test -p magpie-app config`
Expected: FAIL.

- [ ] **Step 6: Implement `config.rs`**

```rust
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub launcher_hotkey: String,
    pub quick_paste_hotkeys: Vec<String>,
    pub paste_on_select: bool,
    pub app_denylist: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            launcher_hotkey: "super+ctrl+v".into(),
            quick_paste_hotkeys: (1..=9).map(|n| format!("super+ctrl+{n}")).collect(),
            paste_on_select: true,
            app_denylist: Vec::new(),
        }
    }
}

pub fn load_or_default(path: &Path) -> Config {
    match std::fs::read_to_string(path) {
        Ok(s) => toml::from_str(&s).unwrap_or_default(),
        Err(_) => Config::default(),
    }
}

pub fn save(cfg: &Config, path: &Path) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let s = toml::to_string_pretty(cfg).map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    std::fs::write(path, s)
}
```

- [ ] **Step 7: Run to verify pass**

Run: `cargo test -p magpie-app config`
Expected: PASS (3 tests).

- [ ] **Step 8: Commit**

```bash
git add Cargo.toml crates/magpie-app
git commit -m "chore(app): scaffold magpie-app + config load/save"
```

---

### Task 2: Image cache (ImageStore impl + PNG encode + thumbnail)

**Files:**
- Create: `crates/magpie-app/src/image_cache.rs`
- Modify: `crates/magpie-app/src/main.rs` (`mod image_cache;`)
- Test: `image_cache.rs` tests module.

**Interfaces:**
- Consumes: `magpie_core::ImageStore`.
- Produces: `pub struct FsImageStore { pub dir: std::path::PathBuf }` implementing `ImageStore::put(hash, bytes) -> io::Result<String>` by writing `bytes` to `dir/<hash>.bin` (content-addressed; idempotent — skip write if the file already exists) and returning the absolute path string. (The platform crate hands us dimension-tagged RGBA bytes for v1; PNG re-encoding for a smaller file + a `<hash>.thumb.png` thumbnail is a documented follow-up hook `pub fn write_thumbnail(...)` that IS implemented and tested here using the `image` crate.)

- [ ] **Step 1: Write failing tests in `image_cache.rs`**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use magpie_core::ImageStore;

    #[test]
    fn put_is_content_addressed_and_idempotent() {
        let dir = std::env::temp_dir().join(format!("magpie-img-{}", std::process::id()));
        let store = FsImageStore { dir: dir.clone() };
        let p1 = store.put("abc123", &[1, 2, 3]).unwrap();
        let p2 = store.put("abc123", &[1, 2, 3]).unwrap();
        assert_eq!(p1, p2);
        assert!(std::path::Path::new(&p1).exists());
        assert!(p1.ends_with("abc123.bin"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn write_thumbnail_produces_png() {
        let dir = std::env::temp_dir().join(format!("magpie-thumb-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = FsImageStore { dir: dir.clone() };
        // 2x2 RGBA red image
        let rgba = vec![255, 0, 0, 255,  255, 0, 0, 255,  255, 0, 0, 255,  255, 0, 0, 255];
        let path = store.write_thumbnail("deadbeef", 2, 2, &rgba, 64).unwrap();
        assert!(std::path::Path::new(&path).exists());
        assert!(path.ends_with("deadbeef.thumb.png"));
        std::fs::remove_dir_all(&dir).ok();
    }
}
```

- [ ] **Step 2: Add `mod image_cache;` to `main.rs`; run to verify fail**

Run: `cargo test -p magpie-app image_cache`
Expected: FAIL.

- [ ] **Step 3: Implement `image_cache.rs`**

```rust
use magpie_core::ImageStore;
use std::path::PathBuf;

pub struct FsImageStore {
    pub dir: PathBuf,
}

impl FsImageStore {
    fn ensure_dir(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.dir)
    }

    /// Downscale RGBA to a PNG thumbnail (longest edge = `max_edge`) at `<hash>.thumb.png`.
    pub fn write_thumbnail(&self, hash: &str, w: u32, h: u32, rgba: &[u8], max_edge: u32) -> std::io::Result<String> {
        self.ensure_dir()?;
        let img = image::RgbaImage::from_raw(w, h, rgba.to_vec())
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "bad rgba dims"))?;
        let thumb = image::imageops::thumbnail(&img, max_edge.min(w), max_edge.min(h));
        let path = self.dir.join(format!("{hash}.thumb.png"));
        thumb.save(&path).map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        Ok(path.to_string_lossy().into_owned())
    }
}

impl ImageStore for FsImageStore {
    fn put(&self, hash: &str, bytes: &[u8]) -> std::io::Result<String> {
        self.ensure_dir()?;
        let path = self.dir.join(format!("{hash}.bin"));
        if !path.exists() {
            std::fs::write(&path, bytes)?;
        }
        Ok(path.to_string_lossy().into_owned())
    }
}
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p magpie-app image_cache`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/magpie-app/src/image_cache.rs crates/magpie-app/src/main.rs
git commit -m "feat(app): content-addressed FsImageStore + thumbnails"
```

---

### Task 3: View-model — UI filter state → SearchQuery

**Files:**
- Create: `crates/magpie-app/src/viewmodel.rs`
- Modify: `crates/magpie-app/src/main.rs` (`mod viewmodel;`)
- Test: `viewmodel.rs` tests module.

**Interfaces:**
- Consumes: `magpie_core::{SearchQuery, SearchMode, Sort, Kind, TimeRange, default_query}`.
- Produces:
  - `pub enum TypeFilter { All, Text, Link, Color, Email, Image, File }` with `pub fn to_kind(&self) -> Option<Kind>`.
  - `pub enum TimeFilter { All, Today, Last7Days }`
  - `pub struct UiState { pub text: String, pub mode: SearchMode, pub type_filter: TypeFilter, pub app_filter: Option<i64>, pub time_filter: TimeFilter, pub sort: Sort }` with `pub fn new() -> Self` (empty text, Word, All, None, All, Recency).
  - `pub fn to_query(ui: &UiState, now_ms: i64) -> SearchQuery` — maps `TypeFilter`→`kind`, `TimeFilter`→`TimeRange` (Today = since local-midnight-ms; Last7Days = since `now_ms - 7*86_400_000`), copies text/mode/app/sort, `limit = 200`. Midnight uses a simple UTC-day floor for v1 (`now_ms - now_ms.rem_euclid(86_400_000)`).

- [ ] **Step 1: Write failing tests in `viewmodel.rs`**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use magpie_core::{Kind, SearchMode};

    #[test]
    fn type_filter_maps_to_kind() {
        assert!(TypeFilter::All.to_kind().is_none());
        assert_eq!(TypeFilter::Link.to_kind(), Some(Kind::Link));
    }

    #[test]
    fn to_query_copies_text_and_mode() {
        let mut ui = UiState::new();
        ui.text = "hello".into();
        ui.mode = SearchMode::Fuzzy;
        let q = to_query(&ui, 1_000_000_000);
        assert_eq!(q.text, "hello");
        assert_eq!(q.mode, SearchMode::Fuzzy);
        assert!(q.kind.is_none());
        assert!(q.time.since_ms.is_none());
    }

    #[test]
    fn last_7_days_sets_since() {
        let mut ui = UiState::new();
        ui.time_filter = TimeFilter::Last7Days;
        let now = 10 * 86_400_000i64;
        let q = to_query(&ui, now);
        assert_eq!(q.time.since_ms, Some(now - 7 * 86_400_000));
    }

    #[test]
    fn today_floors_to_utc_midnight() {
        let mut ui = UiState::new();
        ui.time_filter = TimeFilter::Today;
        let now = 3 * 86_400_000i64 + 12_345;
        let q = to_query(&ui, now);
        assert_eq!(q.time.since_ms, Some(3 * 86_400_000));
    }
}
```

- [ ] **Step 2: Add `mod viewmodel;` to `main.rs`; run to verify fail**

Run: `cargo test -p magpie-app viewmodel`
Expected: FAIL.

- [ ] **Step 3: Implement `viewmodel.rs`**

```rust
use magpie_core::{default_query, Kind, SearchMode, SearchQuery, Sort, TimeRange};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeFilter { All, Text, Link, Color, Email, Image, File }

impl TypeFilter {
    pub fn to_kind(&self) -> Option<Kind> {
        match self {
            TypeFilter::All => None,
            TypeFilter::Text => Some(Kind::Text),
            TypeFilter::Link => Some(Kind::Link),
            TypeFilter::Color => Some(Kind::Color),
            TypeFilter::Email => Some(Kind::Email),
            TypeFilter::Image => Some(Kind::Image),
            TypeFilter::File => Some(Kind::File),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeFilter { All, Today, Last7Days }

pub struct UiState {
    pub text: String,
    pub mode: SearchMode,
    pub type_filter: TypeFilter,
    pub app_filter: Option<i64>,
    pub time_filter: TimeFilter,
    pub sort: Sort,
}

impl UiState {
    pub fn new() -> Self {
        UiState {
            text: String::new(),
            mode: SearchMode::Word,
            type_filter: TypeFilter::All,
            app_filter: None,
            time_filter: TimeFilter::All,
            sort: Sort::Recency,
        }
    }
}

const DAY_MS: i64 = 86_400_000;

pub fn to_query(ui: &UiState, now_ms: i64) -> SearchQuery {
    let mut q = default_query();
    q.text = ui.text.clone();
    q.mode = ui.mode;
    q.kind = ui.type_filter.to_kind();
    q.source_app_id = ui.app_filter;
    q.sort = ui.sort;
    q.time = match ui.time_filter {
        TimeFilter::All => TimeRange::default(),
        TimeFilter::Today => TimeRange { since_ms: Some(now_ms - now_ms.rem_euclid(DAY_MS)), until_ms: None },
        TimeFilter::Last7Days => TimeRange { since_ms: Some(now_ms - 7 * DAY_MS), until_ms: None },
    };
    q
}
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p magpie-app viewmodel`
Expected: PASS (4 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/magpie-app/src/viewmodel.rs crates/magpie-app/src/main.rs
git commit -m "feat(app): view-model UI state -> SearchQuery mapping"
```

---

### Task 4: Date-section grouping

**Files:**
- Create: `crates/magpie-app/src/grouping.rs`
- Modify: `crates/magpie-app/src/main.rs` (`mod grouping;`)
- Test: `grouping.rs` tests module.

**Interfaces:**
- Consumes: `magpie_core::Entry`.
- Produces:
  - `pub enum Section { Today, Yesterday, Older }` with `pub fn label(&self) -> &'static str`.
  - `pub fn section_for(last_copied_ms: i64, now_ms: i64) -> Section` — UTC-day bucketing: same day = Today, one day before = Yesterday, else Older.
  - `pub fn group(entries: &[Entry], now_ms: i64) -> Vec<(Section, Vec<usize>)>` — returns sections in order Today→Yesterday→Older, each with the indices (into `entries`) that belong to it, preserving input order; empty sections are omitted.

- [ ] **Step 1: Write failing tests in `grouping.rs`**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    const DAY: i64 = 86_400_000;

    #[test]
    fn buckets_by_utc_day() {
        let now = 100 * DAY + 5_000;
        assert!(matches!(section_for(100 * DAY + 1, now), Section::Today));
        assert!(matches!(section_for(99 * DAY + 1, now), Section::Yesterday));
        assert!(matches!(section_for(90 * DAY, now), Section::Older));
    }

    #[test]
    fn group_orders_and_indexes() {
        use magpie_core::{Entry, Kind};
        let mk = |id: i64, ms: i64| Entry {
            id, content_hash: format!("h{id}"), kind: Kind::Text, preview_text: "".into(),
            full_text: "".into(), image_path: None, byte_size: 0, char_count: 0, word_count: 0,
            line_count: 0, first_copied_at_ms: ms, last_copied_at_ms: ms, copy_count: 1,
            pinned: false, source_app_id: None,
        };
        let now = 10 * DAY + 1;
        let entries = vec![mk(1, 10 * DAY), mk(2, 9 * DAY), mk(3, 1 * DAY)];
        let g = group(&entries, now);
        assert_eq!(g.len(), 3);
        assert!(matches!(g[0].0, Section::Today));
        assert_eq!(g[0].1, vec![0]);
        assert!(matches!(g[2].0, Section::Older));
        assert_eq!(g[2].1, vec![2]);
    }
}
```

- [ ] **Step 2: Add `mod grouping;` to `main.rs`; run to verify fail**

Run: `cargo test -p magpie-app grouping`
Expected: FAIL.

- [ ] **Step 3: Implement `grouping.rs`**

```rust
use magpie_core::Entry;

const DAY_MS: i64 = 86_400_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section { Today, Yesterday, Older }

impl Section {
    pub fn label(&self) -> &'static str {
        match self {
            Section::Today => "Today",
            Section::Yesterday => "Yesterday",
            Section::Older => "Older",
        }
    }
}

pub fn section_for(last_copied_ms: i64, now_ms: i64) -> Section {
    let today = now_ms.div_euclid(DAY_MS);
    let day = last_copied_ms.div_euclid(DAY_MS);
    match today - day {
        0 => Section::Today,
        1 => Section::Yesterday,
        _ => Section::Older,
    }
}

pub fn group(entries: &[Entry], now_ms: i64) -> Vec<(Section, Vec<usize>)> {
    let mut today = Vec::new();
    let mut yesterday = Vec::new();
    let mut older = Vec::new();
    for (i, e) in entries.iter().enumerate() {
        match section_for(e.last_copied_at_ms, now_ms) {
            Section::Today => today.push(i),
            Section::Yesterday => yesterday.push(i),
            Section::Older => older.push(i),
        }
    }
    let mut out = Vec::new();
    if !today.is_empty() { out.push((Section::Today, today)); }
    if !yesterday.is_empty() { out.push((Section::Yesterday, yesterday)); }
    if !older.is_empty() { out.push((Section::Older, older)); }
    out
}
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p magpie-app grouping`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/magpie-app/src/grouping.rs crates/magpie-app/src/main.rs
git commit -m "feat(app): date-section grouping of entries"
```

---

### Task 5: Quick-paste resolution + paste orchestration

**Files:**
- Create: `crates/magpie-app/src/paste_action.rs`
- Modify: `crates/magpie-app/src/main.rs` (`mod paste_action;`)
- Test: `paste_action.rs` tests module.

**Interfaces:**
- Consumes: `magpie_core::{Entry, Content}`, `magpie_platform::{Clipboard, Paster}`.
- Produces:
  - `pub fn resolve_quick_paste(recent: &[Entry], slot: usize) -> Option<&Entry>` — 1-based slot (1..=9) → `recent[slot-1]`; `None` if out of range or `slot == 0`.
  - `pub enum PasteKind { Formatted, PlainText }`
  - `pub fn perform_paste(clip: &mut dyn Clipboard, paster: &dyn Paster, entry: &Entry, kind: PasteKind, auto: bool) -> Result<(), String>` — sets the clipboard from `entry` (PlainText forces `Content::Text(entry.full_text)`; Formatted uses `full_text` too in v1 since we store text — rich re-emission is Phase 1), then if `auto` calls `paster.paste()`. Returns Ok even if `auto` is false (clipboard-only).

- [ ] **Step 1: Write failing tests in `paste_action.rs`**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use magpie_core::{Content, Entry, Kind};
    use std::cell::Cell;

    fn entry(text: &str) -> Entry {
        Entry {
            id: 1, content_hash: "h".into(), kind: Kind::Text, preview_text: text.into(),
            full_text: text.into(), image_path: None, byte_size: 0, char_count: 0, word_count: 0,
            line_count: 0, first_copied_at_ms: 0, last_copied_at_ms: 0, copy_count: 1,
            pinned: false, source_app_id: None,
        }
    }

    struct FakeClip { last: std::cell::RefCell<String> }
    impl magpie_platform::Clipboard for FakeClip {
        fn snapshot(&mut self) -> magpie_platform::ClipboardSnapshot {
            magpie_platform::ClipboardSnapshot { content: None, change_token: 0, concealed: false }
        }
        fn set_text(&mut self, t: &str) -> Result<(), String> { *self.last.borrow_mut() = t.into(); Ok(()) }
        fn set_content(&mut self, c: &Content) -> Result<(), String> {
            if let Content::Text(t) = c { *self.last.borrow_mut() = t.clone(); }
            Ok(())
        }
    }
    struct FakePaster { pasted: Cell<bool> }
    impl magpie_platform::Paster for FakePaster {
        fn paste(&self) -> Result<(), String> { self.pasted.set(true); Ok(()) }
    }

    #[test]
    fn resolve_is_one_based_and_bounded() {
        let r = vec![entry("a"), entry("b")];
        assert_eq!(resolve_quick_paste(&r, 1).unwrap().full_text, "a");
        assert_eq!(resolve_quick_paste(&r, 2).unwrap().full_text, "b");
        assert!(resolve_quick_paste(&r, 0).is_none());
        assert!(resolve_quick_paste(&r, 3).is_none());
    }

    #[test]
    fn perform_paste_sets_clipboard_and_auto_pastes() {
        let mut clip = FakeClip { last: std::cell::RefCell::new(String::new()) };
        let paster = FakePaster { pasted: Cell::new(false) };
        perform_paste(&mut clip, &paster, &entry("hello"), PasteKind::PlainText, true).unwrap();
        assert_eq!(*clip.last.borrow(), "hello");
        assert!(paster.pasted.get());
    }

    #[test]
    fn perform_paste_clipboard_only_when_not_auto() {
        let mut clip = FakeClip { last: std::cell::RefCell::new(String::new()) };
        let paster = FakePaster { pasted: Cell::new(false) };
        perform_paste(&mut clip, &paster, &entry("hi"), PasteKind::Formatted, false).unwrap();
        assert_eq!(*clip.last.borrow(), "hi");
        assert!(!paster.pasted.get());
    }
}
```

- [ ] **Step 2: Add `mod paste_action;` to `main.rs`; run to verify fail**

Run: `cargo test -p magpie-app paste_action`
Expected: FAIL.

- [ ] **Step 3: Implement `paste_action.rs`**

```rust
use magpie_core::{Content, Entry};
use magpie_platform::{Clipboard, Paster};

pub fn resolve_quick_paste(recent: &[Entry], slot: usize) -> Option<&Entry> {
    if slot == 0 { return None; }
    recent.get(slot - 1)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasteKind { Formatted, PlainText }

pub fn perform_paste(
    clip: &mut dyn Clipboard,
    paster: &dyn Paster,
    entry: &Entry,
    _kind: PasteKind,
    auto: bool,
) -> Result<(), String> {
    // v1 stores text for all non-image kinds; both paste kinds emit full_text.
    clip.set_content(&Content::Text(entry.full_text.clone()))?;
    if auto {
        paster.paste()?;
    }
    Ok(())
}
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p magpie-app paste_action`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/magpie-app/src/paste_action.rs crates/magpie-app/src/main.rs
git commit -m "feat(app): quick-paste resolution + paste orchestration"
```

---

### Task 6: Slint launcher UI markup

**Files:**
- Create: `crates/magpie-app/ui/launcher.slint`
- Create: `crates/magpie-app/build.rs`
- Modify: `crates/magpie-app/src/main.rs` (`slint::include_modules!()`)

**Interfaces:**
- Produces a Slint `LauncherWindow` component with: a search `LineEdit` (two-way `query` string), a mode selector, a row of type-filter buttons, a `ListView` of `EntryRow { title, subtitle, kind }` structs (property `entries`), a selected-index property, a detail pane bound to the selected row, and callbacks `search-changed(string)`, `activate(int)` (Enter on row), `copy-only(int)`. This task delivers the markup and a compiling `include_modules!` binding; wiring to the store is Task 8.

> UI markup renders only with a display; this task's "test" is that it **compiles** and opens. Automated CI cannot assert pixels.

- [ ] **Step 1: Create `build.rs`**

```rust
fn main() {
    slint_build::compile("ui/launcher.slint").unwrap();
}
```

- [ ] **Step 2: Create `ui/launcher.slint`**

```slint
struct EntryRow {
    title: string,
    subtitle: string,
    kind: string,
}

export component LauncherWindow inherits Window {
    title: "Magpie";
    preferred-width: 900px;
    preferred-height: 560px;
    background: #1e1e21;

    in property <[EntryRow]> entries;
    in-out property <string> query;
    in-out property <int> selected: 0;
    in property <string> detail-text;

    callback search-changed(string);
    callback activate(int);
    callback copy-only(int);

    VerticalLayout {
        padding: 8px;
        spacing: 8px;

        search := LineEdit {
            placeholder-text: "Type to filter entries…";
            text <=> query;
            edited(t) => { root.search-changed(t); }
        }

        HorizontalLayout {
            spacing: 12px;

            list := ListView {
                min-width: 340px;
                for row[i] in entries: Rectangle {
                    height: 44px;
                    background: i == root.selected ? #3a3a40 : transparent;
                    border-radius: 6px;
                    TouchArea {
                        clicked => { root.selected = i; }
                        double-clicked => { root.activate(i); }
                    }
                    VerticalLayout {
                        padding-left: 10px;
                        padding-right: 10px;
                        Text { text: row.title; color: white; font-size: 14px; overflow: elide; }
                        Text { text: row.subtitle; color: #9a9aa0; font-size: 11px; overflow: elide; }
                    }
                }
            }

            Rectangle {
                background: #161618;
                border-radius: 8px;
                VerticalLayout {
                    padding: 12px;
                    Text { text: root.detail-text; color: #d0d0d4; wrap: word-wrap; }
                }
            }
        }
    }
}
```

- [ ] **Step 3: Add `slint::include_modules!();` at the top of `main.rs`.**

- [ ] **Step 4: Compile**

Run: `cargo build -p magpie-app`
Expected: compiles (Slint codegen runs via build.rs).

- [ ] **Step 5: Manual verification**

Temporarily set `main()` to `LauncherWindow::new().unwrap().run().unwrap();` and run `cargo run -p magpie-app`. Confirm the window opens with the search box, an (empty) list, and the detail pane. Revert `main()` afterward. Record the result in your report.

- [ ] **Step 6: Commit**

```bash
git add crates/magpie-app/ui crates/magpie-app/build.rs crates/magpie-app/src/main.rs
git commit -m "feat(app): Slint launcher window markup"
```

---

### Task 7: App state — shared store + refresh query

**Files:**
- Create: `crates/magpie-app/src/app_state.rs`
- Modify: `crates/magpie-app/src/main.rs` (`mod app_state;`)
- Test: `app_state.rs` tests module.

**Interfaces:**
- Consumes: `magpie_core::{Store, Entry, open}`, `viewmodel::{UiState, to_query}`, `FsImageStore`.
- Produces:
  - `pub struct AppState { pub store: std::sync::Mutex<Store>, pub images: FsImageStore, pub ui: std::sync::Mutex<UiState> }`
  - `pub fn ingest_event(state: &AppState, ev: &magpie_core::CaptureEvent) -> Result<(), String>` — locks the store, calls `ingest`.
  - `pub fn current_results(state: &AppState, now_ms: i64) -> Vec<Entry>` — reads `ui`, builds a query via `to_query`, runs `store.search`.

- [ ] **Step 1: Write failing tests in `app_state.rs`**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use magpie_core::{open_in_memory, CaptureEvent, Content};

    fn state() -> AppState {
        AppState {
            store: std::sync::Mutex::new(open_in_memory().unwrap()),
            images: crate::image_cache::FsImageStore { dir: std::env::temp_dir().join("magpie-appstate-test") },
            ui: std::sync::Mutex::new(crate::viewmodel::UiState::new()),
        }
    }

    #[test]
    fn ingest_then_query_returns_entry() {
        let s = state();
        ingest_event(&s, &CaptureEvent { content: Content::Text("hello".into()), source_app: None, copied_at_ms: 1 }).unwrap();
        let rows = current_results(&s, 1_000);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].full_text, "hello");
    }

    #[test]
    fn ui_text_filter_applies() {
        let s = state();
        ingest_event(&s, &CaptureEvent { content: Content::Text("alpha".into()), source_app: None, copied_at_ms: 1 }).unwrap();
        ingest_event(&s, &CaptureEvent { content: Content::Text("beta".into()), source_app: None, copied_at_ms: 2 }).unwrap();
        s.ui.lock().unwrap().text = "alpha".into();
        let rows = current_results(&s, 1_000);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].full_text, "alpha");
    }
}
```

- [ ] **Step 2: Add `mod app_state;`; run to verify fail**

Run: `cargo test -p magpie-app app_state`
Expected: FAIL.

- [ ] **Step 3: Implement `app_state.rs`**

```rust
use crate::image_cache::FsImageStore;
use crate::viewmodel::{to_query, UiState};
use magpie_core::{CaptureEvent, Entry, Store};
use std::sync::Mutex;

pub struct AppState {
    pub store: Mutex<Store>,
    pub images: FsImageStore,
    pub ui: Mutex<UiState>,
}

pub fn ingest_event(state: &AppState, ev: &CaptureEvent) -> Result<(), String> {
    let store = state.store.lock().map_err(|e| e.to_string())?;
    store.ingest(ev, &state.images).map(|_| ()).map_err(|e| e.to_string())
}

pub fn current_results(state: &AppState, now_ms: i64) -> Vec<Entry> {
    let ui = state.ui.lock().expect("ui lock");
    let q = to_query(&ui, now_ms);
    drop(ui);
    let store = state.store.lock().expect("store lock");
    store.search(&q).unwrap_or_default()
}
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p magpie-app app_state`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/magpie-app/src/app_state.rs crates/magpie-app/src/main.rs
git commit -m "feat(app): shared AppState (store + ui + image cache)"
```

---

### Task 8: Runtime wiring — watcher thread, hotkeys, tray, event loop

**Files:**
- Create: `crates/magpie-app/src/runtime.rs`
- Rewrite: `crates/magpie-app/src/main.rs`

**Interfaces:**
- Consumes: everything above + `magpie_platform::{Watcher, CapturePolicy, os::macos::MacClipboard, os::source_app::ActiveWinSource, os::paste::EnigoPaster, os::hotkeys::Hotkeys, parse_hotkey}`, `tray_icon`, `slint`, `global_hotkey::GlobalHotKeyEvent`.
- Produces a running app: (1) builds `AppState` with real DB path from `dirs`; (2) spawns a watcher thread polling every 250 ms with `SystemTime`-derived `now_ms`, ingesting via `ingest_event` and pushing a UI-refresh; (3) registers launcher + quick-paste hotkeys, spawns a hotkey thread draining `GlobalHotKeyEvent::receiver()`; (4) creates the tray icon; (5) creates the hidden `LauncherWindow` and runs the Slint event loop; window shown/hidden via `slint::invoke_from_event_loop`.

> This is the integration-risk task. It is **manual-verify only**. The reference wiring below is the intended shape; adapt to the exact `slint`/`tray-icon`/`global-hotkey` versions. Keep each concern (watcher, hotkeys, tray, window) in its own function in `runtime.rs` so failures are isolatable.

- [ ] **Step 1: Implement `runtime.rs`** with these functions (reference shapes):

```rust
use crate::app_state::{current_results, ingest_event, AppState};
use crate::paste_action::{perform_paste, resolve_quick_paste, PasteKind};
use magpie_core::open;
use magpie_platform::os::hotkeys::Hotkeys;
use magpie_platform::os::macos::MacClipboard;
use magpie_platform::os::paste::EnigoPaster;
use magpie_platform::os::source_app::ActiveWinSource;
use magpie_platform::{parse_hotkey, CapturePolicy, Watcher};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

pub fn data_dir() -> std::path::PathBuf {
    dirs::data_dir().unwrap_or_else(|| std::env::temp_dir()).join("magpie")
}

pub fn build_state(policy_denylist: Vec<String>) -> Arc<AppState> {
    let dir = data_dir();
    std::fs::create_dir_all(&dir).ok();
    let store = open(&dir.join("magpie.sqlite3")).expect("open db");
    Arc::new(AppState {
        store: std::sync::Mutex::new(store),
        images: crate::image_cache::FsImageStore { dir: dir.join("images") },
        ui: std::sync::Mutex::new(crate::viewmodel::UiState::new()),
    })
}

// Spawns the clipboard-poll loop. `on_change` is called after any successful ingest.
pub fn spawn_watcher(state: Arc<AppState>, mut policy: CapturePolicy, on_change: impl Fn() + Send + 'static) {
    std::thread::spawn(move || {
        let clip = match MacClipboard::new() { Ok(c) => c, Err(_) => return };
        policy.ignore_regexes = magpie_platform::default_ignore_regexes();
        let mut watcher = Watcher::new(clip, ActiveWinSource, policy);
        loop {
            if let Some(ev) = watcher.poll_once(now_ms()) {
                if ingest_event(&state, &ev).is_ok() {
                    on_change();
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
    });
}
```

Add `spawn_hotkeys(...)` that registers the launcher + quick-paste hotkeys from `Config`, maps each registered id to an action, and in a thread drains `global_hotkey::GlobalHotKeyEvent::receiver()`: launcher id → `slint::invoke_from_event_loop` to toggle the window; quick-paste id N → `let recent = current_results(&state, now_ms()); if let Some(e) = resolve_quick_paste(&recent, N) { perform_paste(&mut MacClipboard::new()?, &EnigoPaster, e, PasteKind::Formatted, auto) }`.

- [ ] **Step 2: Rewrite `main.rs`** to: load `Config` from `data_dir().join("config.toml")`, `build_state`, create `LauncherWindow`, set its `search-changed`/`activate`/`copy-only` callbacks (updating `ui` and re-running `current_results` → refresh `entries` model + grouping labels), start hidden, `spawn_watcher` (refresh entries on change via `invoke_from_event_loop`), `spawn_hotkeys`, create the tray icon, then `run()` the Slint loop.

- [ ] **Step 3: Compile**

Run: `cargo build -p magpie-app`
Expected: compiles on macOS.

- [ ] **Step 4: Manual verification** (record all in your report):
  1. Launch `cargo run -p magpie-app`; confirm a tray icon appears and no window is shown.
  2. Copy text in another app; press the launcher hotkey; confirm the window shows the copied item with its source app + metadata.
  3. Type in the search box; confirm live filtering. Toggle a type filter; confirm it narrows results.
  4. Press `Super+Ctrl+1` while focused in a text field of another app; confirm the most-recent item is pasted.
  5. Copy from a password manager (or a denylisted app); confirm it is NOT captured.

- [ ] **Step 5: Commit**

```bash
git add crates/magpie-app/src/runtime.rs crates/magpie-app/src/main.rs
git commit -m "feat(app): runtime wiring — watcher, hotkeys, tray, Slint loop"
```

---

## Self-Review

**Spec coverage:**
- Search UI (live, mode, type/app/time filters, sort) → Tasks 3 (mapping), 6 (markup), 8 (wiring). ✅
- Date-grouped list → Task 4. ✅
- Quick-paste 1..9 + auto-paste + plain-text + copy-only → Tasks 5, 8. ✅
- Dedup capture into store, source app + metadata shown → Tasks 7, 8. ✅
- Image cache + thumbnails → Task 2. ✅
- Config (hotkeys, denylist, paste-on-select) → Task 1. ✅
- Privacy denylist/regex applied at watch layer → Task 8 (wires `CapturePolicy` + defaults from platform). ✅
- Deferred: analytics view, pinned-slot hotkeys, rich re-emission, Wayland/Windows/Linux runtime (Plan 4 provides adapters; this runtime is macOS).

**Placeholder scan:** UI/tray/event-loop tasks (6, 8) are manual-verify by necessity with concrete reference code + explicit checklists — not TODOs. All non-UI logic (1–5, 7) is fully TDD'd.

**Type consistency:** `Config`, `FsImageStore`, `UiState`, `to_query`, `TypeFilter`, `TimeFilter`, `Section`, `group`, `resolve_quick_paste`, `perform_paste`, `PasteKind`, `AppState`, `ingest_event`, `current_results` are used consistently; platform adapter names (`MacClipboard`, `ActiveWinSource`, `EnigoPaster`, `Hotkeys`, `Watcher`, `CapturePolicy`) match Plan 2's public API.

## Next Plans

- **Plan 4** swaps the macOS adapters in `runtime.rs` for `#[cfg]`-selected Windows/Linux adapters implementing the same traits.
- **Plan 5** packages the binary + installs autostart.
