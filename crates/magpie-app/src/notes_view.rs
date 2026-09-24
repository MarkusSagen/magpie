//! Pure view-model helpers for notes: link extraction and list titles.

/// Extract `[[Page Name]]` references: trimmed, deduped, in first-seen order.
/// Empty `[[]]` and unclosed `[[` are skipped.
pub fn wiki_links(body: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut rest = body;
    while let Some(start) = rest.find("[[") {
        let after = &rest[start + 2..];
        match after.find("]]") {
            Some(end) => {
                let name = after[..end].trim();
                if !name.is_empty() && !out.iter().any(|n| n == name) {
                    out.push(name.to_string());
                }
                rest = &after[end + 2..];
            }
            None => break,
        }
    }
    out
}

/// Distinct `#tag` tokens anywhere in `body` (lowercased, first-seen order). A tag is
/// `#` followed by one or more of [A-Za-z0-9_-]; trailing punctuation is dropped. This
/// intentionally also picks up a task line's `#project`, unifying projects and tags.
pub fn note_tags(body: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for tok in body.split_whitespace() {
        if let Some(after) = tok.strip_prefix('#') {
            let tag: String = after
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
                .collect::<String>()
                .to_lowercase();
            if !tag.is_empty() && !out.iter().any(|t| t == &tag) {
                out.push(tag);
            }
        }
    }
    out
}

/// A human title for the notes list: the page name, unless it's empty or an
/// auto-generated `Note <n>` capture name — then the first non-empty body line.
pub fn note_list_title(note_name: &str, body: &str) -> String {
    let name = note_name.trim();
    let auto = name.is_empty() || name.starts_with("Note ");
    if !auto {
        return name.to_string();
    }
    let line = body
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or(name);
    line.chars().take(60).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wiki_links_extracts_dedups_trims() {
        let b = "see [[Prod Incidents]] and [[ enhancer ]] and [[Prod Incidents]] and [[]]";
        assert_eq!(
            wiki_links(b),
            vec!["Prod Incidents".to_string(), "enhancer".to_string()]
        );
    }

    #[test]
    fn wiki_links_handles_unclosed() {
        assert_eq!(wiki_links("a [[open only"), Vec::<String>::new());
        assert_eq!(wiki_links("none here"), Vec::<String>::new());
    }

    #[test]
    fn note_tags_extracts_distinct_lowercased() {
        let b = "#Work\n- [ ] ship it #prod !high\n#work";
        assert_eq!(note_tags(b), vec!["work".to_string(), "prod".to_string()]);
    }

    #[test]
    fn note_tags_ignores_bare_hash() {
        assert_eq!(note_tags("just a # and #! here"), Vec::<String>::new());
    }

    #[test]
    fn note_tags_drops_trailing_punctuation() {
        assert_eq!(note_tags("all #done."), vec!["done".to_string()]);
    }

    #[test]
    fn title_prefers_name_else_first_line() {
        assert_eq!(note_list_title("Prod Incidents", "body"), "Prod Incidents");
        assert_eq!(
            note_list_title("Note 1725270000000", "\n  Fix mount\nmore"),
            "Fix mount"
        );
        assert_eq!(note_list_title("", "  first\nsecond"), "first");
    }

    #[test]
    fn title_truncates_long_first_line_to_60_chars() {
        let long = "x".repeat(100);
        let out = note_list_title("", &long);
        assert_eq!(out.chars().count(), 60);
        assert_eq!(out, "x".repeat(60));
    }

    #[test]
    fn title_counts_chars_not_bytes_when_truncating() {
        // 70 multi-byte chars: truncation must cut at 60 CHARS, not bytes.
        let long = "é".repeat(70);
        let out = note_list_title("Note 42", &long);
        assert_eq!(out.chars().count(), 60);
    }
}
