# magpie-core Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build `magpie-core`, the pure-Rust library that stores clipboard entries in SQLite (dedup + copy-count + event log), detects content types, and answers filtered/sorted search queries — with no OS or UI dependencies.

**Architecture:** A single library crate in a Cargo workspace. All state lives in one SQLite database (opened via `rusqlite` with the `bundled` feature, so SQLite + FTS5 are compiled statically into the binary — zero system dependency). Pure functions handle metrics/detection/hashing; the `Store` type owns the DB connection and exposes `ingest` (capture → dedup/insert) and `search`. Time is injected as `now_ms: i64` (Unix milliseconds) so logic is deterministic and testable. Image bytes are written through an injected `ImageStore` trait so the core never touches the filesystem in tests.

**Tech Stack:** Rust (edition 2021), `rusqlite` (features `bundled`), `blake3` (content hashing), `regex` (detection + regex search mode), `fuzzy-matcher` (fuzzy ranking). Testing with the built-in `#[test]` harness; SQLite opened in-memory for tests.

## Global Constraints

- **Platforms:** the wider project targets macOS / Linux / Windows. `magpie-core` itself must be platform-agnostic — no `#[cfg(target_os)]`, no OS or UI calls, no filesystem access except through the `ImageStore` trait.
- **Zero system dependencies:** SQLite must be the `rusqlite` `bundled` build (never link a system libsqlite). FTS5 is enabled via the bundled build.
- **Minimal dependencies:** only the four crates named in Tech Stack. Do not add serde, chrono, tokio, or an ORM.
- **Determinism:** never call `SystemTime::now()` / wall-clock inside core. Timestamps enter as `now_ms: i64` parameters.
- **Dedup model:** identical content collapses to one `entries` row with a running `copy_count`; every copy also appends one `copy_events` row.
- **`kind` values (exact strings):** `text`, `link`, `color`, `email`, `rtf`, `html`, `image`, `file`.
- **No expiry:** core never deletes entries on age. (Retention is a future opt-in feature, out of scope here.)
- **Commit style:** conventional commits, one commit per task.

---

### Task 1: Workspace + crate scaffold

**Files:**
- Create: `Cargo.toml` (workspace root)
- Create: `crates/magpie-core/Cargo.toml`
- Create: `crates/magpie-core/src/lib.rs`
- Create: `rust-toolchain.toml`

**Interfaces:**
- Consumes: nothing.
- Produces: a compiling `magpie-core` library; module skeleton `pub mod metrics; pub mod detect; pub mod hash; pub mod model; pub mod store; pub mod search;` (empty modules added as later tasks fill them — for this task `lib.rs` only declares `metrics`).

- [ ] **Step 1: Create the workspace root `Cargo.toml`**

```toml
[workspace]
members = ["crates/magpie-core"]
resolver = "2"

[workspace.package]
edition = "2021"
version = "0.1.0"
license = "MIT"
```

- [ ] **Step 2: Create `crates/magpie-core/Cargo.toml`**

```toml
[package]
name = "magpie-core"
edition.workspace = true
version.workspace = true
license.workspace = true

[dependencies]
rusqlite = { version = "0.32", features = ["bundled"] }
blake3 = "1"
regex = "1"
fuzzy-matcher = "0.3"
```

- [ ] **Step 3: Pin the toolchain in `rust-toolchain.toml`**

```toml
[toolchain]
channel = "stable"
```

- [ ] **Step 4: Create `crates/magpie-core/src/lib.rs` with a trivial test**

```rust
pub mod metrics;

#[cfg(test)]
mod smoke {
    #[test]
    fn workspace_builds() {
        assert_eq!(2 + 2, 4);
    }
}
```

- [ ] **Step 5: Create `crates/magpie-core/src/metrics.rs` empty for now**

```rust
// filled in Task 2
```

- [ ] **Step 6: Run the build/test to confirm the toolchain works**

Run: `cargo test -p magpie-core`
Expected: PASS (1 test, `workspace_builds`).

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml rust-toolchain.toml crates/magpie-core
git commit -m "chore: scaffold magpie-core crate and workspace"
```

---

### Task 2: Content metrics (char / word / line counts)

**Files:**
- Modify: `crates/magpie-core/src/metrics.rs`
- Test: same file (`#[cfg(test)]` module).

**Interfaces:**
- Consumes: nothing.
- Produces: `pub struct TextMetrics { pub char_count: i64, pub word_count: i64, pub line_count: i64 }` and `pub fn text_metrics(s: &str) -> TextMetrics`. Counting rules: `char_count` = number of Unicode scalar values; `word_count` = number of whitespace-separated non-empty tokens; `line_count` = number of `\n`-separated segments, i.e. `1 + count('\n')` for non-empty input, and `0` for the empty string.

- [ ] **Step 1: Write failing tests in `metrics.rs`**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_string_is_all_zero() {
        let m = text_metrics("");
        assert_eq!((m.char_count, m.word_count, m.line_count), (0, 0, 0));
    }

    #[test]
    fn single_line_counts() {
        let m = text_metrics("hello world");
        assert_eq!((m.char_count, m.word_count, m.line_count), (11, 2, 1));
    }

    #[test]
    fn multiline_counts_lines_and_words() {
        let m = text_metrics("a b\nc\n");
        // chars: a,space,b,\n,c,\n = 6 ; words: a,b,c = 3 ; lines: 1 + 2 newlines = 3
        assert_eq!((m.char_count, m.word_count, m.line_count), (6, 3, 3));
    }

    #[test]
    fn unicode_scalar_values_not_bytes() {
        let m = text_metrics("café");
        assert_eq!(m.char_count, 4);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p magpie-core metrics`
Expected: FAIL — `text_metrics` / `TextMetrics` not found.

- [ ] **Step 3: Implement `metrics.rs`**

```rust
pub struct TextMetrics {
    pub char_count: i64,
    pub word_count: i64,
    pub line_count: i64,
}

pub fn text_metrics(s: &str) -> TextMetrics {
    if s.is_empty() {
        return TextMetrics { char_count: 0, word_count: 0, line_count: 0 };
    }
    let char_count = s.chars().count() as i64;
    let word_count = s.split_whitespace().count() as i64;
    let line_count = 1 + s.matches('\n').count() as i64;
    TextMetrics { char_count, word_count, line_count }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p magpie-core metrics`
Expected: PASS (4 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/magpie-core/src/metrics.rs
git commit -m "feat(core): text metrics (char/word/line counts)"
```

---

### Task 3: Content-type detection

**Files:**
- Create: `crates/magpie-core/src/detect.rs`
- Modify: `crates/magpie-core/src/lib.rs` (add `pub mod detect;`)
- Test: `detect.rs` `#[cfg(test)]` module.

**Interfaces:**
- Consumes: nothing.
- Produces:
  - `pub enum Kind { Text, Link, Color, Email, Rtf, Html, Image, File }` with `pub fn as_str(&self) -> &'static str` (returns the exact strings from Global Constraints) and `pub fn from_str(s: &str) -> Option<Kind>`.
  - `pub fn detect_text_kind(s: &str) -> Kind` — classifies a plain-text string as `Link`, `Color`, `Email`, else `Text`. (Image/File/Rtf/Html are set by the caller based on clipboard format, not by this function.) Detection is whole-string (the trimmed input matches the pattern in full), first match wins in order: Link, Email, Color.

- [ ] **Step 1: Write failing tests in `detect.rs`**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_string_roundtrip() {
        for k in [Kind::Text, Kind::Link, Kind::Color, Kind::Email,
                  Kind::Rtf, Kind::Html, Kind::Image, Kind::File] {
            assert_eq!(Kind::from_str(k.as_str()), Some(k));
        }
        assert_eq!(Kind::from_str("nope"), None);
    }

    #[test]
    fn detects_url() {
        assert!(matches!(detect_text_kind("https://example.com/x"), Kind::Link));
        assert!(matches!(detect_text_kind("  http://a.b  "), Kind::Link));
    }

    #[test]
    fn detects_email() {
        assert!(matches!(detect_text_kind("me@example.io"), Kind::Email));
    }

    #[test]
    fn detects_hex_and_rgb_color() {
        assert!(matches!(detect_text_kind("#1a2b3c"), Kind::Color));
        assert!(matches!(detect_text_kind("#fff"), Kind::Color));
        assert!(matches!(detect_text_kind("rgb(10, 20, 30)"), Kind::Color));
    }

    #[test]
    fn plain_text_is_text() {
        assert!(matches!(detect_text_kind("just some words"), Kind::Text));
        assert!(matches!(detect_text_kind("not#acolor"), Kind::Text));
    }
}
```

- [ ] **Step 2: Add `pub mod detect;` to `lib.rs` and run tests to verify they fail**

Run: `cargo test -p magpie-core detect`
Expected: FAIL — `detect` module / items not found.

- [ ] **Step 3: Implement `detect.rs`**

```rust
use std::sync::OnceLock;
use regex::Regex;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind { Text, Link, Color, Email, Rtf, Html, Image, File }

impl Kind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Kind::Text => "text",
            Kind::Link => "link",
            Kind::Color => "color",
            Kind::Email => "email",
            Kind::Rtf => "rtf",
            Kind::Html => "html",
            Kind::Image => "image",
            Kind::File => "file",
        }
    }
    pub fn from_str(s: &str) -> Option<Kind> {
        Some(match s {
            "text" => Kind::Text,
            "link" => Kind::Link,
            "color" => Kind::Color,
            "email" => Kind::Email,
            "rtf" => Kind::Rtf,
            "html" => Kind::Html,
            "image" => Kind::Image,
            "file" => Kind::File,
            _ => return None,
        })
    }
}

fn url_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^https?://\S+$").unwrap())
}
fn email_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^[^\s@]+@[^\s@]+\.[^\s@]+$").unwrap())
}
fn color_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)^(#([0-9a-f]{3}|[0-9a-f]{6})|rgb\(\s*\d{1,3}\s*,\s*\d{1,3}\s*,\s*\d{1,3}\s*\)|hsl\(\s*\d{1,3}\s*,\s*\d{1,3}%\s*,\s*\d{1,3}%\s*\))$").unwrap()
    })
}

pub fn detect_text_kind(s: &str) -> Kind {
    let t = s.trim();
    if url_re().is_match(t) { return Kind::Link; }
    if email_re().is_match(t) { return Kind::Email; }
    if color_re().is_match(t) { return Kind::Color; }
    Kind::Text
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p magpie-core detect`
Expected: PASS (5 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/magpie-core/src/detect.rs crates/magpie-core/src/lib.rs
git commit -m "feat(core): content-type detection (link/color/email/text)"
```

---

### Task 4: Content model + hashing

**Files:**
- Create: `crates/magpie-core/src/model.rs`
- Modify: `crates/magpie-core/src/lib.rs` (add `pub mod model;`)
- Test: `model.rs` `#[cfg(test)]` module.

**Interfaces:**
- Consumes: `Kind` from `detect`.
- Produces:
  - `pub struct AppInfo { pub identifier: String, pub display_name: String, pub icon_path: Option<String> }`
  - `pub enum Content { Text(String), Rich { text: String, html: Option<String>, rtf: Option<String> }, Image { bytes: Vec<u8> }, Files(Vec<String>) }`
  - `pub struct CaptureEvent { pub content: Content, pub source_app: Option<AppInfo>, pub copied_at_ms: i64 }`
  - `pub struct Entry { pub id: i64, pub content_hash: String, pub kind: Kind, pub preview_text: String, pub full_text: String, pub image_path: Option<String>, pub byte_size: i64, pub char_count: i64, pub word_count: i64, pub line_count: i64, pub first_copied_at_ms: i64, pub last_copied_at_ms: i64, pub copy_count: i64, pub pinned: bool, pub source_app_id: Option<i64> }`
  - `pub fn content_hash(content: &Content) -> String` — a stable blake3 hex digest, domain-separated by content variant so a text `"x"` and a file path `"x"` never collide.

- [ ] **Step 1: Write failing tests in `model.rs`**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_text_hashes_equal() {
        let a = content_hash(&Content::Text("hello".into()));
        let b = content_hash(&Content::Text("hello".into()));
        assert_eq!(a, b);
    }

    #[test]
    fn different_text_hashes_differ() {
        assert_ne!(
            content_hash(&Content::Text("a".into())),
            content_hash(&Content::Text("b".into())),
        );
    }

    #[test]
    fn same_string_different_variant_does_not_collide() {
        assert_ne!(
            content_hash(&Content::Text("x".into())),
            content_hash(&Content::Files(vec!["x".into()])),
        );
    }

    #[test]
    fn image_hashes_by_bytes() {
        assert_eq!(
            content_hash(&Content::Image { bytes: vec![1, 2, 3] }),
            content_hash(&Content::Image { bytes: vec![1, 2, 3] }),
        );
        assert_ne!(
            content_hash(&Content::Image { bytes: vec![1, 2, 3] }),
            content_hash(&Content::Image { bytes: vec![9] }),
        );
    }
}
```

- [ ] **Step 2: Add `pub mod model;` to `lib.rs`, run tests to verify they fail**

Run: `cargo test -p magpie-core model`
Expected: FAIL — items not found.

- [ ] **Step 3: Implement `model.rs`**

```rust
use crate::detect::Kind;

#[derive(Debug, Clone)]
pub struct AppInfo {
    pub identifier: String,
    pub display_name: String,
    pub icon_path: Option<String>,
}

#[derive(Debug, Clone)]
pub enum Content {
    Text(String),
    Rich { text: String, html: Option<String>, rtf: Option<String> },
    Image { bytes: Vec<u8> },
    Files(Vec<String>),
}

#[derive(Debug, Clone)]
pub struct CaptureEvent {
    pub content: Content,
    pub source_app: Option<AppInfo>,
    pub copied_at_ms: i64,
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub id: i64,
    pub content_hash: String,
    pub kind: Kind,
    pub preview_text: String,
    pub full_text: String,
    pub image_path: Option<String>,
    pub byte_size: i64,
    pub char_count: i64,
    pub word_count: i64,
    pub line_count: i64,
    pub first_copied_at_ms: i64,
    pub last_copied_at_ms: i64,
    pub copy_count: i64,
    pub pinned: bool,
    pub source_app_id: Option<i64>,
}

pub fn content_hash(content: &Content) -> String {
    let mut h = blake3::Hasher::new();
    match content {
        Content::Text(t) => { h.update(b"text\0"); h.update(t.as_bytes()); }
        Content::Rich { text, .. } => { h.update(b"text\0"); h.update(text.as_bytes()); }
        Content::Image { bytes } => { h.update(b"image\0"); h.update(bytes); }
        Content::Files(paths) => {
            h.update(b"files\0");
            for p in paths { h.update(p.as_bytes()); h.update(b"\0"); }
        }
    }
    h.finalize().to_hex().to_string()
}
```

Note: `Rich` hashes by its plain-text form so a value copied once as rich text and again as plain text dedups to one entry.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p magpie-core model`
Expected: PASS (4 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/magpie-core/src/model.rs crates/magpie-core/src/lib.rs
git commit -m "feat(core): content model + domain-separated blake3 hashing"
```

---

### Task 5: Database schema + `Store::open` + migrations

**Files:**
- Create: `crates/magpie-core/src/store.rs`
- Create: `crates/magpie-core/src/schema.sql`
- Modify: `crates/magpie-core/src/lib.rs` (add `pub mod store;`)
- Test: `store.rs` `#[cfg(test)]` module.

**Interfaces:**
- Consumes: nothing from prior tasks yet (uses `rusqlite`).
- Produces:
  - `pub struct Store { conn: rusqlite::Connection }`
  - `pub type Result<T> = std::result::Result<T, rusqlite::Error>;`
  - `pub fn open(path: &std::path::Path) -> Result<Store>` — opens a file DB, enables WAL + foreign keys, runs the schema.
  - `pub fn open_in_memory() -> Result<Store>` — for tests.
  - Schema (from `schema.sql`, embedded via `include_str!`): tables `apps`, `entries`, `copy_events`, the `entries_fts` FTS5 virtual table, and the three sync triggers, exactly as specified below.

- [ ] **Step 1: Write `schema.sql`**

```sql
CREATE TABLE IF NOT EXISTS apps (
  id            INTEGER PRIMARY KEY,
  identifier    TEXT NOT NULL UNIQUE,
  display_name  TEXT NOT NULL,
  icon_path     TEXT
);

CREATE TABLE IF NOT EXISTS entries (
  id                 INTEGER PRIMARY KEY,
  content_hash       TEXT NOT NULL UNIQUE,
  kind               TEXT NOT NULL,
  preview_text       TEXT NOT NULL,
  full_text          TEXT NOT NULL,
  image_path         TEXT,
  byte_size          INTEGER NOT NULL,
  char_count         INTEGER NOT NULL,
  word_count         INTEGER NOT NULL,
  line_count         INTEGER NOT NULL,
  first_copied_at_ms INTEGER NOT NULL,
  last_copied_at_ms  INTEGER NOT NULL,
  copy_count         INTEGER NOT NULL DEFAULT 1,
  pinned             INTEGER NOT NULL DEFAULT 0,
  source_app_id      INTEGER REFERENCES apps(id)
);

CREATE INDEX IF NOT EXISTS idx_entries_last_copied ON entries(last_copied_at_ms);
CREATE INDEX IF NOT EXISTS idx_entries_kind        ON entries(kind);
CREATE INDEX IF NOT EXISTS idx_entries_app         ON entries(source_app_id);

CREATE TABLE IF NOT EXISTS copy_events (
  id            INTEGER PRIMARY KEY,
  entry_id      INTEGER NOT NULL REFERENCES entries(id),
  copied_at_ms  INTEGER NOT NULL,
  source_app_id INTEGER REFERENCES apps(id)
);

CREATE INDEX IF NOT EXISTS idx_copy_events_entry ON copy_events(entry_id);
CREATE INDEX IF NOT EXISTS idx_copy_events_time  ON copy_events(copied_at_ms);

CREATE VIRTUAL TABLE IF NOT EXISTS entries_fts USING fts5(
  full_text,
  content='entries',
  content_rowid='id'
);

CREATE TRIGGER IF NOT EXISTS entries_ai AFTER INSERT ON entries BEGIN
  INSERT INTO entries_fts(rowid, full_text) VALUES (new.id, new.full_text);
END;
CREATE TRIGGER IF NOT EXISTS entries_ad AFTER DELETE ON entries BEGIN
  INSERT INTO entries_fts(entries_fts, rowid, full_text) VALUES ('delete', old.id, old.full_text);
END;
CREATE TRIGGER IF NOT EXISTS entries_au AFTER UPDATE ON entries BEGIN
  INSERT INTO entries_fts(entries_fts, rowid, full_text) VALUES ('delete', old.id, old.full_text);
  INSERT INTO entries_fts(rowid, full_text) VALUES (new.id, new.full_text);
END;
```

- [ ] **Step 2: Write failing tests in `store.rs`**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_in_memory_creates_tables() {
        let s = open_in_memory().unwrap();
        let names: Vec<String> = s
            .conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert!(names.contains(&"apps".to_string()));
        assert!(names.contains(&"entries".to_string()));
        assert!(names.contains(&"copy_events".to_string()));
    }

    #[test]
    fn fts5_is_available() {
        // Creating the store already runs the FTS5 virtual-table DDL;
        // if FTS5 were missing, open() would have errored.
        let s = open_in_memory().unwrap();
        let count: i64 = s
            .conn
            .query_row("SELECT count(*) FROM entries_fts", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }
}
```

- [ ] **Step 3: Add `pub mod store;` to `lib.rs`, run tests to verify they fail**

Run: `cargo test -p magpie-core store`
Expected: FAIL — `open_in_memory` not found.

- [ ] **Step 4: Implement `store.rs`**

```rust
use rusqlite::Connection;
use std::path::Path;

pub type Result<T> = std::result::Result<T, rusqlite::Error>;

const SCHEMA: &str = include_str!("schema.sql");

pub struct Store {
    pub(crate) conn: Connection,
}

impl Store {
    fn init(conn: Connection) -> Result<Store> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.execute_batch(SCHEMA)?;
        Ok(Store { conn })
    }
}

pub fn open(path: &Path) -> Result<Store> {
    Store::init(Connection::open(path)?)
}

pub fn open_in_memory() -> Result<Store> {
    Store::init(Connection::open_in_memory()?)
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p magpie-core store`
Expected: PASS (2 tests).

- [ ] **Step 6: Commit**

```bash
git add crates/magpie-core/src/store.rs crates/magpie-core/src/schema.sql crates/magpie-core/src/lib.rs
git commit -m "feat(core): SQLite schema, WAL, FTS5, and Store::open"
```

---

### Task 6: App upsert

**Files:**
- Modify: `crates/magpie-core/src/store.rs` (add `impl Store` method)
- Test: `store.rs` tests module.

**Interfaces:**
- Consumes: `AppInfo` from `model`.
- Produces: `impl Store { pub fn upsert_app(&self, app: &AppInfo) -> Result<i64> }` — inserts the app by unique `identifier`, or updates `display_name`/`icon_path` if it already exists, returning the row id. Idempotent: calling twice with the same identifier yields the same id.

- [ ] **Step 1: Add failing test to `store.rs` tests module**

```rust
    #[test]
    fn upsert_app_is_idempotent_by_identifier() {
        use crate::model::AppInfo;
        let s = open_in_memory().unwrap();
        let a = AppInfo { identifier: "com.ghostty".into(), display_name: "Ghostty".into(), icon_path: None };
        let id1 = s.upsert_app(&a).unwrap();
        let a2 = AppInfo { identifier: "com.ghostty".into(), display_name: "Ghostty 2".into(), icon_path: Some("/i.png".into()) };
        let id2 = s.upsert_app(&a2).unwrap();
        assert_eq!(id1, id2);
        let name: String = s.conn.query_row(
            "SELECT display_name FROM apps WHERE id=?1", [id1], |r| r.get(0)).unwrap();
        assert_eq!(name, "Ghostty 2");
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p magpie-core upsert_app`
Expected: FAIL — `upsert_app` not found.

- [ ] **Step 3: Implement `upsert_app` in `impl Store`**

```rust
use crate::model::AppInfo;

impl Store {
    pub fn upsert_app(&self, app: &AppInfo) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO apps (identifier, display_name, icon_path)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(identifier) DO UPDATE SET
                display_name = excluded.display_name,
                icon_path    = excluded.icon_path",
            rusqlite::params![app.identifier, app.display_name, app.icon_path],
        )?;
        self.conn.query_row(
            "SELECT id FROM apps WHERE identifier = ?1",
            [&app.identifier],
            |r| r.get(0),
        )
    }
}
```

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test -p magpie-core upsert_app`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/magpie-core/src/store.rs
git commit -m "feat(core): upsert_app by unique identifier"
```

---

### Task 7: ImageStore trait + prepared capture

**Files:**
- Modify: `crates/magpie-core/src/store.rs`
- Test: `store.rs` tests module.

**Interfaces:**
- Consumes: `Content`, `content_hash`, `Kind`, `detect_text_kind`, `text_metrics`.
- Produces:
  - `pub trait ImageStore { fn put(&self, hash: &str, bytes: &[u8]) -> std::io::Result<String>; }` — persists image bytes keyed by hash, returns a stored path.
  - `pub struct Prepared { pub hash: String, pub kind: crate::detect::Kind, pub preview_text: String, pub full_text: String, pub image_path: Option<String>, pub byte_size: i64, pub char_count: i64, pub word_count: i64, pub line_count: i64 }`
  - `pub fn prepare(content: &Content, images: &dyn ImageStore) -> std::io::Result<Prepared>` — derives all persisted fields from a `Content`. Rules: `preview_text` is the first 200 chars of `full_text`; for `Image`, `full_text = ""`, `kind = Image`, `image_path = Some(images.put(...))`, `byte_size = bytes.len()`; for `Files`, `full_text = paths.join("\n")`, `kind = File`; for `Rich` with `html.is_some()`, `kind = Html` unless the text detects as Link/Email/Color; for `Text`/`Rich`, `kind = detect_text_kind(text)` (Rich with html falls back to `Html` only when detection yields `Text`).

- [ ] **Step 1: Add failing tests to `store.rs` tests module**

```rust
    struct FakeImages;
    impl ImageStore for FakeImages {
        fn put(&self, hash: &str, _bytes: &[u8]) -> std::io::Result<String> {
            Ok(format!("/cache/{hash}.bin"))
        }
    }

    #[test]
    fn prepare_text_sets_kind_and_metrics() {
        use crate::model::Content;
        let p = prepare(&Content::Text("hello world".into()), &FakeImages).unwrap();
        assert_eq!(p.kind.as_str(), "text");
        assert_eq!(p.full_text, "hello world");
        assert_eq!((p.char_count, p.word_count, p.line_count), (11, 2, 1));
        assert!(p.image_path.is_none());
    }

    #[test]
    fn prepare_detects_link() {
        use crate::model::Content;
        let p = prepare(&Content::Text("https://x.io".into()), &FakeImages).unwrap();
        assert_eq!(p.kind.as_str(), "link");
    }

    #[test]
    fn prepare_image_uses_store_and_zero_text() {
        use crate::model::Content;
        let p = prepare(&Content::Image { bytes: vec![1, 2, 3, 4] }, &FakeImages).unwrap();
        assert_eq!(p.kind.as_str(), "image");
        assert_eq!(p.byte_size, 4);
        assert_eq!(p.full_text, "");
        assert!(p.image_path.unwrap().starts_with("/cache/"));
    }

    #[test]
    fn prepare_files_join_with_newline() {
        use crate::model::Content;
        let p = prepare(&Content::Files(vec!["/a".into(), "/b".into()]), &FakeImages).unwrap();
        assert_eq!(p.kind.as_str(), "file");
        assert_eq!(p.full_text, "/a\n/b");
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p magpie-core prepare`
Expected: FAIL — `prepare` / `ImageStore` not found.

- [ ] **Step 3: Implement in `store.rs`**

```rust
use crate::detect::{detect_text_kind, Kind};
use crate::metrics::text_metrics;
use crate::model::{content_hash, Content};

pub trait ImageStore {
    fn put(&self, hash: &str, bytes: &[u8]) -> std::io::Result<String>;
}

pub struct Prepared {
    pub hash: String,
    pub kind: Kind,
    pub preview_text: String,
    pub full_text: String,
    pub image_path: Option<String>,
    pub byte_size: i64,
    pub char_count: i64,
    pub word_count: i64,
    pub line_count: i64,
}

fn preview_of(s: &str) -> String {
    s.chars().take(200).collect()
}

pub fn prepare(content: &Content, images: &dyn ImageStore) -> std::io::Result<Prepared> {
    let hash = content_hash(content);
    match content {
        Content::Image { bytes } => Ok(Prepared {
            kind: Kind::Image,
            preview_text: String::new(),
            full_text: String::new(),
            image_path: Some(images.put(&hash, bytes)?),
            byte_size: bytes.len() as i64,
            char_count: 0, word_count: 0, line_count: 0,
            hash,
        }),
        Content::Files(paths) => {
            let full = paths.join("\n");
            let m = text_metrics(&full);
            Ok(Prepared {
                kind: Kind::File,
                preview_text: preview_of(&full),
                byte_size: full.len() as i64,
                char_count: m.char_count, word_count: m.word_count, line_count: m.line_count,
                full_text: full, image_path: None, hash,
            })
        }
        Content::Text(t) => text_prepared(hash, t, false),
        Content::Rich { text, html, .. } => text_prepared(hash, text, html.is_some()),
    }
}

fn text_prepared(hash: String, text: &str, is_html: bool) -> std::io::Result<Prepared> {
    let mut kind = detect_text_kind(text);
    if is_html && kind == Kind::Text {
        kind = Kind::Html;
    }
    let m = text_metrics(text);
    Ok(Prepared {
        kind,
        preview_text: preview_of(text),
        full_text: text.to_string(),
        image_path: None,
        byte_size: text.len() as i64,
        char_count: m.char_count, word_count: m.word_count, line_count: m.line_count,
        hash,
    })
}
```

- [ ] **Step 4: Run to verify they pass**

Run: `cargo test -p magpie-core prepare`
Expected: PASS (4 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/magpie-core/src/store.rs
git commit -m "feat(core): ImageStore trait + prepare() field derivation"
```

---

### Task 8: Ingest — new entry insert + first copy_event

**Files:**
- Modify: `crates/magpie-core/src/store.rs`
- Test: `store.rs` tests module.

**Interfaces:**
- Consumes: `CaptureEvent`, `prepare`, `upsert_app`, `ImageStore`.
- Produces:
  - `pub struct Ingested { pub entry_id: i64, pub is_new: bool }`
  - `impl Store { pub fn ingest(&self, ev: &CaptureEvent, images: &dyn ImageStore) -> Result<Ingested> }` — the transaction that dedups and records. This task handles the **new-entry** branch: no existing row with the hash → insert into `entries` (`copy_count = 1`, `first = last = ev.copied_at_ms`), resolve `source_app_id` via `upsert_app`, insert one `copy_events` row, return `{ entry_id, is_new: true }`.

- [ ] **Step 1: Add failing test to `store.rs` tests module**

```rust
    #[test]
    fn ingest_new_text_creates_entry_and_event() {
        use crate::model::{CaptureEvent, Content};
        let s = open_in_memory().unwrap();
        let ev = CaptureEvent {
            content: Content::Text("hello".into()),
            source_app: None,
            copied_at_ms: 1000,
        };
        let out = s.ingest(&ev, &FakeImages).unwrap();
        assert!(out.is_new);

        let (cc, first, last): (i64, i64, i64) = s.conn.query_row(
            "SELECT copy_count, first_copied_at_ms, last_copied_at_ms FROM entries WHERE id=?1",
            [out.entry_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap();
        assert_eq!((cc, first, last), (1, 1000, 1000));

        let events: i64 = s.conn.query_row(
            "SELECT count(*) FROM copy_events WHERE entry_id=?1",
            [out.entry_id], |r| r.get(0)).unwrap();
        assert_eq!(events, 1);
    }

    #[test]
    fn ingest_records_source_app() {
        use crate::model::{AppInfo, CaptureEvent, Content};
        let s = open_in_memory().unwrap();
        let ev = CaptureEvent {
            content: Content::Text("x".into()),
            source_app: Some(AppInfo { identifier: "com.ghostty".into(), display_name: "Ghostty".into(), icon_path: None }),
            copied_at_ms: 5,
        };
        let out = s.ingest(&ev, &FakeImages).unwrap();
        let app_id: Option<i64> = s.conn.query_row(
            "SELECT source_app_id FROM entries WHERE id=?1", [out.entry_id], |r| r.get(0)).unwrap();
        assert!(app_id.is_some());
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p magpie-core ingest`
Expected: FAIL — `ingest` not found.

- [ ] **Step 3: Implement `ingest` (new-entry branch only) in `impl Store`**

```rust
use crate::model::CaptureEvent;

pub struct Ingested {
    pub entry_id: i64,
    pub is_new: bool,
}

impl Store {
    pub fn ingest(&self, ev: &CaptureEvent, images: &dyn ImageStore) -> Result<Ingested> {
        let p = prepare(&ev.content, images)
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
        let app_id: Option<i64> = match &ev.source_app {
            Some(a) => Some(self.upsert_app(a)?),
            None => None,
        };

        let existing: Option<i64> = self.conn.query_row(
            "SELECT id FROM entries WHERE content_hash = ?1",
            [&p.hash], |r| r.get(0)).ok();

        let entry_id = match existing {
            Some(id) => id, // dedup branch completed in Task 9
            None => {
                self.conn.execute(
                    "INSERT INTO entries
                       (content_hash, kind, preview_text, full_text, image_path,
                        byte_size, char_count, word_count, line_count,
                        first_copied_at_ms, last_copied_at_ms, copy_count, pinned, source_app_id)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?10,1,0,?11)",
                    rusqlite::params![
                        p.hash, p.kind.as_str(), p.preview_text, p.full_text, p.image_path,
                        p.byte_size, p.char_count, p.word_count, p.line_count,
                        ev.copied_at_ms, app_id
                    ],
                )?;
                self.conn.last_insert_rowid()
            }
        };

        self.conn.execute(
            "INSERT INTO copy_events (entry_id, copied_at_ms, source_app_id) VALUES (?1, ?2, ?3)",
            rusqlite::params![entry_id, ev.copied_at_ms, app_id],
        )?;

        Ok(Ingested { entry_id, is_new: existing.is_none() })
    }
}
```

- [ ] **Step 4: Run to verify they pass**

Run: `cargo test -p magpie-core ingest`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/magpie-core/src/store.rs
git commit -m "feat(core): ingest new entries + copy_events log"
```

---

### Task 9: Ingest — dedup existing entry (bump count + timestamp)

**Files:**
- Modify: `crates/magpie-core/src/store.rs`
- Test: `store.rs` tests module.

**Interfaces:**
- Consumes: everything from Task 8.
- Produces: extends `ingest`'s existing-entry branch — when the hash already exists, increment `copy_count`, set `last_copied_at_ms = ev.copied_at_ms`, leave `first_copied_at_ms` untouched, and still append a `copy_events` row; return `is_new: false`.

- [ ] **Step 1: Add failing test to `store.rs` tests module**

```rust
    #[test]
    fn ingest_duplicate_bumps_count_and_last_time_only() {
        use crate::model::{CaptureEvent, Content};
        let s = open_in_memory().unwrap();
        let mk = |ms| CaptureEvent { content: Content::Text("same".into()), source_app: None, copied_at_ms: ms };

        let a = s.ingest(&mk(100), &FakeImages).unwrap();
        let b = s.ingest(&mk(200), &FakeImages).unwrap();
        assert_eq!(a.entry_id, b.entry_id);
        assert!(a.is_new && !b.is_new);

        let (cc, first, last): (i64, i64, i64) = s.conn.query_row(
            "SELECT copy_count, first_copied_at_ms, last_copied_at_ms FROM entries WHERE id=?1",
            [a.entry_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap();
        assert_eq!((cc, first, last), (2, 100, 200));

        let events: i64 = s.conn.query_row(
            "SELECT count(*) FROM copy_events WHERE entry_id=?1", [a.entry_id], |r| r.get(0)).unwrap();
        assert_eq!(events, 2);
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p magpie-core ingest_duplicate`
Expected: FAIL — `copy_count` stays 1 / `last` not updated.

- [ ] **Step 3: Update the existing-entry branch in `ingest`**

Replace `Some(id) => id, // dedup branch completed in Task 9` with:

```rust
            Some(id) => {
                self.conn.execute(
                    "UPDATE entries
                       SET copy_count = copy_count + 1,
                           last_copied_at_ms = ?2
                     WHERE id = ?1",
                    rusqlite::params![id, ev.copied_at_ms],
                )?;
                id
            }
```

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test -p magpie-core ingest`
Expected: PASS (all ingest tests, 3 total).

- [ ] **Step 5: Commit**

```bash
git add crates/magpie-core/src/store.rs
git commit -m "feat(core): dedup ingest bumps copy_count + last_copied_at"
```

---

### Task 10: Row hydration + `recent()` listing

**Files:**
- Create: `crates/magpie-core/src/search.rs`
- Modify: `crates/magpie-core/src/lib.rs` (add `pub mod search;`)
- Modify: `crates/magpie-core/src/store.rs` (make `conn` reachable — add `pub(crate) fn conn(&self) -> &rusqlite::Connection`)
- Test: `search.rs` tests module.

**Interfaces:**
- Consumes: `Store`, `Entry`, `Kind::from_str`.
- Produces:
  - In `store.rs`: `impl Store { pub(crate) fn conn(&self) -> &rusqlite::Connection { &self.conn } }`
  - In `search.rs`: `pub(crate) const ENTRY_COLUMNS: &str` (the `SELECT` column list) and `pub(crate) fn row_to_entry(row: &rusqlite::Row) -> rusqlite::Result<Entry>`.
  - `impl Store { pub fn recent(&self, limit: i64) -> Result<Vec<Entry>> }` — newest-first by `last_copied_at_ms`, capped at `limit`.

- [ ] **Step 1: Write failing test in `search.rs`**

```rust
#[cfg(test)]
mod tests {
    use crate::model::{CaptureEvent, Content};
    use crate::store::{open_in_memory, ImageStore};

    struct FakeImages;
    impl ImageStore for FakeImages {
        fn put(&self, hash: &str, _b: &[u8]) -> std::io::Result<String> { Ok(format!("/c/{hash}")) }
    }

    fn text_ev(t: &str, ms: i64) -> CaptureEvent {
        CaptureEvent { content: Content::Text(t.into()), source_app: None, copied_at_ms: ms }
    }

    #[test]
    fn recent_is_newest_first_and_limited() {
        let s = open_in_memory().unwrap();
        s.ingest(&text_ev("old", 100), &FakeImages).unwrap();
        s.ingest(&text_ev("mid", 200), &FakeImages).unwrap();
        s.ingest(&text_ev("new", 300), &FakeImages).unwrap();

        let rows = s.recent(2).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].full_text, "new");
        assert_eq!(rows[1].full_text, "mid");
        assert_eq!(rows[0].kind.as_str(), "text");
    }
}
```

- [ ] **Step 2: Add `pub mod search;` to `lib.rs`; run to verify it fails**

Run: `cargo test -p magpie-core recent_is_newest`
Expected: FAIL — `recent` not found.

- [ ] **Step 3: Add `conn()` accessor to `store.rs`**

```rust
impl Store {
    pub(crate) fn conn(&self) -> &rusqlite::Connection {
        &self.conn
    }
}
```

- [ ] **Step 4: Implement hydration + `recent` in `search.rs`**

```rust
use crate::detect::Kind;
use crate::model::Entry;
use crate::store::{Result, Store};

pub(crate) const ENTRY_COLUMNS: &str = "id, content_hash, kind, preview_text, full_text, image_path, \
    byte_size, char_count, word_count, line_count, first_copied_at_ms, last_copied_at_ms, \
    copy_count, pinned, source_app_id";

pub(crate) fn row_to_entry(row: &rusqlite::Row) -> rusqlite::Result<Entry> {
    let kind_str: String = row.get(2)?;
    Ok(Entry {
        id: row.get(0)?,
        content_hash: row.get(1)?,
        kind: Kind::from_str(&kind_str).unwrap_or(Kind::Text),
        preview_text: row.get(3)?,
        full_text: row.get(4)?,
        image_path: row.get(5)?,
        byte_size: row.get(6)?,
        char_count: row.get(7)?,
        word_count: row.get(8)?,
        line_count: row.get(9)?,
        first_copied_at_ms: row.get(10)?,
        last_copied_at_ms: row.get(11)?,
        copy_count: row.get(12)?,
        pinned: row.get::<_, i64>(13)? != 0,
        source_app_id: row.get(14)?,
    })
}

impl Store {
    pub fn recent(&self, limit: i64) -> Result<Vec<Entry>> {
        let sql = format!(
            "SELECT {ENTRY_COLUMNS} FROM entries ORDER BY last_copied_at_ms DESC LIMIT ?1"
        );
        let mut stmt = self.conn().prepare(&sql)?;
        let rows = stmt.query_map([limit], row_to_entry)?;
        rows.collect()
    }
}
```

- [ ] **Step 5: Run to verify it passes**

Run: `cargo test -p magpie-core recent_is_newest`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/magpie-core/src/search.rs crates/magpie-core/src/store.rs crates/magpie-core/src/lib.rs
git commit -m "feat(core): entry hydration + recent() listing"
```

---

### Task 11: Search query type + word/exact modes + filters + sort

**Files:**
- Modify: `crates/magpie-core/src/search.rs`
- Test: `search.rs` tests module.

**Interfaces:**
- Consumes: `Store`, `Entry`, `row_to_entry`, `ENTRY_COLUMNS`.
- Produces:
  - `pub enum SearchMode { Word, Exact, Fuzzy, Regex }`
  - `pub enum Sort { Recency, MostCopied, Alphabetical }`
  - `pub struct TimeRange { pub since_ms: Option<i64>, pub until_ms: Option<i64> }`
  - `pub struct SearchQuery { pub text: String, pub mode: SearchMode, pub kind: Option<crate::detect::Kind>, pub source_app_id: Option<i64>, pub time: TimeRange, pub sort: Sort, pub limit: i64 }` with `pub fn default_query() -> SearchQuery` (empty text, `Word`, no filters, `Recency`, limit 200).
  - `impl Store { pub fn search(&self, q: &SearchQuery) -> Result<Vec<Entry>> }`. This task implements `Word` (FTS5 MATCH, terms ANDed), `Exact` (case-insensitive substring via `LIKE`), empty-text (all rows), plus the `kind` / `source_app_id` / `time` filters and all three `sort`s. `Fuzzy` and `Regex` are added in Task 12 — for now they behave like `Word`.

- [ ] **Step 1: Add failing tests to `search.rs` tests module**

```rust
    use crate::detect::Kind;
    use crate::model::AppInfo;
    use crate::search::{default_query, SearchMode, Sort};

    fn app_ev(t: &str, ms: i64, ident: &str) -> CaptureEvent {
        CaptureEvent {
            content: Content::Text(t.into()),
            source_app: Some(AppInfo { identifier: ident.into(), display_name: ident.into(), icon_path: None }),
            copied_at_ms: ms,
        }
    }

    #[test]
    fn word_mode_ands_terms() {
        let s = open_in_memory().unwrap();
        s.ingest(&text_ev("alpha beta gamma", 1), &FakeImages).unwrap();
        s.ingest(&text_ev("alpha only", 2), &FakeImages).unwrap();
        let mut q = default_query();
        q.text = "alpha gamma".into();
        let rows = s.search(&q).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].full_text, "alpha beta gamma");
    }

    #[test]
    fn exact_mode_is_substring_case_insensitive() {
        let s = open_in_memory().unwrap();
        s.ingest(&text_ev("HelloWorld", 1), &FakeImages).unwrap();
        let mut q = default_query();
        q.mode = SearchMode::Exact;
        q.text = "loworl".into();
        assert_eq!(s.search(&q).unwrap().len(), 1);
    }

    #[test]
    fn filter_by_kind() {
        let s = open_in_memory().unwrap();
        s.ingest(&text_ev("plain text here", 1), &FakeImages).unwrap();
        s.ingest(&text_ev("https://x.io", 2), &FakeImages).unwrap();
        let mut q = default_query();
        q.kind = Some(Kind::Link);
        let rows = s.search(&q).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].kind.as_str(), "link");
    }

    #[test]
    fn filter_by_app_and_time() {
        let s = open_in_memory().unwrap();
        s.ingest(&app_ev("from ghostty", 1000, "com.ghostty"), &FakeImages).unwrap();
        s.ingest(&app_ev("from safari", 2000, "com.safari"), &FakeImages).unwrap();
        let ghostty_id: i64 = s.conn().query_row(
            "SELECT id FROM apps WHERE identifier='com.ghostty'", [], |r| r.get(0)).unwrap();
        let mut q = default_query();
        q.source_app_id = Some(ghostty_id);
        assert_eq!(s.search(&q).unwrap().len(), 1);

        let mut q2 = default_query();
        q2.time.since_ms = Some(1500);
        let rows = s.search(&q2).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].full_text, "from safari");
    }

    #[test]
    fn sort_most_copied() {
        let s = open_in_memory().unwrap();
        s.ingest(&text_ev("once", 1), &FakeImages).unwrap();
        s.ingest(&text_ev("twice", 2), &FakeImages).unwrap();
        s.ingest(&text_ev("twice", 3), &FakeImages).unwrap();
        let mut q = default_query();
        q.sort = Sort::MostCopied;
        let rows = s.search(&q).unwrap();
        assert_eq!(rows[0].full_text, "twice");
        assert_eq!(rows[0].copy_count, 2);
    }

    #[test]
    fn word_text_and_kind_filter_combine() {
        // Guards param ordering: text query AND a filter must both apply.
        let s = open_in_memory().unwrap();
        s.ingest(&text_ev("https://alpha.example", 1), &FakeImages).unwrap(); // link, token 'alpha'
        s.ingest(&text_ev("alpha plain note", 2), &FakeImages).unwrap();       // text, token 'alpha'
        let mut q = default_query();
        q.text = "alpha".into();
        q.kind = Some(Kind::Link);
        let rows = s.search(&q).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].kind.as_str(), "link");
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p magpie-core search`
Expected: FAIL — `SearchQuery` / `search` not found.

- [ ] **Step 3: Implement in `search.rs`**

```rust
use rusqlite::types::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchMode { Word, Exact, Fuzzy, Regex }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sort { Recency, MostCopied, Alphabetical }

#[derive(Debug, Clone, Default)]
pub struct TimeRange { pub since_ms: Option<i64>, pub until_ms: Option<i64> }

#[derive(Debug, Clone)]
pub struct SearchQuery {
    pub text: String,
    pub mode: SearchMode,
    pub kind: Option<Kind>,
    pub source_app_id: Option<i64>,
    pub time: TimeRange,
    pub sort: Sort,
    pub limit: i64,
}

pub fn default_query() -> SearchQuery {
    SearchQuery {
        text: String::new(),
        mode: SearchMode::Word,
        kind: None,
        source_app_id: None,
        time: TimeRange::default(),
        sort: Sort::Recency,
        limit: 200,
    }
}

fn order_clause(sort: Sort) -> &'static str {
    match sort {
        Sort::Recency => "e.last_copied_at_ms DESC",
        Sort::MostCopied => "e.copy_count DESC, e.last_copied_at_ms DESC",
        Sort::Alphabetical => "e.full_text COLLATE NOCASE ASC",
    }
}

fn fts_match(text: &str) -> String {
    // Quote each term and AND them: alpha gamma -> "alpha" AND "gamma"
    text.split_whitespace()
        .map(|t| format!("\"{}\"", t.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" AND ")
}

impl Store {
    pub fn search(&self, q: &SearchQuery) -> Result<Vec<Entry>> {
        // Every clause is pushed to `clauses` and its param(s) to `params` in the
        // SAME order, then joined with AND. The word-mode text query uses an FTS
        // subquery so its MATCH param is just another positional `?` in sequence —
        // no special-case ordering. (Fuzzy/Regex are dispatched in Task 12 before
        // this SQL path runs.)
        let mut clauses: Vec<String> = Vec::new();
        let mut params: Vec<Value> = Vec::new();

        let trimmed = q.text.trim();
        if !trimmed.is_empty() {
            match q.mode {
                SearchMode::Exact => {
                    clauses.push("e.full_text LIKE ?".to_string());
                    params.push(Value::Text(format!("%{}%", trimmed)));
                }
                _ => {
                    clauses.push(
                        "e.id IN (SELECT rowid FROM entries_fts WHERE entries_fts MATCH ?)".to_string(),
                    );
                    params.push(Value::Text(fts_match(trimmed)));
                }
            }
        }
        if let Some(k) = q.kind {
            clauses.push("e.kind = ?".to_string());
            params.push(Value::Text(k.as_str().to_string()));
        }
        if let Some(app) = q.source_app_id {
            clauses.push("e.source_app_id = ?".to_string());
            params.push(Value::Integer(app));
        }
        if let Some(since) = q.time.since_ms {
            clauses.push("e.last_copied_at_ms >= ?".to_string());
            params.push(Value::Integer(since));
        }
        if let Some(until) = q.time.until_ms {
            clauses.push("e.last_copied_at_ms <= ?".to_string());
            params.push(Value::Integer(until));
        }

        let where_sql = if clauses.is_empty() { "1=1".to_string() } else { clauses.join(" AND ") };
        let sql = format!(
            "SELECT {cols} FROM entries e WHERE {where_sql} ORDER BY {order} LIMIT ?",
            cols = entry_cols_prefixed(),
            order = order_clause(q.sort),
        );
        params.push(Value::Integer(q.limit));

        let mut stmt = self.conn().prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(params), row_to_entry)?;
        rows.collect()
    }
}

fn entry_cols_prefixed() -> String {
    ENTRY_COLUMNS
        .split(", ")
        .map(|c| format!("e.{c}"))
        .collect::<Vec<_>>()
        .join(", ")
}
```

- [ ] **Step 4: Run to verify they pass**

Run: `cargo test -p magpie-core search`
Expected: PASS (all search tests).

- [ ] **Step 5: Commit**

```bash
git add crates/magpie-core/src/search.rs
git commit -m "feat(core): search query with word/exact modes, filters, sort"
```

---

### Task 12: Fuzzy ranking + regex mode

**Files:**
- Modify: `crates/magpie-core/src/search.rs`
- Test: `search.rs` tests module.

**Interfaces:**
- Consumes: everything in Task 11; `fuzzy_matcher::skim::SkimMatcherV2`, `regex::Regex`.
- Produces: `search` now honors `SearchMode::Fuzzy` and `SearchMode::Regex`. Both are implemented in Rust over the candidate set:
  - **Fuzzy:** load candidates (all rows passing the non-text filters), score each against `q.text` with a case-insensitive fuzzy matcher; keep only positive matches; entries whose `full_text` contains the query as a case-insensitive **substring** are boosted so they always rank **above** fuzzy-only matches; then apply `limit`.
  - **Regex:** compile `q.text` case-insensitively; keep candidates whose `full_text` matches; invalid regex → empty result (never panic). Sort of surviving rows follows `q.sort`.

- [ ] **Step 1: Add failing tests to `search.rs` tests module**

```rust
    #[test]
    fn fuzzy_matches_noncontiguous_and_ranks_substring_first() {
        let s = open_in_memory().unwrap();
        s.ingest(&text_ev("foo bar baz", 1), &FakeImages).unwrap();  // fuzzy 'fbb'
        s.ingest(&text_ev("fbb exact", 2), &FakeImages).unwrap();    // substring 'fbb'
        s.ingest(&text_ev("nothing", 3), &FakeImages).unwrap();
        let mut q = default_query();
        q.mode = SearchMode::Fuzzy;
        q.text = "fbb".into();
        let rows = s.search(&q).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].full_text, "fbb exact"); // substring ranked above fuzzy-only
    }

    #[test]
    fn regex_mode_filters_and_bad_regex_is_empty() {
        let s = open_in_memory().unwrap();
        s.ingest(&text_ev("order-123", 1), &FakeImages).unwrap();
        s.ingest(&text_ev("order-abc", 2), &FakeImages).unwrap();
        let mut q = default_query();
        q.mode = SearchMode::Regex;
        q.text = r"order-\d+".into();
        assert_eq!(s.search(&q).unwrap().len(), 1);

        q.text = r"order-(".into(); // invalid
        assert_eq!(s.search(&q).unwrap().len(), 0);
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p magpie-core fuzzy_matches`
Expected: FAIL — fuzzy currently behaves like word mode (`fbb` finds nothing via FTS).

- [ ] **Step 3: Implement fuzzy/regex branches in `search.rs`**

Add imports at the top of the file:

```rust
use fuzzy_matcher::skim::SkimMatcherV2;
use fuzzy_matcher::FuzzyMatcher;
use regex::RegexBuilder;
```

Add a helper that loads candidates honoring only the non-text filters, reusing the same filter code path. The cleanest approach: extract the filter-building into `fn candidates(&self, q: &SearchQuery) -> Result<Vec<Entry>>` that runs the Task 11 query **with `text` blanked**, then post-filter in Rust. Implement:

```rust
impl Store {
    fn candidates(&self, q: &SearchQuery) -> Result<Vec<Entry>> {
        let mut base = q.clone();
        base.text = String::new();       // drop text; keep kind/app/time/sort
        base.limit = 100_000;            // wide net; we cap after ranking
        self.search(&base)
    }

    fn search_fuzzy(&self, q: &SearchQuery) -> Result<Vec<Entry>> {
        let matcher = SkimMatcherV2::default().ignore_case();
        let needle = q.text.trim();
        let needle_lc = needle.to_lowercase();
        let mut scored: Vec<(i64, Entry)> = Vec::new();
        for e in self.candidates(q)? {
            let hay = e.full_text.to_lowercase();
            if let Some(score) = matcher.fuzzy_match(&e.full_text, needle) {
                let boost = if hay.contains(&needle_lc) { 1_000_000 } else { 0 };
                scored.push((boost + score, e));
            }
        }
        scored.sort_by(|a, b| b.0.cmp(&a.0));
        Ok(scored.into_iter().take(q.limit as usize).map(|(_, e)| e).collect())
    }

    fn search_regex(&self, q: &SearchQuery) -> Result<Vec<Entry>> {
        let re = match RegexBuilder::new(q.text.trim()).case_insensitive(true).build() {
            Ok(re) => re,
            Err(_) => return Ok(Vec::new()),
        };
        Ok(self
            .candidates(q)?
            .into_iter()
            .filter(|e| re.is_match(&e.full_text))
            .take(q.limit as usize)
            .collect())
    }
}
```

Then, at the very top of `search`, dispatch the two Rust-side modes before building SQL (guard against infinite recursion — `candidates` calls `search` with `Word` mode implicitly by blanking text, which takes the non-fuzzy path since text is empty):

```rust
    pub fn search(&self, q: &SearchQuery) -> Result<Vec<Entry>> {
        if !q.text.trim().is_empty() {
            match q.mode {
                SearchMode::Fuzzy => return self.search_fuzzy(q),
                SearchMode::Regex => return self.search_regex(q),
                _ => {}
            }
        }
        // ... existing Task 11 body unchanged ...
```

- [ ] **Step 4: Run to verify they pass**

Run: `cargo test -p magpie-core`
Expected: PASS (all core tests, including fuzzy + regex).

- [ ] **Step 5: Commit**

```bash
git add crates/magpie-core/src/search.rs
git commit -m "feat(core): fuzzy ranking (substring-first) + regex search mode"
```

---

### Task 13: Pin toggle + public API surface + crate docs

**Files:**
- Modify: `crates/magpie-core/src/store.rs` (add `set_pinned`)
- Modify: `crates/magpie-core/src/lib.rs` (re-exports + crate doc)
- Test: `store.rs` tests module.

**Interfaces:**
- Consumes: `Store`.
- Produces:
  - `impl Store { pub fn set_pinned(&self, entry_id: i64, pinned: bool) -> Result<()> }`
  - `lib.rs` re-exports the public surface: `pub use detect::Kind; pub use model::{AppInfo, Content, CaptureEvent, Entry}; pub use store::{open, open_in_memory, ImageStore, Ingested, Store}; pub use search::{SearchMode, SearchQuery, Sort, TimeRange, default_query};`

- [ ] **Step 1: Add failing test to `store.rs` tests module**

```rust
    #[test]
    fn set_pinned_persists() {
        use crate::model::{CaptureEvent, Content};
        let s = open_in_memory().unwrap();
        let out = s.ingest(&CaptureEvent {
            content: Content::Text("keep me".into()), source_app: None, copied_at_ms: 1,
        }, &FakeImages).unwrap();
        s.set_pinned(out.entry_id, true).unwrap();
        let pinned: i64 = s.conn.query_row(
            "SELECT pinned FROM entries WHERE id=?1", [out.entry_id], |r| r.get(0)).unwrap();
        assert_eq!(pinned, 1);
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p magpie-core set_pinned`
Expected: FAIL — `set_pinned` not found.

- [ ] **Step 3: Implement `set_pinned` and add re-exports**

In `store.rs`:

```rust
impl Store {
    pub fn set_pinned(&self, entry_id: i64, pinned: bool) -> Result<()> {
        self.conn.execute(
            "UPDATE entries SET pinned = ?2 WHERE id = ?1",
            rusqlite::params![entry_id, pinned as i64],
        )?;
        Ok(())
    }
}
```

Replace the top of `lib.rs` module declarations with the doc + re-exports:

```rust
//! `magpie-core` — storage, content-type detection, and search for the Magpie
//! clipboard manager. Platform- and UI-agnostic; time is injected as
//! `copied_at_ms`, and image bytes are written through the `ImageStore` trait.

pub mod detect;
pub mod metrics;
pub mod model;
pub mod search;
pub mod store;

pub use detect::Kind;
pub use model::{AppInfo, CaptureEvent, Content, Entry};
pub use search::{default_query, SearchMode, SearchQuery, Sort, TimeRange};
pub use store::{open, open_in_memory, ImageStore, Ingested, Store};
```

- [ ] **Step 4: Run the full suite to verify everything passes**

Run: `cargo test -p magpie-core`
Expected: PASS (all tests).

- [ ] **Step 5: Verify the public API compiles from outside via a doc-test**

Add to `lib.rs`:

```rust
/// ```
/// use magpie_core::{open_in_memory, CaptureEvent, Content, default_query, ImageStore};
/// struct NoImages;
/// impl ImageStore for NoImages {
///     fn put(&self, h: &str, _b: &[u8]) -> std::io::Result<String> { Ok(h.into()) }
/// }
/// let s = open_in_memory().unwrap();
/// s.ingest(&CaptureEvent { content: Content::Text("hi".into()), source_app: None, copied_at_ms: 1 }, &NoImages).unwrap();
/// assert_eq!(s.search(&default_query()).unwrap().len(), 1);
/// ```
fn _doc_api() {}
```

Run: `cargo test -p magpie-core --doc`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/magpie-core/src
git commit -m "feat(core): pin toggle + public API surface + crate docs"
```

---

## Self-Review

**Spec coverage** (against the design doc):
- Dedup + count → Tasks 8, 9. `copy_events` log → Tasks 8, 9. ✅
- Content-type detection (link/color/email/image/file/text) → Tasks 3, 7. ✅
- Metrics (char/word/line) → Task 2, surfaced in `prepare` (Task 7) and persisted (Task 8). ✅
- Source-app attribution storage → Tasks 6, 8. ✅
- SQLite bundled + WAL + FTS5 → Task 5. ✅
- Image cache via injected store (core stays filesystem-free) → Task 7. ✅
- Search: word default, exact, fuzzy (substring-first), regex, highlighting-data, type/app/time filters, recency/most-copied/alphabetical sort → Tasks 10–12. (Match **highlighting** rendering is a UI concern for the `magpie-app` plan; core returns `full_text`/`preview_text` the UI highlights.) ✅
- Pinned favorites (storage + toggle) → Tasks 5, 13. ✅
- No expiry → nothing deletes by age. ✅
- Not in this plan (correctly deferred to later plans): OS clipboard watch, hotkeys, paste, Slint UI, privacy secret-marker matrix (watch-layer), autostart, analytics view. These belong to `magpie-platform` / `magpie-app` plans.

**Placeholder scan:** no TBD/TODO; every code step has real code; the one "fiddly" note in Task 11 includes a concrete alternative structure and points to the tests as the contract. ✅

**Type consistency:** `Store`, `Ingested`, `Prepared`, `ImageStore`, `CaptureEvent`, `Content`, `Entry`, `Kind`, `SearchQuery`, `SearchMode`, `Sort`, `TimeRange`, `default_query`, `row_to_entry`, `ENTRY_COLUMNS`, `content_hash`, `prepare`, `ingest`, `recent`, `search`, `set_pinned`, `upsert_app`, `open_in_memory` are used consistently across tasks. `Kind::as_str`/`from_str` string values match the schema `kind` column and Global Constraints. ✅

## Next Plans (roadmap, not part of this plan)

1. **`magpie-platform`** — traits (`ClipboardWatcher`, `SourceApp`, `Hotkeys`, `Paster`, `Autostart`) + macOS impls + fakes; the privacy secret-marker matrix at the watch layer.
2. **`magpie-app`** — Slint launcher (search UI with mode toggle, filter chips, type tabs, virtualized list, detail pane), tray, global-hotkey wiring, quick-paste 1..9, paste-as-plain-text.
3. **Windows + Linux** platform impls (event-driven watchers; Wayland best-effort attribution).
4. **Packaging** — static release builds + autostart install recipes per OS.
