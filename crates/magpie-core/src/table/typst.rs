//! Typst `#table(...)` parser (spec §1.5) + Typst export (§3). Literal cells
//! only: a string/content/number literal is read as-is; any other expression
//! (a variable, a function call, `#if`/`#for`) can't be known without running
//! the Typst compiler, so it hard-errors instead of guessing.
//!
//! Typst draws its "literal vs. computed" line at *evaluated syntax*, unlike
//! LaTeX which draws it at *macro call vs. literal escape* — the two parsers
//! are intentionally asymmetric here (see spec §1.5).

use super::{
    build_table, limits, line_of, synthetic_header, Column, DetectedFormat, ParseError, Table,
};
use regex::Regex;
use std::sync::OnceLock;

fn numeric_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^-?\d+(\.\d+)?$").unwrap())
}

fn named_arg_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^([A-Za-z_][A-Za-z0-9_-]*)\s*:\s*(.*)$").unwrap())
}

/// Split `s` on top-level `sep` characters only — depth tracked across
/// `(`/`[`/`{` (any closer decrements, matching-type not enforced, which is
/// fine for span-finding) and paused entirely inside a `"..."` string.
fn split_top_level(s: &str, sep: char) -> Vec<String> {
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut in_string = false;
    let mut cur = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if in_string {
            cur.push(c);
            if c == '\\' {
                if let Some(n) = chars.next() {
                    cur.push(n);
                }
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        match c {
            '"' => {
                in_string = true;
                cur.push(c);
            }
            '(' | '[' | '{' => {
                depth += 1;
                cur.push(c);
            }
            ')' | ']' | '}' => {
                depth -= 1;
                cur.push(c);
            }
            c2 if c2 == sep && depth == 0 => parts.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    parts.push(cur);
    parts
        .into_iter()
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect()
}

/// Find the matching close of the `(...)` argument list for `#table(` at
/// `call_idx`. `(` and `[` both push depth (Typst content args use `[...]`),
/// a `"..."` string pauses matching. Returns `(args_start, args_end)`
/// (`args_end` points at the closing `)`).
fn find_call_span(text: &str, call_idx: usize) -> Result<(usize, usize), ParseError> {
    let open_paren = call_idx + "#table(".len() - 1;
    let mut depth = 0i32;
    let mut in_string = false;
    let mut chars = text[open_paren..].char_indices();
    while let Some((off, c)) = chars.next() {
        if in_string {
            if c == '\\' {
                chars.next();
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        match c {
            '"' => in_string = true,
            '(' | '[' => depth += 1,
            ')' | ']' => {
                depth -= 1;
                if depth == 0 {
                    return Ok((open_paren + 1, open_paren + off));
                }
            }
            _ => {}
        }
    }
    Err(ParseError::Malformed {
        format: DetectedFormat::Typst,
        reason: "unbalanced delimiters".to_string(),
        at_line: line_of(text, open_paren),
    })
}

fn split_named_arg(tok: &str) -> Option<(String, String)> {
    let t = tok.trim();
    if t.starts_with('"') || t.starts_with('[') {
        return None;
    }
    named_arg_re()
        .captures(t)
        .map(|c| (c[1].to_string(), c[2].trim().to_string()))
}

fn extract_call_args(tok: &str, _prefix: &str) -> String {
    let t = tok.trim();
    let start = t.find('(');
    let end = t.rfind(')');
    match (start, end) {
        (Some(s), Some(e)) if s < e => t[s + 1..e].to_string(),
        _ => String::new(),
    }
}

fn unescape_typst_string(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('"') => out.push('"'),
                Some('\\') => out.push('\\'),
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn unescape_typst_content(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('[') => out.push('['),
                Some(']') => out.push(']'),
                Some('\\') => out.push('\\'),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out.trim().to_string()
}

/// Classify one positional argument as a literal cell, or hard-error.
fn classify_cell(tok: &str, input: &str, approx_offset: usize) -> Result<String, ParseError> {
    let t = tok.trim();
    if t.len() >= 2 && t.starts_with('"') && t.ends_with('"') {
        return Ok(unescape_typst_string(&t[1..t.len() - 1]));
    }
    if t.len() >= 2 && t.starts_with('[') && t.ends_with(']') {
        return Ok(unescape_typst_content(&t[1..t.len() - 1]));
    }
    if numeric_re().is_match(t) {
        return Ok(t.to_string());
    }
    if t.starts_with("table.cell(") && t.ends_with(')') {
        let inner = extract_call_args(t, "table.cell");
        let inner_tokens = split_top_level(&inner, ',');
        let mut span_bad = false;
        let mut content_tok: Option<String> = None;
        for it in &inner_tokens {
            if let Some((name, value)) = split_named_arg(it) {
                if (name == "colspan" || name == "rowspan") && value.trim() != "1" {
                    span_bad = true;
                }
            } else if content_tok.is_none() {
                content_tok = Some(it.clone());
            }
        }
        if span_bad {
            return Err(ParseError::UnsupportedConstruct {
                format: DetectedFormat::Typst,
                what: "colspan/rowspan".to_string(),
                at_line: line_of(input, approx_offset),
            });
        }
        return match content_tok {
            Some(inner_cell) => classify_cell(&inner_cell, input, approx_offset),
            None => Err(ParseError::UnsupportedConstruct {
                format: DetectedFormat::Typst,
                what: "computed cell (non-literal expression)".to_string(),
                at_line: line_of(input, approx_offset),
            }),
        };
    }
    Err(ParseError::UnsupportedConstruct {
        format: DetectedFormat::Typst,
        what: "computed cell (non-literal expression)".to_string(),
        at_line: line_of(input, approx_offset),
    })
}

fn parse_columns_value(v: &str) -> Option<usize> {
    let t = v.trim();
    if let Ok(n) = t.parse::<usize>() {
        return Some(n);
    }
    if t.starts_with('(') && t.ends_with(')') && t.len() >= 2 {
        let inner = &t[1..t.len() - 1];
        let parts = split_top_level(inner, ',');
        if !parts.is_empty() {
            return Some(parts.len());
        }
    }
    None
}

/// Parse a `#table(...)` call (spec §1.5). `columns:` (literal int or array of
/// lengths) sets the column count when present; otherwise a `table.header`
/// row's length is used, or parsing fails with `Malformed`.
pub(crate) fn parse_typst_table(input: &str) -> Result<Table, ParseError> {
    let call_idx = input
        .find("#table(")
        .ok_or(ParseError::NoTabularStructure {
            format: DetectedFormat::Typst,
        })?;
    let (args_start, args_end) = find_call_span(input, call_idx)?;
    let args_text = &input[args_start..args_end];

    if args_text.contains("#table(") {
        return Err(ParseError::UnsupportedConstruct {
            format: DetectedFormat::Typst,
            what: "nested table".to_string(),
            at_line: line_of(input, args_start),
        });
    }

    let tokens = split_top_level(args_text, ',');

    let mut columns_named: Option<String> = None;
    let mut header_cells: Option<Vec<String>> = None;
    let mut positional: Vec<String> = Vec::new();

    for tok in tokens {
        if let Some((name, value)) = split_named_arg(&tok) {
            if name == "columns" {
                columns_named = Some(value);
            }
            continue;
        }
        if tok.starts_with("table.header(") && tok.ends_with(')') {
            let inner = extract_call_args(&tok, "table.header");
            let mut cells = Vec::new();
            for ct in split_top_level(&inner, ',') {
                cells.push(classify_cell(&ct, input, args_start)?);
            }
            header_cells = Some(cells);
            continue;
        }
        positional.push(tok);
    }

    let mut body_cells = Vec::with_capacity(positional.len());
    for tok in &positional {
        body_cells.push(classify_cell(tok, input, args_start)?);
    }

    let columns_count = columns_named
        .as_deref()
        .and_then(parse_columns_value)
        .or_else(|| header_cells.as_ref().map(|h| h.len()))
        .ok_or_else(|| ParseError::Malformed {
            format: DetectedFormat::Typst,
            reason: "could not determine the column count".to_string(),
            at_line: line_of(input, args_start),
        })?;

    if columns_count == 0 || body_cells.len() % columns_count != 0 {
        return Err(ParseError::Malformed {
            format: DetectedFormat::Typst,
            reason: "cell count is not divisible by the column count".to_string(),
            at_line: line_of(input, args_start),
        });
    }

    let mut rows: Vec<Vec<String>> = body_cells
        .chunks(columns_count)
        .map(|c| c.to_vec())
        .collect();

    let (header, mut data_rows) = if let Some(mut h) = header_cells {
        if h.len() < columns_count {
            h.resize(columns_count, String::new());
        } else {
            h.truncate(columns_count);
        }
        (h, rows)
    } else if !rows.is_empty() {
        let first = rows[0].clone();
        let rest = &rows[1..];
        // Typst tables are equally common headerless, so an undetermined
        // heuristic still defaults to "header present" (same default as CSV).
        let header_present = super::looks_like_header(&first, rest).unwrap_or(true);
        if header_present {
            let header = rows.remove(0);
            (header, rows)
        } else {
            (synthetic_header(columns_count), rows)
        }
    } else {
        (synthetic_header(columns_count), rows)
    };

    let truncated = data_rows.len() > limits::MAX_ROWS;
    if truncated {
        data_rows.truncate(limits::MAX_ROWS);
    }

    Ok(build_table(
        header,
        data_rows,
        DetectedFormat::Typst,
        Vec::new(),
        truncated,
    ))
}

fn escape_typst_content(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('[', "\\[")
        .replace(']', "\\]")
}

/// Emits `#table(columns: N, [header1], …, [cell], …)` with bracket-content
/// cells (spec §3) — no `stroke:`/`align:` styling.
pub(crate) fn export_typst_table(columns: &[Column]) -> String {
    let n = columns.len();
    let mut cells: Vec<String> = columns
        .iter()
        .map(|c| format!("[{}]", escape_typst_content(&c.name)))
        .collect();
    let row_count = columns.first().map(|c| c.cells.len()).unwrap_or(0);
    for r in 0..row_count {
        for c in columns {
            cells.push(format!("[{}]", escape_typst_content(&c.cells[r])));
        }
    }
    format!("#table(\n  columns: {n},\n  {}\n)\n", cells.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::table::ColumnKind;

    const BENCH: &str = "#table(\n  columns: 3,\n  [name], [mass], [radius],\n  [Mercury], [0.330], [2439],\n  [Venus], [4.87], [6052],\n)\n";

    #[test]
    fn parses_basic_table_with_literal_columns() {
        let t = parse_typst_table(BENCH).unwrap();
        assert_eq!(t.columns.len(), 3);
        assert_eq!(t.columns[0].name, "name");
        assert_eq!(t.row_count, 2);
        assert_eq!(t.columns[1].kind, ColumnKind::Float);
    }

    #[test]
    fn table_header_call_is_used_as_header() {
        let src = "#table(\n  columns: 2,\n  table.header([a], [b]),\n  [1], [2],\n)\n";
        let t = parse_typst_table(src).unwrap();
        assert_eq!(t.columns[0].name, "a");
        assert_eq!(t.row_count, 1);
    }

    #[test]
    fn columns_as_tuple_of_lengths() {
        let src = "#table(\n  columns: (1fr, 1fr, 1fr),\n  [a], [b], [c],\n  [1], [2], [3],\n)\n";
        let t = parse_typst_table(src).unwrap();
        assert_eq!(t.columns.len(), 3);
    }

    #[test]
    fn colspan_greater_than_one_is_a_hard_error() {
        let src =
            "#table(\n  columns: 2,\n  [a], [b],\n  table.cell(colspan: 2, [merged]), [x],\n)\n";
        let err = parse_typst_table(src).unwrap_err();
        assert!(
            matches!(err, ParseError::UnsupportedConstruct { what, .. } if what == "colspan/rowspan")
        );
    }

    #[test]
    fn computed_cell_is_a_hard_error() {
        let src = "#table(\n  columns: 2,\n  [a], [b],\n  compute_value(), [x],\n)\n";
        let err = parse_typst_table(src).unwrap_err();
        assert!(
            matches!(err, ParseError::UnsupportedConstruct { what, .. } if what == "computed cell (non-literal expression)")
        );
    }

    #[test]
    fn nested_table_is_a_hard_error() {
        let src = "#table(columns: 1, [ #table(columns: 1, [x]) ])\n";
        let err = parse_typst_table(src).unwrap_err();
        assert!(
            matches!(err, ParseError::UnsupportedConstruct { what, .. } if what == "nested table")
        );
    }

    #[test]
    fn no_table_call_errors() {
        let err = parse_typst_table("just some text").unwrap_err();
        assert!(matches!(err, ParseError::NoTabularStructure { .. }));
    }

    #[test]
    fn export_then_reimport_round_trips() {
        let columns = vec![
            Column {
                name: "a".into(),
                kind: ColumnKind::Text,
                cells: vec!["x".into(), "y".into()],
            },
            Column {
                name: "n".into(),
                kind: ColumnKind::Integer,
                cells: vec!["1".into(), "2".into()],
            },
        ];
        let out = export_typst_table(&columns);
        let reparsed = parse_typst_table(&out).unwrap();
        assert_eq!(reparsed.columns[0].name, "a");
        assert_eq!(reparsed.row_count, 2);
    }
}
