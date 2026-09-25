//! Per-cell type sniffers and column-type reduction (spec §2).
//!
//! Per-cell precedence: `Bool > Integer > Float > Date > Text` (first match
//! wins). A column's type is the least specific type that fits every
//! non-empty cell in that column; empty cells never veto a type. A column
//! that doesn't unify under one family falls to `Text` — there is no
//! partial/mixed column type.

use super::{ColumnKind, ParseWarning};
use regex::Regex;
use std::sync::OnceLock;

/// One cell's sniffed type, before column-level reduction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CellType {
    Bool,
    Integer,
    Float,
    Date(DateFormat),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DateFormat {
    IsoDate,     // YYYY-MM-DD
    SlashDate,   // YYYY/MM/DD
    DdMonYyyy,   // 14 Sep 2026
    MonDdYyyy,   // Sep 14, 2026
    IsoDateTime, // YYYY-MM-DDTHH:MM:SS[Z]
}

const MONTH_NAMES: [&str; 12] = [
    "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
];

fn month_index(word: &str) -> Option<u32> {
    let w = word.to_ascii_lowercase();
    if w.len() < 3 {
        return None;
    }
    let prefix = &w[..3];
    MONTH_NAMES
        .iter()
        .position(|m| *m == prefix)
        .map(|i| i as u32 + 1)
}

/// Case-insensitive exact match (after trim) against `true`/`false`/`yes`/`no`.
/// Deliberately NOT `1`/`0` (those are Integer, checked first anyway) and not
/// localized words — English literals only in v1.
pub fn sniff_bool(raw: &str) -> Option<bool> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "true" | "yes" => Some(true),
        "false" | "no" => Some(false),
        _ => None,
    }
}

/// Strip a single grouping-separator character from a run of digits, requiring
/// the separator to sit every-three-digits from the right (leftmost group may
/// be 1-3 digits). Returns `None` if the grouping is inconsistent.
fn strip_grouping(digits_and_seps: &str, sep: char) -> Option<String> {
    if !digits_and_seps.contains(sep) {
        return if digits_and_seps.chars().all(|c| c.is_ascii_digit()) && !digits_and_seps.is_empty()
        {
            Some(digits_and_seps.to_string())
        } else {
            None
        };
    }
    let groups: Vec<&str> = digits_and_seps.split(sep).collect();
    if groups.len() < 2 {
        return None;
    }
    if groups[0].is_empty() || groups[0].len() > 3 || !groups[0].chars().all(|c| c.is_ascii_digit())
    {
        return None;
    }
    for g in &groups[1..] {
        if g.len() != 3 || !g.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
    }
    Some(groups.concat())
}

/// Optional leading `-`/`+`, then digits, optionally with grouping separators
/// (`,`, `_`, or a bare space). Returns the canonical (sign + ungrouped digits)
/// text on success.
pub fn sniff_integer(raw: &str) -> Option<String> {
    let s = raw.trim();
    if s.is_empty() {
        return None;
    }
    let (sign, rest) = match s.as_bytes()[0] {
        b'-' => ("-", &s[1..]),
        b'+' => ("", &s[1..]),
        _ => ("", s),
    };
    if rest.is_empty() {
        return None;
    }
    for sep in [',', '_', ' '] {
        if rest.contains(sep) {
            return strip_grouping(rest, sep).map(|d| format!("{sign}{d}"));
        }
    }
    if rest.chars().all(|c| c.is_ascii_digit()) {
        Some(format!("{sign}{rest}"))
    } else {
        None
    }
}

/// Integer's grammar plus an optional `.`-decimal part and/or scientific
/// notation (`1.5e10`, `-3E-4`). A comma decimal (`"3,14"`) is intentionally
/// NOT accepted here — that's the separate, column-level "unambiguous
/// comma-decimal" fallback in `reduce_column` (never silently guessed
/// per-cell).
pub fn sniff_float(raw: &str) -> Option<String> {
    let s = raw.trim();
    if s.is_empty() {
        return None;
    }
    let lower_e = s.to_ascii_lowercase().find('e');
    let (mantissa, exponent) = match lower_e {
        Some(idx) => (&s[..idx], Some(&s[idx + 1..])),
        None => (s, None),
    };
    if let Some(exp) = exponent {
        let exp = exp.trim();
        if exp.is_empty() {
            return None;
        }
        let edigits = match exp.as_bytes()[0] {
            b'-' | b'+' => &exp[1..],
            _ => exp,
        };
        if edigits.is_empty() || !edigits.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
    }
    let m = mantissa.trim();
    if m.is_empty() {
        return None;
    }
    let (sign, rest) = match m.as_bytes()[0] {
        b'-' => ("-", &m[1..]),
        b'+' => ("", &m[1..]),
        _ => ("", m),
    };
    let (int_part, frac_part) = match rest.find('.') {
        Some(idx) => (&rest[..idx], Some(&rest[idx + 1..])),
        None => (rest, None),
    };
    // A plain integer (no '.', no exponent) is Integer's job, not Float's —
    // require a decimal point or an exponent to actually be a Float.
    if frac_part.is_none() && exponent.is_none() {
        return None;
    }
    if int_part.is_empty() {
        return None;
    }
    let int_digits = {
        let mut grouped = None;
        for sep in [',', '_', ' '] {
            if int_part.contains(sep) {
                grouped = strip_grouping(int_part, sep);
                break;
            }
        }
        match grouped {
            Some(d) => d,
            None => {
                if int_part.chars().all(|c| c.is_ascii_digit()) {
                    int_part.to_string()
                } else {
                    return None;
                }
            }
        }
    };
    if let Some(f) = frac_part {
        if f.is_empty() || !f.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
    }
    let mut out = format!("{sign}{int_digits}");
    if let Some(f) = frac_part {
        out.push('.');
        out.push_str(f);
    }
    if let Some(exp) = exponent {
        out.push('e');
        out.push_str(exp.trim());
    }
    Some(out)
}

fn iso_date_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^(\d{4})-(\d{2})-(\d{2})$").unwrap())
}
fn slash_date_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^(\d{4})/(\d{2})/(\d{2})$").unwrap())
}
fn dd_mon_yyyy_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)^(\d{1,2})\s+([A-Za-z]{3,9})\s+(\d{4})$").unwrap())
}
fn mon_dd_yyyy_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)^([A-Za-z]{3,9})\s+(\d{1,2}),\s*(\d{4})$").unwrap())
}
fn iso_datetime_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2}):(\d{2})(Z)?$").unwrap())
}

fn valid_ymd(y: u32, m: u32, d: u32) -> bool {
    let _ = y;
    (1..=12).contains(&m) && (1..=31).contains(&d)
}

/// Matches against a fixed, ordered list of unambiguous date formats. `NN/NN/YYYY`
/// (day/month order unclear) is deliberately not in this list — such a column
/// simply never matches here and falls to Text (§2's "don't guess" rule).
fn sniff_date(raw: &str) -> Option<DateFormat> {
    let s = raw.trim();
    if let Some(c) = iso_date_re().captures(s) {
        let (y, m, d) = (
            c[1].parse().ok()?,
            c[2].parse().ok()?,
            c[3].parse::<u32>().ok()?,
        );
        if valid_ymd(y, m, d) {
            return Some(DateFormat::IsoDate);
        }
    }
    if let Some(c) = slash_date_re().captures(s) {
        let (y, m, d) = (
            c[1].parse().ok()?,
            c[2].parse().ok()?,
            c[3].parse::<u32>().ok()?,
        );
        if valid_ymd(y, m, d) {
            return Some(DateFormat::SlashDate);
        }
    }
    if let Some(c) = dd_mon_yyyy_re().captures(s) {
        let d: u32 = c[1].parse().ok()?;
        if month_index(&c[2]).is_some() && (1..=31).contains(&d) {
            return Some(DateFormat::DdMonYyyy);
        }
    }
    if let Some(c) = mon_dd_yyyy_re().captures(s) {
        let d: u32 = c[2].parse().ok()?;
        if month_index(&c[1]).is_some() && (1..=31).contains(&d) {
            return Some(DateFormat::MonDdYyyy);
        }
    }
    if let Some(c) = iso_datetime_re().captures(s) {
        let (y, m, d) = (
            c[1].parse().ok()?,
            c[2].parse().ok()?,
            c[3].parse::<u32>().ok()?,
        );
        if valid_ymd(y, m, d) {
            return Some(DateFormat::IsoDateTime);
        }
    }
    None
}

fn classify(raw: &str) -> Option<(CellType, String)> {
    let t = raw.trim();
    if let Some(b) = sniff_bool(t) {
        return Some((CellType::Bool, if b { "true" } else { "false" }.to_string()));
    }
    if let Some(canon) = sniff_integer(t) {
        return Some((CellType::Integer, canon));
    }
    if let Some(canon) = sniff_float(t) {
        return Some((CellType::Float, canon));
    }
    if let Some(fmt) = sniff_date(t) {
        return Some((CellType::Date(fmt), t.to_string()));
    }
    None
}

/// Column-level "every cell has exactly one comma, ≤3 digits after it, and no
/// cell has a literal `.`" fallback (spec §2's one locale-sensitive heuristic).
/// Only consulted once ordinary per-cell classification has already failed to
/// unify the column, and only accepted with a `ParseWarning` flagging the
/// assumption.
fn try_comma_decimal_column(non_empty: &[&str]) -> Option<Vec<String>> {
    if non_empty.is_empty() {
        return None;
    }
    let mut out = Vec::with_capacity(non_empty.len());
    for cell in non_empty {
        let t = cell.trim();
        if t.contains('.') {
            return None;
        }
        let comma_count = t.matches(',').count();
        if comma_count != 1 {
            return None;
        }
        let (int_part, frac_part) = t.split_once(',').unwrap();
        let (sign, int_digits) = match int_part.as_bytes().first() {
            Some(b'-') => ("-", &int_part[1..]),
            Some(b'+') => ("", &int_part[1..]),
            _ => ("", int_part),
        };
        if int_digits.is_empty() || !int_digits.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        if frac_part.is_empty()
            || frac_part.len() > 3
            || !frac_part.chars().all(|c| c.is_ascii_digit())
        {
            return None;
        }
        out.push(format!("{sign}{int_digits}.{frac_part}"));
    }
    Some(out)
}

/// Infer a column's type from its raw cell text and produce the canonical
/// display cells (thousands separators stripped, comma-decimal normalized to
/// `.`, bool normalized to lowercase `true`/`false`; Text/Date cells keep
/// their trimmed literal text). Empty cells stay empty and never affect the
/// decision.
pub fn infer_column(
    name: &str,
    raw_cells: &[String],
) -> (ColumnKind, Vec<String>, Vec<ParseWarning>) {
    let non_empty_idx: Vec<usize> = raw_cells
        .iter()
        .enumerate()
        .filter(|(_, c)| !c.trim().is_empty())
        .map(|(i, _)| i)
        .collect();

    if non_empty_idx.is_empty() {
        return (
            ColumnKind::Text,
            vec![String::new(); raw_cells.len()],
            Vec::new(),
        );
    }

    let classified: Vec<Option<(CellType, String)>> = non_empty_idx
        .iter()
        .map(|&i| classify(&raw_cells[i]))
        .collect();

    let all_are = |pred: &dyn Fn(&CellType) -> bool| {
        classified
            .iter()
            .all(|c| matches!(c, Some((t, _)) if pred(t)))
    };

    let place = |canon_by_idx: Vec<(usize, String)>| -> Vec<String> {
        let mut out = vec![String::new(); raw_cells.len()];
        for (i, c) in canon_by_idx {
            out[i] = c;
        }
        out
    };

    if classified
        .iter()
        .all(|c| matches!(c, Some((CellType::Integer, _))))
    {
        let cells = place(
            non_empty_idx
                .iter()
                .zip(classified.iter())
                .map(|(&i, c)| (i, c.as_ref().unwrap().1.clone()))
                .collect(),
        );
        return (ColumnKind::Integer, cells, Vec::new());
    }
    if all_are(&|t| matches!(t, CellType::Integer | CellType::Float)) {
        let cells = place(
            non_empty_idx
                .iter()
                .zip(classified.iter())
                .map(|(&i, c)| (i, c.as_ref().unwrap().1.clone()))
                .collect(),
        );
        return (ColumnKind::Float, cells, Vec::new());
    }
    if let Some((CellType::Date(first_fmt), _)) = &classified[0] {
        if all_are(&|t| matches!(t, CellType::Date(f) if *f == *first_fmt)) {
            let cells = place(
                non_empty_idx
                    .iter()
                    .zip(classified.iter())
                    .map(|(&i, c)| (i, c.as_ref().unwrap().1.clone()))
                    .collect(),
            );
            return (ColumnKind::Date, cells, Vec::new());
        }
    }
    if all_are(&|t| matches!(t, CellType::Bool)) {
        let cells = place(
            non_empty_idx
                .iter()
                .zip(classified.iter())
                .map(|(&i, c)| (i, c.as_ref().unwrap().1.clone()))
                .collect(),
        );
        return (ColumnKind::Bool, cells, Vec::new());
    }

    // Nothing unified — try the comma-decimal fallback before giving up to Text.
    let non_empty_raw: Vec<&str> = non_empty_idx
        .iter()
        .map(|&i| raw_cells[i].as_str())
        .collect();
    if let Some(canon) = try_comma_decimal_column(&non_empty_raw) {
        let cells = place(non_empty_idx.iter().copied().zip(canon).collect());
        let warning = ParseWarning::CommaDecimalAssumed {
            column: name.to_string(),
        };
        return (ColumnKind::Float, cells, vec![warning]);
    }

    // Text: literal trimmed text; empties stay empty.
    let cells: Vec<String> = raw_cells
        .iter()
        .map(|c| {
            if c.trim().is_empty() {
                String::new()
            } else {
                c.trim().to_string()
            }
        })
        .collect();
    (ColumnKind::Text, cells, Vec::new())
}

/// Whether a single cell "looks numeric" for the CSV/Typst header-detection
/// heuristic (Bool/Integer/Float/Date all count as "looks different from a
/// header label").
pub(crate) fn cell_looks_data_like(raw: &str) -> bool {
    let t = raw.trim();
    if t.is_empty() {
        return false;
    }
    sniff_bool(t).is_some()
        || sniff_integer(t).is_some()
        || sniff_float(t).is_some()
        || sniff_date(t).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bool_synonyms() {
        assert_eq!(sniff_bool("true"), Some(true));
        assert_eq!(sniff_bool("YES"), Some(true));
        assert_eq!(sniff_bool(" False "), Some(false));
        assert_eq!(sniff_bool("no"), Some(false));
        assert_eq!(sniff_bool("1"), None);
        assert_eq!(sniff_bool("0"), None);
    }

    #[test]
    fn zero_one_stay_integer_not_bool() {
        let (kind, cells, _) = infer_column("flag", &["0".into(), "1".into(), "1".into()]);
        assert_eq!(kind, ColumnKind::Integer);
        assert_eq!(cells, vec!["0", "1", "1"]);
    }

    #[test]
    fn plain_and_grouped_integers() {
        assert_eq!(sniff_integer("88"), Some("88".into()));
        assert_eq!(sniff_integer("-4,879"), Some("-4879".into()));
        assert_eq!(sniff_integer("1_898"), Some("1898".into()));
        assert_eq!(sniff_integer("1 898"), Some("1898".into()));
        assert_eq!(sniff_integer("12,34"), None); // not a valid 3-digit grouping
        assert_eq!(sniff_integer(""), None);
        assert_eq!(sniff_integer("12a"), None);
    }

    #[test]
    fn float_grammar_and_scientific() {
        assert_eq!(sniff_float("3.14"), Some("3.14".into()));
        assert_eq!(sniff_float("1.5e10"), Some("1.5e10".into()));
        assert_eq!(sniff_float("-3E-4"), Some("-3e-4".into()));
        assert_eq!(sniff_float("88"), None); // plain integer -> not Float's job
        assert_eq!(sniff_float("1,898.5"), Some("1898.5".into()));
    }

    #[test]
    fn comma_decimal_not_guessed_per_cell() {
        assert_eq!(sniff_float("3,14"), None);
    }

    #[test]
    fn comma_decimal_column_accepted_when_unambiguous() {
        let (kind, cells, warnings) = infer_column("val", &["3,14".into(), "2,5".into()]);
        assert_eq!(kind, ColumnKind::Float);
        assert_eq!(cells, vec!["3.14", "2.5"]);
        assert_eq!(warnings.len(), 1);
        assert!(
            matches!(&warnings[0], ParseWarning::CommaDecimalAssumed { column } if column == "val")
        );
    }

    #[test]
    fn comma_decimal_column_rejected_if_any_cell_has_a_dot() {
        let (kind, _, warnings) = infer_column("val", &["3,14".into(), "2.5".into()]);
        assert_eq!(kind, ColumnKind::Text);
        assert!(warnings.is_empty());
    }

    #[test]
    fn date_formats() {
        assert!(sniff_date("2026-09-24").is_some());
        assert!(sniff_date("2026/09/24").is_some());
        assert!(sniff_date("14 Sep 2026").is_some());
        assert!(sniff_date("Sep 14, 2026").is_some());
        assert!(sniff_date("2026-09-24T10:30:00Z").is_some());
        assert!(sniff_date("2026-09-24T10:30:00").is_some());
    }

    #[test]
    fn ambiguous_nn_nn_yyyy_is_not_guessed() {
        // Neither part clearly a month vs a day -> not in our accepted-format
        // list at all -> falls to Text at the column level.
        let (kind, _, _) = infer_column("d", &["01/02/2026".into(), "03/04/2026".into()]);
        assert_eq!(kind, ColumnKind::Text);
    }

    #[test]
    fn mixed_real_dates_plus_one_non_date_falls_to_text() {
        let (kind, _, _) = infer_column(
            "d",
            &[
                "2026-09-24".into(),
                "2026-09-25".into(),
                "not a date".into(),
            ],
        );
        assert_eq!(kind, ColumnKind::Text);
    }

    #[test]
    fn one_bad_cell_in_numeric_column_falls_to_text() {
        let (kind, _, _) = infer_column("n", &["1".into(), "2".into(), "N/A".into()]);
        assert_eq!(kind, ColumnKind::Text);
    }

    #[test]
    fn all_empty_column_is_text() {
        let (kind, cells, _) = infer_column("e", &["".into(), "  ".into()]);
        assert_eq!(kind, ColumnKind::Text);
        assert_eq!(cells, vec!["", ""]);
    }

    #[test]
    fn empty_cells_never_veto_numeric_type() {
        let (kind, cells, _) = infer_column("n", &["1".into(), "".into(), "3".into()]);
        assert_eq!(kind, ColumnKind::Integer);
        assert_eq!(cells, vec!["1", "", "3"]);
    }

    #[test]
    fn integer_column_with_a_genuine_float_becomes_float() {
        let (kind, cells, _) = infer_column("n", &["1".into(), "2.5".into(), "3".into()]);
        assert_eq!(kind, ColumnKind::Float);
        assert_eq!(cells, vec!["1", "2.5", "3"]);
    }
}
