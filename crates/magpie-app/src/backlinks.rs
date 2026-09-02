//! Pure backlink/mention computation over notes: which other notes reference the
//! open note, split into linked references (`[[name]]`) and unlinked mentions.
//! Case-insensitive; bodies are decomposed with `split('\n')` so a line index
//! round-trips (find → link) exactly.
use crate::notes_view::wiki_links;
use magpie_core::Note;

/// One referencing line in some note. `line` is trimmed for display.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reference {
    pub note_id: i64,
    pub note_name: String,
    pub line_index: usize,
    pub line: String,
}

/// References to a target note, split by whether the line already links it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BacklinkResult {
    pub linked: Vec<Reference>,
    pub unlinked: Vec<Reference>,
}

/// Scan `notes` for lines referencing `target_name` (case-insensitive), skipping the
/// target note itself. A line with `[[target]]` is a linked reference; a line with a
/// standalone plain-text `target` that is not already such a link is an unlinked
/// mention. Note order then line order preserved.
pub fn find_references(target_name: &str, notes: &[Note]) -> BacklinkResult {
    let target = target_name.trim();
    let mut out = BacklinkResult::default();
    if target.is_empty() {
        return out;
    }
    let target_lc = target.to_lowercase();
    for note in notes {
        if note.name.trim().to_lowercase() == target_lc {
            continue;
        }
        for (i, line) in note.body.split('\n').enumerate() {
            let make = || Reference {
                note_id: note.id,
                note_name: note.name.clone(),
                line_index: i,
                line: line.trim().to_string(),
            };
            if line_links_to(line, &target_lc) {
                out.linked.push(make());
            } else if standalone_match(line, target).is_some() {
                out.unlinked.push(make());
            }
        }
    }
    out
}

/// Wrap the first standalone occurrence of `target` on line `line_index` of `body`
/// as `[[…]]`, preserving the matched text's original casing. No-op (returns `body`
/// unchanged) if the index is out of range, the line already links the target, or no
/// standalone occurrence is found. Idempotent: a second call does nothing.
pub fn link_mention(body: &str, line_index: usize, target: &str) -> String {
    let target = target.trim();
    let lines: Vec<&str> = body.split('\n').collect();
    if line_index >= lines.len() {
        return body.to_string();
    }
    let line = lines[line_index];
    if line_links_to(line, &target.to_lowercase()) {
        return body.to_string();
    }
    let Some((s, e)) = standalone_match(line, target) else {
        return body.to_string();
    };
    let new_line = format!("{}[[{}]]{}", &line[..s], &line[s..e], &line[e..]);
    let mut result: Vec<&str> = lines.clone();
    result[line_index] = &new_line;
    result.join("\n")
}

/// True if `line` contains a `[[…]]` whose name equals `target_lc` (already lower).
fn line_links_to(line: &str, target_lc: &str) -> bool {
    wiki_links(line)
        .iter()
        .any(|name| name.trim().to_lowercase() == target_lc)
}

/// Byte range of the first standalone, ASCII-case-insensitive occurrence of `target`
/// in `line`. Standalone = the character immediately before and after the match (if
/// any) is not alphanumeric. `None` if `target` is empty or has no such occurrence.
fn standalone_match(line: &str, target: &str) -> Option<(usize, usize)> {
    let tb = target.as_bytes();
    if tb.is_empty() {
        return None;
    }
    let lb = line.as_bytes();
    let mut i = 0;
    while i + tb.len() <= lb.len() {
        if lb[i..i + tb.len()].eq_ignore_ascii_case(tb)
            && line.is_char_boundary(i)
            && line.is_char_boundary(i + tb.len())
        {
            let before_alnum = line[..i]
                .chars()
                .next_back()
                .map(|c| c.is_alphanumeric())
                .unwrap_or(false);
            let after_alnum = line[i + tb.len()..]
                .chars()
                .next()
                .map(|c| c.is_alphanumeric())
                .unwrap_or(false);
            if !before_alnum && !after_alnum {
                return Some((i, i + tb.len()));
            }
        }
        i += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(id: i64, name: &str, body: &str) -> Note {
        Note {
            id,
            name: name.to_string(),
            is_daily: false,
            body: body.to_string(),
            created_at_ms: 0,
            updated_at_ms: 0,
            source_app_id: None,
            source_entry_id: None,
        }
    }

    #[test]
    fn classifies_linked_unlinked_and_excludes_self() {
        let notes = vec![
            note(1, "Target", "self mention Target should be skipped"),
            note(2, "A", "see [[Target]] here"),
            note(3, "B", "just a target mention"),
            note(4, "C", "my targetting is unrelated"),
            note(5, "D", "[[Target]] and target again"),
        ];
        let r = find_references("Target", &notes);
        let linked: Vec<i64> = r.linked.iter().map(|x| x.note_id).collect();
        let unlinked: Vec<i64> = r.unlinked.iter().map(|x| x.note_id).collect();
        assert_eq!(linked, vec![2, 5]);
        assert_eq!(unlinked, vec![3]);
    }

    #[test]
    fn standalone_respects_word_boundaries() {
        let notes = vec![
            note(2, "A", "target"),
            note(3, "B", "(target)"),
            note(4, "C", "notebook"),
        ];
        assert_eq!(find_references("target", &notes).unlinked.len(), 2);
        assert!(find_references("note", &notes).unlinked.is_empty());
    }

    #[test]
    fn link_mention_wraps_first_occurrence_preserving_case() {
        let body = "line one\nsee Target now\nline three";
        let out = link_mention(body, 1, "target");
        assert_eq!(out, "line one\nsee [[Target]] now\nline three");
    }

    #[test]
    fn link_mention_is_idempotent_and_noop_when_absent() {
        let body = "see [[Target]] now";
        assert_eq!(link_mention(body, 0, "target"), body);
        let body2 = "nothing here";
        assert_eq!(link_mention(body2, 0, "target"), body2);
        assert_eq!(link_mention("x", 5, "target"), "x");
    }
}
