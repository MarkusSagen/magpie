use crate::store::{Result, Store};
use rusqlite::OptionalExtension;

impl Store {
    /// Concatenate the `full_text` of the given entry ids, in order, joined by
    /// `separator`. Missing ids are skipped.
    pub fn merged_text(&self, ids: &[i64], separator: &str) -> Result<String> {
        let mut parts: Vec<String> = Vec::new();
        for &id in ids {
            let text: Option<String> = self
                .conn()
                .query_row("SELECT full_text FROM entries WHERE id = ?1", [id], |r| {
                    r.get::<_, String>(0)
                })
                .optional()?;
            if let Some(t) = text {
                parts.push(t);
            }
        }
        Ok(parts.join(separator))
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
    fn joins_in_order_with_separator() {
        let s = open_in_memory().unwrap();
        let a = ingest(&s, "alpha");
        let b = ingest(&s, "beta");
        let c = ingest(&s, "gamma");
        assert_eq!(s.merged_text(&[a, c], "\n").unwrap(), "alpha\ngamma");
        assert_eq!(
            s.merged_text(&[c, b, a], ", ").unwrap(),
            "gamma, beta, alpha"
        );
    }

    #[test]
    fn skips_missing_ids_and_handles_empty() {
        let s = open_in_memory().unwrap();
        let a = ingest(&s, "only");
        assert_eq!(s.merged_text(&[a, 9999], " ").unwrap(), "only");
        assert_eq!(s.merged_text(&[], " ").unwrap(), "");
    }
}
