//! Streamlined, source/type-scoped clipboard search: pull `key:value` tokens (and
//! bare category words) out of the query so the same search box narrows by source and
//! type without leaving the keyboard. Pure and testable — modeled on
//! [`crate::filesearch::parse_locations_prefix`]; the search layer turns the parsed
//! fields into SQL clauses.
//!
//! Grammar (tokens may appear anywhere; everything else is free text):
//! - `app:<name>`            — source app, substring (e.g. `app:vivaldi`)
//! - `url:<sub>` / `site:<sub>` — the clip's URL or the source page it came from
//! - `type:<kind>` / `kind:<kind>` — a content kind (`image`, `link`, `text`, …)
//! - `is:pinned`             — pinned only; `is:<kind>`/`is:<category>` also accepted
//! - `after:<date>` / `before:<date>` / `on:<date>` — time range; date is `YYYY`,
//!   `YYYY-MM`, or `YYYY-MM-DD` (bounds inclusive)
//! - `cat:<category>` / `category:<category>` — a source category (browser, editor,
//!   terminal, chat, notes, design)
//! - bare `browser` / `editor` / `terminal` — the category, unprefixed (only these
//!   three; `note`/`chat`/`web`/`design` are common search words, so they filter only
//!   when prefixed with `cat:`)
//!
//! Unknown keys are left as literal text (`foo:bar` searches for `foo:bar`), so the
//! parser never hijacks an ordinary query.

use crate::appcat::{canonical_category, canonical_category_bare};
use crate::detect::Kind;
use crate::eval::{days_from_civil, days_in_month};

const DAY_MS: i64 = 86_400_000;

/// Parse a date bound `YYYY` / `YYYY-MM` / `YYYY-MM-DD` into the (start, end) epoch-ms
/// of that period (UTC day boundaries, consistent with the preset time filters). `None`
/// if it isn't a valid date, so a stray `after:foo` stays literal text.
fn parse_date_range(v: &str) -> Option<(i64, i64)> {
    let parts: Vec<&str> = v.split('-').collect();
    let y: i64 = parts.first()?.parse().ok()?;
    if !(1970..=3000).contains(&y) {
        return None;
    }
    match parts.len() {
        1 => Some((
            days_from_civil(y, 1, 1) * DAY_MS,
            (days_from_civil(y, 12, 31) + 1) * DAY_MS - 1,
        )),
        2 => {
            let m: i64 = parts[1].parse().ok()?;
            if !(1..=12).contains(&m) {
                return None;
            }
            Some((
                days_from_civil(y, m, 1) * DAY_MS,
                (days_from_civil(y, m, days_in_month(y, m)) + 1) * DAY_MS - 1,
            ))
        }
        3 => {
            let m: i64 = parts[1].parse().ok()?;
            let d: i64 = parts[2].parse().ok()?;
            if !(1..=12).contains(&m) || !(1..=days_in_month(y, m)).contains(&d) {
                return None;
            }
            let start = days_from_civil(y, m, d) * DAY_MS;
            Some((start, start + DAY_MS - 1))
        }
        _ => None,
    }
}

/// The structured result of parsing a raw query string.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParsedSearch {
    /// Free text left after tokens are removed (the actual needle).
    pub text: String,
    /// `app:` — source-app name substring (lowercased).
    pub app: Option<String>,
    /// `url:` / `site:` — URL / source-page substring (lowercased).
    pub url: Option<String>,
    /// `type:` / `kind:` (or `is:<kind>`) — content kind.
    pub kind: Option<Kind>,
    /// bare category word or `cat:` — canonical source category.
    pub category: Option<String>,
    /// `after:`/`since:` (or `on:`/`date:`) — inclusive lower time bound, epoch ms.
    pub after: Option<i64>,
    /// `before:`/`until:` (or `on:`/`date:`) — inclusive upper time bound, epoch ms.
    pub before: Option<i64>,
    /// `is:pinned`.
    pub pinned: bool,
}

/// Map a user-typed kind word (with common aliases) to a [`Kind`].
fn kind_alias(v: &str) -> Option<Kind> {
    Some(match v {
        "text" | "txt" | "plain" => Kind::Text,
        "link" | "links" | "url" | "urls" => Kind::Link,
        "color" | "colour" | "colors" => Kind::Color,
        "email" | "emails" | "mail" => Kind::Email,
        "image" | "img" | "images" | "pic" | "pics" | "photo" | "screenshot" => Kind::Image,
        "file" | "files" | "path" => Kind::File,
        "html" => Kind::Html,
        "rtf" => Kind::Rtf,
        "git" | "git_remote" | "gitremote" | "repo" => Kind::GitRemote,
        _ => return None,
    })
}

/// Parse `raw` into free text plus structured filters. Later occurrences of the same
/// key win; the residual text preserves original order and case.
pub fn parse_search_tokens(raw: &str) -> ParsedSearch {
    let mut p = ParsedSearch::default();
    let mut words: Vec<&str> = Vec::new();

    for tok in raw.split_whitespace() {
        if let Some((key, val)) = tok.split_once(':') {
            let val_lc = val.trim().to_lowercase();
            let mut consumed = true;
            match key.to_lowercase().as_str() {
                _ if val_lc.is_empty() => consumed = false, // `app:` with no value → literal
                "app" => p.app = Some(val_lc),
                "url" | "site" | "link" => p.url = Some(val_lc),
                "after" | "since" => match parse_date_range(&val_lc) {
                    Some((s, _)) => p.after = Some(s),
                    None => consumed = false,
                },
                "before" | "until" => match parse_date_range(&val_lc) {
                    Some((_, e)) => p.before = Some(e),
                    None => consumed = false,
                },
                "on" | "date" => match parse_date_range(&val_lc) {
                    Some((s, e)) => {
                        p.after = Some(s);
                        p.before = Some(e);
                    }
                    None => consumed = false,
                },
                "type" | "kind" => match kind_alias(&val_lc) {
                    Some(k) => p.kind = Some(k),
                    None => consumed = false,
                },
                "cat" | "category" | "source" => match canonical_category(&val_lc) {
                    Some(c) => p.category = Some(c.to_string()),
                    None => consumed = false,
                },
                "is" => {
                    if val_lc == "pinned" || val_lc == "pin" || val_lc == "starred" {
                        p.pinned = true;
                    } else if let Some(k) = kind_alias(&val_lc) {
                        p.kind = Some(k);
                    } else if let Some(c) = canonical_category(&val_lc) {
                        p.category = Some(c.to_string());
                    } else {
                        consumed = false;
                    }
                }
                _ => consumed = false, // unknown key → literal text
            }
            if consumed {
                continue;
            }
        }
        // Bare category word (e.g. `browser`) acts as a source filter; anything else
        // is ordinary search text. Only unambiguous category words qualify bare — a
        // plain search for `note`/`chat`/`web` stays text (use `cat:notes` to filter).
        if let Some(c) = canonical_category_bare(tok) {
            p.category = Some(c.to_string());
        } else {
            words.push(tok);
        }
    }

    p.text = words.join(" ");
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_is_untouched() {
        let p = parse_search_tokens("hello world");
        assert_eq!(p.text, "hello world");
        assert_eq!(
            p,
            ParsedSearch {
                text: "hello world".into(),
                ..Default::default()
            }
        );
    }

    #[test]
    fn app_token_extracted() {
        let p = parse_search_tokens("app:vivaldi invoice");
        assert_eq!(p.app.as_deref(), Some("vivaldi"));
        assert_eq!(p.text, "invoice");
    }

    #[test]
    fn url_and_site_aliases() {
        assert_eq!(
            parse_search_tokens("url:enablebanking").url.as_deref(),
            Some("enablebanking")
        );
        assert_eq!(
            parse_search_tokens("site:github.com pr").url.as_deref(),
            Some("github.com")
        );
        assert_eq!(parse_search_tokens("site:github.com pr").text, "pr");
    }

    #[test]
    fn type_and_kind_aliases() {
        assert_eq!(parse_search_tokens("type:image").kind, Some(Kind::Image));
        assert_eq!(
            parse_search_tokens("kind:img cat photo").kind,
            Some(Kind::Image)
        );
        // 'url' as a KIND alias means link, distinct from the url: key.
        assert_eq!(parse_search_tokens("type:url").kind, Some(Kind::Link));
    }

    #[test]
    fn is_pinned_and_kind_and_category() {
        assert!(parse_search_tokens("is:pinned todo").pinned);
        assert_eq!(parse_search_tokens("is:pinned todo").text, "todo");
        assert_eq!(parse_search_tokens("is:image").kind, Some(Kind::Image));
        assert_eq!(
            parse_search_tokens("is:browser").category.as_deref(),
            Some("browser")
        );
    }

    #[test]
    fn bare_category_word() {
        let p = parse_search_tokens("browser enablebanking");
        assert_eq!(p.category.as_deref(), Some("browser"));
        assert_eq!(p.text, "enablebanking");
    }

    #[test]
    fn common_words_are_not_bare_categories() {
        // `note`/`notes`/`chat`/`web` are ordinary search terms bare; only prefixed.
        assert_eq!(parse_search_tokens("note").text, "note");
        assert!(parse_search_tokens("note").category.is_none());
        assert_eq!(parse_search_tokens("chat log").text, "chat log");
        assert_eq!(
            parse_search_tokens("web design ideas").text,
            "web design ideas"
        );
        // But explicit prefixes still work for those categories.
        assert_eq!(
            parse_search_tokens("cat:notes").category.as_deref(),
            Some("notes")
        );
    }

    #[test]
    fn cat_key_and_synonyms() {
        assert_eq!(
            parse_search_tokens("cat:ide main.rs").category.as_deref(),
            Some("editor")
        );
        assert_eq!(parse_search_tokens("cat:ide main.rs").text, "main.rs");
    }

    #[test]
    fn unknown_key_and_empty_value_stay_literal() {
        assert_eq!(parse_search_tokens("foo:bar").text, "foo:bar");
        assert_eq!(parse_search_tokens("app:").text, "app:");
        assert!(parse_search_tokens("app:").app.is_none());
    }

    #[test]
    fn combined_tokens() {
        let p = parse_search_tokens("app:chrome url:stripe type:link refund");
        assert_eq!(p.app.as_deref(), Some("chrome"));
        assert_eq!(p.url.as_deref(), Some("stripe"));
        assert_eq!(p.kind, Some(Kind::Link));
        assert_eq!(p.text, "refund");
    }

    #[test]
    fn date_range_tokens() {
        // 2026-01-01 start = days_from_civil(2026,1,1) * DAY_MS.
        let start_2026 = days_from_civil(2026, 1, 1) * DAY_MS;
        let p = parse_search_tokens("after:2026-01-01 report");
        assert_eq!(p.after, Some(start_2026));
        assert_eq!(p.text, "report");

        // on: a single day sets both bounds to that day's start/end.
        let p2 = parse_search_tokens("on:2026-01-01");
        assert_eq!(p2.after, Some(start_2026));
        assert_eq!(p2.before, Some(start_2026 + DAY_MS - 1));

        // A whole-month before: bound ends at the last ms of that month.
        let p3 = parse_search_tokens("before:2026-02");
        let feb_end = (days_from_civil(2026, 2, days_in_month(2026, 2)) + 1) * DAY_MS - 1;
        assert_eq!(p3.before, Some(feb_end));

        // Invalid dates stay literal.
        assert_eq!(parse_search_tokens("after:notadate").text, "after:notadate");
        assert!(parse_search_tokens("after:notadate").after.is_none());
        assert_eq!(parse_search_tokens("on:2026-13-01").text, "on:2026-13-01");
    }

    #[test]
    fn later_key_wins() {
        assert_eq!(
            parse_search_tokens("app:safari app:vivaldi").app.as_deref(),
            Some("vivaldi")
        );
    }
}
