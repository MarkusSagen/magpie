use crate::store::{Result, Store};

impl Store {
    /// `(entry_id, source app display_name)` for entries whose `source_app_id`
    /// resolves to an app. Entries with a NULL `source_app_id` are omitted.
    /// Ordered by entry id for stable output.
    pub fn app_name_pairs(&self) -> Result<Vec<(i64, String)>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT e.id, a.display_name
             FROM entries e JOIN apps a ON a.id = e.source_app_id
             ORDER BY e.id",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
        rows.collect()
    }

    /// Distinct source apps that have at least one entry: `(app_id, display_name)`,
    /// ordered by name. For the advanced-search app filter.
    pub fn apps_in_use(&self) -> Result<Vec<(i64, String)>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT DISTINCT a.id, a.display_name
             FROM apps a JOIN entries e ON e.source_app_id = a.id
             ORDER BY a.display_name",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
        rows.collect()
    }

    /// `(entry_id, app icon_path)` for entries whose source app has a non-null icon.
    pub fn app_icon_pairs(&self) -> Result<Vec<(i64, String)>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT e.id, a.icon_path
             FROM entries e JOIN apps a ON a.id = e.source_app_id
             WHERE a.icon_path IS NOT NULL
             ORDER BY e.id",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
        rows.collect()
    }
}

#[cfg(test)]
mod tests {
    use crate::model::{AppInfo, CaptureEvent, Content};
    use crate::store::{open_in_memory, ImageStore, Store};

    struct Noop;
    impl ImageStore for Noop {
        fn put(&self, h: &str, _b: &[u8]) -> std::io::Result<String> {
            Ok(h.to_string())
        }
    }

    fn ingest(s: &Store, t: &str, app: Option<&str>) -> i64 {
        s.ingest(
            &CaptureEvent {
                content: Content::Text(t.into()),
                source_app: app.map(|name| AppInfo {
                    identifier: name.to_string(),
                    display_name: name.to_string(),
                    icon_path: None,
                }),
                copied_at_ms: 1,
            },
            &Noop,
        )
        .unwrap()
        .entry_id
    }

    #[test]
    fn pairs_include_apps_and_omit_null() {
        let s = open_in_memory().unwrap();
        let a = ingest(&s, "with app", Some("Terminal"));
        let _b = ingest(&s, "no app", None);
        let pairs = s.app_name_pairs().unwrap();
        assert_eq!(pairs, vec![(a, "Terminal".to_string())]);
    }

    #[test]
    fn apps_in_use_lists_distinct_apps() {
        let s = open_in_memory().unwrap();
        ingest(&s, "a", Some("Ghostty"));
        ingest(&s, "b", Some("Ghostty"));
        ingest(&s, "c", Some("Vivaldi"));
        ingest(&s, "d", None);
        let apps: Vec<String> = s.apps_in_use().unwrap().into_iter().map(|(_, n)| n).collect();
        assert_eq!(apps, vec!["Ghostty".to_string(), "Vivaldi".to_string()]);
    }

    #[test]
    fn icon_pairs_return_non_null_icon_paths() {
        let s = open_in_memory().unwrap();
        let a = s
            .ingest(
                &CaptureEvent {
                    content: Content::Text("x".into()),
                    source_app: Some(AppInfo {
                        identifier: "Foo".into(),
                        display_name: "Foo".into(),
                        icon_path: Some("/i/foo.png".into()),
                    }),
                    copied_at_ms: 1,
                },
                &Noop,
            )
            .unwrap()
            .entry_id;
        let _b = ingest(&s, "no icon", Some("Bar"));
        assert_eq!(
            s.app_icon_pairs().unwrap(),
            vec![(a, "/i/foo.png".to_string())]
        );
    }
}
