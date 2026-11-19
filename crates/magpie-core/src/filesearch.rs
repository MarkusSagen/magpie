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

/// While the search field holds `in:<a>,<b>,<partial>` with no space yet (the user is
/// still typing folders), return `(head, fragment)`: everything before the fragment
/// being typed, and the fragment itself. `in:~/a,~/co` → `("in:~/a,", "~/co")`.
pub fn location_fragment(raw: &str) -> Option<(&str, &str)> {
    let rest = raw.strip_prefix("in:")?;
    if rest.contains(char::is_whitespace) {
        return None;
    }
    let cut = raw.rfind(',').map(|i| i + 1).unwrap_or(3);
    Some((&raw[..cut], &raw[cut..]))
}

/// Convert a sed replacement into a `regex` crate template: `\\N` → `${N}`,
/// `&` → `${0}` (whole match), `\\&` → literal `&`, `\\\\` → `\\`. `$1` / `${name}`
/// pass through unchanged, so both sed and regex-crate styles work.
pub fn sed_template(rep: &str) -> String {
    let mut out = String::with_capacity(rep.len() + 8);
    let mut it = rep.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '\\' => match it.next() {
                Some(d) if d.is_ascii_digit() => {
                    out.push_str("${");
                    out.push(d);
                    out.push('}');
                }
                Some('&') => out.push('&'),
                Some('\\') => out.push('\\'),
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some(o) => {
                    out.push('\\');
                    out.push(o);
                }
                None => out.push('\\'),
            },
            '&' => out.push_str("${0}"),
            o => out.push(o),
        }
    }
    out
}

/// Apply a sed substitution **line by line** (sed semantics: `^`/`$` anchor to each
/// line; without `g` only the first match per line). Line terminators (`\n` /
/// `\r\n`) and a missing final newline are preserved byte-for-byte. `template` is
/// the output of [`sed_template`]. This is the single source of truth for both the
/// replace preview and the write, so what you see is what gets written.
pub fn sed_apply(content: &str, re: &regex::Regex, template: &str, global: bool) -> String {
    let mut out = String::with_capacity(content.len());
    for chunk in content.split_inclusive('\n') {
        let (body, end) = if let Some(b) = chunk.strip_suffix("\r\n") {
            (b, "\r\n")
        } else if let Some(b) = chunk.strip_suffix('\n') {
            (b, "\n")
        } else {
            (chunk, "")
        };
        if global {
            out.push_str(&re.replace_all(body, template));
        } else {
            out.push_str(&re.replace(body, template));
        }
        out.push_str(end);
    }
    out
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

    #[test]
    fn sed_template_table() {
        assert_eq!(sed_template(r"\1-\2"), "${1}-${2}");
        assert_eq!(sed_template("$1"), "$1");
        assert_eq!(sed_template("[&]"), "[${0}]");
        assert_eq!(sed_template(r"a\&b"), "a&b");
        assert_eq!(sed_template(r"a\\b"), r"a\b");
        assert_eq!(sed_template("plain"), "plain");
    }

    #[test]
    fn sed_apply_is_per_line_and_preserves_endings() {
        let re = regex::Regex::new("^foo").unwrap();
        let t = sed_template("bar");
        // `^` anchors per line (whole-file replace would only touch line 1).
        assert_eq!(
            sed_apply("foo\nfoo x\nxfoo\n", &re, &t, false),
            "bar\nbar x\nxfoo\n"
        );
        // CRLF + no trailing newline preserved.
        assert_eq!(sed_apply("foo\r\nfoo", &re, &t, false), "bar\r\nbar");
        // Without g: first match per LINE, not per file.
        let re = regex::Regex::new("a").unwrap();
        assert_eq!(sed_apply("aa\naa\n", &re, "b", false), "ba\nba\n");
        assert_eq!(sed_apply("aa\naa\n", &re, "b", true), "bb\nbb\n");
        // Capture groups via sed-style refs.
        let re = regex::Regex::new(r"(\w+)@(\w+)").unwrap();
        assert_eq!(
            sed_apply("me@host\n", &re, &sed_template(r"\2:\1"), true),
            "host:me\n"
        );
        // Empty input stays empty.
        assert_eq!(sed_apply("", &re, "x", true), "");
    }

    #[test]
    fn location_fragment_table() {
        assert_eq!(location_fragment("in:"), Some(("in:", "")));
        assert_eq!(location_fragment("in:~/co"), Some(("in:", "~/co")));
        assert_eq!(location_fragment("in:~/a,~/co"), Some(("in:~/a,", "~/co")));
        assert_eq!(location_fragment("in:~/a,"), Some(("in:~/a,", "")));
        assert_eq!(location_fragment("in:~/a foo"), None); // query started
        assert_eq!(location_fragment("foo"), None);
    }
}
