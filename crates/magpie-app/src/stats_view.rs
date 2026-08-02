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
