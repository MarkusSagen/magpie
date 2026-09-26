//! Pure file-search helpers (Phase 15): the `s/pattern/replacement/flags` sed
//! parser and case-mode logic. The actual filesystem walk + content matching live
//! in the app (ripgrep's `ignore`/`grep-*`); everything testable without IO is here.

/// How the query text is interpreted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchMode {
    /// Literal substring (still regex-escaped for the engine).
    Text,
    /// The query is a regular expression.
    Regex,
    /// The query is a `s/pat/rep/flags` substitution (search + replace preview).
    Sed,
}

/// Case sensitivity selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaseMode {
    /// Case-insensitive unless the query contains an uppercase letter (ripgrep's default).
    Smart,
    Sensitive,
    Insensitive,
}

impl CaseMode {
    /// Resolve to a concrete case-insensitive flag for `query`.
    pub fn is_insensitive(self, query: &str) -> bool {
        match self {
            CaseMode::Sensitive => false,
            CaseMode::Insensitive => true,
            CaseMode::Smart => !query.chars().any(|c| c.is_uppercase()),
        }
    }
}

/// A parsed `s/pattern/replacement/flags` command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SedCommand {
    pub pattern: String,
    pub replacement: String,
    /// `g` — replace every match on a line (else the first).
    pub global: bool,
    /// `i` — case-insensitive.
    pub case_insensitive: bool,
}

/// Parse `s/pat/rep/flags`. The delimiter is the character after `s` (usually `/`);
/// `\<delim>` inside a field is an escaped literal delimiter. Returns `None` if the
/// shape is not a substitution. Capture-group refs (`$1`, `\1`) are left intact for
/// the regex replacer to interpret.
pub fn parse_sed(input: &str) -> Option<SedCommand> {
    let s = input.trim();
    let mut chars = s.chars();
    if chars.next()? != 's' {
        return None;
    }
    let delim = chars.next()?;
    if delim.is_alphanumeric() || delim.is_whitespace() {
        return None;
    }
    // Split the remainder into up to 3 fields on unescaped delimiters.
    let rest: Vec<char> = chars.collect();
    let mut fields: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut i = 0;
    while i < rest.len() {
        let c = rest[i];
        if c == '\\' && i + 1 < rest.len() && rest[i + 1] == delim {
            cur.push(delim);
            i += 2;
            continue;
        }
        if c == delim {
            fields.push(std::mem::take(&mut cur));
            i += 1;
            continue;
        }
        cur.push(c);
        i += 1;
    }
    fields.push(cur);
    // Expect: pattern, replacement, [flags].
    if fields.len() < 2 {
        return None;
    }
    let pattern = fields[0].clone();
    if pattern.is_empty() {
        return None;
    }
    let replacement = fields[1].clone();
    let flags = fields.get(2).cloned().unwrap_or_default();
    Some(SedCommand {
        pattern,
        replacement,
        global: flags.contains('g'),
        case_insensitive: flags.contains('i'),
    })
}

/// Split a leading `in:` / location prefix off a raw search-bar string, returning
/// `(locations, query)`. `in:~/a,~/b term` → (["~/a","~/b"], "term"). Without an
/// `in:` prefix, locations is empty and the whole string is the query.
pub fn parse_locations_prefix(raw: &str) -> (Vec<String>, String) {
    let s = raw.trim_start();
    if let Some(rest) = s.strip_prefix("in:") {
        // Locations run until the first space; the rest is the query.
        let (locs, query) = match rest.split_once(char::is_whitespace) {
            Some((l, q)) => (l, q.trim()),
            None => (rest, ""),
        };
        let locations = locs
            .split(',')
            .map(|p| p.trim().to_string())
            .filter(|p| !p.is_empty())
            .collect();
        (locations, query.to_string())
    } else {
        (Vec::new(), raw.trim().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smart_case() {
        assert!(CaseMode::Smart.is_insensitive("foo"));
        assert!(!CaseMode::Smart.is_insensitive("Foo"));
        assert!(!CaseMode::Sensitive.is_insensitive("foo"));
        assert!(CaseMode::Insensitive.is_insensitive("Foo"));
    }

    #[test]
    fn sed_basic() {
        let c = parse_sed("s/foo/bar/g").unwrap();
        assert_eq!(c.pattern, "foo");
        assert_eq!(c.replacement, "bar");
        assert!(c.global);
        assert!(!c.case_insensitive);
    }

    #[test]
    fn sed_flags_and_capture_refs() {
        let c = parse_sed("s/(\\w+)@(\\w+)/$2.$1/gi").unwrap();
        assert_eq!(c.pattern, "(\\w+)@(\\w+)");
        assert_eq!(c.replacement, "$2.$1");
        assert!(c.global && c.case_insensitive);
    }

    #[test]
    fn sed_escaped_delimiter_and_empty_replacement() {
        let c = parse_sed("s/a\\/b//").unwrap();
        assert_eq!(c.pattern, "a/b");
        assert_eq!(c.replacement, "");
    }

    #[test]
    fn sed_rejects_non_sed() {
        assert!(parse_sed("hello world").is_none());
        assert!(parse_sed("s/only").is_none());
        assert!(parse_sed("sfoo").is_none());
    }

    #[test]
    fn locations_prefix() {
        let (locs, q) = parse_locations_prefix("in:~/a,~/b user_version");
        assert_eq!(locs, vec!["~/a".to_string(), "~/b".to_string()]);
        assert_eq!(q, "user_version");
        let (locs2, q2) = parse_locations_prefix("just a query");
        assert!(locs2.is_empty());
        assert_eq!(q2, "just a query");
    }
}
