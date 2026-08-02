use crate::detect::Kind;
use crate::store::{Result, Store};

pub(crate) const DAY_MS: i64 = 86_400_000;

pub struct StatsRange {
    pub since_ms: Option<i64>,
    pub now_ms: i64,
}

pub struct Totals {
    pub copies: i64,
    pub unique_entries: i64,
    pub distinct_apps: i64,
}

pub struct MostCopied {
    pub entry_id: i64,
    pub preview: String,
    pub kind: Kind,
    pub count: i64,
}

pub struct DayBucket {
    pub start_ms: i64,
    pub count: i64,
}

pub struct AppCount {
    pub name: String,
    pub count: i64,
}

pub struct KindCount {
    pub kind: Kind,
    pub count: i64,
}

pub struct Stats {
    pub totals: Totals,
    pub most_copied: Vec<MostCopied>,
    pub over_time: Vec<DayBucket>,
    pub per_app: Vec<AppCount>,
    pub by_type: Vec<KindCount>,
}

impl Store {
    pub(crate) fn totals(&self, lo: i64, hi: i64) -> Result<Totals> {
        self.conn().query_row(
            "SELECT COUNT(*), COUNT(DISTINCT entry_id), COUNT(DISTINCT source_app_id)
             FROM copy_events WHERE copied_at_ms BETWEEN ?1 AND ?2",
            [lo, hi],
            |r| {
                Ok(Totals {
                    copies: r.get(0)?,
                    unique_entries: r.get(1)?,
                    distinct_apps: r.get(2)?,
                })
            },
        )
    }

    pub(crate) fn most_copied(&self, lo: i64, hi: i64, top_n: i64) -> Result<Vec<MostCopied>> {
        let mut stmt = self.conn().prepare(
            "SELECT ce.entry_id, COUNT(*) AS c, e.preview_text, e.kind
             FROM copy_events ce JOIN entries e ON e.id = ce.entry_id
             WHERE ce.copied_at_ms BETWEEN ?1 AND ?2
             GROUP BY ce.entry_id
             ORDER BY c DESC, ce.entry_id
             LIMIT ?3",
        )?;
        let rows = stmt.query_map([lo, hi, top_n], |r| {
            let kind_str: String = r.get(3)?;
            Ok(MostCopied {
                entry_id: r.get(0)?,
                count: r.get(1)?,
                preview: r.get(2)?,
                kind: Kind::from_str(&kind_str).unwrap_or(Kind::Text),
            })
        })?;
        rows.collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AppInfo, CaptureEvent, Content};
    use crate::store::{open_in_memory, ImageStore, Store};

    struct Noop;
    impl ImageStore for Noop {
        fn put(&self, h: &str, _b: &[u8]) -> std::io::Result<String> {
            Ok(h.to_string())
        }
    }
    fn ev(text: &str, ms: i64, app: Option<&str>) -> CaptureEvent {
        CaptureEvent {
            content: Content::Text(text.into()),
            source_app: app.map(|a| AppInfo {
                identifier: a.into(),
                display_name: a.into(),
                icon_path: None,
            }),
            copied_at_ms: ms,
        }
    }
    fn seed(store: &Store, evs: &[CaptureEvent]) {
        for e in evs {
            store.ingest(e, &Noop).unwrap();
        }
    }

    #[test]
    fn totals_counts_copies_unique_and_apps_in_range() {
        let s = open_in_memory().unwrap();
        seed(
            &s,
            &[
                ev("a", 100, Some("Ghostty")),
                ev("a", 200, Some("Ghostty")), // dup content, same app
                ev("b", 300, Some("Safari")),
                ev("c", 50, None),             // no app
                ev("old", 5, Some("Ghostty")), // out of range below
            ],
        );
        let t = s.totals(10, 1000).unwrap();
        assert_eq!(t.copies, 4);
        assert_eq!(t.unique_entries, 3); // a, b, c
        assert_eq!(t.distinct_apps, 2); // Ghostty, Safari (NULL excluded)
    }

    #[test]
    fn most_copied_orders_by_count_and_limits() {
        let s = open_in_memory().unwrap();
        seed(
            &s,
            &[
                ev("thrice", 1, None),
                ev("thrice", 2, None),
                ev("thrice", 3, None),
                ev("once", 4, None),
                ev("twice", 5, None),
                ev("twice", 6, None),
            ],
        );
        let rows = s.most_copied(i64::MIN, 1000, 2).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].preview, "thrice");
        assert_eq!(rows[0].count, 3);
        assert_eq!(rows[0].kind.as_str(), "text");
        assert_eq!(rows[1].preview, "twice");
        assert_eq!(rows[1].count, 2);
    }
}
