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
        Ok(self
            .note_by_name(n)?
            .expect("row exists after INSERT OR IGNORE"))
    }

    pub fn daily_note(&self, day: &str, now_ms: i64) -> Result<Note> {
        self.conn().execute(
            "INSERT OR IGNORE INTO notes (name, is_daily, body, created_at_ms, updated_at_ms)
             VALUES (?1, 1, '', ?2, ?2)",
            rusqlite::params![day, now_ms],
        )?;
        Ok(self
            .note_by_name(day)?
            .expect("row exists after INSERT OR IGNORE"))
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
