//! CSV/TSV: hand-written RFC-4180-ish state machine (spec §1.1) + export (§3).
//! No `csv` crate — a quoted field can hold the delimiter/newlines/`""`
//! escapes, which a `split(',')` can't handle correctly.

use super::{
    build_table, looks_like_header, maybe_drop_blank_rows, synthetic_header, Column,
    DetectedFormat, ParseWarning, Table,
};
use std::collections::HashSet;

#[derive(PartialEq)]
enum St {
    FieldStart,
    Bare,
    Quoted,
    QuotedQuote,
}

fn push_row(
    rows: &mut Vec<Vec<String>>,
    row: &mut Vec<String>,
    blank_rows: &mut HashSet<usize>,
    malformed_rows: &mut Vec<usize>,
    had_malformed: &mut bool,
) {
    let is_blank = row.len() == 1 && row[0].is_empty();
    let completed = std::mem::take(row);
    rows.push(completed);
    let idx = rows.len() - 1;
    if is_blank {
        blank_rows.insert(idx);
    }
    if *had_malformed {
        malformed_rows.push(idx);
        *had_malformed = false;
    }
}

/// Tokenize into raw rows. Stops reading once `max_rows` rows have been
/// collected (spec §4: "the parser does not attempt to read past the cap") —
/// `truncated` is true iff any non-whitespace content remained past that
/// point.
fn tokenize(
    input: &str,
    delim: char,
    max_rows: usize,
) -> (Vec<Vec<String>>, HashSet<usize>, Vec<usize>, bool) {
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut blank_rows: HashSet<usize> = HashSet::new();
    let mut malformed_rows: Vec<usize> = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut state = St::FieldStart;
    let mut had_malformed = false;

    let mut iter = input.char_indices().peekable();
    while let Some((idx, c)) = iter.next() {
        if rows.len() >= max_rows {
            let truncated = !input[idx..].trim().is_empty();
            return (rows, blank_rows, malformed_rows, truncated);
        }
        match state {
            St::FieldStart => match c {
                d if d == delim => {
                    row.push(std::mem::take(&mut field));
                }
                '"' => state = St::Quoted,
                '\r' => {
                    if let Some((_, '\n')) = iter.peek() {
                        iter.next();
                    }
                    row.push(std::mem::take(&mut field));
                    push_row(
                        &mut rows,
                        &mut row,
                        &mut blank_rows,
                        &mut malformed_rows,
                        &mut had_malformed,
                    );
                }
                '\n' => {
                    row.push(std::mem::take(&mut field));
                    push_row(
                        &mut rows,
                        &mut row,
                        &mut blank_rows,
                        &mut malformed_rows,
                        &mut had_malformed,
                    );
                }
                _ => {
                    field.push(c);
                    state = St::Bare;
                }
            },
            St::Bare => match c {
                d if d == delim => {
                    row.push(std::mem::take(&mut field));
                    state = St::FieldStart;
                }
                '\r' => {
                    if let Some((_, '\n')) = iter.peek() {
                        iter.next();
                    }
                    row.push(std::mem::take(&mut field));
                    push_row(
                        &mut rows,
                        &mut row,
                        &mut blank_rows,
                        &mut malformed_rows,
                        &mut had_malformed,
                    );
                    state = St::FieldStart;
                }
                '\n' => {
                    row.push(std::mem::take(&mut field));
                    push_row(
                        &mut rows,
                        &mut row,
                        &mut blank_rows,
                        &mut malformed_rows,
                        &mut had_malformed,
                    );
                    state = St::FieldStart;
                }
                '"' => {
                    // A `"` after non-whitespace inside an unquoted field is
                    // malformed — pass the byte through literally, best-effort
                    // (never a hard error), plus a warning.
                    field.push(c);
                    had_malformed = true;
                }
                _ => field.push(c),
            },
            St::Quoted => {
                if c == '"' {
                    state = St::QuotedQuote;
                } else {
                    field.push(c);
                }
            }
            St::QuotedQuote => match c {
                '"' => {
                    field.push('"');
                    state = St::Quoted;
                }
                d if d == delim => {
                    row.push(std::mem::take(&mut field));
                    state = St::FieldStart;
                }
                '\r' => {
                    if let Some((_, '\n')) = iter.peek() {
                        iter.next();
                    }
                    row.push(std::mem::take(&mut field));
                    push_row(
                        &mut rows,
                        &mut row,
                        &mut blank_rows,
                        &mut malformed_rows,
                        &mut had_malformed,
                    );
                    state = St::FieldStart;
                }
                '\n' => {
                    row.push(std::mem::take(&mut field));
                    push_row(
                        &mut rows,
                        &mut row,
                        &mut blank_rows,
                        &mut malformed_rows,
                        &mut had_malformed,
                    );
                    state = St::FieldStart;
                }
                _ => {
                    field.push(c);
                    had_malformed = true;
                    state = St::Bare;
                }
            },
        }
    }
    let pending = !field.is_empty() || !row.is_empty() || state != St::FieldStart;
    if pending {
        if rows.len() < max_rows {
            row.push(field);
            push_row(
                &mut rows,
                &mut row,
                &mut blank_rows,
                &mut malformed_rows,
                &mut had_malformed,
            );
        } else {
            return (rows, blank_rows, malformed_rows, true);
        }
    }
    (rows, blank_rows, malformed_rows, false)
}

/// Parse CSV/TSV text with a known delimiter into a typed `Table`. Never hard
/// errors — worst case, everything degrades to `Text` cells + warnings.
pub(crate) fn parse(input: &str, delim: char, format: DetectedFormat) -> Table {
    // Trailing blank lines are ignored (spec §1.1); a blank line *inside* the
    // data still becomes a row, handled via `maybe_drop_blank_rows` below.
    let trimmed = input.trim_end_matches(['\n', '\r']);
    let (raw_rows, blank_rows, malformed_rows, truncated) =
        tokenize(trimmed, delim, super::limits::MAX_ROWS + 1);

    if raw_rows.is_empty() {
        return Table {
            columns: Vec::new(),
            row_count: 0,
            format,
            warnings: Vec::new(),
            truncated: false,
        };
    }

    let mut warnings = Vec::new();
    if !malformed_rows.is_empty() {
        warnings.push(ParseWarning::MalformedQuoting {
            rows: malformed_rows,
        });
    }

    let first = raw_rows[0].clone();
    let rest = &raw_rows[1..];
    // Undetermined → default to header present (CSV's documented default).
    let header_present = looks_like_header(&first, rest).unwrap_or(true);

    let (header, mut data_rows, data_blank_rows) = if header_present {
        let mut rows = raw_rows;
        let header = rows.remove(0);
        let blanks: HashSet<usize> = blank_rows
            .into_iter()
            .filter_map(|i| i.checked_sub(1))
            .collect();
        (header, rows, blanks)
    } else {
        let ncols = first.len();
        (synthetic_header(ncols), raw_rows, blank_rows)
    };

    if let Some(w) = maybe_drop_blank_rows(&header, &mut data_rows, &data_blank_rows) {
        warnings.push(w);
    }

    build_table(header, data_rows, format, warnings, truncated)
}

fn quote_if_needed(cell: &str, delim: char) -> String {
    if cell.contains(delim) || cell.contains('"') || cell.contains('\n') {
        format!("\"{}\"", cell.replace('"', "\"\""))
    } else {
        cell.to_string()
    }
}

fn write_row(out: &mut String, cells: &[String], delim: char) {
    for (i, cell) in cells.iter().enumerate() {
        if i > 0 {
            out.push(delim);
        }
        out.push_str(&quote_if_needed(cell, delim));
    }
    out.push('\n');
}

/// RFC-4180 CSV export: a cell is quoted iff it contains the delimiter, a
/// `"`, or a newline (`"` doubled inside). Header row always written (spec
/// §3 — no headerless export option).
pub(crate) fn export(columns: &[Column], delim: char) -> String {
    let mut out = String::new();
    let header: Vec<String> = columns.iter().map(|c| c.name.clone()).collect();
    write_row(&mut out, &header, delim);
    let row_count = columns.first().map(|c| c.cells.len()).unwrap_or(0);
    for r in 0..row_count {
        let row: Vec<String> = columns.iter().map(|c| c.cells[r].clone()).collect();
        write_row(&mut out, &row, delim);
    }
    out
}

fn escape_tsv_cell(cell: &str) -> String {
    cell.replace('\\', "\\\\")
        .replace('\t', "\\t")
        .replace('\n', "\\n")
}

/// TSV has no standard quoting convention, so a literal tab/newline is
/// backslash-escaped instead of quoted (spec §3).
pub(crate) fn export_tsv(columns: &[Column]) -> String {
    let mut out = String::new();
    let header: Vec<String> = columns.iter().map(|c| escape_tsv_cell(&c.name)).collect();
    out.push_str(&header.join("\t"));
    out.push('\n');
    let row_count = columns.first().map(|c| c.cells.len()).unwrap_or(0);
    for r in 0..row_count {
        let row: Vec<String> = columns
            .iter()
            .map(|c| escape_tsv_cell(&c.cells[r]))
            .collect();
        out.push_str(&row.join("\t"));
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::table::ColumnKind;

    #[test]
    fn header_detected_when_numeric_data_below() {
        let t = parse("name,age\nAnn,30\nBob,40\n", ',', DetectedFormat::Csv);
        assert_eq!(t.columns[0].name, "name");
        assert_eq!(t.row_count, 2);
        assert_eq!(t.columns[1].kind, ColumnKind::Integer);
    }

    #[test]
    fn no_header_when_first_row_looks_numeric_too() {
        let t = parse("1,2\n3,4\n", ',', DetectedFormat::Csv);
        assert_eq!(t.columns[0].name, "Col 1");
        assert_eq!(t.row_count, 2);
    }

    #[test]
    fn all_text_defaults_to_header_present() {
        let t = parse("a,b\nc,d\n", ',', DetectedFormat::Csv);
        assert_eq!(t.columns[0].name, "a");
        assert_eq!(t.row_count, 1);
    }

    #[test]
    fn ragged_rows_are_padded_and_truncated_with_warnings() {
        let t = parse("a,b,c\n1,2,3\n4,5\n6,7,8,9\n", ',', DetectedFormat::Csv);
        assert_eq!(t.row_count, 3);
        assert_eq!(t.columns[2].cells, vec!["3", "", "8"]);
        assert!(t
            .warnings
            .iter()
            .any(|w| matches!(w, ParseWarning::RowsPadded { .. })));
        assert!(t
            .warnings
            .iter()
            .any(|w| matches!(w, ParseWarning::RowsTruncated { .. })));
    }

    #[test]
    fn quoted_fields_with_delimiter_newline_and_escaped_quote() {
        let input = "a,b\n\"1,1\",\"line1\nline2\"\n\"say \"\"hi\"\"\",2\n";
        let t = parse(input, ',', DetectedFormat::Csv);
        assert_eq!(t.row_count, 2);
        assert_eq!(t.columns[0].cells[0], "1,1");
        assert_eq!(t.columns[1].cells[0], "line1\nline2");
        assert_eq!(t.columns[0].cells[1], "say \"hi\"");
    }

    #[test]
    fn trailing_blank_lines_are_ignored() {
        let t = parse("a,b\n1,2\n\n\n", ',', DetectedFormat::Csv);
        assert_eq!(t.row_count, 1);
    }

    #[test]
    fn blank_line_inside_numeric_data_is_dropped() {
        let t = parse("a,b\n1,2\n\n3,4\n", ',', DetectedFormat::Csv);
        assert_eq!(t.row_count, 2);
        assert!(t
            .warnings
            .iter()
            .any(|w| matches!(w, ParseWarning::BlankRowsDropped { .. })));
    }

    #[test]
    fn blank_line_inside_text_data_becomes_one_empty_row() {
        let t = parse("a,b\nx,y\n\nz,w\n", ',', DetectedFormat::Csv);
        assert_eq!(t.row_count, 3);
        assert_eq!(t.columns[0].cells[1], "");
    }

    #[test]
    fn csv_export_quotes_only_when_needed() {
        let columns = vec![Column {
            name: "a".into(),
            kind: ColumnKind::Text,
            cells: vec!["plain".into(), "has,comma".into(), "has\"quote".into()],
        }];
        let out = export(&columns, ',');
        assert_eq!(out, "a\nplain\n\"has,comma\"\n\"has\"\"quote\"\n");
    }

    #[test]
    fn tsv_export_escapes_literal_tab() {
        let columns = vec![Column {
            name: "a".into(),
            kind: ColumnKind::Text,
            cells: vec!["has\ttab".into()],
        }];
        let out = export_tsv(&columns);
        assert_eq!(out, "a\nhas\\ttab\n");
    }
}
