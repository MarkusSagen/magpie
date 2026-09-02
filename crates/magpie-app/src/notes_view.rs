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
    fn title_prefers_name_else_first_line() {
        assert_eq!(note_list_title("Prod Incidents", "body"), "Prod Incidents");
        assert_eq!(
            note_list_title("Note 1725270000000", "\n  Fix mount\nmore"),
            "Fix mount"
        );
        assert_eq!(note_list_title("", "  first\nsecond"), "first");
    }
}
