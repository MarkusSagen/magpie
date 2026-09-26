//! Markdown → line model for the Notes v2 rich preview (Phase 14). Pure and
//! tested: it turns a note body into a `Vec<Line>` of block kinds + inline spans
//! (bold/italic/code/links/wiki + task tokens as chips), which the app renders as
//! a virtualized list. Rendering (and any inline live-preview editing) lives in
//! the app; this module has no UI dependency.

/// Block-level kind of a rendered line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    Plain,
    H1,
    H2,
    H3,
    Bullet,
    /// A numbered list item; carries its marker (`1.`).
    Numbered(String),
    Quote,
    /// A checkbox task line; `true` = done (struck through).
    Task {
        done: bool,
    },
}

/// An inline piece of a line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Span {
    Text(String),
    Bold(String),
    Italic(String),
    Code(String),
    /// `[[wiki]]` note link — the target title.
    Wiki(String),
    /// A URL; `label` is the shown text, `domain` the bare host for a favicon chip.
    Url {
        href: String,
        label: String,
        domain: String,
    },
    /// `!high` / `!med` / `!low` priority (3/2/1).
    Prio(u8),
    /// `@date` chip (the token after `@`).
    DateChip(String),
    /// `#tag` chip.
    Tag(String),
    /// `+recur` chip (e.g. `+1w`).
    Recur(String),
}

/// One rendered line: its block kind and inline spans.
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub block: Block,
    pub spans: Vec<Span>,
}

/// The bare host of a URL, for a favicon chip (`https://github.com/x` → `github.com`).
pub fn url_domain(url: &str) -> String {
    let after = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
    let host = after.split(['/', '?', '#']).next().unwrap_or(after);
    host.strip_prefix("www.").unwrap_or(host).to_string()
}

fn is_url_char(c: char) -> bool {
    !c.is_whitespace() && c != ')' && c != ']' && c != '>'
}

/// Split a line's inline content into spans (bold/italic/code/wiki/url + task
/// tokens). Order-preserving; unrecognised text stays `Span::Text`.
fn parse_inline(s: &str) -> Vec<Span> {
    let chars: Vec<char> = s.chars().collect();
    let mut spans = Vec::new();
    let mut buf = String::new();
    let mut i = 0;
    let flush = |buf: &mut String, spans: &mut Vec<Span>| {
        if !buf.is_empty() {
            spans.push(Span::Text(std::mem::take(buf)));
        }
    };
    while i < chars.len() {
        let c = chars[i];
        // Inline code `...`
        if c == '`' {
            if let Some(end) = chars[i + 1..].iter().position(|&x| x == '`') {
                let inner: String = chars[i + 1..i + 1 + end].iter().collect();
                flush(&mut buf, &mut spans);
                spans.push(Span::Code(inner));
                i += end + 2;
                continue;
            }
        }
        // Bold **...**
        if c == '*' && i + 1 < chars.len() && chars[i + 1] == '*' {
            if let Some(end) = find_delim(&chars, i + 2, "**") {
                let inner: String = chars[i + 2..end].iter().collect();
                flush(&mut buf, &mut spans);
                spans.push(Span::Bold(inner));
                i = end + 2;
                continue;
            }
        }
        // Italic *...* or _..._
        if (c == '*' || c == '_') && i + 1 < chars.len() && chars[i + 1] != c {
            let delim = c.to_string();
            if let Some(end) = find_delim(&chars, i + 1, &delim) {
                let inner: String = chars[i + 1..end].iter().collect();
                if !inner.trim().is_empty() {
                    flush(&mut buf, &mut spans);
                    spans.push(Span::Italic(inner));
                    i = end + 1;
                    continue;
                }
            }
        }
        // Wiki [[target]]
        if c == '[' && i + 1 < chars.len() && chars[i + 1] == '[' {
            if let Some(end) = find_delim(&chars, i + 2, "]]") {
                let inner: String = chars[i + 2..end].iter().collect();
                flush(&mut buf, &mut spans);
                spans.push(Span::Wiki(inner));
                i = end + 2;
                continue;
            }
        }
        // URL http(s)://…
        if (c == 'h')
            && (chars[i..].starts_with(&['h', 't', 't', 'p', ':', '/', '/'])
                || chars[i..].starts_with(&['h', 't', 't', 'p', 's', ':', '/', '/']))
        {
            let start = i;
            let mut j = i;
            while j < chars.len() && is_url_char(chars[j]) {
                j += 1;
            }
            // Trim trailing sentence punctuation.
            while j > start && matches!(chars[j - 1], '.' | ',' | ';' | ':' | '!' | '?') {
                j -= 1;
            }
            let href: String = chars[start..j].iter().collect();
            flush(&mut buf, &mut spans);
            let domain = url_domain(&href);
            spans.push(Span::Url {
                label: domain.clone(),
                domain,
                href,
            });
            i = j;
            continue;
        }
        // Task tokens: only at a token boundary (start of line or after space).
        let at_boundary = i == 0 || chars[i - 1].is_whitespace();
        if at_boundary && matches!(c, '!' | '@' | '#' | '+') {
            let start = i + 1;
            let mut j = start;
            while j < chars.len() && !chars[j].is_whitespace() {
                j += 1;
            }
            let word: String = chars[start..j].iter().collect();
            if !word.is_empty()
                && word
                    .chars()
                    .all(|x| x.is_alphanumeric() || x == '-' || x == '_' || x == '/')
            {
                let span = match c {
                    '!' => match word.to_lowercase().as_str() {
                        "high" | "h" | "1" | "a" => Some(Span::Prio(3)),
                        "med" | "medium" | "m" | "2" | "b" => Some(Span::Prio(2)),
                        "low" | "l" | "3" | "c" => Some(Span::Prio(1)),
                        _ => None,
                    },
                    '@' => Some(Span::DateChip(word.clone())),
                    '#' => Some(Span::Tag(word.clone())),
                    '+' => Some(Span::Recur(word.clone())),
                    _ => None,
                };
                if let Some(span) = span {
                    flush(&mut buf, &mut spans);
                    spans.push(span);
                    i = j;
                    continue;
                }
            }
        }
        buf.push(c);
        i += 1;
    }
    flush(&mut buf, &mut spans);
    spans
}

/// Find the char index where `delim` starts, at or after `from`, on the same line.
fn find_delim(chars: &[char], from: usize, delim: &str) -> Option<usize> {
    let d: Vec<char> = delim.chars().collect();
    let mut i = from;
    while i + d.len() <= chars.len() {
        if chars[i..i + d.len()] == d[..] {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Render a whole note body into a line model.
pub fn render_note(body: &str) -> Vec<Line> {
    body.lines().map(render_line).collect()
}

/// Render a single source line into a `Line`.
pub fn render_line(raw: &str) -> Line {
    let trimmed = raw.trim_start();
    // Headings.
    if let Some(rest) = trimmed.strip_prefix("### ") {
        return Line {
            block: Block::H3,
            spans: parse_inline(rest),
        };
    }
    if let Some(rest) = trimmed.strip_prefix("## ") {
        return Line {
            block: Block::H2,
            spans: parse_inline(rest),
        };
    }
    if let Some(rest) = trimmed.strip_prefix("# ") {
        return Line {
            block: Block::H1,
            spans: parse_inline(rest),
        };
    }
    // Blockquote.
    if let Some(rest) = trimmed.strip_prefix("> ") {
        return Line {
            block: Block::Quote,
            spans: parse_inline(rest),
        };
    }
    // Checkbox tasks: `- [ ]`, `- [x]`, `* [ ]` (also `[/]` in-progress → not done).
    for bullet in ["- ", "* "] {
        if let Some(after) = trimmed.strip_prefix(bullet) {
            let low = after.to_lowercase();
            if let Some(rest) = low
                .strip_prefix("[ ] ")
                .map(|_| &after[4..])
                .or_else(|| low.strip_prefix("[/] ").map(|_| &after[4..]))
            {
                return Line {
                    block: Block::Task { done: false },
                    spans: parse_inline(rest),
                };
            }
            if low.starts_with("[x] ") {
                return Line {
                    block: Block::Task { done: true },
                    spans: parse_inline(&after[4..]),
                };
            }
            // Plain bullet.
            return Line {
                block: Block::Bullet,
                spans: parse_inline(after),
            };
        }
    }
    // Numbered list `1. `, `2) `.
    if let Some((marker, rest)) = split_numbered(trimmed) {
        return Line {
            block: Block::Numbered(marker),
            spans: parse_inline(rest),
        };
    }
    Line {
        block: Block::Plain,
        spans: parse_inline(trimmed),
    }
}

/// If `s` starts with `N.` or `N)` + space, return `("N.", rest)`.
fn split_numbered(s: &str) -> Option<(String, &str)> {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i == 0 || i + 1 >= bytes.len() {
        return None;
    }
    if (bytes[i] == b'.' || bytes[i] == b')') && bytes[i + 1] == b' ' {
        return Some((format!("{}.", &s[..i]), &s[i + 2..]));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headings_and_blocks() {
        assert_eq!(render_line("# Title").block, Block::H1);
        assert_eq!(render_line("## Sub").block, Block::H2);
        assert_eq!(render_line("### Small").block, Block::H3);
        assert_eq!(render_line("> quoted").block, Block::Quote);
        assert_eq!(render_line("- item").block, Block::Bullet);
        assert_eq!(render_line("1. first").block, Block::Numbered("1.".into()));
    }

    #[test]
    fn checkboxes() {
        assert_eq!(render_line("- [ ] todo").block, Block::Task { done: false });
        assert_eq!(render_line("- [x] done").block, Block::Task { done: true });
        assert_eq!(render_line("- [/] wip").block, Block::Task { done: false });
    }

    #[test]
    fn inline_formatting() {
        let spans = parse_inline("a **bold** and *italic* and `code`");
        assert!(spans.contains(&Span::Bold("bold".into())));
        assert!(spans.contains(&Span::Italic("italic".into())));
        assert!(spans.contains(&Span::Code("code".into())));
    }

    #[test]
    fn wiki_and_url_and_tokens() {
        let spans = parse_inline(
            "see [[Graphite redesign]] at https://github.com/MarkusSagen/magpie #ui @fri !high +1w",
        );
        assert!(spans.contains(&Span::Wiki("Graphite redesign".into())));
        assert!(spans
            .iter()
            .any(|s| matches!(s, Span::Url { domain, .. } if domain == "github.com")));
        assert!(spans.contains(&Span::Tag("ui".into())));
        assert!(spans.contains(&Span::DateChip("fri".into())));
        assert!(spans.contains(&Span::Prio(3)));
        assert!(spans.contains(&Span::Recur("1w".into())));
    }

    #[test]
    fn url_domain_strips_scheme_and_www() {
        assert_eq!(url_domain("https://www.example.com/a/b"), "example.com");
        assert_eq!(
            url_domain("http://news.ycombinator.com/newest"),
            "news.ycombinator.com"
        );
    }

    #[test]
    fn hash_mid_word_is_not_a_tag() {
        // `#` must be at a token boundary — `C#` stays literal text.
        let spans = parse_inline("I code in C#");
        assert!(spans.iter().all(|s| !matches!(s, Span::Tag(_))));
    }

    #[test]
    fn full_note_line_count() {
        let body = "# Roadmap\n\n- [ ] task @fri\ntext with [[link]]";
        let lines = render_note(body);
        assert_eq!(lines.len(), 4);
        assert_eq!(lines[0].block, Block::H1);
        assert_eq!(lines[2].block, Block::Task { done: false });
    }
}
