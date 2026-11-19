//! Data table parsers/exporters (spec:
//! `docs/superpowers/specs/2026-09-24-magpie-data-design.md`).
//!
//! Turns pasted/dropped/file-loaded tabular text (CSV, TSV, JSON,
//! GFM Markdown, LaTeX `tabular`, Typst `#table`, or plain lines) into a typed
//! [`Table`] — one [`ColumnKind`] per column, inferred per §2 — and back out
//! to any of those formats via [`export`]. Every parser is hand-rolled (no
//! `csv`/`serde_json`/markdown crate; `magpie-core` has none), matching the
//! existing hand-escaped JSON in `export.rs`.
//!
//! ```
//! use magpie_core::table::{parse, ReadAs, ColumnKind};
//! let t = parse("a,b\n1,2\n3,4\n", ReadAs::Auto).unwrap();
//! assert_eq!(t.columns.len(), 2);
//! assert_eq!(t.columns[0].kind, ColumnKind::Integer);
//! ```

mod csv;
mod detect;
mod infer;
mod json_lite;
mod latex;
pub mod limits;
mod markdown;
mod typst;

use std::collections::HashSet;
use std::fmt;

/// A column's inferred type. Precedence per cell is `Bool > Integer > Float >
/// Date > Text`; a column that doesn't unify under one family is `Text` (no
/// partial/mixed types) — see `infer::infer_column`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnKind {
    Integer,
    Float,
    Date,
    Bool,
    Text,
}

impl ColumnKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ColumnKind::Integer => "integer",
            ColumnKind::Float => "float",
            ColumnKind::Date => "date",
            ColumnKind::Bool => "bool",
            ColumnKind::Text => "text",
        }
    }
}

/// One typed column: its header name, inferred kind, and one display cell of
/// text per row (empty string = empty cell). Cells are already normalized at
/// parse time (thousands separators stripped, comma-decimal converted to `.`,
/// bool lowercased) — see spec §4's "format once" rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Column {
    pub name: String,
    pub kind: ColumnKind,
    pub cells: Vec<String>,
}

/// A parsed, typed grid plus the metadata needed to render the Data view's
/// caption/warnings (spec's `ParseOutcome`, folded into one type since this
/// crate has no separate "grid" concept to keep it apart from).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Table {
    pub columns: Vec<Column>,
    pub row_count: usize,
    pub format: DetectedFormat,
    pub warnings: Vec<ParseWarning>,
    pub truncated: bool,
}

impl Table {
    pub fn column_count(&self) -> usize {
        self.columns.len()
    }
}

/// Which parser actually produced a `Table` (shown as "detected {Format}" next
/// to the Read-as control).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetectedFormat {
    Csv,
    Tsv,
    Json,
    Markdown,
    LaTeX,
    Typst,
    Lines,
}

impl DetectedFormat {
    pub fn as_str(&self) -> &'static str {
        match self {
            DetectedFormat::Csv => "CSV",
            DetectedFormat::Tsv => "TSV",
            DetectedFormat::Json => "JSON",
            DetectedFormat::Markdown => "Markdown",
            DetectedFormat::LaTeX => "LaTeX",
            DetectedFormat::Typst => "Typst",
            DetectedFormat::Lines => "Lines",
        }
    }
}

impl fmt::Display for DetectedFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The "Read as" override (spec §1's five-way segmented control, plus explicit
/// `Tsv`/`Markdown` entries for programmatic/test use — Auto still detects
/// those two without the caller needing to pick them).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadAs {
    Auto,
    Csv,
    Tsv,
    Json,
    Markdown,
    LaTeX,
    Typst,
    Lines,
}

/// Export target (spec §3). Mirrors `ReadAs`'s formats minus `Auto`/`Lines`
/// (there's no "export as Lines").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    Csv,
    Tsv,
    Json,
    Markdown,
    LaTeX,
    Typst,
}

/// A non-fatal condition surfaced alongside a successful parse (spec §1's
/// "de-duplicated warning" style — one entry describing every affected row,
/// not one per row).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseWarning {
    /// Rows shorter than the header's column count were padded with empty cells.
    RowsPadded { rows: Vec<usize> },
    /// Rows longer than the header's column count had trailing cells dropped.
    RowsTruncated { rows: Vec<usize> },
    /// JSON array-of-objects rows didn't all share the same key set.
    DifferingKeys,
    /// A JSON object was unwrapped via its single array-valued key.
    WrapperKeyUsed { key: String },
    /// More columns were found than `limits::MAX_COLUMNS`; extras were dropped.
    TooManyColumns { kept: usize, dropped: usize },
    /// A column of `"3,14"`-style values was accepted as Float via the
    /// unambiguous comma-decimal fallback (spec §2's one locale guess).
    CommaDecimalAssumed { column: String },
    /// Fully-blank source lines were dropped because every column turned out
    /// numeric (spec §1.1's "avoid a spurious all-empty row" rule).
    BlankRowsDropped { count: usize },
    /// A quoted CSV field was followed by stray characters before the next
    /// delimiter/newline (malformed but recovered best-effort).
    MalformedQuoting { rows: Vec<usize> },
    /// A Markdown cell containing a newline was joined with `<br>` on export
    /// (lossy).
    NewlineJoinedWithBr { rows: Vec<usize> },
}

impl fmt::Display for ParseWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseWarning::RowsPadded { rows } => write!(
                f,
                "{} row(s) had fewer columns than the header, padded with empty: {rows:?}",
                rows.len()
            ),
            ParseWarning::RowsTruncated { rows } => write!(
                f,
                "{} row(s) had more columns than the header, extra cells dropped: {rows:?}",
                rows.len()
            ),
            ParseWarning::DifferingKeys => {
                write!(f, "rows had differing keys — missing values shown blank")
            }
            ParseWarning::WrapperKeyUsed { key } => {
                write!(f, "used the array under wrapper key \"{key}\"")
            }
            ParseWarning::TooManyColumns { kept, dropped } => {
                write!(f, "kept the first {kept} columns, dropped {dropped} more")
            }
            ParseWarning::CommaDecimalAssumed { column } => {
                write!(f, "column \"{column}\" was read as comma-decimal numbers")
            }
            ParseWarning::BlankRowsDropped { count } => {
                write!(f, "{count} blank row(s) dropped from an all-numeric table")
            }
            ParseWarning::MalformedQuoting { rows } => write!(
                f,
                "{} row(s) had a stray quote outside quoting rules: {rows:?}",
                rows.len()
            ),
            ParseWarning::NewlineJoinedWithBr { rows } => write!(
                f,
                "{} cell(s) with embedded newlines were joined with <br> on export: {rows:?}",
                rows.len()
            ),
        }
    }
}

/// Everything that can stop a parse cold (spec §1's error enum, plus
/// `TooLarge` for the 25 MB byte cap from §4 — that rejection happens before
/// any format-specific parsing starts, so it doesn't fit the original four
/// variants cleanly).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    Empty,
    NoTabularStructure {
        format: DetectedFormat,
    },
    UnsupportedConstruct {
        format: DetectedFormat,
        what: String,
        at_line: usize,
    },
    Malformed {
        format: DetectedFormat,
        reason: String,
        at_line: usize,
    },
    TooLarge {
        bytes: usize,
        max_bytes: usize,
    },
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::Empty => write!(f, "the input is empty"),
            ParseError::NoTabularStructure { format } => {
                write!(f, "no tabular structure found for {format}")
            }
            ParseError::UnsupportedConstruct {
                format,
                what,
                at_line,
            } => write!(
                f,
                "this table uses `{what}` (line {at_line}) — Magpie only reads plain grid cells ({format})"
            ),
            ParseError::Malformed {
                format,
                reason,
                at_line,
            } => write!(f, "malformed {format} at line {at_line}: {reason}"),
            ParseError::TooLarge { bytes, max_bytes } => write!(
                f,
                "this input is {} — Magpie reads tables up to {}",
                human_bytes(*bytes),
                human_bytes(*max_bytes)
            ),
        }
    }
}

impl std::error::Error for ParseError {}

fn human_bytes(n: usize) -> String {
    const MB: usize = 1024 * 1024;
    if n >= MB {
        format!("{:.1} MB", n as f64 / MB as f64)
    } else {
        format!("{} bytes", n)
    }
}

/// Parse `input` per the given `read_as` override (spec §1). `Auto` runs the
/// detection order in §1.7; every other variant forces that specific parser.
pub fn parse(input: &str, read_as: ReadAs) -> Result<Table, ParseError> {
    if input.len() > limits::MAX_BYTES {
        return Err(ParseError::TooLarge {
            bytes: input.len(),
            max_bytes: limits::MAX_BYTES,
        });
    }
    if input.trim().is_empty() {
        return Err(ParseError::Empty);
    }
    match read_as {
        ReadAs::Auto => detect::parse_auto(input),
        ReadAs::Csv => {
            let delim = detect::sniff_delimiter(input).unwrap_or(',');
            let format = if delim == '\t' {
                DetectedFormat::Tsv
            } else {
                DetectedFormat::Csv
            };
            Ok(csv::parse(input, delim, format))
        }
        ReadAs::Tsv => Ok(csv::parse(input, '\t', DetectedFormat::Tsv)),
        ReadAs::Json => json_lite::parse_json_table(input),
        ReadAs::Markdown => markdown::parse_markdown_table(input),
        ReadAs::LaTeX => latex::parse_latex_table(input),
        ReadAs::Typst => typst::parse_typst_table(input),
        ReadAs::Lines => Ok(lines_table(input)),
    }
}

/// Render a typed grid back to `format` (spec §3). Always operates on the full
/// `columns` passed in — the row cap only applies to interactive `parse`, not
/// export (spec §4: "export is not capped").
pub fn export(columns: &[Column], format: ExportFormat) -> String {
    match format {
        ExportFormat::Csv => csv::export(columns, ','),
        ExportFormat::Tsv => csv::export_tsv(columns),
        ExportFormat::Json => json_lite::export_json(columns),
        ExportFormat::Markdown => markdown::export_markdown_table(columns),
        ExportFormat::LaTeX => latex::export_latex_table(columns),
        ExportFormat::Typst => typst::export_typst_table(columns),
    }
}

fn lines_table(input: &str) -> Table {
    let all_lines: Vec<&str> = input.lines().filter(|l| !l.trim().is_empty()).collect();
    let truncated = all_lines.len() > limits::MAX_ROWS;
    let kept: Vec<String> = all_lines
        .into_iter()
        .take(limits::MAX_ROWS)
        .map(|s| s.trim().to_string())
        .collect();
    let row_count = kept.len();
    let (kind, cells, warnings) = infer::infer_column("Col 1", &kept);
    Table {
        columns: vec![Column {
            name: "Col 1".to_string(),
            kind,
            cells,
        }],
        row_count,
        format: DetectedFormat::Lines,
        warnings,
        truncated,
    }
}

/// Shared header-presence heuristic (spec §1.1/§1.5): the candidate row is a
/// header iff every one of its cells is non-numeric-looking while at least one
/// *other* row has a numeric-looking cell somewhere. Returns `None` when the
/// heuristic can't tell either way (e.g. every cell everywhere is text-like) —
/// callers decide their own default for that case.
pub(crate) fn looks_like_header(candidate: &[String], other_rows: &[Vec<String>]) -> Option<bool> {
    let all_non_numeric = candidate.iter().all(|c| !infer::cell_looks_data_like(c));
    if !all_non_numeric {
        return Some(false);
    }
    let any_other_numeric = other_rows
        .iter()
        .any(|row| row.iter().any(|c| infer::cell_looks_data_like(c)));
    if any_other_numeric {
        Some(true)
    } else {
        None
    }
}

/// Pad short rows / truncate long rows to `width`, returning de-duplicated
/// warnings naming the affected row indices (spec §1.1).
pub(crate) fn normalize_row_widths(rows: &mut [Vec<String>], width: usize) -> Vec<ParseWarning> {
    let mut warnings = Vec::new();
    let mut padded = Vec::new();
    let mut truncated = Vec::new();
    for (i, row) in rows.iter_mut().enumerate() {
        match row.len().cmp(&width) {
            std::cmp::Ordering::Less => {
                padded.push(i);
                row.resize(width, String::new());
            }
            std::cmp::Ordering::Greater => {
                truncated.push(i);
                row.truncate(width);
            }
            std::cmp::Ordering::Equal => {}
        }
    }
    if !padded.is_empty() {
        warnings.push(ParseWarning::RowsPadded { rows: padded });
    }
    if !truncated.is_empty() {
        warnings.push(ParseWarning::RowsTruncated { rows: truncated });
    }
    warnings
}

/// Drop extra columns past `limits::MAX_COLUMNS`, keeping the first N (spec
/// §4).
pub(crate) fn apply_column_cap(
    header: &mut Vec<String>,
    rows: &mut [Vec<String>],
) -> Option<ParseWarning> {
    if header.len() > limits::MAX_COLUMNS {
        let dropped = header.len() - limits::MAX_COLUMNS;
        header.truncate(limits::MAX_COLUMNS);
        for row in rows.iter_mut() {
            row.truncate(limits::MAX_COLUMNS);
        }
        Some(ParseWarning::TooManyColumns {
            kept: limits::MAX_COLUMNS,
            dropped,
        })
    } else {
        None
    }
}

/// Assemble the final typed `Table` from a raw header + data-row grid: applies
/// the column cap, pads/truncates ragged rows, then infers each column
/// independently (spec §2/§4). Shared by every format's parser so the
/// cap/padding/inference logic lives in exactly one place.
pub(crate) fn build_table(
    mut header: Vec<String>,
    mut rows: Vec<Vec<String>>,
    format: DetectedFormat,
    mut warnings: Vec<ParseWarning>,
    truncated: bool,
) -> Table {
    if let Some(w) = apply_column_cap(&mut header, &mut rows) {
        warnings.push(w);
    }
    let width = header.len();
    warnings.extend(normalize_row_widths(&mut rows, width));
    let row_count = rows.len();
    let mut columns = Vec::with_capacity(width);
    for (ci, name) in header.into_iter().enumerate() {
        let raw_cells: Vec<String> = rows.iter().map(|r| r[ci].clone()).collect();
        let (kind, cells, col_warnings) = infer::infer_column(&name, &raw_cells);
        warnings.extend(col_warnings);
        columns.push(Column { name, kind, cells });
    }
    Table {
        columns,
        row_count,
        format,
        warnings,
        truncated,
    }
}

/// Synthetic `Col 1, Col 2, …` headers (1-based), used whenever a format has
/// no header row.
pub(crate) fn synthetic_header(n: usize) -> Vec<String> {
    (1..=n).map(|i| format!("Col {i}")).collect()
}

/// 1-based line number of the byte offset `byte_idx` within `text` — shared by
/// the LaTeX/Typst parsers for `ParseError::{UnsupportedConstruct,Malformed}`'s
/// `at_line`.
pub(crate) fn line_of(text: &str, byte_idx: usize) -> usize {
    text[..byte_idx.min(text.len())].matches('\n').count() + 1
}

/// Drop rows whose *every* cell is empty, but only when doing so is safe: all
/// resulting column kinds must be non-Text (spec §1.1 — avoids a spurious
/// all-empty row surviving in an otherwise fully numeric/date/bool table).
/// `blank_rows` names indices (into `rows`, pre-drop) that came from a
/// literal, structurally-empty source line — only those are eligible.
pub(crate) fn maybe_drop_blank_rows(
    header: &[String],
    rows: &mut Vec<Vec<String>>,
    blank_rows: &HashSet<usize>,
) -> Option<ParseWarning> {
    if blank_rows.is_empty() {
        return None;
    }
    let ncols = header.len();
    let mut all_non_text = true;
    for ci in 0..ncols {
        let raw: Vec<String> = rows
            .iter()
            .map(|r| r.get(ci).cloned().unwrap_or_default())
            .collect();
        let (kind, _, _) = infer::infer_column("", &raw);
        if kind == ColumnKind::Text {
            all_non_text = false;
            break;
        }
    }
    if !all_non_text {
        return None;
    }
    let mut idx = 0usize;
    let before = rows.len();
    rows.retain(|_| {
        let keep = !blank_rows.contains(&idx);
        idx += 1;
        keep
    });
    let dropped = before - rows.len();
    if dropped == 0 {
        None
    } else {
        Some(ParseWarning::BlankRowsDropped { count: dropped })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_errors() {
        assert_eq!(parse("", ReadAs::Auto), Err(ParseError::Empty));
        assert_eq!(parse("   \n  ", ReadAs::Auto), Err(ParseError::Empty));
    }

    #[test]
    fn too_large_input_is_rejected_before_parsing() {
        let big = "a".repeat(limits::MAX_BYTES + 1);
        match parse(&big, ReadAs::Lines) {
            Err(ParseError::TooLarge { bytes, max_bytes }) => {
                assert_eq!(bytes, big.len());
                assert_eq!(max_bytes, limits::MAX_BYTES);
            }
            other => panic!("expected TooLarge, got {other:?}"),
        }
    }

    #[test]
    fn display_impls_are_human_readable() {
        let e = ParseError::UnsupportedConstruct {
            format: DetectedFormat::LaTeX,
            what: "\\multicolumn".to_string(),
            at_line: 4,
        };
        assert!(e.to_string().contains("multicolumn"));
        assert!(e.to_string().contains('4'));
        let w = ParseWarning::TooManyColumns {
            kept: 500,
            dropped: 3,
        };
        assert!(w.to_string().contains("500"));
    }

    #[test]
    fn column_cap_drops_extra_columns() {
        let mut header = synthetic_header(3);
        let mut rows: Vec<Vec<String>> = vec![vec!["1".into(), "2".into(), "3".into()]];
        assert!(apply_column_cap(&mut header, &mut rows).is_none());

        let mut header2: Vec<String> = (1..=(limits::MAX_COLUMNS + 5))
            .map(|i| format!("c{i}"))
            .collect();
        let mut rows2: Vec<Vec<String>> = vec![(1..=(limits::MAX_COLUMNS + 5))
            .map(|i| i.to_string())
            .collect()];
        let w = apply_column_cap(&mut header2, &mut rows2).unwrap();
        assert_eq!(header2.len(), limits::MAX_COLUMNS);
        assert_eq!(rows2[0].len(), limits::MAX_COLUMNS);
        assert!(
            matches!(w, ParseWarning::TooManyColumns { kept, dropped } if kept == limits::MAX_COLUMNS && dropped == 5)
        );
    }
}
