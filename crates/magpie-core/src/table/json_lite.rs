//! Hand-rolled JSON tokenizer/parser (object/array/string/number/bool/null
//! only — spec §1.2) + JSON export (§3). No `serde_json`.

use super::{
    build_table, limits, synthetic_header, Column, ColumnKind, DetectedFormat, ParseError,
    ParseWarning, Table,
};
use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum JsonValue {
    Null,
    Bool(bool),
    Number(String),
    String(String),
    Array(Vec<JsonValue>),
    Object(Vec<(String, JsonValue)>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct JsonError {
    pub reason: String,
    pub at_line: usize,
}

struct Parser {
    chars: Vec<char>,
    i: usize,
    line: usize,
}

impl Parser {
    fn new(s: &str) -> Self {
        Parser {
            chars: s.chars().collect(),
            i: 0,
            line: 1,
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.i).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.i += 1;
        if c == '\n' {
            self.line += 1;
        }
        Some(c)
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(' ' | '\t' | '\r' | '\n')) {
            self.bump();
        }
    }

    fn err(&self, reason: &str) -> JsonError {
        JsonError {
            reason: reason.to_string(),
            at_line: self.line,
        }
    }

    fn parse_value(&mut self) -> Result<JsonValue, JsonError> {
        self.skip_ws();
        match self.peek() {
            Some('{') => self.parse_object(),
            Some('[') => self.parse_array(),
            Some('"') => self.parse_string().map(JsonValue::String),
            Some('t') => self.parse_literal("true", JsonValue::Bool(true)),
            Some('f') => self.parse_literal("false", JsonValue::Bool(false)),
            Some('n') => self.parse_literal("null", JsonValue::Null),
            Some(c) if c == '-' || c.is_ascii_digit() => self.parse_number(),
            Some(c) => Err(self.err(&format!("unexpected character '{c}'"))),
            None => Err(self.err("unexpected end of input")),
        }
    }

    fn parse_literal(&mut self, word: &str, value: JsonValue) -> Result<JsonValue, JsonError> {
        for expected in word.chars() {
            match self.bump() {
                Some(c) if c == expected => {}
                _ => return Err(self.err(&format!("expected literal '{word}'"))),
            }
        }
        Ok(value)
    }

    fn parse_string(&mut self) -> Result<String, JsonError> {
        self.bump(); // opening quote
        let mut out = String::new();
        loop {
            match self.bump() {
                None => return Err(self.err("unterminated string")),
                Some('"') => return Ok(out),
                Some('\\') => match self.bump() {
                    Some('"') => out.push('"'),
                    Some('\\') => out.push('\\'),
                    Some('/') => out.push('/'),
                    Some('n') => out.push('\n'),
                    Some('t') => out.push('\t'),
                    Some('r') => out.push('\r'),
                    Some('b') => out.push('\u{8}'),
                    Some('f') => out.push('\u{c}'),
                    Some('u') => {
                        let mut code = 0u32;
                        for _ in 0..4 {
                            let d = self.bump().ok_or_else(|| self.err("bad \\u escape"))?;
                            let digit = d.to_digit(16).ok_or_else(|| self.err("bad \\u escape"))?;
                            code = code * 16 + digit;
                        }
                        out.push(char::from_u32(code).unwrap_or('\u{FFFD}'));
                    }
                    _ => return Err(self.err("bad escape sequence")),
                },
                Some(c) => out.push(c),
            }
        }
    }

    fn parse_number(&mut self) -> Result<JsonValue, JsonError> {
        let start = self.i;
        if self.peek() == Some('-') {
            self.bump();
        }
        if !matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
            return Err(self.err("invalid number"));
        }
        if self.peek() == Some('0') {
            self.bump();
        } else {
            while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                self.bump();
            }
        }
        if self.peek() == Some('.') {
            self.bump();
            if !matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                return Err(self.err("invalid number"));
            }
            while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                self.bump();
            }
        }
        if matches!(self.peek(), Some('e' | 'E')) {
            self.bump();
            if matches!(self.peek(), Some('+' | '-')) {
                self.bump();
            }
            if !matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                return Err(self.err("invalid number"));
            }
            while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                self.bump();
            }
        }
        let text: String = self.chars[start..self.i].iter().collect();
        Ok(JsonValue::Number(text))
    }

    fn parse_array(&mut self) -> Result<JsonValue, JsonError> {
        self.bump(); // '['
        let mut items = Vec::new();
        self.skip_ws();
        if self.peek() == Some(']') {
            self.bump();
            return Ok(JsonValue::Array(items));
        }
        loop {
            items.push(self.parse_value()?);
            self.skip_ws();
            match self.bump() {
                Some(',') => {
                    self.skip_ws();
                    if self.peek() == Some(']') {
                        return Err(self.err("trailing comma before ']'"));
                    }
                }
                Some(']') => break,
                _ => return Err(self.err("expected ',' or ']'")),
            }
        }
        Ok(JsonValue::Array(items))
    }

    fn parse_object(&mut self) -> Result<JsonValue, JsonError> {
        self.bump(); // '{'
        let mut items = Vec::new();
        self.skip_ws();
        if self.peek() == Some('}') {
            self.bump();
            return Ok(JsonValue::Object(items));
        }
        loop {
            self.skip_ws();
            if self.peek() != Some('"') {
                return Err(self.err("expected a quoted string key"));
            }
            let key = self.parse_string()?;
            self.skip_ws();
            if self.bump() != Some(':') {
                return Err(self.err("expected ':' after object key"));
            }
            let value = self.parse_value()?;
            items.push((key, value));
            self.skip_ws();
            match self.bump() {
                Some(',') => {
                    self.skip_ws();
                    if self.peek() == Some('}') {
                        return Err(self.err("trailing comma before '}'"));
                    }
                }
                Some('}') => break,
                _ => return Err(self.err("expected ',' or '}'")),
            }
        }
        Ok(JsonValue::Object(items))
    }
}

pub(crate) fn parse_json(input: &str) -> Result<JsonValue, JsonError> {
    let mut p = Parser::new(input);
    let v = p.parse_value()?;
    p.skip_ws();
    if p.i != p.chars.len() {
        return Err(p.err("trailing content after JSON value"));
    }
    Ok(v)
}

fn cell_text(v: &JsonValue) -> String {
    match v {
        JsonValue::Null => String::new(),
        JsonValue::Bool(b) => if *b { "true" } else { "false" }.to_string(),
        JsonValue::Number(n) => n.clone(),
        JsonValue::String(s) => s.clone(),
        JsonValue::Array(_) | JsonValue::Object(_) => stringify_compact(v),
    }
}

/// Compact (no-space) JSON re-serialization, used to stringify a nested
/// object/array value into one Text cell (spec §1.2 — nested structures are
/// not flattened/dotted in v1).
fn stringify_compact(v: &JsonValue) -> String {
    match v {
        JsonValue::Null => "null".to_string(),
        JsonValue::Bool(b) => b.to_string(),
        JsonValue::Number(n) => n.clone(),
        JsonValue::String(s) => format!("\"{}\"", crate::export::json_escape(s)),
        JsonValue::Array(items) => {
            let parts: Vec<String> = items.iter().map(stringify_compact).collect();
            format!("[{}]", parts.join(","))
        }
        JsonValue::Object(entries) => {
            let parts: Vec<String> = entries
                .iter()
                .map(|(k, v)| {
                    format!(
                        "\"{}\":{}",
                        crate::export::json_escape(k),
                        stringify_compact(v)
                    )
                })
                .collect();
            format!("{{{}}}", parts.join(","))
        }
    }
}

fn objects_to_grid(items: &[JsonValue]) -> (Vec<String>, Vec<Vec<String>>, Vec<ParseWarning>) {
    let mut keys: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for item in items {
        if let JsonValue::Object(entries) = item {
            for (k, _) in entries {
                if seen.insert(k.clone()) {
                    keys.push(k.clone());
                }
            }
        }
    }
    let mut differing = false;
    let mut rows = Vec::with_capacity(items.len());
    for item in items {
        if let JsonValue::Object(entries) = item {
            if !keys.iter().all(|k| entries.iter().any(|(ek, _)| ek == k)) {
                differing = true;
            }
            let row: Vec<String> = keys
                .iter()
                .map(|k| {
                    entries
                        .iter()
                        .find(|(ek, _)| ek == k)
                        .map(|(_, v)| cell_text(v))
                        .unwrap_or_default()
                })
                .collect();
            rows.push(row);
        }
    }
    let warnings = if differing {
        vec![ParseWarning::DifferingKeys]
    } else {
        Vec::new()
    };
    (keys, rows, warnings)
}

fn arrays_to_grid(items: &[JsonValue]) -> (Vec<String>, Vec<Vec<String>>) {
    let rows_raw: Vec<Vec<String>> = items
        .iter()
        .map(|v| match v {
            JsonValue::Array(cells) => cells.iter().map(cell_text).collect(),
            other => vec![cell_text(other)],
        })
        .collect();
    if rows_raw.is_empty() {
        return (Vec::new(), Vec::new());
    }
    let row_all_strings = |v: &JsonValue| match v {
        JsonValue::Array(cells) => cells.iter().all(|c| matches!(c, JsonValue::String(_))),
        _ => false,
    };
    let first_all_strings = row_all_strings(&items[0]);
    let others_all_strings = items.len() > 1 && items[1..].iter().all(row_all_strings);
    let header_present = first_all_strings && !others_all_strings;
    if header_present {
        (rows_raw[0].clone(), rows_raw[1..].to_vec())
    } else {
        let ncols = rows_raw.iter().map(|r| r.len()).max().unwrap_or(0);
        (synthetic_header(ncols), rows_raw)
    }
}

/// Parse JSON text into a typed `Table` (spec §1.2). Two accepted shapes:
/// array-of-objects (union-of-keys columns) and array-of-arrays (`Col N`
/// headers, or a detected header row). A bare object is only tabular if it
/// has exactly one array-valued key (`{"rows": [...]}`).
pub(crate) fn parse_json_table(input: &str) -> Result<Table, ParseError> {
    let value = parse_json(input).map_err(|e| ParseError::Malformed {
        format: DetectedFormat::Json,
        reason: e.reason,
        at_line: e.at_line,
    })?;

    let (array, wrapper): (Vec<JsonValue>, Option<String>) = match value {
        JsonValue::Array(items) => (items, None),
        JsonValue::Object(entries) => {
            let array_entries: Vec<&(String, JsonValue)> = entries
                .iter()
                .filter(|(_, v)| matches!(v, JsonValue::Array(_)))
                .collect();
            if entries.len() == 1 && array_entries.len() == 1 {
                match entries.into_iter().next() {
                    Some((k, JsonValue::Array(items))) => (items, Some(k)),
                    _ => unreachable!(),
                }
            } else {
                return Err(ParseError::NoTabularStructure {
                    format: DetectedFormat::Json,
                });
            }
        }
        _ => {
            return Err(ParseError::NoTabularStructure {
                format: DetectedFormat::Json,
            })
        }
    };

    let mut warnings = Vec::new();
    if let Some(k) = wrapper {
        warnings.push(ParseWarning::WrapperKeyUsed { key: k });
    }

    if array.is_empty() {
        return Err(ParseError::NoTabularStructure {
            format: DetectedFormat::Json,
        });
    }

    let truncated = array.len() > limits::MAX_ROWS;
    let capped: Vec<JsonValue> = array.into_iter().take(limits::MAX_ROWS).collect();

    let all_objects = capped.iter().all(|v| matches!(v, JsonValue::Object(_)));
    let all_arrays = capped.iter().all(|v| matches!(v, JsonValue::Array(_)));

    if all_objects {
        let (header, rows, key_warnings) = objects_to_grid(&capped);
        warnings.extend(key_warnings);
        Ok(build_table(
            header,
            rows,
            DetectedFormat::Json,
            warnings,
            truncated,
        ))
    } else if all_arrays {
        let (header, rows) = arrays_to_grid(&capped);
        Ok(build_table(
            header,
            rows,
            DetectedFormat::Json,
            warnings,
            truncated,
        ))
    } else {
        Err(ParseError::NoTabularStructure {
            format: DetectedFormat::Json,
        })
    }
}

/// Array-of-objects JSON export: native JSON types matching each column's
/// inferred kind; empty cells become `null` (spec §3).
pub(crate) fn export_json(columns: &[Column]) -> String {
    let row_count = columns.first().map(|c| c.cells.len()).unwrap_or(0);
    let mut out = String::from("[");
    for r in 0..row_count {
        if r > 0 {
            out.push(',');
        }
        out.push('{');
        for (i, c) in columns.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push('"');
            out.push_str(&crate::export::json_escape(&c.name));
            out.push_str("\":");
            let cell = &c.cells[r];
            if cell.is_empty() {
                out.push_str("null");
            } else {
                match c.kind {
                    ColumnKind::Integer | ColumnKind::Float => out.push_str(cell),
                    ColumnKind::Bool => out.push_str(cell),
                    ColumnKind::Date | ColumnKind::Text => {
                        out.push('"');
                        out.push_str(&crate::export::json_escape(cell));
                        out.push('"');
                    }
                }
            }
        }
        out.push('}');
    }
    out.push(']');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_all_scalar_kinds() {
        let v = parse_json(r#"{"a":1,"b":1.5,"c":"x","d":true,"e":null}"#).unwrap();
        match v {
            JsonValue::Object(entries) => assert_eq!(entries.len(), 5),
            _ => panic!("expected object"),
        }
    }

    #[test]
    fn trailing_comma_is_a_hard_error() {
        let e = parse_json(r#"[{"a":1},]"#).unwrap_err();
        assert!(e.reason.contains("comma"));
    }

    #[test]
    fn unterminated_string_is_a_hard_error() {
        assert!(parse_json(r#"{"a": "oops}"#).is_err());
    }

    #[test]
    fn array_of_objects_union_of_keys_and_missing_key_blank() {
        let t = parse_json_table(r#"[{"a":1,"b":"x"},{"a":2}]"#).unwrap();
        assert_eq!(t.columns.len(), 2);
        assert_eq!(t.columns[1].name, "b");
        assert_eq!(t.columns[1].cells, vec!["x", ""]);
        assert!(t
            .warnings
            .iter()
            .any(|w| matches!(w, ParseWarning::DifferingKeys)));
    }

    #[test]
    fn nested_value_is_stringified_as_text() {
        let t = parse_json_table(r#"[{"c":[1,2]}]"#).unwrap();
        assert_eq!(t.columns[0].kind, ColumnKind::Text);
        assert_eq!(t.columns[0].cells[0], "[1,2]");
    }

    #[test]
    fn array_of_arrays_with_string_header() {
        let t = parse_json_table(r#"[["name","age"],["Ann",30]]"#).unwrap();
        assert_eq!(t.columns[0].name, "name");
        assert_eq!(t.columns[1].kind, ColumnKind::Integer);
        assert_eq!(t.row_count, 1);
    }

    #[test]
    fn array_of_arrays_headerless() {
        let t = parse_json_table(r#"[[1,2],[3,4]]"#).unwrap();
        assert_eq!(t.columns[0].name, "Col 1");
        assert_eq!(t.row_count, 2);
    }

    #[test]
    fn wrapper_object_with_one_array_key() {
        let t = parse_json_table(r#"{"rows":[{"a":1},{"a":2}]}"#).unwrap();
        assert_eq!(t.row_count, 2);
        assert!(t
            .warnings
            .iter()
            .any(|w| matches!(w, ParseWarning::WrapperKeyUsed { key } if key == "rows")));
    }

    #[test]
    fn bare_object_without_single_array_key_is_not_tabular() {
        let err = parse_json_table(r#"{"a":1,"b":2}"#).unwrap_err();
        assert!(matches!(err, ParseError::NoTabularStructure { .. }));
    }

    #[test]
    fn export_json_uses_native_types_and_null_for_empty() {
        let columns = vec![
            Column {
                name: "n".into(),
                kind: ColumnKind::Integer,
                cells: vec!["1".into(), "".into()],
            },
            Column {
                name: "t".into(),
                kind: ColumnKind::Text,
                cells: vec!["a".into(), "b".into()],
            },
        ];
        let out = export_json(&columns);
        assert_eq!(out, r#"[{"n":1,"t":"a"},{"n":null,"t":"b"}]"#);
    }
}
