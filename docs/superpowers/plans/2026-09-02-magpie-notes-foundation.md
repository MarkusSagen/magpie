# Notes Foundation (Phase 1) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a notes/pages workspace to Magpie — daily + named pages, an editor, `[[link]]` navigation, and capturing a note from a clipboard entry with provenance — as Phase 1 of the notes/tasks/links subsystem.

**Architecture:** Additive SQLite `notes` table + a `magpie_core::notes` module (`Note` struct + get-or-create/update/list methods on `Store`, mirroring `tags.rs`/`slots.rs`). Pure app helpers (`wiki_links`, `note_list_title`) are unit-tested; a new **Notes mode** in `launcher.slint` (list + editor + links strip + provenance) is wired in `runtime.rs` and launch-verified.

**Tech Stack:** Rust, rusqlite (bundled, FTS5/WAL already configured), Slint 1.8 UI.

**Spec:** `docs/superpowers/specs/2026-09-02-magpie-notes-foundation-design.md`

## Global Constraints

- **No new dependencies.** Reuse rusqlite + Slint already in the workspace.
- **Additive schema only** — `CREATE TABLE IF NOT EXISTS`, no migration. `notes.name` is UNIQUE (one page per name).
- **Core stays wall-clock-free** — time is injected as `now_ms: i64`; ISO day strings are computed by the app (`magpie_app::format_time`), never in core.
- **Toolchain / gates:** build & test with `env -u RUSTUP_TOOLCHAIN ~/.cargo/bin/cargo …`. Every task ends green on `cargo test --workspace`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`. Run `cargo fmt` before committing.
- **UI shortcuts use `event.modifiers.control`** (= ⌘ on macOS via Slint's winit mapping), never `.meta`.
- **Commit style:** conventional commits; end the message body with the `Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>` trailer. One commit per task.
- Notes are stored plaintext in the same DB as clips (documented; not an at-rest boundary). Clipboard retention must NOT touch `notes`.

---

### Task 1: `notes` table + core CRUD

**Files:**
- Modify: `crates/magpie-core/src/schema.sql` (append the `notes` table + indexes)
- Create: `crates/magpie-core/src/notes.rs` (`Note`, `row_to_note`, CRUD on `Store`)
- Modify: `crates/magpie-core/src/lib.rs` (`pub mod notes;` + `pub use notes::Note;`)
- Test: `crates/magpie-core/tests/notes.rs`

**Interfaces:**
- Consumes: `crate::store::{Result, Store}`, `self.conn()`, `open_in_memory()`.
- Produces:
  - `pub struct Note { pub id: i64, pub name: String, pub is_daily: bool, pub body: String, pub created_at_ms: i64, pub updated_at_ms: i64, pub source_app_id: Option<i64>, pub source_entry_id: Option<i64> }`
  - `Store::daily_note(&self, day: &str, now_ms: i64) -> Result<Note>`
  - `Store::upsert_note_by_name(&self, name: &str, now_ms: i64) -> Result<Note>`
  - `Store::get_note(&self, id: i64) -> Result<Option<Note>>`
  - `Store::note_by_name(&self, name: &str) -> Result<Option<Note>>`
  - `Store::update_note_body(&self, id: i64, body: &str, now_ms: i64) -> Result<bool>`
  - `Store::recent_notes(&self, limit: i64) -> Result<Vec<Note>>`
  - `Store::all_note_names(&self) -> Result<Vec<String>>`

- [ ] **Step 1: Add the schema.** Append to `crates/magpie-core/src/schema.sql`:

```sql
CREATE TABLE IF NOT EXISTS notes (
  id              INTEGER PRIMARY KEY,
  name            TEXT NOT NULL,
  is_daily        INTEGER NOT NULL DEFAULT 0,
  body            TEXT NOT NULL DEFAULT '',
  created_at_ms   INTEGER NOT NULL,
  updated_at_ms   INTEGER NOT NULL,
  source_app_id   INTEGER,
  source_entry_id INTEGER
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_notes_name ON notes(name);
CREATE INDEX IF NOT EXISTS idx_notes_updated ON notes(updated_at_ms);
```

- [ ] **Step 2: Write the failing tests** in `crates/magpie-core/tests/notes.rs`:

```rust
use magpie_core::{open_in_memory, Store};

fn store() -> Store {
    open_in_memory().unwrap()
}

#[test]
fn upsert_by_name_is_idempotent_and_trims() {
    let s = store();
    let a = s.upsert_note_by_name("  Prod Incidents  ", 10).unwrap();
    let b = s.upsert_note_by_name("Prod Incidents", 20).unwrap();
    assert_eq!(a.id, b.id); // same page
    assert_eq!(a.name, "Prod Incidents");
    assert!(!a.is_daily);
    assert!(s.upsert_note_by_name("   ", 30).is_err()); // empty name rejected
}

#[test]
fn daily_note_get_or_create() {
    let s = store();
    let a = s.daily_note("2026-09-02", 100).unwrap();
    let b = s.daily_note("2026-09-02", 200).unwrap();
    assert_eq!(a.id, b.id);
    assert!(a.is_daily);
    assert_eq!(a.name, "2026-09-02");
}

#[test]
fn update_body_bumps_updated_and_reports_missing() {
    let s = store();
    let n = s.upsert_note_by_name("page", 100).unwrap();
    assert!(s.update_note_body(n.id, "hello [[world]]", 500).unwrap());
    let got = s.get_note(n.id).unwrap().unwrap();
    assert_eq!(got.body, "hello [[world]]");
    assert_eq!(got.updated_at_ms, 500);
    assert!(!s.update_note_body(999_999, "x", 600).unwrap()); // unknown id
}

#[test]
fn recent_notes_orders_by_updated_desc() {
    let s = store();
    let a = s.upsert_note_by_name("a", 100).unwrap();
    let b = s.upsert_note_by_name("b", 200).unwrap();
    s.update_note_body(a.id, "later", 300).unwrap(); // a now most recent
    let names: Vec<String> = s.recent_notes(10).unwrap().into_iter().map(|n| n.name).collect();
    assert_eq!(names, vec!["a".to_string(), "b".to_string()]);
    let _ = b;
}

#[test]
fn all_note_names_sorted_distinct() {
    let s = store();
    s.upsert_note_by_name("zeta", 1).unwrap();
    s.upsert_note_by_name("alpha", 2).unwrap();
    assert_eq!(s.all_note_names().unwrap(), vec!["alpha".to_string(), "zeta".to_string()]);
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `env -u RUSTUP_TOOLCHAIN ~/.cargo/bin/cargo test -p magpie-core --test notes`
Expected: FAIL — `notes` module / methods don't exist.

- [ ] **Step 4: Implement `crates/magpie-core/src/notes.rs`:**

```rust
use crate::store::{Result, Store};
use rusqlite::OptionalExtension;

const NOTE_COLS: &str =
    "id, name, is_daily, body, created_at_ms, updated_at_ms, source_app_id, source_entry_id";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note {
    pub id: i64,
    pub name: String,
    pub is_daily: bool,
    pub body: String,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub source_app_id: Option<i64>,
    pub source_entry_id: Option<i64>,
}

fn row_to_note(r: &rusqlite::Row) -> rusqlite::Result<Note> {
    Ok(Note {
        id: r.get(0)?,
        name: r.get(1)?,
        is_daily: r.get::<_, i64>(2)? != 0,
        body: r.get(3)?,
        created_at_ms: r.get(4)?,
        updated_at_ms: r.get(5)?,
        source_app_id: r.get(6)?,
        source_entry_id: r.get(7)?,
    })
}

impl Store {
    pub fn note_by_name(&self, name: &str) -> Result<Option<Note>> {
        self.conn()
            .query_row(
                &format!("SELECT {NOTE_COLS} FROM notes WHERE name = ?1"),
                [name],
                row_to_note,
            )
            .optional()
    }

    pub fn get_note(&self, id: i64) -> Result<Option<Note>> {
        self.conn()
            .query_row(
                &format!("SELECT {NOTE_COLS} FROM notes WHERE id = ?1"),
                [id],
                row_to_note,
            )
            .optional()
    }

    pub fn upsert_note_by_name(&self, name: &str, now_ms: i64) -> Result<Note> {
        let n = name.trim();
        if n.is_empty() {
            return Err(rusqlite::Error::InvalidParameterName(
                "empty note name".into(),
            ));
        }
        self.conn().execute(
            "INSERT OR IGNORE INTO notes (name, is_daily, body, created_at_ms, updated_at_ms)
             VALUES (?1, 0, '', ?2, ?2)",
            rusqlite::params![n, now_ms],
        )?;
        Ok(self.note_by_name(n)?.expect("row exists after INSERT OR IGNORE"))
    }

    pub fn daily_note(&self, day: &str, now_ms: i64) -> Result<Note> {
        self.conn().execute(
            "INSERT OR IGNORE INTO notes (name, is_daily, body, created_at_ms, updated_at_ms)
             VALUES (?1, 1, '', ?2, ?2)",
            rusqlite::params![day, now_ms],
        )?;
        Ok(self.note_by_name(day)?.expect("row exists after INSERT OR IGNORE"))
    }

    pub fn update_note_body(&self, id: i64, body: &str, now_ms: i64) -> Result<bool> {
        let n = self.conn().execute(
            "UPDATE notes SET body = ?2, updated_at_ms = ?3 WHERE id = ?1",
            rusqlite::params![id, body, now_ms],
        )?;
        Ok(n > 0)
    }

    pub fn recent_notes(&self, limit: i64) -> Result<Vec<Note>> {
        let mut stmt = self.conn().prepare(&format!(
            "SELECT {NOTE_COLS} FROM notes ORDER BY updated_at_ms DESC, id DESC LIMIT ?1"
        ))?;
        let rows = stmt.query_map([limit], row_to_note)?;
        rows.collect()
    }

    pub fn all_note_names(&self) -> Result<Vec<String>> {
        let mut stmt = self
            .conn()
            .prepare("SELECT name FROM notes ORDER BY name")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        rows.collect()
    }
}
```

Add to `crates/magpie-core/src/lib.rs` (with the other `pub mod`s and `pub use`s):

```rust
pub mod notes;
pub use notes::Note;
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `env -u RUSTUP_TOOLCHAIN ~/.cargo/bin/cargo test -p magpie-core --test notes`
Expected: PASS (5 tests).

- [ ] **Step 6: Gate + commit**

```bash
env -u RUSTUP_TOOLCHAIN ~/.cargo/bin/cargo clippy -p magpie-core --all-targets -- -D warnings
env -u RUSTUP_TOOLCHAIN ~/.cargo/bin/cargo fmt
git add crates/magpie-core/src/schema.sql crates/magpie-core/src/notes.rs crates/magpie-core/src/lib.rs crates/magpie-core/tests/notes.rs
git commit -m "feat(core): notes table + pages CRUD (daily + named, get-or-create)"
```

---

### Task 2: Capture a note from a clipboard entry (provenance)

**Files:**
- Modify: `crates/magpie-core/src/notes.rs` (add `create_note_from_entry` + private `unique_note_name`)
- Test: `crates/magpie-core/tests/notes.rs` (append)

**Interfaces:**
- Consumes: Task 1's `Note`, `get_note`, `note_by_name`; the existing `entries` table (`full_text`, `source_app_id`) populated via `Store::ingest`.
- Produces: `Store::create_note_from_entry(&self, entry_id: i64, now_ms: i64) -> Result<Note>` — new page whose `body` = the entry's `full_text`, `name` = first non-empty line (≤60 chars, deduped with ` (2)`…), provenance `source_app_id` copied from the entry and `source_entry_id` = `entry_id`.

- [ ] **Step 1: Write the failing test** (append to `tests/notes.rs`):

```rust
use magpie_core::{CaptureEvent, Content, ImageStore};

struct NoopImg;
impl ImageStore for NoopImg {
    fn put(&self, h: &str, _b: &[u8]) -> std::io::Result<String> {
        Ok(h.to_string())
    }
}

#[test]
fn create_note_from_entry_copies_provenance() {
    let s = store();
    let ing = s
        .ingest(
            &CaptureEvent {
                content: Content::Text("Fix the mount\nsecond line".into()),
                source_app: None,
                copied_at_ms: 42,
            },
            &NoopImg,
        )
        .unwrap();
    let note = s.create_note_from_entry(ing.entry_id, 100).unwrap();
    assert_eq!(note.name, "Fix the mount"); // first line
    assert_eq!(note.body, "Fix the mount\nsecond line");
    assert_eq!(note.source_entry_id, Some(ing.entry_id));
    // A second capture of the same entry gets a suffixed unique name.
    let note2 = s.create_note_from_entry(ing.entry_id, 200).unwrap();
    assert_eq!(note2.name, "Fix the mount (2)");
}
```

> NOTE: `Ingested`'s field is `entry_id` (from `magpie_core::Ingested`). If the field name differs, read `crates/magpie-core/src/store.rs` for the actual name and adjust the test + doc — do not guess.

- [ ] **Step 2: Run test to verify it fails**

Run: `env -u RUSTUP_TOOLCHAIN ~/.cargo/bin/cargo test -p magpie-core --test notes create_note_from_entry`
Expected: FAIL — method missing.

- [ ] **Step 3: Implement** (append inside `impl Store` in `notes.rs`):

```rust
    fn unique_note_name(&self, base: &str) -> Result<String> {
        if self.note_by_name(base)?.is_none() {
            return Ok(base.to_string());
        }
        for i in 2..10_000 {
            let cand = format!("{base} ({i})");
            if self.note_by_name(&cand)?.is_none() {
                return Ok(cand);
            }
        }
        Ok(format!("{base} (dup)"))
    }

    pub fn create_note_from_entry(&self, entry_id: i64, now_ms: i64) -> Result<Note> {
        let (full_text, app_id): (String, Option<i64>) = self.conn().query_row(
            "SELECT full_text, source_app_id FROM entries WHERE id = ?1",
            [entry_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let first = full_text.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
        let base: String = if first.is_empty() {
            format!("Note {now_ms}")
        } else {
            first.chars().take(60).collect()
        };
        let name = self.unique_note_name(&base)?;
        self.conn().execute(
            "INSERT INTO notes
               (name, is_daily, body, created_at_ms, updated_at_ms, source_app_id, source_entry_id)
             VALUES (?1, 0, ?2, ?3, ?3, ?4, ?5)",
            rusqlite::params![name, full_text, now_ms, app_id, entry_id],
        )?;
        let id = self.conn().last_insert_rowid();
        Ok(self.get_note(id)?.expect("row exists after INSERT"))
    }
```

- [ ] **Step 4: Run test to verify it passes**

Run: `env -u RUSTUP_TOOLCHAIN ~/.cargo/bin/cargo test -p magpie-core --test notes`
Expected: PASS (all notes tests).

- [ ] **Step 5: Gate + commit**

```bash
env -u RUSTUP_TOOLCHAIN ~/.cargo/bin/cargo clippy -p magpie-core --all-targets -- -D warnings
env -u RUSTUP_TOOLCHAIN ~/.cargo/bin/cargo fmt
git add crates/magpie-core/src/notes.rs crates/magpie-core/tests/notes.rs
git commit -m "feat(core): create_note_from_entry — capture a note from a clip with provenance"
```

---

### Task 3: App pure helpers — `wiki_links` + `note_list_title`

**Files:**
- Create: `crates/magpie-app/src/notes_view.rs`
- Modify: `crates/magpie-app/src/lib.rs` (`pub mod notes_view;`)

**Interfaces:**
- Produces:
  - `pub fn wiki_links(body: &str) -> Vec<String>` — `[[Name]]` refs, trimmed, deduped, first-seen order; empty `[[]]` skipped.
  - `pub fn note_list_title(note_name: &str, body: &str) -> String` — the page name, unless it's empty or an auto `Note <n>` capture name, in which case the first non-empty body line (≤60 chars).

- [ ] **Step 1: Write the failing tests** in `crates/magpie-app/src/notes_view.rs` (module + tests together):

```rust
// (implementation added in Step 3)

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wiki_links_extracts_dedups_trims() {
        let b = "see [[Prod Incidents]] and [[ enhancer ]] and [[Prod Incidents]] and [[]]";
        assert_eq!(
            wiki_links(b),
            vec!["Prod Incidents".to_string(), "enhancer".to_string()]
        );
    }

    #[test]
    fn wiki_links_handles_unclosed() {
        assert_eq!(wiki_links("a [[open only"), Vec::<String>::new());
        assert_eq!(wiki_links("none here"), Vec::<String>::new());
    }

    #[test]
    fn title_prefers_name_else_first_line() {
        assert_eq!(note_list_title("Prod Incidents", "body"), "Prod Incidents");
        assert_eq!(note_list_title("Note 1725270000000", "\n  Fix mount\nmore"), "Fix mount");
        assert_eq!(note_list_title("", "  first\nsecond"), "first");
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `env -u RUSTUP_TOOLCHAIN ~/.cargo/bin/cargo test -p magpie-app --lib notes_view`
Expected: FAIL — functions not defined.

- [ ] **Step 3: Implement** (prepend above the `#[cfg(test)]` module in `notes_view.rs`):

```rust
//! Pure view-model helpers for notes: link extraction and list titles.

/// Extract `[[Page Name]]` references: trimmed, deduped, in first-seen order.
/// Empty `[[]]` and unclosed `[[` are skipped.
pub fn wiki_links(body: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut rest = body;
    while let Some(start) = rest.find("[[") {
        let after = &rest[start + 2..];
        match after.find("]]") {
            Some(end) => {
                let name = after[..end].trim();
                if !name.is_empty() && !out.iter().any(|n| n == name) {
                    out.push(name.to_string());
                }
                rest = &after[end + 2..];
            }
            None => break,
        }
    }
    out
}

/// A human title for the notes list: the page name, unless it's empty or an
/// auto-generated `Note <n>` capture name — then the first non-empty body line.
pub fn note_list_title(note_name: &str, body: &str) -> String {
    let name = note_name.trim();
    let auto = name.is_empty() || name.starts_with("Note ");
    if !auto {
        return name.to_string();
    }
    let line = body
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or(name);
    line.chars().take(60).collect()
}
```

Add to `crates/magpie-app/src/lib.rs` (with the other `pub mod`s):

```rust
pub mod notes_view;
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `env -u RUSTUP_TOOLCHAIN ~/.cargo/bin/cargo test -p magpie-app --lib notes_view`
Expected: PASS (3 tests).

- [ ] **Step 5: Gate + commit**

```bash
env -u RUSTUP_TOOLCHAIN ~/.cargo/bin/cargo clippy -p magpie-app --all-targets -- -D warnings
env -u RUSTUP_TOOLCHAIN ~/.cargo/bin/cargo fmt
git add crates/magpie-app/src/notes_view.rs crates/magpie-app/src/lib.rs
git commit -m "feat(app): notes_view pure helpers (wiki_links, note_list_title)"
```

---

### Task 4: Notes mode scaffold — struct, props, mode toggle, list

**Files:**
- Modify: `crates/magpie-app/ui/launcher.slint` (add `NoteRow` struct; notes props/callbacks; a `mode-notes` view state; a "Notes" tab in the top bar; the notes list + editor-pane skeleton)
- Modify: `crates/magpie-app/src/runtime.rs` (notes state + `refresh_notes`, wire `set-mode-notes`, `open-note`; land on today's daily note)

**Interfaces:**
- Consumes: `Store::daily_note`, `recent_notes`, `get_note` (Task 1); `magpie_app::notes_view::{note_list_title}` (Task 3); `magpie_app::format_time` for the ISO day + relative time.
- Produces (Slint, consumed by Task 5/6):
  - `struct NoteRow { id: int, title: string, when: string, is_daily: bool }`
  - `in property <[NoteRow]> notes;` `in-out property <int> note-id: -1;` (currently open note id)
  - `in property <string> note-body;` `in property <string> note-provenance;` `in property <[string]> note-links;`
  - `in-out property <string> note-filter;` `in-out property <bool> notes-mode: false;`
  - callbacks: `set-mode-notes(bool)`, `open-note(int)`, `new-note()`, `open-page(string)`, `edit-note-body(string)`, `note-from-entry(int)`
- Produces (Rust): `fn refresh_notes(ui: &LauncherWindow, state: &AppState)` — sets `notes`, and if `note-id < 0` opens today's daily note.

- [ ] **Step 1: Add the Slint struct + properties.** In `launcher.slint`, near the other `struct` definitions add:

```slint
struct NoteRow {
    id: int,
    title: string,
    when: string,
    is_daily: bool,
}
```

In `LauncherWindow`, add the properties/callbacks (near the existing preview/clipboard props):

```slint
    in property <[NoteRow]> notes;
    in-out property <int> note-id: -1;
    in property <string> note-body;
    in property <string> note-provenance;
    in property <[string]> note-links;
    in-out property <string> note-filter;
    in-out property <bool> notes-mode: false;
    callback set-mode-notes(bool);
    callback open-note(int);
    callback new-note();
    callback open-page(string);
    callback edit-note-body(string);
    callback note-from-entry(int);
```

- [ ] **Step 2: Add the "Notes" tab + Notes layout.** In the top tab bar (where `Stats` lives) add a clickable "Notes" label calling `root.set-mode-notes(!root.notes-mode)`. Add a top-level `if root.notes-mode: Rectangle { ... }` overlay covering the clipboard body with a two-pane layout:

```slint
if root.notes-mode: Rectangle {
    background: #1e1e21;
    HorizontalLayout {
        padding: 8px; spacing: 8px;
        // Left: filter + new + list
        VerticalLayout {
            width: 300px; spacing: 6px;
            HorizontalLayout {
                spacing: 6px;
                Rectangle {
                    height: 30px; background: #2a2a30; border-radius: 6px;
                    TextInput {
                        text <=> root.note-filter; color: #d0d0d4; font-size: 13px;
                        vertical-alignment: center;
                    }
                }
                Rectangle {
                    width: 60px; height: 30px; border-radius: 6px;
                    background: nnew.has-hover ? #34343c : #2a2a30;
                    nnew := TouchArea { clicked => { root.new-note(); } }
                    Text { text: "New"; color: #d0d0d4; font-size: 12px; }
                }
            }
            ListView {
                for n in root.notes: Rectangle {
                    height: 44px;
                    background: n.id == root.note-id ? #2f2f37 : (nrow.has-hover ? #26262c : transparent);
                    border-radius: 6px;
                    nrow := TouchArea { clicked => { root.open-note(n.id); } }
                    VerticalLayout {
                        padding-left: 8px; padding-right: 8px;
                        Text { text: (n.is_daily ? "📌 " : "") + n.title; color: #e6e6ea; font-size: 13px; overflow: elide; }
                        Text { text: n.when; color: #8a8a90; font-size: 11px; }
                    }
                }
            }
        }
        // Right: editor pane (body filled in Task 5)
        Rectangle {
            background: #161618; border-radius: 8px;
            VerticalLayout { padding: 12px; spacing: 8px;
                Text { text: root.note-provenance; color: #8a8a90; font-size: 11px; }
                // editor + links strip added in Task 5
            }
        }
    }
}
```

- [ ] **Step 3: Wire the Rust callbacks + refresh_notes** in `runtime.rs`. Add a helper and callbacks (inside `start()` where the other `ui.on_*` are registered):

```rust
fn refresh_notes(ui: &LauncherWindow, state: &AppState) {
    let now = now_ms();
    let day = magpie_app::format_time::abs_date(now); // "YYYY-MM-DD"
    let (rows, open_id, body, prov, links) = {
        let store = match state.store.lock() {
            Ok(s) => s,
            Err(e) => e.into_inner(),
        };
        // Ensure today's daily note exists.
        let daily = store.daily_note(&day, now).ok();
        let recent = store.recent_notes(200).unwrap_or_default();
        let rows: Vec<NoteRow> = recent
            .iter()
            .map(|n| NoteRow {
                id: n.id as i32,
                title: SharedString::from(magpie_app::notes_view::note_list_title(&n.name, &n.body)),
                when: SharedString::from(relative_time(n.updated_at_ms, now)),
                is_daily: n.is_daily,
            })
            .collect();
        // Which note is open? Keep the current one if still present, else the daily note.
        let cur = ui.get_note_id();
        let open = if cur >= 0 && recent.iter().any(|n| n.id as i32 == cur) {
            cur
        } else {
            daily.as_ref().map(|d| d.id as i32).unwrap_or(-1)
        };
        let opened = recent.iter().find(|n| n.id as i32 == open);
        let (body, prov, links) = match opened {
            Some(n) => (
                n.body.clone(),
                note_provenance(n),
                magpie_app::notes_view::wiki_links(&n.body),
            ),
            None => (String::new(), String::new(), Vec::new()),
        };
        (rows, open, body, prov, links)
    };
    ui.set_notes(ModelRc::new(VecModel::from(rows)));
    ui.set_note_id(open_id);
    ui.set_note_body(SharedString::from(body));
    ui.set_note_provenance(SharedString::from(prov));
    ui.set_note_links(ModelRc::new(VecModel::from(
        links.into_iter().map(SharedString::from).collect::<Vec<_>>(),
    )));
}

// Provenance line. Phase 1 keeps it simple: whether the note was captured from a
// clip or authored in Magpie, plus the creation date. (Enriching "captured" with the
// exact source app name — via an app-id→name lookup — is a later refinement.)
fn note_provenance(n: &magpie_core::Note) -> String {
    let when = abs_date(n.created_at_ms);
    if n.source_entry_id.is_some() {
        format!("captured · {when}")
    } else {
        format!("created in Magpie · {when}")
    }
}
```

Register callbacks:

```rust
{
    let s = state.clone(); let w = ui.as_weak();
    ui.on_set_mode_notes(move |on| {
        if let Some(ui) = w.upgrade() {
            ui.set_notes_mode(on);
            if on { refresh_notes(&ui, &s); }
        }
    });
}
{
    let s = state.clone(); let w = ui.as_weak();
    ui.on_open_note(move |id| {
        if let Some(ui) = w.upgrade() {
            ui.set_note_id(id);
            refresh_notes(&ui, &s);
        }
    });
}
```

- [ ] **Step 4: Build + launch-verify**

```bash
env -u RUSTUP_TOOLCHAIN ~/.cargo/bin/cargo build -p magpie-app 2>&1 | tail -2
env -u RUSTUP_TOOLCHAIN ~/.cargo/bin/cargo clippy --all-targets -- -D warnings 2>&1 | tail -2
env -u RUSTUP_TOOLCHAIN ~/.cargo/bin/cargo fmt --check
timeout 8 env -u RUSTUP_TOOLCHAIN ~/.cargo/bin/cargo run -p magpie-app 2>/tmp/n.log; echo "exit=$? (124=alive)"; grep -iE "panic|error" /tmp/n.log | head
```
Expected: compiles, clippy clean, fmt clean, exit 124 (no panic). Manual (user): click "Notes" → the notes pane appears with today's daily note in the list.

- [ ] **Step 5: Commit**

```bash
git add crates/magpie-app/ui/launcher.slint crates/magpie-app/src/runtime.rs
git commit -m "feat(app): Notes mode scaffold — tab toggle, notes list, daily-note landing"
```

---

### Task 5: Note editor + persistence + links strip

**Files:**
- Modify: `crates/magpie-app/ui/launcher.slint` (editor `TextInput` in the right pane + Links strip)
- Modify: `crates/magpie-app/src/runtime.rs` (`edit-note-body`, `new-note`, `open-page` callbacks)

**Interfaces:**
- Consumes: Task 4's props/callbacks; `Store::update_note_body`, `upsert_note_by_name` (Task 1); `notes_view::wiki_links` (Task 3).
- Produces: editing a note persists to the DB and refreshes the Links strip; clicking a `[[link]]` chip opens/creates that page.

- [ ] **Step 1: Add the editor + links strip** to the right pane in the `if root.notes-mode` block (replace the "editor added in Task 5" comment):

```slint
Flickable {
    viewport-width: self.width;
    viewport-height: ned.preferred-height;
    ned := TextInput {
        width: parent.width;
        text: root.note-body;
        color: #d0d0d4;
        font-size: 13px;
        single-line: false;
        wrap: word-wrap;
        edited => { root.edit-note-body(self.text); }
    }
}
// Links strip: clickable [[page]] chips parsed from the body.
if root.note-links.length > 0: HorizontalLayout {
    spacing: 6px;
    Text { text: "Links:"; color: #8a8a90; font-size: 11px; vertical-alignment: center; }
    for name in root.note-links: Rectangle {
        height: 24px; border-radius: 12px;
        background: chip.has-hover ? #34343c : #2a2a30;
        HorizontalLayout { padding-left: 10px; padding-right: 10px;
            Text { text: name; color: #9ab0ff; font-size: 12px; vertical-alignment: center; }
        }
        chip := TouchArea { clicked => { root.open-page(name); } }
    }
}
```

- [ ] **Step 2: Wire the callbacks** in `runtime.rs`:

```rust
{
    let s = state.clone(); let w = ui.as_weak();
    ui.on_edit_note_body(move |body| {
        if let Some(ui) = w.upgrade() {
            let id = ui.get_note_id();
            if id >= 0 {
                let store = match s.store.lock() { Ok(g) => g, Err(e) => e.into_inner() };
                let _ = store.update_note_body(id as i64, body.as_str(), now_ms());
            }
            // Refresh only the links strip (cheap) — don't rebuild the list on every keystroke.
            ui.set_note_links(ModelRc::new(VecModel::from(
                magpie_app::notes_view::wiki_links(body.as_str())
                    .into_iter().map(SharedString::from).collect::<Vec<_>>(),
            )));
        }
    });
}
{
    let s = state.clone(); let w = ui.as_weak();
    ui.on_open_page(move |name| {
        if let Some(ui) = w.upgrade() {
            let id = {
                let store = match s.store.lock() { Ok(g) => g, Err(e) => e.into_inner() };
                store.upsert_note_by_name(name.as_str(), now_ms()).ok().map(|n| n.id as i32)
            };
            if let Some(id) = id { ui.set_note_id(id); refresh_notes(&ui, &s); }
        }
    });
}
{
    let s = state.clone(); let w = ui.as_weak();
    ui.on_new_note(move || {
        if let Some(ui) = w.upgrade() {
            // Unique "Untitled" page.
            let id = {
                let store = match s.store.lock() { Ok(g) => g, Err(e) => e.into_inner() };
                let mut name = "Untitled".to_string();
                let mut i = 2;
                while store.note_by_name(&name).ok().flatten().is_some() {
                    name = format!("Untitled ({i})"); i += 1;
                }
                store.upsert_note_by_name(&name, now_ms()).ok().map(|n| n.id as i32)
            };
            if let Some(id) = id { ui.set_note_id(id); refresh_notes(&ui, &s); }
        }
    });
}
```

- [ ] **Step 3: Build + launch-verify**

```bash
env -u RUSTUP_TOOLCHAIN ~/.cargo/bin/cargo build -p magpie-app 2>&1 | tail -2
env -u RUSTUP_TOOLCHAIN ~/.cargo/bin/cargo clippy --all-targets -- -D warnings 2>&1 | tail -2
env -u RUSTUP_TOOLCHAIN ~/.cargo/bin/cargo fmt --check
timeout 8 env -u RUSTUP_TOOLCHAIN ~/.cargo/bin/cargo run -p magpie-app 2>/tmp/n.log; echo "exit=$? (124=alive)"; grep -iE "panic|error" /tmp/n.log | head
```
Expected: exit 124, no panic. Manual (user): type in the editor incl. `[[Some Page]]`; a "Links: Some Page" chip appears; clicking it opens that (new) page; text persists after switching notes.

- [ ] **Step 4: Commit**

```bash
git add crates/magpie-app/ui/launcher.slint crates/magpie-app/src/runtime.rs
git commit -m "feat(app): note editor with persistence + clickable [[link]] chips"
```

---

### Task 6: Capture "New note from this entry" (⌘K action + shortcut)

**Files:**
- Modify: `crates/magpie-app/src/runtime.rs` (add a `note` action to the `ACTIONS` list + `run-action` dispatch; wire `note-from-entry`)
- Modify: `crates/magpie-app/ui/launcher.slint` (`run-action` handles `"note"`; a direct `⌘J` shortcut in the search key handler)

**Interfaces:**
- Consumes: `Store::create_note_from_entry` (Task 2); `current_results` (existing); `refresh_notes` + `set-mode-notes` (Task 4).
- Produces: from the clipboard list, an action/shortcut creates a note from the selected entry and switches to Notes mode with it open.

- [ ] **Step 1: Add the ⌘K action.** In `runtime.rs`, add to the `ACTIONS` array:

```rust
    ("note", "🗒", "New note from this entry", "⌘J"),
```

- [ ] **Step 2: Wire `note-from-entry`** in `runtime.rs`:

```rust
{
    let s = state.clone(); let w = ui.as_weak();
    ui.on_note_from_entry(move |idx| {
        if let Some(ui) = w.upgrade() {
            let recent = current_results(&s, now_ms());
            if let Some(entry) = recent.get(idx as usize) {
                let id = {
                    let store = match s.store.lock() { Ok(g) => g, Err(e) => e.into_inner() };
                    store.create_note_from_entry(entry.id, now_ms()).ok().map(|n| n.id as i32)
                };
                if let Some(id) = id {
                    ui.set_note_id(id);
                    ui.set_notes_mode(true);
                    refresh_notes(&ui, &s);
                }
            }
        }
    });
}
```

- [ ] **Step 3: Dispatch from `run-action` + add the shortcut** in `launcher.slint`. In the `run-action(id) =>` handler add:

```slint
        else if id == "note" { root.note-from-entry(root.selected); }
```

In the search `TextInput` `key-pressed` handler, alongside the other `event.modifiers.control` letters, add:

```slint
                                    if event.text == "j" { root.note-from-entry(root.selected); return accept; }
```

- [ ] **Step 4: Build + launch-verify**

```bash
env -u RUSTUP_TOOLCHAIN ~/.cargo/bin/cargo build -p magpie-app 2>&1 | tail -2
env -u RUSTUP_TOOLCHAIN ~/.cargo/bin/cargo clippy --all-targets -- -D warnings 2>&1 | tail -2
env -u RUSTUP_TOOLCHAIN ~/.cargo/bin/cargo fmt --check
env -u RUSTUP_TOOLCHAIN ~/.cargo/bin/cargo test --workspace 2>&1 | grep -E "test result: FAILED|error\[" || echo "no failures"
timeout 8 env -u RUSTUP_TOOLCHAIN ~/.cargo/bin/cargo run -p magpie-app 2>/tmp/n.log; echo "exit=$? (124=alive)"; grep -iE "panic|error" /tmp/n.log | head
```
Expected: full suite green, exit 124. Manual (user): select a clip → ⌘J (or ⌘K → "New note from this entry") → Notes mode opens with a new note whose body is the clip text and provenance shows "captured · <date>".

- [ ] **Step 5: Commit**

```bash
git add crates/magpie-app/src/runtime.rs crates/magpie-app/ui/launcher.slint
git commit -m "feat(app): capture a note from a clipboard entry (⌘J / ⌘K action)"
```

---

## Notes for the implementer

- **Help cheat-sheet:** add a `{ k: "⌘J", d: "New note from this entry" }` row to the Help modal in `launcher.slint` when doing Task 6 (keeps the shortcut discoverable) — fold it into Task 6's commit.
- **`abs_date`/`relative_time`** are in `magpie_app::format_time` and already imported in `runtime.rs`. `abs_date(now_ms)` yields the `YYYY-MM-DD` used as the daily-note name.
- **Lock recovery:** use `match state.store.lock() { Ok(g) => g, Err(e) => e.into_inner() }` (poison-tolerant, matching the app's convention) rather than `.expect()`.
- **Do not** add notes to clipboard retention, the clipboard search box, or masking in this phase.
