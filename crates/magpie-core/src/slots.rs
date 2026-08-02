use crate::model::Entry;
use crate::search::{row_to_entry, ENTRY_COLUMNS};
use crate::store::{Result, Store};

fn slot_err() -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(Box::new(std::io::Error::new(
        std::io::ErrorKind::InvalidInput,
        "slot must be in 1..=9",
    )))
}

impl Store {
    pub fn assign_slot(&self, slot: i64, entry_id: i64) -> Result<()> {
        if !(1..=9).contains(&slot) {
            return Err(slot_err());
        }
        let tx = self.conn().unchecked_transaction()?;
        // one slot per entry: drop any prior slot holding this entry
        tx.execute("DELETE FROM slots WHERE entry_id = ?1", [entry_id])?;
        // one entry per slot: upsert
        tx.execute(
            "INSERT INTO slots (slot, entry_id) VALUES (?1, ?2)
             ON CONFLICT(slot) DO UPDATE SET entry_id = excluded.entry_id",
            [slot, entry_id],
        )?;
        // slots are kept: pin the entry
        tx.execute("UPDATE entries SET pinned = 1 WHERE id = ?1", [entry_id])?;
        tx.commit()?;
        Ok(())
    }

    pub fn clear_slot(&self, slot: i64) -> Result<()> {
        if !(1..=9).contains(&slot) {
            return Err(slot_err());
        }
        self.conn().execute("DELETE FROM slots WHERE slot = ?1", [slot])?;
        Ok(())
    }

    pub fn slot_map(&self) -> Result<Vec<(i64, i64)>> {
        let mut stmt = self
            .conn()
            .prepare("SELECT slot, entry_id FROM slots ORDER BY slot")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))?;
        rows.collect()
    }

    pub fn slot_entry(&self, slot: i64) -> Result<Option<Entry>> {
        if !(1..=9).contains(&slot) {
            return Ok(None);
        }
        let cols = ENTRY_COLUMNS
            .split(", ")
            .map(|c| format!("e.{c}"))
            .collect::<Vec<_>>()
            .join(", ");
        let sql =
            format!("SELECT {cols} FROM entries e JOIN slots s ON s.entry_id = e.id WHERE s.slot = ?1");
        let mut stmt = self.conn().prepare(&sql)?;
        let mut rows = stmt.query_map([slot], row_to_entry)?;
        match rows.next() {
            Some(r) => Ok(Some(r?)),
            None => Ok(None),
        }
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
    fn ingest(s: &Store, t: &str, ms: i64) -> i64 {
        s.ingest(
            &CaptureEvent { content: Content::Text(t.into()), source_app: None, copied_at_ms: ms },
            &Noop,
        )
        .unwrap()
        .entry_id
    }
    fn pinned(s: &Store, id: i64) -> bool {
        s.recent(100).unwrap().into_iter().find(|e| e.id == id).unwrap().pinned
    }

    #[test]
    fn assign_inserts_mapping_and_pins_entry() {
        let s = open_in_memory().unwrap();
        let id = ingest(&s, "a", 1);
        s.assign_slot(3, id).unwrap();
        assert_eq!(s.slot_map().unwrap(), vec![(3, id)]);
        assert!(pinned(&s, id));
    }

    #[test]
    fn reassigning_an_entry_moves_it_between_slots() {
        let s = open_in_memory().unwrap();
        let id = ingest(&s, "a", 1);
        s.assign_slot(3, id).unwrap();
        s.assign_slot(5, id).unwrap();
        assert_eq!(s.slot_map().unwrap(), vec![(5, id)]);
    }

    #[test]
    fn assigning_to_occupied_slot_replaces_occupant() {
        let s = open_in_memory().unwrap();
        let a = ingest(&s, "a", 1);
        let b = ingest(&s, "b", 2);
        s.assign_slot(3, a).unwrap();
        s.assign_slot(3, b).unwrap();
        assert_eq!(s.slot_map().unwrap(), vec![(3, b)]);
    }

    #[test]
    fn clear_slot_removes_and_is_idempotent() {
        let s = open_in_memory().unwrap();
        let id = ingest(&s, "a", 1);
        s.assign_slot(3, id).unwrap();
        s.clear_slot(3).unwrap();
        s.clear_slot(3).unwrap();
        assert!(s.slot_map().unwrap().is_empty());
    }

    #[test]
    fn out_of_range_slot_errors_and_writes_nothing() {
        let s = open_in_memory().unwrap();
        let id = ingest(&s, "a", 1);
        assert!(s.assign_slot(0, id).is_err());
        assert!(s.assign_slot(10, id).is_err());
        assert!(s.clear_slot(0).is_err());
        assert!(s.slot_map().unwrap().is_empty());
    }

    #[test]
    fn slot_entry_returns_assigned_entry_or_none() {
        let s = open_in_memory().unwrap();
        let id = ingest(&s, "hello", 1);
        assert!(s.slot_entry(3).unwrap().is_none());
        s.assign_slot(3, id).unwrap();
        let e = s.slot_entry(3).unwrap().unwrap();
        assert_eq!(e.id, id);
        assert_eq!(e.full_text, "hello");
        assert!(s.slot_entry(0).unwrap().is_none());
    }
}
