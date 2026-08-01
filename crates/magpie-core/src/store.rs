use crate::detect::{detect_text_kind, Kind};
use crate::metrics::text_metrics;
use crate::model::{content_hash, AppInfo, Content};
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

impl Store {
    fn init(conn: Connection) -> Result<Store> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.execute_batch(SCHEMA)?;
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
}

pub fn open(path: &Path) -> Result<Store> {
    Store::init(Connection::open(path)?)
}

pub fn open_in_memory() -> Result<Store> {
    Store::init(Connection::open_in_memory()?)
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
        let a = AppInfo { identifier: "com.ghostty".into(), display_name: "Ghostty".into(), icon_path: None };
        let id1 = s.upsert_app(&a).unwrap();
        let a2 = AppInfo { identifier: "com.ghostty".into(), display_name: "Ghostty 2".into(), icon_path: Some("/i.png".into()) };
        let id2 = s.upsert_app(&a2).unwrap();
        assert_eq!(id1, id2);
        let name: String = s.conn.query_row(
            "SELECT display_name FROM apps WHERE id=?1", [id1], |r| r.get(0)).unwrap();
        assert_eq!(name, "Ghostty 2");
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
