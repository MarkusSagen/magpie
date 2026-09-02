use magpie_core::StatsRange;

pub struct Bar {
    pub label: String,
    pub display: String,
    pub value_norm: f32,
}

/// Normalize a `(label, display, value)` series into bars scaled against the
/// series' own maximum. A max of 0 yields all-zero bars (no divide-by-zero).
pub fn to_bars(items: &[(String, String, i64)]) -> Vec<Bar> {
    let max = items.iter().map(|(_, _, v)| *v).max().unwrap_or(0);
    items
        .iter()
        .map(|(label, display, v)| Bar {
            label: label.clone(),
            display: display.clone(),
            value_norm: if max > 0 { *v as f32 / max as f32 } else { 0.0 },
        })
        .collect()
}

/// A caption for the "Copies over time" chart: the span it covers and its peak
/// bucket. Without it the bars are anonymous — no dates, no scale.
/// `buckets` is `(start_ms, count)` in chronological order.
pub fn over_time_caption(buckets: &[(i64, i64)], weekly: bool) -> String {
    let (Some(first), Some(last)) = (buckets.first(), buckets.last()) else {
        return String::new();
    };
    let peak = buckets.iter().map(|(_, c)| *c).max().unwrap_or(0);
    let unit = if weekly { "week" } else { "day" };
    format!(
        "{} → {} · peak {} {} in a {}",
        crate::format_time::abs_date(first.0),
        crate::format_time::abs_date(last.0),
        peak,
        if peak == 1 { "copy" } else { "copies" },
        unit
    )
}

const DAY_MS: i64 = 86_400_000;

/// Map a range-selector index to a `StatsRange`: 0=7d, 1=30d, 2=90d, else All.
pub fn range_from_index(idx: i32, now_ms: i64) -> StatsRange {
    let since_ms = match idx {
        0 => Some(now_ms - 7 * DAY_MS),
        1 => Some(now_ms - 30 * DAY_MS),
        2 => Some(now_ms - 90 * DAY_MS),
        _ => None,
    };
    StatsRange { since_ms, now_ms }
}

#[cfg(test)]
mod caption_tests {
    use super::over_time_caption;
    const DAY: i64 = 86_400_000;

    #[test]
    fn empty_series_has_no_caption() {
        assert_eq!(over_time_caption(&[], false), "");
    }

    #[test]
    fn spans_first_to_last_bucket_with_peak() {
        let buckets = [(1_000 * DAY, 2), (1_001 * DAY, 7), (1_002 * DAY, 0)];
        assert_eq!(
            over_time_caption(&buckets, false),
            "1972-09-27 → 1972-09-29 · peak 7 copies in a day"
        );
    }

    #[test]
    fn weekly_buckets_and_singular_peak() {
        let buckets = [(1_000 * DAY, 1)];
        assert_eq!(
            over_time_caption(&buckets, true),
            "1972-09-27 → 1972-09-27 · peak 1 copy in a week"
        );
    }
}
