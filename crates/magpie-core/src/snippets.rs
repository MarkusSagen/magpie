//! Snippets & commands (Phase 16a). Named text with an optional keyword and
//! placeholders (`{date}`, `{time}`, `{clipboard}`, `{cursor}`, `{input:…}`,
//! `{snippet:…}`); commands are shell snippets with `{{arg}}` params that are
//! pasted, never executed. Expansion and import are pure and tested; the store
//! methods are thin CRUD over the `snippets` table (migration v5).

use crate::store::Store;
use chrono::{FixedOffset, TimeZone, Utc};
use rusqlite::{params, OptionalExtension, Result};
use std::collections::HashMap;

/// A stored snippet or command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snippet {
    pub id: i64,
    pub name: String,
    pub keyword: String,
    pub body: String,
    /// `snippet` or `command`.
    pub kind: String,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

impl Snippet {
    pub fn is_command(&self) -> bool {
        self.kind == "command"
    }
}

/// Context the pure expander reads (never touches the OS itself).
pub struct ExpandCtx<'a> {
    pub epoch_secs: i64,
    pub utc_offset_secs: i32,
    pub clipboard: &'a str,
    pub inputs: &'a HashMap<String, String>,
}

/// The result of expanding a snippet body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Expanded {
    pub text: String,
    /// Byte offset of `{cursor}` in `text`, if present (where to place the caret).
    pub cursor: Option<usize>,
}

fn fmt_datetime(ctx: &ExpandCtx, fmt: &str) -> String {
    let offset =
        FixedOffset::east_opt(ctx.utc_offset_secs).unwrap_or(FixedOffset::east_opt(0).unwrap());
    let dt = Utc
        .timestamp_opt(ctx.epoch_secs, 0)
        .single()
        .unwrap_or_default()
        .with_timezone(&offset);
    dt.format(fmt).to_string()
}

/// Expand `body` against `ctx`. `{snippet:name}` is resolved via `lookup` (the
/// named snippet's body), recursively, up to a small depth. `{{arg}}` command
/// params are left verbatim.
pub fn expand<F>(body: &str, ctx: &ExpandCtx, lookup: &F) -> Expanded
where
    F: Fn(&str) -> Option<String>,
{
    expand_depth(body, ctx, lookup, 0)
}

fn expand_depth<F>(body: &str, ctx: &ExpandCtx, lookup: &F, depth: usize) -> Expanded
where
    F: Fn(&str) -> Option<String>,
{
    let chars: Vec<char> = body.chars().collect();
    let mut out = String::new();
    let mut cursor = None;
    let mut i = 0;
    while i < chars.len() {
        // Command arg `{{…}}` — copy verbatim (pasted, never executed).
        if chars[i] == '{' && i + 1 < chars.len() && chars[i + 1] == '{' {
            if let Some(end) = find_seq(&chars, i + 2, "}}") {
                for &c in &chars[i..end + 2] {
                    out.push(c);
                }
                i = end + 2;
                continue;
            }
        }
        // Placeholder `{…}`.
        if chars[i] == '{' {
            if let Some(end) = chars[i + 1..].iter().position(|&c| c == '}') {
                let token: String = chars[i + 1..i + 1 + end].iter().collect();
                let (head, arg) = match token.split_once(':') {
                    Some((h, a)) => (h.trim(), Some(a)),
                    None => (token.trim(), None),
                };
                let replacement: Option<String> = match head {
                    "cursor" => {
                        cursor = Some(out.len());
                        Some(String::new())
                    }
                    "date" => Some(fmt_datetime(ctx, arg.unwrap_or("%Y-%m-%d"))),
                    "time" => Some(fmt_datetime(ctx, arg.unwrap_or("%H:%M"))),
                    "clipboard" => Some(ctx.clipboard.to_string()),
                    "input" => {
                        let label = arg.unwrap_or("").trim();
                        Some(ctx.inputs.get(label).cloned().unwrap_or_default())
                    }
                    "snippet" => {
                        let name = arg.unwrap_or("").trim();
                        if depth < 8 {
                            lookup(name).map(|b| expand_depth(&b, ctx, lookup, depth + 1).text)
                        } else {
                            Some(String::new())
                        }
                    }
                    _ => None, // unknown token → leave literal
                };
                if let Some(rep) = replacement {
                    out.push_str(&rep);
                    i = i + 1 + end + 1;
                    continue;
                }
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    Expanded { text: out, cursor }
}

fn find_seq(chars: &[char], from: usize, seq: &str) -> Option<usize> {
    let s: Vec<char> = seq.chars().collect();
    let mut i = from;
    while i + s.len() <= chars.len() {
        if chars[i..i + s.len()] == s[..] {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// The `{input:Label}` labels a body prompts for, in order, de-duplicated.
pub fn required_inputs(body: &str) -> Vec<String> {
    let chars: Vec<char> = body.chars().collect();
    let mut labels = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '{' && !(i + 1 < chars.len() && chars[i + 1] == '{') {
            if let Some(end) = chars[i + 1..].iter().position(|&c| c == '}') {
                let token: String = chars[i + 1..i + 1 + end].iter().collect();
                if let Some(label) = token.trim().strip_prefix("input:") {
                    let label = label.trim().to_string();
                    if !labels.contains(&label) {
                        labels.push(label);
                    }
                }
                i = i + 1 + end + 1;
                continue;
            }
        }
        i += 1;
    }
    labels
}

// ---------------------------------------------------------------------------
// Import
// ---------------------------------------------------------------------------

/// Parse espanso `matches:` YAML into snippets (`trigger`→keyword, `replace`→body).
/// A tiny hand-rolled parser for the common shape — no serde_yaml dependency.
pub fn parse_espanso_yaml(text: &str) -> Vec<Snippet> {
    let mut out = Vec::new();
    let mut trigger = String::new();
    let mut replace = String::new();
    let mut in_replace_block = false;
    let mut block_indent = 0usize;
    let flush = |trigger: &mut String, replace: &mut String, out: &mut Vec<Snippet>| {
        if !trigger.is_empty() || !replace.is_empty() {
            let name = if !trigger.is_empty() {
                trigger.trim_matches(&['"', '\''][..]).to_string()
            } else {
                "Imported".to_string()
            };
            out.push(Snippet {
                id: 0,
                name,
                keyword: trigger.trim_matches(&['"', '\''][..]).to_string(),
                body: replace.trim_end().to_string(),
                kind: "snippet".into(),
                created_at_ms: 0,
                updated_at_ms: 0,
            });
        }
        trigger.clear();
        replace.clear();
    };
    for line in text.lines() {
        let indent = line.len() - line.trim_start().len();
        let t = line.trim_start();
        if in_replace_block {
            if t.is_empty() || indent > block_indent {
                replace.push_str(line.get(block_indent + 2..).unwrap_or(""));
                replace.push('\n');
                continue;
            }
            in_replace_block = false;
        }
        if let Some(rest) = t.strip_prefix("- trigger:") {
            flush(&mut trigger, &mut replace, &mut out);
            trigger = rest.trim().to_string();
        } else if let Some(rest) = t.strip_prefix("trigger:") {
            trigger = rest.trim().to_string();
        } else if let Some(rest) = t.strip_prefix("replace:") {
            let r = rest.trim();
            if r == "|" || r == "|-" || r == ">" {
                in_replace_block = true;
                block_indent = indent;
            } else {
                replace = r.trim_matches(&['"', '\''][..]).to_string();
            }
        }
    }
    flush(&mut trigger, &mut replace, &mut out);
    out
}

/// Parse a navi `.cheat` file into commands (`% tag` header ignored; `# desc`
/// then a command line → one command each).
pub fn parse_navi_cheat(text: &str) -> Vec<Snippet> {
    let mut out = Vec::new();
    let mut desc = String::new();
    for line in text.lines() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('%') || t.starts_with(';') {
            continue;
        }
        if let Some(d) = t.strip_prefix("# ") {
            desc = d.trim().to_string();
            continue;
        }
        if t.starts_with('$') {
            // navi variable definition — skip (kept literal in the command).
            continue;
        }
        // A command line.
        let name = if desc.is_empty() {
            t.chars().take(48).collect()
        } else {
            std::mem::take(&mut desc)
        };
        out.push(Snippet {
            id: 0,
            name,
            keyword: String::new(),
            body: t.to_string(),
            kind: "command".into(),
            created_at_ms: 0,
            updated_at_ms: 0,
        });
    }
    out
}

// ---------------------------------------------------------------------------
// Store CRUD
// ---------------------------------------------------------------------------

fn row_to_snippet(r: &rusqlite::Row) -> Result<Snippet> {
    Ok(Snippet {
        id: r.get(0)?,
        name: r.get(1)?,
        keyword: r.get(2)?,
        body: r.get(3)?,
        kind: r.get(4)?,
        created_at_ms: r.get(5)?,
        updated_at_ms: r.get(6)?,
    })
}

const COLS: &str = "id, name, keyword, body, kind, created_at_ms, updated_at_ms";

impl Store {
    pub fn add_snippet(
        &self,
        name: &str,
        keyword: &str,
        body: &str,
        kind: &str,
        now_ms: i64,
    ) -> Result<i64> {
        self.conn().execute(
            "INSERT INTO snippets (name, keyword, body, kind, created_at_ms, updated_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
            params![name, keyword, body, kind, now_ms],
        )?;
        Ok(self.conn().last_insert_rowid())
    }

    pub fn list_snippets(&self) -> Result<Vec<Snippet>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {COLS} FROM snippets ORDER BY kind, name COLLATE NOCASE"
        ))?;
        let rows = stmt.query_map([], row_to_snippet)?;
        rows.collect()
    }

    pub fn get_snippet(&self, id: i64) -> Result<Option<Snippet>> {
        self.conn()
            .query_row(
                &format!("SELECT {COLS} FROM snippets WHERE id = ?1"),
                [id],
                row_to_snippet,
            )
            .optional()
    }

    pub fn snippet_by_keyword(&self, keyword: &str) -> Result<Option<Snippet>> {
        if keyword.is_empty() {
            return Ok(None);
        }
        self.conn()
            .query_row(
                &format!("SELECT {COLS} FROM snippets WHERE keyword = ?1 LIMIT 1"),
                [keyword],
                row_to_snippet,
            )
            .optional()
    }

    pub fn update_snippet(
        &self,
        id: i64,
        name: &str,
        keyword: &str,
        body: &str,
        kind: &str,
        now_ms: i64,
    ) -> Result<()> {
        self.conn().execute(
            "UPDATE snippets SET name=?2, keyword=?3, body=?4, kind=?5, updated_at_ms=?6 WHERE id=?1",
            params![id, name, keyword, body, kind, now_ms],
        )?;
        Ok(())
    }

    pub fn delete_snippet(&self, id: i64) -> Result<()> {
        self.conn()
            .execute("DELETE FROM snippets WHERE id = ?1", [id])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::open_in_memory;

    fn ctx<'a>(clip: &'a str, inputs: &'a HashMap<String, String>) -> ExpandCtx<'a> {
        ExpandCtx {
            // 2025-01-15 12:00:00 UTC.
            epoch_secs: 1_736_942_400,
            utc_offset_secs: 0,
            clipboard: clip,
            inputs,
        }
    }

    #[test]
    fn expand_date_clipboard_cursor() {
        let inputs = HashMap::new();
        let none = |_: &str| None;
        let e = expand(
            "## {date:%Y-%m-%d} — {clipboard}\n{cursor}end",
            &ctx("hello", &inputs),
            &none,
        );
        assert_eq!(e.text, "## 2025-01-15 — hello\nend");
        assert_eq!(e.cursor, Some("## 2025-01-15 — hello\n".len()));
    }

    #[test]
    fn expand_input_and_nested_snippet() {
        let mut inputs = HashMap::new();
        inputs.insert("Meeting with".to_string(), "Design".to_string());
        let lookup = |name: &str| {
            if name == "sig" {
                Some("Best,\n{input:Meeting with}".to_string())
            } else {
                None
            }
        };
        let e = expand(
            "Hi {input:Meeting with}\n{snippet:sig}",
            &ctx("", &inputs),
            &lookup,
        );
        assert_eq!(e.text, "Hi Design\nBest,\nDesign");
    }

    #[test]
    fn command_args_are_literal() {
        let inputs = HashMap::new();
        let none = |_: &str| None;
        let e = expand(
            "git rebase {{target}} --onto {{base}}",
            &ctx("", &inputs),
            &none,
        );
        assert_eq!(e.text, "git rebase {{target}} --onto {{base}}");
    }

    #[test]
    fn required_inputs_are_ordered_unique() {
        assert_eq!(
            required_inputs("{input:A} {input:B} {input:A}"),
            vec!["A".to_string(), "B".to_string()]
        );
    }

    #[test]
    fn import_espanso_and_navi() {
        let yaml = "matches:\n  - trigger: \";sig\"\n    replace: \"Best,\\nMarkus\"\n  - trigger: \";date\"\n    replace: \"{date}\"\n";
        let s = parse_espanso_yaml(yaml);
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].keyword, ";sig");
        assert_eq!(s[1].body, "{date}");

        let cheat = "% git\n# undo last commit, keep changes\ngit reset --soft HEAD~1\n";
        let c = parse_navi_cheat(cheat);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].kind, "command");
        assert_eq!(c[0].name, "undo last commit, keep changes");
        assert_eq!(c[0].body, "git reset --soft HEAD~1");
    }

    #[test]
    fn store_crud_and_keyword() {
        let s = open_in_memory().unwrap();
        let id = s
            .add_snippet("Sig", ";sig", "Best,\nMarkus", "snippet", 1)
            .unwrap();
        assert_eq!(s.list_snippets().unwrap().len(), 1);
        assert_eq!(s.snippet_by_keyword(";sig").unwrap().unwrap().id, id);
        s.update_snippet(id, "Sig", ";signature", "Best", "snippet", 2)
            .unwrap();
        assert_eq!(s.get_snippet(id).unwrap().unwrap().keyword, ";signature");
        s.delete_snippet(id).unwrap();
        assert!(s.list_snippets().unwrap().is_empty());
    }
}
