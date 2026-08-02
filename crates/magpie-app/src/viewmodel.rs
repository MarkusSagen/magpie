use magpie_core::{default_query, Kind, SearchMode, SearchQuery, Sort, TimeRange};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeFilter {
    All,
    Text,
    Link,
    Color,
    Email,
    Image,
    File,
}

impl TypeFilter {
    pub fn to_kind(self) -> Option<Kind> {
        match self {
            TypeFilter::All => None,
            TypeFilter::Text => Some(Kind::Text),
            TypeFilter::Link => Some(Kind::Link),
            TypeFilter::Color => Some(Kind::Color),
            TypeFilter::Email => Some(Kind::Email),
            TypeFilter::Image => Some(Kind::Image),
            TypeFilter::File => Some(Kind::File),
        }
    }
}

/// Map a type-filter row index to a `TypeFilter` (0 All · 1 Text · 2 Link ·
/// 3 Color · 4 Image · 5 File; out-of-range → All).
pub fn type_filter_from_index(i: i32) -> TypeFilter {
    match i {
        1 => TypeFilter::Text,
        2 => TypeFilter::Link,
        3 => TypeFilter::Color,
        4 => TypeFilter::Image,
        5 => TypeFilter::File,
        _ => TypeFilter::All,
    }
}

/// Map a sort control index to a `Sort` (0 Recency · 1 Most copied; else Recency).
pub fn sort_from_index(i: i32) -> Sort {
    match i {
        1 => Sort::MostCopied,
        _ => Sort::Recency,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeFilter {
    All,
    Today,
    Last7Days,
    Last30Days,
}

/// Map an advanced-search time-range index: 0 All · 1 Today · 2 Last7Days ·
/// 3 Last30Days (out of range → All).
pub fn time_filter_from_index(i: i32) -> TimeFilter {
    match i {
        1 => TimeFilter::Today,
        2 => TimeFilter::Last7Days,
        3 => TimeFilter::Last30Days,
        _ => TimeFilter::All,
    }
}

pub struct UiState {
    pub text: String,
    pub mode: SearchMode,
    pub type_filter: TypeFilter,
    pub app_filter: Option<i64>,
    pub time_filter: TimeFilter,
    pub sort: Sort,
    pub tag: Option<String>,
    pub pinned_only: bool,
}

impl UiState {
    pub fn new() -> Self {
        UiState {
            text: String::new(),
            mode: SearchMode::Word,
            type_filter: TypeFilter::All,
            app_filter: None,
            time_filter: TimeFilter::All,
            sort: Sort::Recency,
            tag: None,
            pinned_only: false,
        }
    }
}

impl Default for UiState {
    fn default() -> Self {
        Self::new()
    }
}

const DAY_MS: i64 = 86_400_000;

pub fn to_query(ui: &UiState, now_ms: i64) -> SearchQuery {
    let mut q = default_query();
    q.text = ui.text.clone();
    q.mode = ui.mode;
    q.kind = ui.type_filter.to_kind();
    q.source_app_id = ui.app_filter;
    q.sort = ui.sort;
    q.tag = ui.tag.clone();
    q.pinned_only = ui.pinned_only;
    q.time = match ui.time_filter {
        TimeFilter::All => TimeRange::default(),
        TimeFilter::Today => TimeRange {
            since_ms: Some(now_ms - now_ms.rem_euclid(DAY_MS)),
            until_ms: None,
        },
        TimeFilter::Last7Days => TimeRange {
            since_ms: Some(now_ms - 7 * DAY_MS),
            until_ms: None,
        },
        TimeFilter::Last30Days => TimeRange {
            since_ms: Some(now_ms - 30 * DAY_MS),
            until_ms: None,
        },
    };
    q
}

#[cfg(test)]
mod tests {
    use super::*;
    use magpie_core::{Kind, SearchMode};

    #[test]
    fn tag_flows_into_query() {
        let mut ui = UiState::new();
        assert!(to_query(&ui, 0).tag.is_none());
        ui.tag = Some("work".into());
        assert_eq!(to_query(&ui, 0).tag, Some("work".to_string()));
    }

    #[test]
    fn type_filter_maps_to_kind() {
        assert!(TypeFilter::All.to_kind().is_none());
        assert_eq!(TypeFilter::Link.to_kind(), Some(Kind::Link));
    }

    #[test]
    fn type_filter_index_mapping() {
        assert_eq!(type_filter_from_index(0), TypeFilter::All);
        assert_eq!(type_filter_from_index(1), TypeFilter::Text);
        assert_eq!(type_filter_from_index(2), TypeFilter::Link);
        assert_eq!(type_filter_from_index(3), TypeFilter::Color);
        assert_eq!(type_filter_from_index(4), TypeFilter::Image);
        assert_eq!(type_filter_from_index(5), TypeFilter::File);
        assert_eq!(type_filter_from_index(99), TypeFilter::All);
    }

    #[test]
    fn sort_index_mapping() {
        assert_eq!(sort_from_index(0), Sort::Recency);
        assert_eq!(sort_from_index(1), Sort::MostCopied);
        assert_eq!(sort_from_index(99), Sort::Recency);
    }

    #[test]
    fn to_query_copies_text_and_mode() {
        let mut ui = UiState::new();
        ui.text = "hello".into();
        ui.mode = SearchMode::Fuzzy;
        let q = to_query(&ui, 1_000_000_000);
        assert_eq!(q.text, "hello");
        assert_eq!(q.mode, SearchMode::Fuzzy);
        assert!(q.kind.is_none());
        assert!(q.time.since_ms.is_none());
    }

    #[test]
    fn time_filter_index_and_30_days() {
        assert_eq!(time_filter_from_index(0), TimeFilter::All);
        assert_eq!(time_filter_from_index(3), TimeFilter::Last30Days);
        let mut ui = UiState::new();
        ui.time_filter = TimeFilter::Last30Days;
        let now = 100 * 86_400_000i64;
        assert_eq!(to_query(&ui, now).time.since_ms, Some(now - 30 * 86_400_000));
    }

    #[test]
    fn pinned_only_flows_into_query() {
        let mut ui = UiState::new();
        assert!(!to_query(&ui, 0).pinned_only);
        ui.pinned_only = true;
        assert!(to_query(&ui, 0).pinned_only);
    }

    #[test]
    fn last_7_days_sets_since() {
        let mut ui = UiState::new();
        ui.time_filter = TimeFilter::Last7Days;
        let now = 10 * 86_400_000i64;
        let q = to_query(&ui, now);
        assert_eq!(q.time.since_ms, Some(now - 7 * 86_400_000));
    }

    #[test]
    fn today_floors_to_utc_midnight() {
        let mut ui = UiState::new();
        ui.time_filter = TimeFilter::Today;
        let now = 3 * 86_400_000i64 + 12_345;
        let q = to_query(&ui, now);
        assert_eq!(q.time.since_ms, Some(3 * 86_400_000));
    }
}
