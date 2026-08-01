use magpie_core::Entry;

const DAY_MS: i64 = 86_400_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Today,
    Yesterday,
    Older,
}

impl Section {
    pub fn label(&self) -> &'static str {
        match self {
            Section::Today => "Today",
            Section::Yesterday => "Yesterday",
            Section::Older => "Older",
        }
    }
}

pub fn section_for(last_copied_ms: i64, now_ms: i64) -> Section {
    let today = now_ms.div_euclid(DAY_MS);
    let day = last_copied_ms.div_euclid(DAY_MS);
    match today - day {
        0 => Section::Today,
        1 => Section::Yesterday,
        _ => Section::Older,
    }
}

pub fn group(entries: &[Entry], now_ms: i64) -> Vec<(Section, Vec<usize>)> {
    let mut today = Vec::new();
    let mut yesterday = Vec::new();
    let mut older = Vec::new();
    for (i, e) in entries.iter().enumerate() {
        match section_for(e.last_copied_at_ms, now_ms) {
            Section::Today => today.push(i),
            Section::Yesterday => yesterday.push(i),
            Section::Older => older.push(i),
        }
    }
    let mut out = Vec::new();
    if !today.is_empty() {
        out.push((Section::Today, today));
    }
    if !yesterday.is_empty() {
        out.push((Section::Yesterday, yesterday));
    }
    if !older.is_empty() {
        out.push((Section::Older, older));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: i64 = 86_400_000;

    #[test]
    fn buckets_by_utc_day() {
        let now = 100 * DAY + 5_000;
        assert!(matches!(section_for(100 * DAY + 1, now), Section::Today));
        assert!(matches!(section_for(99 * DAY + 1, now), Section::Yesterday));
        assert!(matches!(section_for(90 * DAY, now), Section::Older));
    }

    #[test]
    fn group_orders_and_indexes() {
        use magpie_core::{Entry, Kind};
        let mk = |id: i64, ms: i64| Entry {
            id,
            content_hash: format!("h{id}"),
            kind: Kind::Text,
            preview_text: "".into(),
            full_text: "".into(),
            image_path: None,
            byte_size: 0,
            char_count: 0,
            word_count: 0,
            line_count: 0,
            first_copied_at_ms: ms,
            last_copied_at_ms: ms,
            copy_count: 1,
            pinned: false,
            source_app_id: None,
        };
        let now = 10 * DAY + 1;
        let entries = vec![mk(1, 10 * DAY), mk(2, 9 * DAY), mk(3, DAY)];
        let g = group(&entries, now);
        assert_eq!(g.len(), 3);
        assert!(matches!(g[0].0, Section::Today));
        assert_eq!(g[0].1, vec![0]);
        assert!(matches!(g[2].0, Section::Older));
        assert_eq!(g[2].1, vec![2]);
    }
}
