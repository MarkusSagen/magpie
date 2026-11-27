//! One catalog and bounded, pure pipeline for clipboard and selection actions.
use crate::{transforms, Store};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    SortAscending,
    SortDescending,
    Reverse,
    Unique,
    Trim,
    RemoveEmpty,
    Whitespace,
    Split,
    Join,
    Prefix,
    Suffix,
    RemovePrefix,
    RemoveSuffix,
    Number,
    Replace,
    Camel,
    Pascal,
    Snake,
    Kebab,
    Screaming,
    Train,
    Title,
    Upper,
    Lower,
    Sql,
    Base64Encode,
    Base64Decode,
    UrlEncode,
    UrlDecode,
    HtmlEncode,
    HtmlDecode,
    StripTags,
    Sha256,
    Md5,
    Count,
}

pub struct Definition {
    pub action: Action,
    pub id: &'static str,
    pub label: &'static str,
    pub group: &'static str,
    pub first: &'static str,
    pub second: &'static str,
    pub default_text: &'static str,
}
macro_rules! catalog {
    ($(($variant:ident, $id:literal, $label:literal, $group:literal, $a:literal, $b:literal, $default:literal)),* $(,)?) => {
        pub const CATALOG: &[Definition] = &[$(Definition { action: Action::$variant, id: $id, label: $label, group: $group, first: $a, second: $b, default_text: $default }),*];
    }
}
catalog![
    (
        SortAscending,
        "sort_ascending",
        "Sort ascending",
        "Lines",
        "",
        "",
        ""
    ),
    (
        SortDescending,
        "sort_descending",
        "Sort descending",
        "Lines",
        "",
        "",
        ""
    ),
    (
        Reverse,
        "reverse",
        "Reverse line order",
        "Lines",
        "",
        "",
        ""
    ),
    (
        Unique,
        "unique",
        "Remove duplicate lines",
        "Lines",
        "",
        "",
        ""
    ),
    (Trim, "trim", "Trim each line", "Clean", "", "", ""),
    (
        RemoveEmpty,
        "remove_empty",
        "Remove empty lines",
        "Clean",
        "",
        "",
        ""
    ),
    (
        Whitespace,
        "whitespace",
        "Normalize whitespace",
        "Clean",
        "",
        "",
        ""
    ),
    (
        Split,
        "split",
        "Split into lines",
        "Lines",
        "Separator (literal)",
        "",
        ","
    ),
    (
        Join,
        "join",
        "Join lines",
        "Lines",
        "Separator (\\n and \\t supported)",
        "",
        ", "
    ),
    (
        Prefix,
        "prefix",
        "Add prefix / bullets",
        "Lines",
        "Prefix",
        "",
        "- "
    ),
    (Suffix, "suffix", "Add suffix", "Lines", "Suffix", "", ""),
    (
        RemovePrefix,
        "remove_prefix",
        "Remove prefix",
        "Lines",
        "Prefix",
        "",
        "- "
    ),
    (
        RemoveSuffix,
        "remove_suffix",
        "Remove suffix",
        "Lines",
        "Suffix",
        "",
        ""
    ),
    (
        Number,
        "number",
        "Number lines",
        "Lines",
        "Starting number",
        "",
        "1"
    ),
    (
        Replace,
        "replace",
        "Find and replace",
        "Clean",
        "Find (enable regex below for patterns)",
        "Replace with",
        ""
    ),
    (Camel, "camel", "camelCase", "Case", "", "", ""),
    (Pascal, "pascal", "PascalCase", "Case", "", "", ""),
    (Snake, "snake", "snake_case", "Case", "", "", ""),
    (Kebab, "kebab", "kebab-case", "Case", "", "", ""),
    (
        Screaming,
        "screaming",
        "SCREAMING_SNAKE",
        "Case",
        "",
        "",
        ""
    ),
    (Train, "train", "Train-Case", "Case", "", "", ""),
    (Title, "title", "Title Case", "Case", "", "", ""),
    (Upper, "upper", "UPPERCASE", "Case", "", "", ""),
    (Lower, "lower", "lowercase", "Case", "", "", ""),
    (Sql, "sql", "SQL IN clause", "Format", "", "", ""),
    (
        Base64Encode,
        "base64_encode",
        "Base64 encode",
        "Encode",
        "",
        "",
        ""
    ),
    (
        Base64Decode,
        "base64_decode",
        "Base64 decode",
        "Encode",
        "",
        "",
        ""
    ),
    (UrlEncode, "url_encode", "URL encode", "Encode", "", "", ""),
    (UrlDecode, "url_decode", "URL decode", "Encode", "", "", ""),
    (
        HtmlEncode,
        "html_encode",
        "HTML escape",
        "Encode",
        "",
        "",
        ""
    ),
    (
        HtmlDecode,
        "html_decode",
        "HTML unescape",
        "Encode",
        "",
        "",
        ""
    ),
    (StripTags, "strip_tags", "Strip tags", "Clean", "", "", ""),
    (Sha256, "sha256", "SHA-256", "Other", "", "", ""),
    (Md5, "md5", "MD5", "Other", "", "", ""),
    (
        Count,
        "count",
        "Count words, lines and characters",
        "Other",
        "",
        "",
        ""
    ),
];
impl Action {
    pub fn definition(self) -> &'static Definition {
        CATALOG
            .iter()
            .find(|d| d.action == self)
            .expect("catalog covers Action")
    }
    pub fn from_id(id: &str) -> Option<Self> {
        CATALOG.iter().find(|d| d.id == id).map(|d| d.action)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub action: Action,
    pub text: String,
    pub replacement: String,
    pub case_sensitive: bool,
    pub regex: bool,
    pub enabled: bool,
}
impl Step {
    pub fn new(action: Action) -> Self {
        Self {
            action,
            text: action.definition().default_text.into(),
            replacement: String::new(),
            case_sensitive: true,
            regex: false,
            enabled: true,
        }
    }
}
fn bounded(text: String) -> Result<String, String> {
    if text.len() > transforms::MAX_INPUT_BYTES {
        Err("Text exceeds the 2 MiB limit".into())
    } else {
        Ok(text)
    }
}
// Build incrementally: even an enormous prefix or replacement cannot amplify a
// small input into an unbounded allocation before the output limit is checked.
fn collect_bounded(
    parts: impl IntoIterator<Item = String>,
    separator: &str,
) -> Result<String, String> {
    let mut out = String::new();
    for (i, part) in parts.into_iter().enumerate() {
        let sep = if i == 0 { "" } else { separator };
        if out
            .len()
            .saturating_add(sep.len())
            .saturating_add(part.len())
            > transforms::MAX_INPUT_BYTES
        {
            return Err("Result exceeds the 2 MiB limit".into());
        }
        out.push_str(sep);
        out.push_str(&part);
    }
    Ok(out)
}
pub fn evaluate(input: &str, steps: &[Step]) -> Result<String, String> {
    if steps.len() > 64 {
        return Err("A cleaner can contain at most 64 steps".into());
    }
    let mut result = bounded(input.to_owned())?;
    for (i, step) in steps.iter().enumerate() {
        if step.enabled {
            result = apply(&result, step)
                .map_err(|e| format!("Step {} ({}): {e}", i + 1, step.action.definition().label))?;
        }
    }
    Ok(result)
}
fn apply(input: &str, s: &Step) -> Result<String, String> {
    use transforms::{Mode, SortOrder, SortUniqueTrimOptions};
    use Action::*;
    let lines = || {
        input
            .split_terminator('\n')
            .map(|l| l.strip_suffix('\r').unwrap_or(l))
    };
    let map_lines = |f: &dyn Fn(&str) -> String| {
        let mut out = collect_bounded(lines().map(f), "\n")?;
        if input.ends_with('\n') && !out.is_empty() {
            out.push('\n');
        }
        bounded(out)
    };
    let existing = |kind| transforms::run(kind, input).map_err(|e| e.to_string());
    let value = match s.action {
        SortAscending | SortDescending | Unique => {
            let trailing = input.ends_with('\n');
            let mut text = transforms::sort_unique_trim(
                input,
                &SortUniqueTrimOptions {
                    sort: match s.action {
                        SortAscending => Some(SortOrder::Ascending),
                        SortDescending => Some(SortOrder::Descending),
                        _ => None,
                    },
                    unique: s.action == Unique,
                    trim: false,
                    case_sensitive: s.case_sensitive,
                },
            )
            .map_err(|e| e.to_string())?;
            if trailing && !text.is_empty() {
                text.push('\n');
            }
            text
        }
        Reverse => {
            let mut v = lines().collect::<Vec<_>>();
            v.reverse();
            let mut out = v.join("\n");
            if input.ends_with('\n') && !out.is_empty() {
                out.push('\n');
            }
            out
        }
        Trim => return map_lines(&|l| l.trim().into()),
        RemoveEmpty => {
            let mut out = lines()
                .filter(|l| !l.is_empty())
                .collect::<Vec<_>>()
                .join("\n");
            if input.ends_with('\n') && !out.is_empty() {
                out.push('\n');
            }
            out
        }
        Whitespace => return map_lines(&|l| l.split_whitespace().collect::<Vec<_>>().join(" ")),
        Split => {
            if s.text.is_empty() {
                return Err("Choose a non-empty separator".into());
            }
            return collect_bounded(input.split(&s.text).map(str::to_owned), "\n");
        }
        Join => {
            return collect_bounded(
                lines().map(str::to_owned),
                &transforms::unescape_sep(&s.text),
            )
        }
        Prefix | Suffix | RemovePrefix | RemoveSuffix => {
            bounded(s.text.clone())?;
            return map_lines(&|l| match s.action {
                Prefix => format!("{}{l}", s.text),
                Suffix => format!("{l}{}", s.text),
                RemovePrefix => l.strip_prefix(&s.text).unwrap_or(l).into(),
                _ => l.strip_suffix(&s.text).unwrap_or(l).into(),
            });
        }
        Number => {
            let start = s
                .text
                .parse::<i64>()
                .map_err(|_| "Starting number must be an integer")?;
            let count = lines().count();
            if count > 0 {
                start
                    .checked_add((count - 1) as i64)
                    .ok_or("Number overflow")?;
            }
            let mut out = collect_bounded(
                lines()
                    .enumerate()
                    .map(|(i, l)| format!("{}. {l}", start + i as i64)),
                "\n",
            )?;
            if input.ends_with('\n') && !out.is_empty() {
                out.push('\n');
            }
            return bounded(out);
        }
        Replace => {
            bounded(s.text.clone())?;
            if s.text.is_empty() {
                return Err("Find must not be empty".into());
            }
            let pattern = if s.regex {
                s.text.clone()
            } else {
                regex::escape(&s.text)
            };
            let re = regex::RegexBuilder::new(&pattern)
                .case_insensitive(!s.case_sensitive)
                .build()
                .map_err(|e| e.to_string())?;
            bounded(s.replacement.clone())?;
            let mut out = String::new();
            let mut end = 0;
            for caps in re.captures_iter(input) {
                let m = caps.get(0).ok_or("Invalid match")?;
                let mut replacement = String::new();
                if s.regex {
                    replacement = expand_bounded(&caps, &s.replacement)?;
                } else {
                    replacement.push_str(&s.replacement);
                }
                if out
                    .len()
                    .saturating_add(m.start() - end)
                    .saturating_add(replacement.len())
                    > transforms::MAX_INPUT_BYTES
                {
                    return Err("Result exceeds the 2 MiB limit".into());
                }
                out.push_str(&input[end..m.start()]);
                out.push_str(&replacement);
                end = m.end();
            }
            out.push_str(&input[end..]);
            out
        }
        Upper => input.to_uppercase(),
        Lower => input.to_lowercase(),
        Camel | Pascal | Snake | Kebab | Screaming | Train | Title => {
            let n = match s.action {
                Camel => 0,
                Pascal => 1,
                Snake => 2,
                Kebab => 3,
                Screaming => 4,
                Train => 5,
                Title => 6,
                Upper => 7,
                _ => 8,
            };
            crate::case::all_cases(input)[n].1.clone()
        }
        Sql => existing(transforms::TransformKind::SqlInClause)?,
        Base64Encode | Base64Decode => transforms::base64_transform(
            input,
            &transforms::Base64Options {
                mode: if s.action == Base64Decode {
                    Mode::Decode
                } else {
                    Mode::Encode
                },
                ..Default::default()
            },
        )
        .map_err(|e| e.to_string())?,
        UrlEncode | UrlDecode => transforms::url_transform(
            input,
            &transforms::UrlOptions {
                mode: if s.action == UrlDecode {
                    Mode::Decode
                } else {
                    Mode::Encode
                },
                ..Default::default()
            },
        )
        .map_err(|e| e.to_string())?,
        HtmlEncode | HtmlDecode => transforms::html_entities(
            input,
            &transforms::HtmlEntityOptions {
                mode: if s.action == HtmlDecode {
                    Mode::Decode
                } else {
                    Mode::Encode
                },
            },
        )
        .map_err(|e| e.to_string())?,
        StripTags => strip_tags(input),
        Sha256 | Md5 => transforms::hash(
            input,
            &transforms::HashOptions {
                algorithm: if s.action == Md5 {
                    transforms::HashAlgorithm::Md5
                } else {
                    transforms::HashAlgorithm::Sha256
                },
                ..Default::default()
            },
        )
        .map_err(|e| e.to_string())?,
        Count => existing(transforms::TransformKind::Count)?,
    };
    bounded(value)
}
fn strip_tags(input: &str) -> String {
    use html5ever::tokenizer::states::RawKind;
    use html5ever::tokenizer::{
        BufferQueue, TagKind, Token, TokenSink, TokenSinkResult, Tokenizer,
    };
    use std::cell::{Cell, RefCell};

    #[derive(Default)]
    struct PlainText {
        output: RefCell<String>,
        hidden: Cell<bool>,
        line_break: Cell<bool>,
    }

    impl TokenSink for PlainText {
        type Handle = ();

        fn process_token(&self, token: Token, _: u64) -> TokenSinkResult<()> {
            match token {
                Token::TagToken(tag) => match tag.name.as_ref() {
                    "script" | "style" => {
                        if tag.kind == TagKind::EndTag {
                            self.hidden.set(false);
                        } else if !tag.self_closing {
                            self.hidden.set(true);
                            return TokenSinkResult::RawData(if tag.name.as_ref() == "script" {
                                RawKind::ScriptData
                            } else {
                                RawKind::Rawtext
                            });
                        }
                    }
                    "br" | "hr" => self.output.borrow_mut().push('\n'),
                    "address" | "article" | "aside" | "blockquote" | "div" | "footer" | "h1"
                    | "h2" | "h3" | "h4" | "h5" | "h6" | "header" | "li" | "main" | "p" | "pre"
                    | "section" | "tr" => self.line_break.set(true),
                    _ => {}
                },
                Token::CharacterTokens(text) if !self.hidden.get() => {
                    let mut output = self.output.borrow_mut();
                    if self.line_break.replace(false)
                        && !output.is_empty()
                        && !output.ends_with('\n')
                        && !text.starts_with('\n')
                    {
                        output.push('\n');
                    }
                    output.push_str(&text);
                }
                _ => {}
            }
            TokenSinkResult::Continue
        }

        // Accept XML CDATA as literal text in the same action as HTML fragments.
        fn adjusted_current_node_present_but_not_in_html_namespace(&self) -> bool {
            true
        }
    }

    let buffer = BufferQueue::default();
    buffer.push_back(input.into());
    let tokenizer = Tokenizer::new(PlainText::default(), Default::default());
    let _ = tokenizer.feed(&buffer);
    tokenizer.end();
    tokenizer.sink.output.into_inner()
}

// Regex capture expansion with a hard limit before each append. Unlike expand(),
// repeated $1 references cannot allocate an arbitrarily amplified intermediate.
fn expand_bounded(caps: &regex::Captures<'_>, template: &str) -> Result<String, String> {
    let mut out = String::new();
    let mut rest = template;
    let push = |out: &mut String, text: &str| -> Result<(), String> {
        if out.len().saturating_add(text.len()) > transforms::MAX_INPUT_BYTES {
            return Err("Result exceeds the 2 MiB limit".into());
        }
        out.push_str(text);
        Ok(())
    };
    while let Some(pos) = rest.find('$') {
        push(&mut out, &rest[..pos])?;
        rest = &rest[pos + 1..];
        if let Some(tail) = rest.strip_prefix('$') {
            push(&mut out, "$")?;
            rest = tail;
            continue;
        }
        let (name, tail) = if let Some(braced) = rest.strip_prefix('{') {
            if let Some(end) = braced.find('}') {
                (&braced[..end], &braced[end + 1..])
            } else {
                push(&mut out, "$")?;
                continue;
            }
        } else {
            let end = rest
                .char_indices()
                .find(|(_, c)| !c.is_alphanumeric() && *c != '_')
                .map(|(i, _)| i)
                .unwrap_or(rest.len());
            if end == 0 {
                push(&mut out, "$")?;
                continue;
            }
            (&rest[..end], &rest[end..])
        };
        let capture = if let Ok(index) = name.parse::<usize>() {
            caps.get(index)
        } else {
            caps.name(name)
        };
        if let Some(value) = capture {
            push(&mut out, value.as_str())?;
        }
        rest = tail;
    }
    push(&mut out, rest)?;
    Ok(out)
}
#[derive(Debug, Clone)]
pub struct Recipe {
    pub id: i64,
    pub name: String,
    pub steps: Vec<Step>,
}
impl Store {
    pub fn save_text_recipe(
        &self,
        id: Option<i64>,
        name: &str,
        steps: &[Step],
        now: i64,
    ) -> Result<i64, String> {
        if name.trim().is_empty() {
            return Err("Give the cleaner a name".into());
        }
        if steps.is_empty() || steps.len() > 64 {
            return Err("A cleaner needs 1–64 steps".into());
        }
        let json = serde_json::to_string(steps).map_err(|e| e.to_string())?;
        bounded(json.clone())?;
        match id {
            Some(id) => {
                let n = self
                    .conn()
                    .execute(
                        "UPDATE text_recipes SET name=?1,steps=?2,updated_at_ms=?3 WHERE id=?4",
                        rusqlite::params![name.trim(), json, now, id],
                    )
                    .map_err(|e| e.to_string())?;
                if n == 0 {
                    return Err("Cleaner no longer exists".into());
                }
                Ok(id)
            }
            None => {
                self.conn()
                    .execute(
                        "INSERT INTO text_recipes(name,steps,updated_at_ms) VALUES (?1,?2,?3)",
                        rusqlite::params![name.trim(), json, now],
                    )
                    .map_err(|e| e.to_string())?;
                Ok(self.conn().last_insert_rowid())
            }
        }
    }
    pub fn text_recipes(&self) -> Result<Vec<Recipe>, String> {
        let mut stmt = self
            .conn()
            .prepare("SELECT id,name,steps FROM text_recipes ORDER BY name COLLATE NOCASE,id")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })
            .map_err(|e| e.to_string())?;
        rows.map(|r| {
            let (id, name, json) = r.map_err(|e| e.to_string())?;
            let steps = serde_json::from_str(&json).map_err(|e| format!("Cleaner {name}: {e}"))?;
            Ok(Recipe { id, name, steps })
        })
        .collect()
    }
    pub fn delete_text_recipe(&self, id: i64) -> Result<(), String> {
        self.conn()
            .execute("DELETE FROM text_recipes WHERE id=?1", [id])
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn strip_tags_extracts_html_and_xml_text() {
        let strip = |input| evaluate(input, &[Step::new(Action::StripTags)]).unwrap();
        assert_eq!(
            strip("<p title='a > b'>A &amp; B</p><!-- omit --><script>if (x < 4) f()</script><style>x{}</style><p>C<br>D</p>"),
            "A & B\nC\nD"
        );
        assert_eq!(
            strip("<?xml version='1.0'?><r><![CDATA[x < y & z]]><b>&lt;tag&gt;</b></r>"),
            "x < y & z<tag>"
        );
        assert_eq!(strip("3 < 5"), "3 < 5");
    }

    #[test]
    fn requested_workflow() {
        let mut split = Step::new(Action::Split);
        split.text = ",".into();
        let steps = vec![
            split,
            Step::new(Action::Trim),
            Step::new(Action::RemoveEmpty),
            Step::new(Action::Unique),
            Step::new(Action::SortAscending),
            Step::new(Action::Number),
        ];
        assert_eq!(
            evaluate(" pear,apple, pear,,banana", &steps).unwrap(),
            "1. apple\n2. banana\n3. pear"
        );
    }
    #[test]
    fn order_and_whitespace_are_explicit() {
        assert_eq!(
            evaluate(" b\na\n b\n", &[Step::new(Action::Unique)]).unwrap(),
            " b\na\n"
        );
        assert_eq!(
            evaluate("2\r\n10\r\n", &[Step::new(Action::Reverse)]).unwrap(),
            "10\n2\n"
        );
    }
    #[test]
    fn options_and_disabled_steps_compose_without_losing_blank_fields() {
        let mut prefix = Step::new(Action::Prefix);
        prefix.text = "• ".into();
        let mut remove = Step::new(Action::RemovePrefix);
        remove.text = "• ".into();
        let mut join = Step::new(Action::Join);
        join.text = " | ".into();
        let mut disabled = Step::new(Action::Replace);
        disabled.regex = true;
        disabled.text = "[".into();
        disabled.enabled = false;
        assert_eq!(
            evaluate("a\r\n\r\nb\r\n", &[prefix, remove, disabled, join]).unwrap(),
            "a |  | b"
        );
        let mut unique = Step::new(Action::Unique);
        unique.case_sensitive = false;
        assert_eq!(
            evaluate("Pear\npear\nAPPLE\nPear", &[unique]).unwrap(),
            "Pear\nAPPLE"
        );
    }
    #[test]
    fn literal_regex_and_limits() {
        let mut s = Step::new(Action::Replace);
        s.text = ".".into();
        s.replacement = "$1".into();
        assert_eq!(evaluate("a.b", &[s.clone()]).unwrap(), "a$1b");
        s.regex = true;
        s.text = "[".into();
        assert!(evaluate("abc", &[s]).is_err());
        assert!(evaluate("a", &[Step::new(Action::Base64Decode)]).is_err());
        let mut s = Step::new(Action::Prefix);
        s.text = "x".repeat(transforms::MAX_INPUT_BYTES);
        assert!(evaluate("a\nb", &[s]).is_err());
    }
    #[test]
    fn registry_and_recipes() {
        let store = crate::open_in_memory().unwrap();
        for d in CATALOG {
            assert_eq!(Action::from_id(d.id), Some(d.action));
            assert_eq!(serde_json::to_value(d.action).unwrap(), d.id);
        }
        let steps = vec![Step::new(Action::Trim), Step::new(Action::Upper)];
        let id = store.save_text_recipe(None, "Clean", &steps, 1).unwrap();
        assert_eq!(store.text_recipes().unwrap()[0].steps, steps);
        store
            .save_text_recipe(Some(id), "Renamed", &steps, 2)
            .unwrap();
        assert_eq!(store.text_recipes().unwrap().len(), 1);
        store.delete_text_recipe(id).unwrap();
        assert!(store.text_recipes().unwrap().is_empty());
    }
}

#[cfg(test)]
mod robustness_tests {
    use super::*;
    #[test]
    fn unicode_case_and_literal_split_preserve_content() {
        assert_eq!(
            evaluate("Hej, världen!\n🙂", &[Step::new(Action::Upper)]).unwrap(),
            "HEJ, VÄRLDEN!\n🙂"
        );
        let mut split = Step::new(Action::Split);
        split.text = "|".into();
        assert_eq!(evaluate("a|| b|", &[split]).unwrap(), "a\n\n b\n");
        assert_eq!(
            evaluate("b\r\na\r\n", &[Step::new(Action::SortAscending)]).unwrap(),
            "a\nb\n"
        );
        let mut step = Step::new(Action::Number);
        step.text = i64::MAX.to_string();
        assert!(evaluate("a\nb", &[step]).is_err());
    }
    #[test]
    fn regex_capture_expansion_is_bounded_and_matches_regex_semantics() {
        let mut s = Step::new(Action::Replace);
        s.regex = true;
        s.text = "(?P<word>[a-z]+)".into();
        s.replacement = "${word}:$1:$$".into();
        assert_eq!(evaluate("hi", &[s.clone()]).unwrap(), "hi:hi:$");
        s.text = "(a+)".into();
        s.replacement = "$1$1".into();
        assert!(evaluate(&"a".repeat(transforms::MAX_INPUT_BYTES / 2 + 1), &[s]).is_err());
    }
    #[test]
    fn recipe_survives_encrypted_reopen_and_unknown_actions_are_not_executed() {
        let dir =
            std::env::temp_dir().join(format!("magpie-cleaner-encrypted-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("cleaners.sqlite3");
        let _ = std::fs::remove_file(&path);
        let key = "12".repeat(32);
        {
            let store = crate::open_encrypted(&path, &key).unwrap();
            store
                .save_text_recipe(None, "Private cleaner", &[Step::new(Action::Trim)], 1)
                .unwrap();
        }
        {
            let store = crate::open_encrypted(&path, &key).unwrap();
            assert_eq!(store.text_recipes().unwrap()[0].name, "Private cleaner");
            store
                .conn()
                .execute(
                    "UPDATE text_recipes SET steps = '[{\"action\":\"shell\"}]'",
                    [],
                )
                .unwrap();
            assert!(store.text_recipes().is_err());
        }
        assert!(!std::fs::read(&path)
            .unwrap()
            .windows(15)
            .any(|b| b == b"Private cleaner"));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
