use crate::store::{Result, Store};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy)]
pub struct RetentionPolicy {
    pub max_entries: Option<i64>,
    pub max_age_ms: Option<i64>,
    pub max_image_bytes: Option<i64>,
}

impl RetentionPolicy {
    pub fn is_noop(&self) -> bool {
        self.max_entries.is_none() && self.max_age_ms.is_none() && self.max_image_bytes.is_none()
    }
}

pub struct Removed {
    pub entries_deleted: usize,
    pub image_paths: Vec<String>,
}

impl Store {
    pub fn enforce_retention(&self, policy: &RetentionPolicy, now_ms: i64) -> Result<Removed> {
        if policy.is_noop() {
            return Ok(Removed { entries_deleted: 0, image_paths: Vec::new() });
        }
        let mut victims: BTreeSet<i64> = BTreeSet::new();

        if let Some(age) = policy.max_age_ms {
            let cutoff = now_ms - age;
            let mut stmt = self
                .conn()
                .prepare("SELECT id FROM entries WHERE pinned = 0 AND last_copied_at_ms < ?1")?;
            let ids = stmt.query_map([cutoff], |r| r.get::<_, i64>(0))?;
            for id in ids {
                victims.insert(id?);
            }
        }

        if let Some(n) = policy.max_entries {
            let mut stmt = self.conn().prepare(
                "SELECT id FROM entries WHERE pinned = 0 AND id NOT IN
                   (SELECT id FROM entries WHERE pinned = 0
                    ORDER BY last_copied_at_ms DESC LIMIT ?1)",
            )?;
            let ids = stmt.query_map([n], |r| r.get::<_, i64>(0))?;
            for id in ids {
                victims.insert(id?);
            }
        }

        self.delete_entries(victims)
    }

    /// Delete a victim set: collect image paths, remove copy_events then entries
    /// (FTS trigger cleans the index), all in one transaction.
    fn delete_entries(&self, victims: BTreeSet<i64>) -> Result<Removed> {
        if victims.is_empty() {
            return Ok(Removed { entries_deleted: 0, image_paths: Vec::new() });
        }
        let ids: Vec<i64> = victims.into_iter().collect();
        let placeholders = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");

        let mut stmt = self.conn().prepare(&format!(
            "SELECT image_path FROM entries WHERE id IN ({placeholders}) AND image_path IS NOT NULL"
        ))?;
        let image_paths: Vec<String> = stmt
            .query_map(rusqlite::params_from_iter(ids.iter()), |r| {
                r.get::<_, String>(0)
            })?
            .collect::<Result<Vec<String>>>()?;

        let tx = self.conn().unchecked_transaction()?;
        tx.execute(
            &format!("DELETE FROM copy_events WHERE entry_id IN ({placeholders})"),
            rusqlite::params_from_iter(ids.iter()),
        )?;
        let deleted = tx.execute(
            &format!("DELETE FROM entries WHERE id IN ({placeholders})"),
            rusqlite::params_from_iter(ids.iter()),
        )?;
        tx.commit()?;

        Ok(Removed { entries_deleted: deleted, image_paths })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{CaptureEvent, Content};
    use crate::store::{open_in_memory, ImageStore, Store};

    struct Noop;
    impl ImageStore for Noop {
        fn put(&self, h: &str, _b: &[u8]) -> std::io::Result<String> {
            Ok(h.to_string())
        }
    }
    fn text(s: &str, ms: i64) -> CaptureEvent {
        CaptureEvent { content: Content::Text(s.into()), source_app: None, copied_at_ms: ms }
    }
    fn seed(store: &Store, evs: &[CaptureEvent]) {
        for e in evs {
            store.ingest(e, &Noop).unwrap();
        }
    }
    fn texts(store: &Store) -> Vec<String> {
        store.recent(100).unwrap().into_iter().map(|e| e.full_text).collect()
    }

    #[test]
    fn noop_policy_deletes_nothing() {
        let s = open_in_memory().unwrap();
        seed(&s, &[text("a", 1), text("b", 2)]);
        let r = s
            .enforce_retention(
                &RetentionPolicy { max_entries: None, max_age_ms: None, max_image_bytes: None },
                1000,
            )
            .unwrap();
        assert_eq!(r.entries_deleted, 0);
        assert_eq!(texts(&s).len(), 2);
    }

    #[test]
    fn age_deletes_old_but_keeps_pinned_and_recent() {
        let s = open_in_memory().unwrap();
        let old = s.ingest(&text("old", 100), &Noop).unwrap();
        let old_pinned = s.ingest(&text("old-pinned", 200), &Noop).unwrap();
        s.ingest(&text("recent", 9_500), &Noop).unwrap(); // after cutoff (9_000)
        s.set_pinned(old_pinned.entry_id, true).unwrap();

        let policy = RetentionPolicy { max_entries: None, max_age_ms: Some(1_000), max_image_bytes: None };
        let r = s.enforce_retention(&policy, 10_000).unwrap();
        assert_eq!(r.entries_deleted, 1); // only unpinned "old"
        let remaining = texts(&s);
        assert!(remaining.contains(&"recent".to_string()));
        assert!(remaining.contains(&"old-pinned".to_string()));
        assert!(!remaining.contains(&"old".to_string()));
        let _ = old;
    }

    #[test]
    fn count_keeps_newest_n_unpinned_plus_pinned() {
        let s = open_in_memory().unwrap();
        let a = s.ingest(&text("a", 1), &Noop).unwrap(); // oldest
        s.ingest(&text("b", 2), &Noop).unwrap();
        s.ingest(&text("c", 3), &Noop).unwrap();
        s.ingest(&text("d", 4), &Noop).unwrap(); // newest
        s.set_pinned(a.entry_id, true).unwrap();

        let policy = RetentionPolicy { max_entries: Some(2), max_age_ms: None, max_image_bytes: None };
        let r = s.enforce_retention(&policy, 100).unwrap();
        assert_eq!(r.entries_deleted, 1);
        let remaining = texts(&s);
        assert_eq!(remaining.len(), 3);
        assert!(remaining.contains(&"a".to_string())); // pinned
        assert!(remaining.contains(&"c".to_string()));
        assert!(remaining.contains(&"d".to_string()));
        assert!(!remaining.contains(&"b".to_string()));
    }
}
