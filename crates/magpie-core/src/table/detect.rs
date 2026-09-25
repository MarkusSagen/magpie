//! `Auto` detection order (spec §1.7) and CSV/TSV delimiter sniffing (§1.1).

use super::{csv, json_lite, latex, markdown, typst, DetectedFormat, ParseError, Table};

/// Count `,`/`\t`/`;` occurrences outside double-quoted spans on one line.
fn count_delims_outside_quotes(line: &str) -> (usize, usize, usize) {
    let mut in_quotes = false;
    let (mut commas, mut tabs, mut semis) = (0usize, 0usize, 0usize);
    for c in line.chars() {
        match c {
            '"' => in_quotes = !in_quotes,
            ',' if !in_quotes => commas += 1,
            '\t' if !in_quotes => tabs += 1,
            ';' if !in_quotes => semis += 1,
            _ => {}
        }
    }
    (commas, tabs, semis)
}

/// Sniff the delimiter from the first non-blank line (max count of `,`/`\t`/`;`,
/// tie → `,`), then require the same count on each of the first 5 sampled
/// non-blank lines (or all of them, if fewer). Returns `None` on a mismatch —
/// callers then don't treat the input as CSV/TSV at all.
pub(crate) fn sniff_delimiter(input: &str) -> Option<char> {
    let lines: Vec<&str> = input.lines().filter(|l| !l.trim().is_empty()).collect();
    let first = *lines.first()?;
    let (c0, t0, s0) = count_delims_outside_quotes(first);
    let mut delim = ',';
    let mut best = c0;
    if t0 > best {
        delim = '\t';
        best = t0;
    }
    if s0 > best {
        delim = ';';
        best = s0;
    }
    if best == 0 {
        return None;
    }
    let expected = best;
    let sample = &lines[..lines.len().min(5)];
    for line in &sample[1..] {
        let (c, t, s) = count_delims_outside_quotes(line);
        let n = match delim {
            ',' => c,
            '\t' => t,
            _ => s,
        };
        if n != expected {
            return None;
        }
    }
    Some(delim)
}

/// Run the Auto detection order: JSON → LaTeX → Typst → Markdown → CSV/TSV →
/// Lines. A hard error from a *structurally detected* high-priority format
/// (JSON/LaTeX/Typst) is propagated, not swallowed into a lower fallback.
pub(crate) fn parse_auto(input: &str) -> Result<Table, ParseError> {
    let trimmed = input.trim_start();
    // `[`/`{` is a strong-enough signal to commit to JSON outright — a hard
    // parse error here (malformed JSON) is propagated, not swallowed into a
    // lower fallback (same "don't swallow a high-priority format's error"
    // rule as LaTeX/Typst below).
    if trimmed.starts_with('[') || trimmed.starts_with('{') {
        return json_lite::parse_json_table(input);
    }

    if input.contains("\\begin{tabular}") {
        return latex::parse_latex_table(input);
    }

    if input.contains("#table(") {
        return typst::parse_typst_table(input);
    }

    if markdown::detect(input).is_some() {
        return markdown::parse_markdown_table(input);
    }

    if let Some(delim) = sniff_delimiter(input) {
        let format = if delim == '\t' {
            DetectedFormat::Tsv
        } else {
            DetectedFormat::Csv
        };
        return Ok(csv::parse(input, delim, format));
    }

    Ok(super::lines_table(input))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniffs_comma_by_default_and_tie_breaks_to_comma() {
        assert_eq!(sniff_delimiter("a,b,c\n1,2,3\n"), Some(','));
    }

    #[test]
    fn sniffs_tab() {
        assert_eq!(sniff_delimiter("a\tb\tc\n1\t2\t3\n"), Some('\t'));
    }

    #[test]
    fn sniffs_semicolon() {
        assert_eq!(sniff_delimiter("a;b;c\n1;2;3\n"), Some(';'));
    }

    #[test]
    fn quoted_delimiters_are_not_counted() {
        assert_eq!(sniff_delimiter("\"a,b\",c\n\"1,1\",2\n"), Some(','));
    }

    #[test]
    fn inconsistent_counts_fail_the_sniff() {
        assert_eq!(sniff_delimiter("a,b,c\n1,2\n"), None);
    }
}
