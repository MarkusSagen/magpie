//! Integration tests for `magpie_core::table` (spec:
//! `docs/superpowers/specs/2026-09-24-magpie-data-design.md`).
//!
//! - Fixture-driven tests: one per `tests/fixtures/table/*` file, asserting
//!   `(row_count, column_count, Vec<ColumnKind>)`, and for the four
//!   "must hard-error" fixtures, the specific `ParseError` variant + line.
//! - Round-trip tests: parse → export → parse, asserting the typed grid is
//!   unchanged for lossless formats, and the *specific* documented loss for
//!   the lossy ones (LaTeX/Typst).
//! - A table-driven type-inference sweep through the public `parse` API.
//! - A generated (not committed) 60k-row CSV exercising the row cap.

use magpie_core::table::{
    export, limits, parse, Column, ColumnKind, DetectedFormat, ExportFormat, ParseError,
    ParseWarning, ReadAs,
};
use std::fs;
use std::path::PathBuf;

fn fixture(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/table")
        .join(name);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("failed to read fixture {name}: {e}"))
}

// ---------------------------------------------------------------------------
// Fixture-driven happy-path tests
// ---------------------------------------------------------------------------

#[test]
fn planets_csv_header_and_types() {
    let t = parse(&fixture("planets.csv"), ReadAs::Auto).unwrap();
    assert_eq!(t.format, DetectedFormat::Csv);
    assert_eq!(t.row_count, 8);
    assert_eq!(t.columns.len(), 6);
    assert_eq!(t.columns[0].name, "name");
    let kinds: Vec<ColumnKind> = t.columns.iter().map(|c| c.kind).collect();
    assert_eq!(
        kinds,
        vec![
            ColumnKind::Text,
            ColumnKind::Float,
            ColumnKind::Float,
            ColumnKind::Float,
            ColumnKind::Float,
            ColumnKind::Bool,
        ]
    );
}

#[test]
fn planets_no_header_synthesizes_columns() {
    let t = parse(&fixture("planets_no_header.csv"), ReadAs::Auto).unwrap();
    assert_eq!(t.row_count, 8);
    assert_eq!(t.columns.len(), 6);
    assert_eq!(t.columns[0].name, "Col 1");
    assert_eq!(t.columns[0].cells[0], "Mercury");
}

#[test]
fn planets_semicolon_delimiter_is_sniffed() {
    let t = parse(&fixture("planets_semicolon.csv"), ReadAs::Auto).unwrap();
    assert_eq!(t.format, DetectedFormat::Csv);
    assert_eq!(t.row_count, 3);
    assert_eq!(t.columns.len(), 3);
    assert_eq!(t.columns[0].name, "name");
}

#[test]
fn quoted_fields_edge_cases() {
    // Forced `Csv`: the embedded newline inside a quoted field splits Auto's
    // naive line-based delimiter sample, so this exercises the explicit
    // override path a real user would reach for here (spec §1.1).
    let t = parse(&fixture("quoted_fields.csv"), ReadAs::Csv).unwrap();
    assert_eq!(t.row_count, 2);
    assert_eq!(t.columns.len(), 2);
    assert_eq!(t.columns[0].cells[0], "Ann, Smith");
    assert_eq!(t.columns[1].cells[0], "Loves \"quotes\"");
    assert_eq!(t.columns[1].cells[1], "Line one\nLine two");
}

#[test]
fn ragged_rows_are_padded_and_truncated() {
    // Forced `Csv`: a genuinely ragged file fails Auto's own consistent-count
    // delimiter sniff by design (spec §1.1) and falls through to Lines — the
    // override is how a user (or this test) opts into the CSV parser anyway.
    let t = parse(&fixture("ragged_rows.csv"), ReadAs::Csv).unwrap();
    assert_eq!(t.row_count, 3);
    assert_eq!(t.columns.len(), 3);
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
fn array_of_objects_union_of_keys_and_nested_value_as_text() {
    let t = parse(&fixture("array_of_objects.json"), ReadAs::Auto).unwrap();
    assert_eq!(t.format, DetectedFormat::Json);
    assert_eq!(t.row_count, 2);
    let names: Vec<&str> = t.columns.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, vec!["name", "age", "tags", "score"]);
    assert_eq!(t.columns[2].kind, ColumnKind::Text); // nested "tags" array stringified
    assert_eq!(t.columns[2].cells[0], "[\"x\",\"y\"]");
    assert_eq!(t.columns[1].cells[1], ""); // Bob has no "age"
    assert!(t
        .warnings
        .iter()
        .any(|w| matches!(w, ParseWarning::DifferingKeys)));
}

#[test]
fn array_of_arrays_with_detected_header() {
    let t = parse(&fixture("array_of_arrays.json"), ReadAs::Auto).unwrap();
    assert_eq!(t.row_count, 2);
    assert_eq!(t.columns[0].name, "name");
    assert_eq!(t.columns[1].kind, ColumnKind::Integer);
}

#[test]
fn array_of_arrays_headerless() {
    let t = parse(&fixture("array_of_arrays_headerless.json"), ReadAs::Auto).unwrap();
    assert_eq!(t.row_count, 3);
    assert_eq!(t.columns[0].name, "Col 1");
    assert_eq!(t.columns[0].kind, ColumnKind::Integer);
}

#[test]
fn malformed_json_hard_errors_with_a_line_number() {
    let err = parse(&fixture("malformed.json"), ReadAs::Auto).unwrap_err();
    match err {
        ParseError::Malformed {
            format, at_line, ..
        } => {
            assert_eq!(format, DetectedFormat::Json);
            assert!(
                at_line >= 3,
                "expected the trailing comma's line, got {at_line}"
            );
        }
        other => panic!("expected Malformed, got {other:?}"),
    }
}

#[test]
fn markdown_table_with_escaped_pipe_and_ragged_row() {
    let t = parse(&fixture("table.md"), ReadAs::Auto).unwrap();
    assert_eq!(t.format, DetectedFormat::Markdown);
    assert_eq!(t.row_count, 2);
    assert_eq!(t.columns[1].cells[0], "uses | pipe");
    assert_eq!(t.columns[1].cells[1], ""); // ragged row padded
}

#[test]
fn results_tex_parses_the_literal_table() {
    let t = parse(&fixture("results.tex"), ReadAs::Auto).unwrap();
    assert_eq!(t.format, DetectedFormat::LaTeX);
    assert_eq!(t.row_count, 5);
    assert_eq!(t.columns.len(), 3);
    assert_eq!(t.columns[0].name, "Name");
    assert_eq!(t.columns[1].kind, ColumnKind::Float);
}

#[test]
fn multicolumn_tex_hard_errors() {
    let err = parse(&fixture("multicolumn.tex"), ReadAs::Auto).unwrap_err();
    match err {
        ParseError::UnsupportedConstruct {
            format,
            what,
            at_line,
        } => {
            assert_eq!(format, DetectedFormat::LaTeX);
            assert_eq!(what, "\\multicolumn");
            assert!(at_line > 0);
        }
        other => panic!("expected UnsupportedConstruct, got {other:?}"),
    }
}

#[test]
fn bench_typ_parses_the_literal_table() {
    let t = parse(&fixture("bench.typ"), ReadAs::Auto).unwrap();
    assert_eq!(t.format, DetectedFormat::Typst);
    assert_eq!(t.row_count, 11);
    assert_eq!(t.columns.len(), 5);
    assert_eq!(t.columns[0].name, "name");
    assert_eq!(t.columns[4].kind, ColumnKind::Bool);
}

#[test]
fn colspan_typ_hard_errors() {
    let err = parse(&fixture("colspan.typ"), ReadAs::Auto).unwrap_err();
    match err {
        ParseError::UnsupportedConstruct { format, what, .. } => {
            assert_eq!(format, DetectedFormat::Typst);
            assert_eq!(what, "colspan/rowspan");
        }
        other => panic!("expected UnsupportedConstruct, got {other:?}"),
    }
}

#[test]
fn computed_cell_typ_hard_errors() {
    let err = parse(&fixture("computed_cell.typ"), ReadAs::Auto).unwrap_err();
    match err {
        ParseError::UnsupportedConstruct { format, what, .. } => {
            assert_eq!(format, DetectedFormat::Typst);
            assert_eq!(what, "computed cell (non-literal expression)");
        }
        other => panic!("expected UnsupportedConstruct, got {other:?}"),
    }
}

#[test]
fn plain_lines_fallback() {
    let t = parse(&fixture("plain_lines.txt"), ReadAs::Auto).unwrap();
    assert_eq!(t.format, DetectedFormat::Lines);
    assert_eq!(t.row_count, 41);
    assert_eq!(t.columns.len(), 1);
    assert_eq!(t.columns[0].kind, ColumnKind::Text);
}

#[test]
fn plain_lines_of_bare_numbers_infer_numeric() {
    let nums: String = (1..=41)
        .map(|n| n.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let t = parse(&nums, ReadAs::Lines).unwrap();
    assert_eq!(t.row_count, 41);
    assert_eq!(t.columns[0].kind, ColumnKind::Integer);
}

// ---------------------------------------------------------------------------
// Round-trip tests: parse -> export -> parse
// ---------------------------------------------------------------------------

fn round_trip_columns(columns: &[Column], format: ExportFormat, read_as: ReadAs) -> Vec<Column> {
    let text = export(columns, format);
    parse(&text, read_as)
        .unwrap_or_else(|e| panic!("round-trip through {format:?} failed to re-parse: {e}"))
        .columns
}

#[test]
fn round_trip_csv_tsv_json_markdown_preserve_the_grid() {
    let t = parse(&fixture("planets.csv"), ReadAs::Auto).unwrap();
    for (format, read_as) in [
        (ExportFormat::Csv, ReadAs::Csv),
        (ExportFormat::Tsv, ReadAs::Tsv),
        (ExportFormat::Json, ReadAs::Json),
        (ExportFormat::Markdown, ReadAs::Markdown),
    ] {
        let cols = round_trip_columns(&t.columns, format, read_as);
        assert_eq!(
            cols, t.columns,
            "round-trip through {format:?} changed the grid"
        );
    }
}

#[test]
fn round_trip_latex_preserves_plain_grid_content() {
    let t = parse(&fixture("results.tex"), ReadAs::Auto).unwrap();
    let cols = round_trip_columns(&t.columns, ExportFormat::LaTeX, ReadAs::LaTeX);
    assert_eq!(cols, t.columns);
}

#[test]
fn latex_export_is_documented_lossy_for_a_macro_cell_containing_a_raw_ampersand() {
    // Spec §3: a cell that already contains a raw LaTeX macro is emitted
    // as-is (unescaped) on export, since re-escaping it would corrupt an
    // intentional macro call. That means a raw `&` sitting alongside the
    // macro is *not* escaped either — on re-import it's misread as a cell
    // separator. This is the documented round-trip limitation, asserted as
    // a concrete corrupted value rather than a vague "this is lossy" check.
    let columns = vec![
        Column {
            name: "link".into(),
            kind: ColumnKind::Text,
            cells: vec!["\\cmd & extra".into()],
        },
        Column {
            name: "n".into(),
            kind: ColumnKind::Integer,
            cells: vec!["1".into()],
        },
    ];
    let text = export(&columns, ExportFormat::LaTeX);
    let reparsed = parse(&text, ReadAs::LaTeX)
        .expect("no braces are involved, so this corrupts silently rather than erroring");
    assert_ne!(
        reparsed.columns[0].cells[0], columns[0].cells[0],
        "the embedded raw '&' should have split what was meant to be one cell"
    );
}

#[test]
fn round_trip_typst_preserves_grid_including_special_characters() {
    let t = parse(&fixture("bench.typ"), ReadAs::Auto).unwrap();
    let cols = round_trip_columns(&t.columns, ExportFormat::Typst, ReadAs::Typst);
    assert_eq!(cols, t.columns);

    // Unlike LaTeX, Typst export always brackets + escapes content (no
    // "already contains a macro" exception), so even a cell with a raw
    // backslash round-trips correctly — only styling (stroke/align), not
    // cell content, is lost for Typst (spec §3).
    let columns = vec![Column {
        name: "x".into(),
        kind: ColumnKind::Text,
        cells: vec!["back\\slash".into()],
    }];
    let cols2 = round_trip_columns(&columns, ExportFormat::Typst, ReadAs::Typst);
    assert_eq!(cols2, columns);
}

// ---------------------------------------------------------------------------
// Table-driven type-inference sweep (through the public `parse` API)
// ---------------------------------------------------------------------------

#[test]
fn type_inference_table_driven_cases() {
    // Single-column cases, forced through TSV (no literal tabs in any value)
    // so none of these collide with CSV's comma-as-delimiter.
    let cases: Vec<(&str, &str, ColumnKind)> = vec![
        ("plain integer", "n\n1\n2\n3\n", ColumnKind::Integer),
        (
            "comma-grouped integer",
            "n\n1,898\n2,000\n",
            ColumnKind::Integer,
        ),
        (
            "underscore-grouped integer",
            "n\n1_898\n2_000\n",
            ColumnKind::Integer,
        ),
        ("plain float", "n\n1.5\n2.25\n", ColumnKind::Float),
        (
            "scientific notation",
            "n\n1.5e10\n2e-3\n",
            ColumnKind::Float,
        ),
        (
            "int column with one float becomes float",
            "n\n1\n2.5\n3\n",
            ColumnKind::Float,
        ),
        (
            "bool synonyms",
            "b\ntrue\nfalse\nyes\nno\n",
            ColumnKind::Bool,
        ),
        (
            "zero and one stay integer, not bool",
            "b\n0\n1\n1\n",
            ColumnKind::Integer,
        ),
        ("iso date", "d\n2026-01-01\n2026-02-01\n", ColumnKind::Date),
        (
            "iso datetime",
            "d\n2026-01-01T10:00:00Z\n2026-02-01T11:30:00\n",
            ColumnKind::Date,
        ),
        (
            "dd mon yyyy date",
            "d\n14 Sep 2026\n01 Oct 2026\n",
            ColumnKind::Date,
        ),
        (
            "ambiguous nn/nn/yyyy is not guessed",
            "d\n01/02/2026\n03/04/2026\n",
            ColumnKind::Text,
        ),
        (
            "mixed real dates plus one non-date falls to text",
            "d\n2026-01-01\nnot a date\n",
            ColumnKind::Text,
        ),
        (
            "one bad cell in numeric column falls to text",
            "n\n1\n2\nN/A\n",
            ColumnKind::Text,
        ),
        (
            "comma-decimal column accepted unambiguously",
            "n\n3,14\n2,5\n",
            ColumnKind::Float,
        ),
    ];
    for (label, text, expected) in cases {
        let t = parse(text, ReadAs::Tsv).unwrap();
        assert_eq!(t.columns[0].kind, expected, "case: {label}");
    }

    // All-empty column (needs real comma-delimited multi-column CSV, values
    // themselves contain no commas).
    let t = parse("n,m\n,1\n,2\n", ReadAs::Csv).unwrap();
    assert_eq!(
        t.columns[0].kind,
        ColumnKind::Text,
        "all-empty column should be Text"
    );
    assert_eq!(t.columns[1].kind, ColumnKind::Integer);
}

// ---------------------------------------------------------------------------
// Large-file cap (spec §4) — generated in-test, never committed
// ---------------------------------------------------------------------------

#[test]
fn huge_csv_is_truncated_at_the_row_cap() {
    let mut src = String::from("id,val\n");
    for i in 0..60_000u32 {
        src.push_str(&format!("{i},{}\n", i * 2));
    }

    // The raw source itself holds all 60k rows untouched — only the parsed,
    // interactively-held `Table` is capped (spec §4's "the parser does not
    // attempt to read past the cap" applies to what's *held*, not the bytes
    // themselves). Re-exporting a truncated view means re-parsing these same
    // original bytes without the cap — a caller-level concern once it wires
    // this module's `export` (which itself operates on already-typed
    // `&[Column]`, per this module's public surface) to the original source.
    assert_eq!(src.lines().count() - 1, 60_000);

    let t = parse(&src, ReadAs::Csv).unwrap();
    assert!(t.truncated);
    assert_eq!(t.row_count, limits::MAX_ROWS);
    assert_eq!(t.row_count, 50_000);
    assert_eq!(t.columns[0].cells[0], "0");
    assert_eq!(t.columns[0].cells[49_999], "49999");
    assert_eq!(t.columns[1].cells[49_999], "99998");
}

#[test]
fn too_many_columns_are_dropped_with_a_warning() {
    let header = (1..=(limits::MAX_COLUMNS + 10))
        .map(|i| format!("c{i}"))
        .collect::<Vec<_>>()
        .join(",");
    let row = (1..=(limits::MAX_COLUMNS + 10))
        .map(|i| i.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let src = format!("{header}\n{row}\n");
    let t = parse(&src, ReadAs::Csv).unwrap();
    assert_eq!(t.columns.len(), limits::MAX_COLUMNS);
    assert!(t
        .warnings
        .iter()
        .any(|w| matches!(w, ParseWarning::TooManyColumns { kept, dropped } if *kept == limits::MAX_COLUMNS && *dropped == 10)));
}
