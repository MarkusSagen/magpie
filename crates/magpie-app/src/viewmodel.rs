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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeFilter {
    All,
    Today,
    Last7Days,
}

pub struct UiState {
    pub text: String,
    pub mode: SearchMode,
    pub type_filter: TypeFilter,
    pub app_filter: Option<i64>,
    pub time_filter: TimeFilter,
    pub sort: Sort,
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
    };
    q
}

#[cfg(test)]
mod tests {
    use super::*;
    use magpie_core::{Kind, SearchMode};

    #[test]
    fn type_filter_maps_to_kind() {
        assert!(TypeFilter::All.to_kind().is_none());
        assert_eq!(TypeFilter::Link.to_kind(), Some(Kind::Link));
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
