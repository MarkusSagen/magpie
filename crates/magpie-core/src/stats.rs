use crate::detect::Kind;
use crate::store::{Result, Store};
use std::collections::HashMap;

pub(crate) const DAY_MS: i64 = 86_400_000;

/// Bucket size for the over-time series: daily, or weekly for an all-time range
/// whose span exceeds 90 days (keeps the chart readable).
pub(crate) fn bucket_ms(range: &StatsRange, earliest_ms: i64) -> i64 {
    if range.since_ms.is_none() {
        let span_days = range.now_ms / DAY_MS - earliest_ms / DAY_MS;
        if span_days > 90 {
            return 7 * DAY_MS;
        }
    }
    DAY_MS
}

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

    pub(crate) fn over_time(&self, lo: i64, hi: i64, range: &StatsRange) -> Result<Vec<DayBucket>> {
        // earliest in-range event drives the all-time start + weekly decision
        let earliest: i64 = self
            .conn()
            .query_row(
                "SELECT MIN(copied_at_ms) FROM copy_events WHERE copied_at_ms BETWEEN ?1 AND ?2",
                [lo, hi],
                |r| r.get::<_, Option<i64>>(0),
            )?
            .unwrap_or(range.now_ms);

        let start_ms = range.since_ms.unwrap_or(earliest);
        let bucket = bucket_ms(range, earliest);

        let mut stmt = self.conn().prepare(
            "SELECT copied_at_ms / ?3 AS b, COUNT(*)
             FROM copy_events WHERE copied_at_ms BETWEEN ?1 AND ?2
             GROUP BY b",
        )?;
        let counts: HashMap<i64, i64> = stmt
            .query_map([lo, hi, bucket], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
            })?
            .collect::<Result<HashMap<i64, i64>>>()?;

        let start_b = start_ms.div_euclid(bucket);
        let end_b = range.now_ms.div_euclid(bucket);
        let mut out = Vec::new();
        for b in start_b..=end_b {
            out.push(DayBucket {
                start_ms: b * bucket,
                count: *counts.get(&b).unwrap_or(&0),
            });
        }
        Ok(out)
    }

    pub(crate) fn per_app(&self, lo: i64, hi: i64, top_n: i64) -> Result<Vec<AppCount>> {
        let mut stmt = self.conn().prepare(
            "SELECT a.display_name, COUNT(*) AS c
             FROM copy_events ce JOIN apps a ON a.id = ce.source_app_id
             WHERE ce.copied_at_ms BETWEEN ?1 AND ?2
             GROUP BY ce.source_app_id
             ORDER BY c DESC, a.display_name
             LIMIT ?3",
        )?;
        let rows = stmt.query_map([lo, hi, top_n], |r| {
            Ok(AppCount {
                name: r.get(0)?,
                count: r.get(1)?,
            })
        })?;
        rows.collect()
    }

    pub(crate) fn by_type(&self, lo: i64, hi: i64) -> Result<Vec<KindCount>> {
        let mut stmt = self.conn().prepare(
            "SELECT e.kind, COUNT(*) AS c
             FROM copy_events ce JOIN entries e ON e.id = ce.entry_id
             WHERE ce.copied_at_ms BETWEEN ?1 AND ?2
             GROUP BY e.kind
             ORDER BY c DESC",
        )?;
        let rows = stmt.query_map([lo, hi], |r| {
            let kind_str: String = r.get(0)?;
            Ok(KindCount {
                kind: Kind::from_str(&kind_str).unwrap_or(Kind::Text),
                count: r.get(1)?,
            })
        })?;
        rows.collect()
    }

    /// Compose all series for `range` (top-N applies to most-copied and per-app).
    pub fn stats(&self, range: &StatsRange, top_n: i64) -> Result<Stats> {
        let lo = range.since_ms.unwrap_or(i64::MIN);
        let hi = range.now_ms;
        Ok(Stats {
            totals: self.totals(lo, hi)?,
            most_copied: self.most_copied(lo, hi, top_n)?,
            over_time: self.over_time(lo, hi, range)?,
            per_app: self.per_app(lo, hi, top_n)?,
            by_type: self.by_type(lo, hi)?,
        })
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

    const DAY: i64 = 86_400_000;

    #[test]
    fn over_time_day_buckets_are_zero_filled() {
        let s = open_in_memory().unwrap();
        seed(
            &s,
            &[
                ev("a", 100 * DAY + 1, None),
                ev("b", 100 * DAY + 2, None),
                ev("c", 102 * DAY + 5, None),
            ],
        );
        let range = StatsRange {
            since_ms: Some(100 * DAY),
            now_ms: 102 * DAY + 10,
        };
        let buckets = s
            .over_time(range.since_ms.unwrap(), range.now_ms, &range)
            .unwrap();
        assert_eq!(buckets.len(), 3);
        assert_eq!(buckets[0].start_ms, 100 * DAY);
        assert_eq!(buckets[0].count, 2);
        assert_eq!(buckets[1].start_ms, 101 * DAY);
        assert_eq!(buckets[1].count, 0);
        assert_eq!(buckets[2].count, 1);
    }

    #[test]
    fn over_time_all_time_switches_to_weekly_beyond_90_days() {
        let s = open_in_memory().unwrap();
        seed(&s, &[ev("x", 0, None), ev("y", 200 * DAY, None)]);
        let range = StatsRange {
            since_ms: None,
            now_ms: 200 * DAY,
        };
        let buckets = s.over_time(i64::MIN, range.now_ms, &range).unwrap();
        assert!(
            buckets.len() < 40,
            "weekly bucketing keeps the series short: {}",
            buckets.len()
        );
        let total: i64 = buckets.iter().map(|b| b.count).sum();
        assert_eq!(total, 2);
    }

    #[test]
    fn per_app_counts_exclude_null_apps() {
        let s = open_in_memory().unwrap();
        seed(
            &s,
            &[
                ev("a", 1, Some("Ghostty")),
                ev("b", 2, Some("Ghostty")),
                ev("c", 3, Some("Safari")),
                ev("d", 4, None),
            ],
        );
        let rows = s.per_app(i64::MIN, 1000, 10).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].name, "Ghostty");
        assert_eq!(rows[0].count, 2);
        assert_eq!(rows[1].name, "Safari");
    }

    #[test]
    fn by_type_counts_per_kind() {
        let s = open_in_memory().unwrap();
        seed(
            &s,
            &[
                ev("plain words", 1, None),
                ev("https://a.io", 2, None),
                ev("https://a.io", 3, None),
            ],
        );
        let rows = s.by_type(i64::MIN, 1000).unwrap();
        assert_eq!(rows[0].kind.as_str(), "link");
        assert_eq!(rows[0].count, 2);
        assert_eq!(rows[1].kind.as_str(), "text");
    }

    #[test]
    fn stats_composes_all_series() {
        let s = open_in_memory().unwrap();
        seed(
            &s,
            &[
                ev("a", 1, Some("Ghostty")),
                ev("a", 2, Some("Ghostty")),
                ev("b", 3, None),
            ],
        );
        let range = StatsRange {
            since_ms: None,
            now_ms: 1000,
        };
        let st = s.stats(&range, 10).unwrap();
        assert_eq!(st.totals.copies, 3);
        assert_eq!(st.most_copied[0].count, 2);
        assert_eq!(st.per_app[0].name, "Ghostty");
        assert!(!st.by_type.is_empty());
        assert!(!st.over_time.is_empty());
    }
}
