//! LaTeX `tabular` parser (spec §1.4) + LaTeX export (§3). Literal cells
//! only: a macro call or inline math is kept as raw source text, never
//! evaluated. Hard-stops (not best-effort): `\multicolumn`/`\multirow`,
//! nested `tabular`/`array`/`matrix`, and unbalanced braces inside a cell.

use super::{build_table, limits, line_of, Column, ColumnKind, DetectedFormat, ParseError, Table};
use regex::Regex;
use std::sync::OnceLock;

const BEGIN: &str = "\\begin{tabular}";
const END: &str = "\\end{tabular}";

fn structural_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"\\(hline|toprule|midrule|bottomrule)\b|\\cline\{[^}]*\}").unwrap()
    })
}

fn has_unclosed_enclosing_env(text: &str, before_idx: usize, env: &str) -> bool {
    let begin_tag = format!("\\begin{{{env}}}");
    let end_tag = format!("\\end{{{env}}}");
    let prefix = &text[..before_idx];
    prefix.matches(begin_tag.as_str()).count() > prefix.matches(end_tag.as_str()).count()
}

struct TabularSpan {
    colspec: String,
    body_start: usize,
    body_end: usize,
}

/// Locate the first `\begin{tabular}...\end{tabular}` pair, rejecting nesting
/// (a second `\begin{tabular}` before the matching end, or this tabular
/// sitting inside an unclosed `array`/`matrix`) as `UnsupportedConstruct`.
fn find_tabular(text: &str) -> Result<TabularSpan, ParseError> {
    let begin_idx = text.find(BEGIN).ok_or(ParseError::NoTabularStructure {
        format: DetectedFormat::LaTeX,
    })?;

    for env in ["array", "matrix"] {
        if has_unclosed_enclosing_env(text, begin_idx, env) {
            return Err(ParseError::UnsupportedConstruct {
                format: DetectedFormat::LaTeX,
                what: "nested tabular".to_string(),
                at_line: line_of(text, begin_idx),
            });
        }
    }

    let after_begin = begin_idx + BEGIN.len();
    let ws_end = text[after_begin..]
        .find(|c: char| !c.is_whitespace())
        .map(|o| after_begin + o)
        .unwrap_or(after_begin);

    let (colspec, body_start) = if text[ws_end..].starts_with('{') {
        let mut depth = 0i32;
        let mut close_idx = ws_end;
        for (off, c) in text[ws_end..].char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        close_idx = ws_end + off;
                        break;
                    }
                }
                _ => {}
            }
        }
        (text[ws_end + 1..close_idx].to_string(), close_idx + 1)
    } else {
        (String::new(), after_begin)
    };

    let cursor = body_start;
    let rel_begin = text[cursor..].find(BEGIN);
    let rel_end = text[cursor..].find(END);
    match (rel_begin, rel_end) {
        (Some(b), Some(e)) if b < e => Err(ParseError::UnsupportedConstruct {
            format: DetectedFormat::LaTeX,
            what: "nested tabular".to_string(),
            at_line: line_of(text, cursor + b),
        }),
        (_, Some(e)) => Ok(TabularSpan {
            colspec,
            body_start,
            body_end: cursor + e,
        }),
        (_, None) => Err(ParseError::NoTabularStructure {
            format: DetectedFormat::LaTeX,
        }),
    }
}

fn colspec_column_count(colspec: &str) -> usize {
    colspec
        .chars()
        .filter(|c| matches!(c, 'l' | 'c' | 'r'))
        .count()
}

/// Split the tabular body into row chunks on the literal `\\` row separator,
/// optionally followed by whitespace and a `[len]` spacing argument. Returns
/// `(chunk_text, byte_offset_within_body)` pairs.
fn split_rows(body: &str) -> Vec<(String, usize)> {
    let mut rows = Vec::new();
    let bytes = body.as_bytes();
    let mut start = 0usize;
    let mut i = 0usize;
    while i + 1 < bytes.len() {
        if bytes[i] == b'\\' && bytes[i + 1] == b'\\' {
            rows.push((body[start..i].to_string(), start));
            i += 2;
            while i < bytes.len() && (bytes[i] as char).is_whitespace() {
                i += 1;
            }
            if i < bytes.len() && bytes[i] == b'[' {
                if let Some(close) = body[i..].find(']') {
                    i += close + 1;
                }
            }
            start = i;
        } else {
            i += 1;
        }
    }
    if start < body.len() {
        let trailing = &body[start..];
        if !trailing.trim().is_empty() {
            rows.push((trailing.to_string(), start));
        }
    }
    rows
}

/// Split a cleaned row chunk into cells on unescaped `&` (`\&` is a literal
/// ampersand, kept for the later unescape pass).
fn split_cells(s: &str) -> Vec<String> {
    let mut cells = Vec::new();
    let mut cur = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            cur.push(c);
            if let Some(next) = chars.next() {
                cur.push(next);
            }
        } else if c == '&' {
            cells.push(std::mem::take(&mut cur));
        } else {
            cur.push(c);
        }
    }
    cells.push(cur);
    cells
}

fn brace_depth_balanced(s: &str) -> bool {
    let mut depth = 0i32;
    for c in s.chars() {
        match c {
            '{' => depth += 1,
            '}' => depth -= 1,
            _ => {}
        }
        if depth < 0 {
            return false;
        }
    }
    depth == 0
}

/// Unescape only the common text-mode sequences (spec §1.4); anything else
/// (a macro call, inline math) is left as literal source text.
fn unescape_latex(s: &str) -> String {
    let mut out = s.replace("\\textbackslash", "\u{1}");
    out = out.replace("\\%", "%");
    out = out.replace("\\&", "&");
    out = out.replace("\\_", "_");
    out = out.replace("\\#", "#");
    out = out.replace("\\$", "$");
    out = out.replace('~', " ");
    out = out.replace("---", "—");
    out = out.replace("--", "–");
    out.replace('\u{1}', "\\")
}

/// Parse a LaTeX `tabular` environment (spec §1.4). The first row is always
/// the header; `\multicolumn`/`\multirow` and unbalanced braces hard-error
/// rather than producing a partial table.
pub(crate) fn parse_latex_table(input: &str) -> Result<Table, ParseError> {
    let span = find_tabular(input)?;
    let body = &input[span.body_start..span.body_end];
    let row_chunks = split_rows(body);

    let mut rows: Vec<Vec<String>> = Vec::new();
    for (chunk, offset) in &row_chunks {
        let cleaned = structural_re().replace_all(chunk, "");
        if cleaned.trim().is_empty() {
            continue; // pure \hline/\toprule/… separator, not a data row
        }
        let at_line = line_of(input, span.body_start + offset);
        let mut cells = Vec::new();
        for raw in split_cells(&cleaned) {
            if raw.contains("\\multicolumn") {
                return Err(ParseError::UnsupportedConstruct {
                    format: DetectedFormat::LaTeX,
                    what: "\\multicolumn".to_string(),
                    at_line,
                });
            }
            if raw.contains("\\multirow") {
                return Err(ParseError::UnsupportedConstruct {
                    format: DetectedFormat::LaTeX,
                    what: "\\multirow".to_string(),
                    at_line,
                });
            }
            if !brace_depth_balanced(&raw) {
                return Err(ParseError::Malformed {
                    format: DetectedFormat::LaTeX,
                    reason: "unbalanced braces".to_string(),
                    at_line,
                });
            }
            cells.push(unescape_latex(raw.trim()).trim().to_string());
        }
        rows.push(cells);
    }

    if rows.is_empty() {
        return Err(ParseError::NoTabularStructure {
            format: DetectedFormat::LaTeX,
        });
    }

    let colcount = {
        let n = colspec_column_count(&span.colspec);
        if n > 0 {
            n
        } else {
            rows[0].len()
        }
    };

    let mut header = rows.remove(0);
    if header.len() < colcount {
        header.resize(colcount, String::new());
    } else {
        header.truncate(colcount);
    }

    let truncated = rows.len() > limits::MAX_ROWS;
    if truncated {
        rows.truncate(limits::MAX_ROWS);
    }

    Ok(build_table(
        header,
        rows,
        DetectedFormat::LaTeX,
        Vec::new(),
        truncated,
    ))
}

fn escape_latex_specials(s: &str) -> String {
    s.replace('%', "\\%")
        .replace('&', "\\&")
        .replace('_', "\\_")
        .replace('#', "\\#")
        .replace('$', "\\$")
}

/// A cell already containing a raw LaTeX macro (kept literal on import) is
/// emitted as-is — re-escaping it would corrupt an intentional macro call
/// (spec §3's noted round-trip limitation).
fn export_cell(cell: &str) -> String {
    if cell.contains('\\') {
        cell.to_string()
    } else {
        escape_latex_specials(cell)
    }
}

/// Plain booktabs `tabular` (spec §3): `l` for Text/Date, `r` for
/// numeric/Bool; no custom column spec, comments, or cell macros are
/// preserved from any original source.
pub(crate) fn export_latex_table(columns: &[Column]) -> String {
    let aligns: Vec<&str> = columns
        .iter()
        .map(|c| match c.kind {
            ColumnKind::Text | ColumnKind::Date => "l",
            ColumnKind::Integer | ColumnKind::Float | ColumnKind::Bool => "r",
        })
        .collect();
    let mut out = String::new();
    out.push_str("\\begin{tabular}{");
    out.push_str(&aligns.join(" "));
    out.push_str("}\n\\toprule\n");
    let header: Vec<String> = columns.iter().map(|c| export_cell(&c.name)).collect();
    out.push_str(&header.join(" & "));
    out.push_str(" \\\\\n\\midrule\n");
    let row_count = columns.first().map(|c| c.cells.len()).unwrap_or(0);
    for r in 0..row_count {
        let row: Vec<String> = columns.iter().map(|c| export_cell(&c.cells[r])).collect();
        out.push_str(&row.join(" & "));
        out.push_str(" \\\\\n");
    }
    out.push_str("\\bottomrule\n\\end{tabular}\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const RESULTS: &str = "\\begin{tabular}{lrr}\n\\toprule\nName & Mass & Radius \\\\\n\\midrule\nEarth & 1.0 & 1.0 \\\\\nMars & 0.107 & 0.532 \\\\\n\\bottomrule\n\\end{tabular}\n";

    #[test]
    fn parses_basic_tabular_with_booktabs_rules() {
        let t = parse_latex_table(RESULTS).unwrap();
        assert_eq!(t.columns.len(), 3);
        assert_eq!(t.columns[0].name, "Name");
        assert_eq!(t.row_count, 2);
        assert_eq!(t.columns[1].kind, ColumnKind::Float);
    }

    #[test]
    fn multicolumn_is_a_hard_error() {
        let src = "\\begin{tabular}{lll}\n\\multicolumn{2}{c}{Span} & C \\\\\nA & B & C \\\\\n\\end{tabular}\n";
        let err = parse_latex_table(src).unwrap_err();
        match err {
            ParseError::UnsupportedConstruct { what, .. } => assert_eq!(what, "\\multicolumn"),
            other => panic!("expected UnsupportedConstruct, got {other:?}"),
        }
    }

    #[test]
    fn nested_tabular_is_a_hard_error() {
        let src =
            "\\begin{tabular}{l}\n\\begin{tabular}{l}\nA \\\\\n\\end{tabular}\n\\end{tabular}\n";
        let err = parse_latex_table(src).unwrap_err();
        assert!(
            matches!(err, ParseError::UnsupportedConstruct { what, .. } if what == "nested tabular")
        );
    }

    #[test]
    fn unbalanced_braces_are_malformed() {
        let src = "\\begin{tabular}{ll}\nA & \\textbf{B \\\\\nC & D \\\\\n\\end{tabular}\n";
        let err = parse_latex_table(src).unwrap_err();
        assert!(
            matches!(err, ParseError::Malformed { reason, .. } if reason == "unbalanced braces")
        );
    }

    #[test]
    fn macro_calls_stay_literal_not_evaluated() {
        let src =
            "\\begin{tabular}{ll}\nA & B \\\\\n\\textbf{Total} & \\num{4.2} \\\\\n\\end{tabular}\n";
        let t = parse_latex_table(src).unwrap();
        assert_eq!(t.columns[0].cells[0], "\\textbf{Total}");
        assert_eq!(t.columns[1].cells[0], "\\num{4.2}");
    }

    #[test]
    fn no_tabular_environment_errors() {
        let err = parse_latex_table("just some text, no tables here").unwrap_err();
        assert!(matches!(err, ParseError::NoTabularStructure { .. }));
    }

    #[test]
    fn escapes_are_unescaped_on_import() {
        let src = "\\begin{tabular}{l}\n50\\% \\& more \\\\\n\\end{tabular}\n";
        let t = parse_latex_table(src).unwrap();
        assert_eq!(t.columns[0].name, "50% & more");
    }

    #[test]
    fn export_escapes_specials_but_not_existing_macros() {
        let columns = vec![Column {
            name: "pct".into(),
            kind: ColumnKind::Text,
            cells: vec!["50% & up".into(), "\\textbf{Total}".into()],
        }];
        let out = export_latex_table(&columns);
        assert!(out.contains("50\\% \\& up"));
        assert!(out.contains("\\textbf{Total}"));
    }
}
