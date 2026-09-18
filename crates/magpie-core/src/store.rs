use crate::detect::{detect_text_kind, Kind};
use crate::metrics::text_metrics;
use crate::model::{content_hash, AppInfo, CaptureEvent, Content};
use rusqlite::Connection;
use std::path::Path;

pub type Result<T> = std::result::Result<T, rusqlite::Error>;

const SCHEMA: &str = include_str!("schema.sql");

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
            char_count: 0,
            word_count: 0,
            line_count: 0,
            hash,
        }),
        Content::Files(paths) => {
            let full = paths.join("\n");
            let m = text_metrics(&full);
            Ok(Prepared {
                kind: Kind::File,
                preview_text: preview_of(&full),
                byte_size: full.len() as i64,
                char_count: m.char_count,
                word_count: m.word_count,
                line_count: m.line_count,
                full_text: full,
                image_path: None,
                hash,
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
        char_count: m.char_count,
        word_count: m.word_count,
        line_count: m.line_count,
        hash,
    })
}

pub struct Store {
    pub(crate) conn: Connection,
}

/// Ordered, additive schema migrations applied after the baseline SCHEMA. Each entry
/// runs once, in order, bumping `PRAGMA user_version`. NEVER edit or reorder an existing
/// entry (that breaks already-migrated DBs) — only append. The baseline CREATE
/// statements in schema.sql stay frozen; all later schema changes live here.
const MIGRATIONS: &[&str] = &[
    // v1: per-bookmark tags (comma-separated, normalized lowercase) for filtering.
    "ALTER TABLE bookmarks ADD COLUMN tags TEXT NOT NULL DEFAULT '';",
    // v2: per-note baseline hash of the last vault-synced content (for 3-way sync).
    "ALTER TABLE notes ADD COLUMN vault_synced_hash TEXT NOT NULL DEFAULT '';",
];

fn run_migrations(conn: &Connection) -> Result<()> {
    let current: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    for (i, sql) in MIGRATIONS.iter().enumerate() {
        let version = (i as i64) + 1;
        if current < version {
            conn.execute_batch(sql)?;
            // pragma_update can't parametrize user_version; format the literal (it's our own i64).
            conn.execute_batch(&format!("PRAGMA user_version = {version};"))?;
        }
    }
    Ok(())
}

impl Store {
    fn init(conn: Connection) -> Result<Store> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.execute_batch(SCHEMA)?;
        run_migrations(&conn)?;
        Ok(Store { conn })
    }

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

    pub fn ingest(&self, ev: &CaptureEvent, images: &dyn ImageStore) -> Result<Ingested> {
        let p = prepare(&ev.content, images)
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
        let app_id: Option<i64> = match &ev.source_app {
            Some(a) => Some(self.upsert_app(a)?),
            None => None,
        };

        let existing: Option<i64> = self
            .conn
            .query_row(
                "SELECT id FROM entries WHERE content_hash = ?1",
                [&p.hash],
                |r| r.get(0),
            )
            .ok();

        let entry_id = match existing {
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
            None => {
                self.conn.execute(
                    "INSERT INTO entries
                       (content_hash, kind, preview_text, full_text, image_path,
                        byte_size, char_count, word_count, line_count,
                        first_copied_at_ms, last_copied_at_ms, copy_count, pinned, source_app_id)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?10,1,0,?11)",
                    rusqlite::params![
                        p.hash,
                        p.kind.as_str(),
                        p.preview_text,
                        p.full_text,
                        p.image_path,
                        p.byte_size,
                        p.char_count,
                        p.word_count,
                        p.line_count,
                        ev.copied_at_ms,
                        app_id
                    ],
                )?;
                self.conn.last_insert_rowid()
            }
        };

        self.conn.execute(
            "INSERT INTO copy_events (entry_id, copied_at_ms, source_app_id) VALUES (?1, ?2, ?3)",
            rusqlite::params![entry_id, ev.copied_at_ms, app_id],
        )?;

        Ok(Ingested {
            entry_id,
            is_new: existing.is_none(),
        })
    }
}

impl Store {
    pub(crate) fn conn(&self) -> &Connection {
        &self.conn
    }

    pub fn set_pinned(&self, entry_id: i64, pinned: bool) -> Result<()> {
        self.conn.execute(
            "UPDATE entries SET pinned = ?2 WHERE id = ?1",
            rusqlite::params![entry_id, pinned as i64],
        )?;
        Ok(())
    }
}

pub struct Ingested {
    pub entry_id: i64,
    pub is_new: bool,
}

pub fn open(path: &Path) -> Result<Store> {
    Store::init(Connection::open(path)?)
}

pub fn open_in_memory() -> Result<Store> {
    Store::init(Connection::open_in_memory()?)
}

/// Apply the SQLCipher key to a freshly-opened connection. MUST run before any
/// other DB access. Our key is a hex string, so the passphrase form (PBKDF2) is used.
fn apply_key(conn: &Connection, key: &str) -> Result<()> {
    // key is 64 hex chars from our own generator — no quotes to escape, but double
    // any single quote defensively.
    conn.execute_batch(&format!("PRAGMA key = '{}';", key.replace('\'', "''")))
}

/// Open an encrypted DB with `key` (creating a fresh encrypted DB if the file is
/// new). Fails fast (Err) on a wrong key or a non-SQLCipher file.
pub fn open_encrypted(path: &Path, key: &str) -> Result<Store> {
    let conn = Connection::open(path)?;
    apply_key(&conn, key)?;
    // Probe: right key reads sqlite_master; wrong key / plaintext errors here.
    conn.query_row("SELECT count(*) FROM sqlite_master", [], |r| {
        r.get::<_, i64>(0)
    })?;
    Store::init(conn)
}

/// Open an encrypted DB, migrating a legacy plaintext file in place on first run.
/// Backs the plaintext file up (kept) before migrating. Returns an encrypted Store.
pub fn open_or_migrate_encrypted(path: &Path, key: &str) -> Result<Store> {
    if !path.exists() {
        return open_encrypted(path, key);
    }
    match open_encrypted(path, key) {
        Ok(s) => Ok(s),
        Err(_) => {
            migrate_plaintext_to_encrypted(path, key)?;
            open_encrypted(path, key)
        }
    }
}

/// `magpie.sqlite3` + `suffix` (operates on the FULL filename, so "-wal" and
/// ".pre-encrypt-backup" both work).
fn sibling(path: &Path, suffix: &str) -> std::path::PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push(suffix);
    std::path::PathBuf::from(s)
}

/// Convert a plaintext SQLite DB at `path` into a SQLCipher DB encrypted with `key`.
/// Backs up the original to `<path>.pre-encrypt-backup` (kept). Uses SQLCipher's
/// `sqlcipher_export`. Folds any WAL into the main file first, and removes stale
/// WAL/SHM after the swap.
fn migrate_plaintext_to_encrypted(path: &Path, key: &str) -> Result<()> {
    let map_io = |e: std::io::Error| rusqlite::Error::ToSqlConversionFailure(Box::new(e));
    // 1. Safety backup (kept).
    std::fs::copy(path, sibling(path, ".pre-encrypt-backup")).map_err(map_io)?;
    // 2. Export plaintext → a new encrypted file.
    let enc = sibling(path, ".enc-tmp");
    let _ = std::fs::remove_file(&enc);
    {
        let conn = Connection::open(path)?; // no key = plain SQLite under SQLCipher
        conn.pragma_update(None, "journal_mode", "DELETE")?; // fold WAL into the main file
        let enc_str = enc.to_str().ok_or_else(|| {
            map_io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "non-utf8 path",
            ))
        })?;
        // sqlcipher_export() copies schema + data but NOT the page-1 `user_version`
        // header field, so the destination would start at 0 — making run_migrations()
        // re-run already-applied ALTERs on next open (and fail with "duplicate
        // column"). Carry the source's user_version across explicitly.
        let user_version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        conn.execute_batch(&format!(
            "ATTACH DATABASE '{}' AS enc KEY '{}'; \
             SELECT sqlcipher_export('enc'); \
             PRAGMA enc.user_version = {user_version}; \
             DETACH DATABASE enc;",
            enc_str.replace('\'', "''"),
            key.replace('\'', "''"),
        ))?;
    } // conn closes here
      // 3. Swap encrypted file into place; drop stale WAL/SHM of the old plaintext DB.
    std::fs::rename(&enc, path).map_err(map_io)?;
    let _ = std::fs::remove_file(sibling(path, "-wal"));
    let _ = std::fs::remove_file(sibling(path, "-shm"));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let p = prepare(
            &Content::Image {
                bytes: vec![1, 2, 3, 4],
            },
            &FakeImages,
        )
        .unwrap();
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

    #[test]
    fn set_pinned_persists() {
        use crate::model::{CaptureEvent, Content};
        let s = open_in_memory().unwrap();
        let out = s
            .ingest(
                &CaptureEvent {
                    content: Content::Text("keep me".into()),
                    source_app: None,
                    copied_at_ms: 1,
                },
                &FakeImages,
            )
            .unwrap();
        s.set_pinned(out.entry_id, true).unwrap();
        let pinned: i64 = s
            .conn
            .query_row(
                "SELECT pinned FROM entries WHERE id=?1",
                [out.entry_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(pinned, 1);
    }

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

        let (cc, first, last): (i64, i64, i64) = s
            .conn
            .query_row(
                "SELECT copy_count, first_copied_at_ms, last_copied_at_ms FROM entries WHERE id=?1",
                [out.entry_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!((cc, first, last), (1, 1000, 1000));

        let events: i64 = s
            .conn
            .query_row(
                "SELECT count(*) FROM copy_events WHERE entry_id=?1",
                [out.entry_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(events, 1);
    }

    #[test]
    fn ingest_records_source_app() {
        use crate::model::{AppInfo, CaptureEvent, Content};
        let s = open_in_memory().unwrap();
        let ev = CaptureEvent {
            content: Content::Text("x".into()),
            source_app: Some(AppInfo {
                identifier: "com.ghostty".into(),
                display_name: "Ghostty".into(),
                icon_path: None,
            }),
            copied_at_ms: 5,
        };
        let out = s.ingest(&ev, &FakeImages).unwrap();
        let app_id: Option<i64> = s
            .conn
            .query_row(
                "SELECT source_app_id FROM entries WHERE id=?1",
                [out.entry_id],
                |r| r.get(0),
            )
            .unwrap();
        assert!(app_id.is_some());
    }

    #[test]
    fn ingest_duplicate_bumps_count_and_last_time_only() {
        use crate::model::{CaptureEvent, Content};
        let s = open_in_memory().unwrap();
        let mk = |ms| CaptureEvent {
            content: Content::Text("same".into()),
            source_app: None,
            copied_at_ms: ms,
        };

        let a = s.ingest(&mk(100), &FakeImages).unwrap();
        let b = s.ingest(&mk(200), &FakeImages).unwrap();
        assert_eq!(a.entry_id, b.entry_id);
        assert!(a.is_new && !b.is_new);

        let (cc, first, last): (i64, i64, i64) = s
            .conn
            .query_row(
                "SELECT copy_count, first_copied_at_ms, last_copied_at_ms FROM entries WHERE id=?1",
                [a.entry_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!((cc, first, last), (2, 100, 200));

        let events: i64 = s
            .conn
            .query_row(
                "SELECT count(*) FROM copy_events WHERE entry_id=?1",
                [a.entry_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(events, 2);
    }

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
    fn upsert_app_is_idempotent_by_identifier() {
        use crate::model::AppInfo;
        let s = open_in_memory().unwrap();
        let a = AppInfo {
            identifier: "com.ghostty".into(),
            display_name: "Ghostty".into(),
            icon_path: None,
        };
        let id1 = s.upsert_app(&a).unwrap();
        let a2 = AppInfo {
            identifier: "com.ghostty".into(),
            display_name: "Ghostty 2".into(),
            icon_path: Some("/i.png".into()),
        };
        let id2 = s.upsert_app(&a2).unwrap();
        assert_eq!(id1, id2);
        let name: String = s
            .conn
            .query_row("SELECT display_name FROM apps WHERE id=?1", [id1], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(name, "Ghostty 2");
    }

    #[test]
    fn migrations_bump_user_version() {
        let s = open_in_memory().unwrap();
        let version: i64 = s
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert!(version >= 2);
    }

    #[test]
    fn migrations_are_idempotent_on_reopen() {
        // A fresh in-memory DB reaches the latest version, and re-running
        // `run_migrations` against an already-migrated connection must not error
        // (e.g. "duplicate column") or change the version.
        let s = open_in_memory().unwrap();
        let version_before: i64 = s
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert!(version_before >= 2);

        run_migrations(&s.conn).unwrap();

        let version_after: i64 = s
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version_before, version_after);
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
