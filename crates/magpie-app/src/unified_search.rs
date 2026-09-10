//! Search across Magpie's knowledge surfaces (notes, tasks, bookmarks) at once.
//! The clipboard has its own search box; this fills the gap for the rest.
use magpie_core::Store;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UnifiedResults {
    /// (note_id, title)
    pub notes: Vec<(i64, String)>,
    /// (note_id, task title) — jump to the owning note
    pub tasks: Vec<(i64, String)>,
    /// (url, title)
    pub bookmarks: Vec<(String, String)>,
}

/// Case-insensitive search over notes (name+body), tasks (title), and bookmarks.
/// Empty/whitespace query → empty results. Each list capped at `limit`.
pub fn unified_search(store: &Store, query: &str, now_ms: i64, limit: usize) -> UnifiedResults {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return UnifiedResults::default();
    }
    let notes = store
        .all_notes()
        .unwrap_or_default()
        .into_iter()
        .filter(|n| n.name.to_lowercase().contains(&q) || n.body.to_lowercase().contains(&q))
        .take(limit)
        .map(|n| (n.id, crate::notes_view::note_list_title(&n.name, &n.body)))
        .collect();
    let tasks = crate::tasks::all_tasks(store, now_ms)
        .into_iter()
        .filter(|t| t.title.to_lowercase().contains(&q))
        .take(limit)
        .map(|t| (t.note_id, t.title))
        .collect();
    let bookmarks = store
        .list_bookmarks(query.trim(), limit as i64)
        .unwrap_or_default()
        .into_iter()
        .map(|b| {
            (
                b.url,
                if b.title.is_empty() {
                    b.domain
                } else {
                    b.title
                },
            )
        })
        .collect();
    UnifiedResults {
        notes,
        tasks,
        bookmarks,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn searches_all_surfaces() {
        let s = magpie_core::open_in_memory().unwrap();
        let n = s.upsert_note_by_name("Prod Incidents", 10).unwrap();
        s.update_note_body(n.id, "- [ ] fix the incident runbook", 20)
            .unwrap();
        s.add_bookmark(
            "https://status.example.com",
            "Incident status page",
            "status.example.com",
            30,
        )
        .unwrap();
        let r = unified_search(&s, "incident", 100, 10);
        assert_eq!(r.notes.len(), 1);
        assert_eq!(r.notes[0].0, n.id);
        assert_eq!(r.tasks.len(), 1);
        assert!(r.tasks[0].1.to_lowercase().contains("incident"));
        assert_eq!(r.bookmarks.len(), 1);
        assert_eq!(r.bookmarks[0].0, "https://status.example.com");
        // empty query → nothing
        assert_eq!(unified_search(&s, "  ", 100, 10), UnifiedResults::default());
        // no match → empty
        assert!(unified_search(&s, "zzzznope", 100, 10).notes.is_empty());
    }
}
