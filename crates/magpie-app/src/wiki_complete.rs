//! Caret-aware `[[wiki link]]` autocomplete helpers for the note editor. Pure, so
//! they're unit-testable independent of Slint.

/// If the caret sits inside an OPEN `[[…` link (a `[[` before the caret with no
/// `]]` or newline between it and the caret), return `(partial_start_byte, partial)`
/// where `partial` is the text between `[[` and the caret. Else `None`.
pub fn active_wiki_query(text: &str, caret: usize) -> Option<(usize, &str)> {
    let caret = caret.min(text.len());
    let before = &text[..caret];
    let open = before.rfind("[[")?;
    let partial = &before[open + 2..];
    // Invalid if the "partial" already closed the link or spans a newline.
    if partial.contains("]]") || partial.contains(']') || partial.contains('\n') {
        return None;
    }
    Some((open + 2, partial))
}

/// Replace the open partial (from just after `[[` to the caret) with `name]]`,
/// closing the link. Returns (new_text, new_caret_byte). If there's no active
/// query, returns the text unchanged with the original caret.
pub fn apply_wiki_completion(text: &str, caret: usize, name: &str) -> (String, usize) {
    let caret = caret.min(text.len());
    match active_wiki_query(text, caret) {
        Some((start, _partial)) => {
            let mut out = String::with_capacity(text.len() + name.len() + 2);
            out.push_str(&text[..start]);
            out.push_str(name);
            out.push_str("]]");
            let new_caret = out.len();
            out.push_str(&text[caret..]);
            (out, new_caret)
        }
        None => (text.to_string(), caret),
    }
}

/// Note names matching `partial` (case-insensitive substring), best-effort ranked:
/// prefix matches first, then the rest, each in input order; capped at `limit`.
pub fn rank_matches(names: &[String], partial: &str, limit: usize) -> Vec<String> {
    let p = partial.trim().to_lowercase();
    let mut pre: Vec<&String> = Vec::new();
    let mut sub: Vec<&String> = Vec::new();
    for n in names {
        let l = n.to_lowercase();
        if p.is_empty() || l.starts_with(&p) {
            pre.push(n);
        } else if l.contains(&p) {
            sub.push(n);
        }
    }
    pre.into_iter().chain(sub).take(limit).cloned().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_query_open_link() {
        let text = "hello [[foo";
        assert_eq!(active_wiki_query(text, text.len()), Some((8, "foo")));
    }

    #[test]
    fn active_query_no_open_bracket() {
        assert_eq!(active_wiki_query("hello world", 5), None);
    }

    #[test]
    fn active_query_closed_link() {
        let text = "see [[foo]]";
        assert_eq!(active_wiki_query(text, text.len()), None);
    }

    #[test]
    fn active_query_newline_breaks_it() {
        let text = "[[foo\nbar";
        assert_eq!(active_wiki_query(text, text.len()), None);
    }

    #[test]
    fn active_query_mid_partial_before_pipe() {
        let text = "a [[bar| baz";
        let caret = text.find('|').unwrap();
        assert_eq!(active_wiki_query(text, caret), Some((4, "bar")));
    }

    #[test]
    fn apply_completion_basic() {
        let text = "see [[fo";
        let (out, caret) = apply_wiki_completion(text, 8, "Foo Bar");
        assert_eq!(out, "see [[Foo Bar]]");
        assert_eq!(caret, out.len());
    }

    #[test]
    fn apply_completion_preserves_trailing_text() {
        let text = "x [[fo rest";
        let caret = "x [[fo".len();
        let (out, new_caret) = apply_wiki_completion(text, caret, "Foo");
        assert_eq!(out, "x [[Foo]] rest");
        assert_eq!(new_caret, "x [[Foo]]".len());
    }

    #[test]
    fn rank_matches_prefix_beats_substring_and_case_insensitive() {
        let names = vec![
            "Alpha".to_string(),
            "Beta".to_string(),
            "Cabal".to_string(),
            "abacus".to_string(),
        ];
        let m = rank_matches(&names, "a", 10);
        // Prefix matches (case-insensitive starts_with "a"): Alpha, abacus.
        // Substring matches (contains "a" but not prefix), input order: Beta, Cabal.
        assert_eq!(m, vec!["Alpha", "abacus", "Beta", "Cabal"]);
    }

    #[test]
    fn rank_matches_empty_partial_returns_all_capped() {
        let names = vec!["One".to_string(), "Two".to_string(), "Three".to_string()];
        let m = rank_matches(&names, "", 2);
        assert_eq!(m, vec!["One", "Two"]);
    }
}
