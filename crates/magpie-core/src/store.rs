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
