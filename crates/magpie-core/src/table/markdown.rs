//! GFM pipe-table parser (spec §1.3) + Markdown export (§3). Inline Markdown
//! (`**bold**`, links, …) is kept as literal source text — v1 does not
//! strip/render it.

use super::{build_table, limits, Column, DetectedFormat, ParseError, Table};
use regex::Regex;
use std::sync::OnceLock;

fn separator_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^\s*\|?\s*:?-+:?\s*(\|\s*:?-+:?\s*)*\|?\s*$").unwrap())
}

fn looks_like_pipe_row(line: &str) -> bool {
    !line.trim().is_empty() && line.contains('|')
}

/// Find a header-row + GFM-separator-row pair anywhere in `input`; returns the
/// header row's `lines()` index. Alignment (`:---`, `---:`, `:---:`) is
/// accepted but not otherwise interpreted (spec: alignment never affects type
/// inference).
pub(crate) fn detect(input: &str) -> Option<usize> {
    let lines: Vec<&str> = input.lines().collect();
    if lines.len() < 2 {
        return None;
    }
    (0..lines.len() - 1)
        .find(|&i| looks_like_pipe_row(lines[i]) && separator_re().is_match(lines[i + 1]))
}

/// Split one pipe row into cells: `\|` is a literal pipe (GFM escape), a
/// leading/trailing `|` doesn't create an extra empty cell, and each cell is
/// trimmed.
fn split_pipe_row(line: &str) -> Vec<String> {
    let t = line.trim();
    let mut cells = Vec::new();
    let mut cur = String::new();
    let mut chars = t.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' && chars.peek() == Some(&'|') {
            cur.push('|');
            chars.next();
        } else if c == '|' {
            cells.push(std::mem::take(&mut cur));
        } else {
            cur.push(c);
        }
    }
    cells.push(cur);
    if t.starts_with('|') && !cells.is_empty() {
        cells.remove(0);
    }
    if t.ends_with('|') && !cells.is_empty() {
        cells.pop();
    }
    cells.into_iter().map(|c| c.trim().to_string()).collect()
}

/// Parse the first GFM pipe table found in `input`. Header is the first pipe
/// row; the alignment row is consumed for structure only, then discarded.
/// Ragged data rows are padded/truncated like CSV (via `build_table`). Rows
/// stop being read at `limits::MAX_ROWS` (spec §4).
pub(crate) fn parse_markdown_table(input: &str) -> Result<Table, ParseError> {
    let lines: Vec<&str> = input.lines().collect();
    let header_idx = detect(input).ok_or(ParseError::NoTabularStructure {
        format: DetectedFormat::Markdown,
    })?;
    let header = split_pipe_row(lines[header_idx]);

    let mut data_rows: Vec<Vec<String>> = Vec::new();
    let mut truncated = false;
    let mut i = header_idx + 2;
    while i < lines.len() {
        let line = lines[i];
        if line.trim().is_empty() {
            break;
        }
        if data_rows.len() >= limits::MAX_ROWS {
            truncated = true;
            break;
        }
        data_rows.push(split_pipe_row(line));
        i += 1;
    }

    Ok(build_table(
        header,
        data_rows,
        DetectedFormat::Markdown,
        Vec::new(),
        truncated,
    ))
}

fn escape_cell(cell: &str) -> String {
    let escaped = cell.replace('|', "\\|");
    if escaped.contains('\n') {
        escaped.replace('\n', "<br>")
    } else {
        escaped
    }
}

fn write_pipe_row(out: &mut String, cells: &[String]) {
    out.push('|');
    for c in cells {
        out.push(' ');
        out.push_str(c);
        out.push_str(" |");
    }
    out.push('\n');
}

/// GFM pipe table with a `---` alignment row (all columns left-aligned — spec
/// §3: original per-column alignment isn't modeled on `Column`, so it's not
/// round-tripped).
pub(crate) fn export_markdown_table(columns: &[Column]) -> String {
    let mut out = String::new();
    let header: Vec<String> = columns.iter().map(|c| escape_cell(&c.name)).collect();
    write_pipe_row(&mut out, &header);
    let sep: Vec<String> = columns.iter().map(|_| "---".to_string()).collect();
    write_pipe_row(&mut out, &sep);
    let row_count = columns.first().map(|c| c.cells.len()).unwrap_or(0);
    for r in 0..row_count {
        let row: Vec<String> = columns.iter().map(|c| escape_cell(&c.cells[r])).collect();
        write_pipe_row(&mut out, &row);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::table::ColumnKind;

    #[test]
    fn parses_a_simple_gfm_table() {
        let md = "| name | age |\n| --- | --- |\n| Ann | 30 |\n| Bob | 40 |\n";
        let t = parse_markdown_table(md).unwrap();
        assert_eq!(t.columns[0].name, "name");
        assert_eq!(t.columns[1].kind, ColumnKind::Integer);
        assert_eq!(t.row_count, 2);
    }

    #[test]
    fn no_leading_trailing_pipes_still_parses() {
        let md = "a | b\n--- | ---\n1 | 2\n";
        let t = parse_markdown_table(md).unwrap();
        assert_eq!(t.columns.len(), 2);
        assert_eq!(t.row_count, 1);
    }

    #[test]
    fn escaped_pipe_is_literal() {
        let md = "| a | b |\n| --- | --- |\n| x\\|y | 2 |\n";
        let t = parse_markdown_table(md).unwrap();
        assert_eq!(t.columns[0].cells[0], "x|y");
    }

    #[test]
    fn ragged_row_is_padded() {
        let md = "| a | b | c |\n| --- | --- | --- |\n| 1 | 2 |\n";
        let t = parse_markdown_table(md).unwrap();
        assert_eq!(t.columns[2].cells, vec![""]);
    }

    #[test]
    fn no_table_found_errors() {
        let err = parse_markdown_table("just some\nplain text\n").unwrap_err();
        assert!(matches!(err, ParseError::NoTabularStructure { .. }));
    }

    #[test]
    fn export_round_trips_and_escapes_pipe() {
        let columns = vec![Column {
            name: "a|b".into(),
            kind: ColumnKind::Text,
            cells: vec!["has|pipe".into()],
        }];
        let out = export_markdown_table(&columns);
        assert!(out.contains("a\\|b"));
        let reparsed = parse_markdown_table(&out).unwrap();
        assert_eq!(reparsed.columns[0].cells[0], "has|pipe");
    }
}
