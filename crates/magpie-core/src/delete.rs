use crate::retention::Removed;
use crate::store::{Result, Store};
use std::collections::BTreeSet;

impl Store {
    /// Delete a single entry and its copy_events/tags/slots. Returns the image
    /// paths the caller should remove from disk (core stays FS-free).
    pub fn delete_entry(&self, id: i64) -> Result<Removed> {
        let mut set = BTreeSet::new();
        set.insert(id);
        self.delete_entries(set)
    }
}

#[cfg(test)]
mod tests {
    use crate::model::{CaptureEvent, Content};
    use crate::store::{open_in_memory, ImageStore, Store};

    struct Noop;
    impl ImageStore for Noop {
        fn put(&self, h: &str, _b: &[u8]) -> std::io::Result<String> {
            Ok(h.to_string())
        }
    }
    fn ingest(s: &Store, t: &str) -> i64 {
        s.ingest(
            &CaptureEvent {
                content: Content::Text(t.into()),
                source_app: None,
                copied_at_ms: 1,
            },
            &Noop,
        )
        .unwrap()
        .entry_id
    }

    #[test]
    fn delete_removes_only_the_target() {
        let s = open_in_memory().unwrap();
        let a = ingest(&s, "keep");
        let b = ingest(&s, "drop");
        s.delete_entry(b).unwrap();
        let remaining: Vec<i64> = s.recent(100).unwrap().into_iter().map(|e| e.id).collect();
        assert!(remaining.contains(&a));
        assert!(!remaining.contains(&b));
    }
}
