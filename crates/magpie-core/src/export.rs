//! Export data to portable formats: notes → a Markdown vault, clipboard → JSONL,
//! and a clean single-file DB snapshot. No serde (core has none) — JSON is
//! hand-escaped.
use crate::store::{Result, Store};
use std::fs;
use std::io::Write;
use std::path::Path;

/// Map an io error into the crate's rusqlite-based `Result`.
fn io_err(e: std::io::Error) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(Box::new(e))
}

/// A safe file stem: replace path/reserved/control chars with '-', trim, fall back
/// to "untitled".
pub fn sanitize_filename(name: &str) -> String {
    let mut out: String = name
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                '-'
            } else {
                c
            }
        })
        .collect();
    out = out.trim().trim_matches('.').trim().to_string();
    if out.is_empty() {
        "untitled".to_string()
    } else {
        out
    }
}

/// Escape a string for a JSON double-quoted value (body only, no surrounding quotes).
pub fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// Write each note to `<dir>/<safe-name>.md`. Collisions get `-<id>` appended.
/// Returns the number of files written.
pub fn export_markdown(store: &Store, dir: &Path) -> Result<usize> {
    fs::create_dir_all(dir).map_err(io_err)?;
    let notes = store.all_notes()?;
    let mut used: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut count = 0usize;
    for n in notes {
        let mut stem = sanitize_filename(&n.name);
        if !used.insert(stem.clone()) {
            stem = format!("{stem}-{}", n.id);
            used.insert(stem.clone());
        }
        let path = dir.join(format!("{stem}.md"));
        let mut f = fs::File::create(&path).map_err(io_err)?;
        f.write_all(n.body.as_bytes()).map_err(io_err)?;
        count += 1;
    }
    Ok(count)
}

/// Write every clipboard entry as one JSON object per line. Returns the count.
pub fn export_clipboard_jsonl(store: &Store, path: &Path) -> Result<usize> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(io_err)?;
    }
    let mut f = fs::File::create(path).map_err(io_err)?;
    let mut stmt = store.conn().prepare(
        "SELECT e.id, e.kind, e.full_text, e.first_copied_at_ms, e.last_copied_at_ms,
                e.copy_count, e.pinned, a.display_name,
                (SELECT group_concat(tag, ',') FROM entry_tags WHERE entry_id = e.id)
         FROM entries e LEFT JOIN apps a ON e.source_app_id = a.id
         ORDER BY e.last_copied_at_ms DESC",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, i64>(3)?,
            r.get::<_, i64>(4)?,
            r.get::<_, i64>(5)?,
            r.get::<_, i64>(6)?,
            r.get::<_, Option<String>>(7)?,
            r.get::<_, Option<String>>(8)?,
        ))
    })?;
    let mut count = 0usize;
    for row in rows {
        let (id, kind, text, first, last, copies, pinned, source, tags) = row?;
        let source_json = match source {
            Some(s) => format!("\"{}\"", json_escape(&s)),
            None => "null".to_string(),
        };
        let tags_json = match tags {
            Some(t) if !t.is_empty() => {
                let items: Vec<String> = t
                    .split(',')
                    .map(|x| format!("\"{}\"", json_escape(x)))
                    .collect();
                format!("[{}]", items.join(","))
            }
            _ => "[]".to_string(),
        };
        let line = format!(
            "{{\"id\":{id},\"kind\":\"{}\",\"text\":\"{}\",\"source\":{source_json},\"tags\":{tags_json},\"first_copied_ms\":{first},\"last_copied_ms\":{last},\"copy_count\":{copies},\"pinned\":{}}}\n",
            json_escape(&kind),
            json_escape(&text),
            pinned != 0
        );
        f.write_all(line.as_bytes()).map_err(io_err)?;
        count += 1;
    }
    Ok(count)
}

/// A clean single-file DB snapshot at `dest_db` (SQLite VACUUM INTO; WAL-safe).
pub fn backup_db(store: &Store, dest_db: &Path) -> Result<()> {
    let dest = dest_db.to_str().ok_or_else(|| {
        io_err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "non-utf8 path",
        ))
    })?;
    store
        .conn()
        .execute("VACUUM INTO ?1", rusqlite::params![dest])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_and_json_escape() {
        assert_eq!(sanitize_filename("a/b:c"), "a-b-c");
        assert_eq!(sanitize_filename("  "), "untitled");
        assert_eq!(json_escape("a\"b\\c\n"), "a\\\"b\\\\c\\n");
    }

    #[test]
    fn export_notes_and_clipboard_to_tmp() {
        let s = crate::open_in_memory().unwrap();
        s.upsert_note_by_name("Ideas", 10).unwrap();
        let n = s.note_by_name("Ideas").unwrap().unwrap();
        s.update_note_body(n.id, "- [ ] ship it\nsee [[Other]]", 20)
            .unwrap();
        let dir = std::env::temp_dir().join(format!("magpie-export-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let count = export_markdown(&s, &dir).unwrap();
        assert!(count >= 1);
        let body = fs::read_to_string(dir.join("Ideas.md")).unwrap();
        assert!(body.contains("[[Other]]"));
        let jsonl = dir.join("clipboard.jsonl");
        let _ = export_clipboard_jsonl(&s, &jsonl).unwrap();
        assert!(fs::metadata(&jsonl).is_ok());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn backup_db_snapshot() {
        let s = crate::open_in_memory().unwrap();
        s.upsert_note_by_name("N", 1).unwrap();
        let dest =
            std::env::temp_dir().join(format!("magpie-backup-{}.sqlite3", std::process::id()));
        let _ = fs::remove_file(&dest);
        backup_db(&s, &dest).unwrap();
        assert!(fs::metadata(&dest).unwrap().len() > 0);
        let _ = fs::remove_file(&dest);
    }
}
