use magpie_app::stats_view::{range_from_index, to_bars, Bar};

fn item(label: &str, display: &str, v: i64) -> (String, String, i64) {
    (label.into(), display.into(), v)
}

#[test]
fn to_bars_normalizes_against_series_max() {
    let bars = to_bars(&[item("a", "A", 10), item("b", "B", 5), item("c", "C", 0)]);
    assert_eq!(bars.len(), 3);
    assert!((bars[0].value_norm - 1.0).abs() < 1e-6);
    assert!((bars[1].value_norm - 0.5).abs() < 1e-6);
    assert!((bars[2].value_norm - 0.0).abs() < 1e-6);
    assert_eq!(bars[0].label, "a");
    assert_eq!(bars[0].display, "A");
}

#[test]
fn to_bars_all_zero_is_safe() {
    let bars = to_bars(&[item("a", "A", 0), item("b", "B", 0)]);
    assert!(bars.iter().all(|b| b.value_norm == 0.0));
}

#[test]
fn to_bars_empty_is_empty() {
    let bars: Vec<Bar> = to_bars(&[]);
    assert!(bars.is_empty());
}

#[test]
fn range_from_index_maps_windows() {
    let now = 100 * 86_400_000i64;
    assert_eq!(range_from_index(0, now).since_ms, Some(now - 7 * 86_400_000));
    assert_eq!(range_from_index(1, now).since_ms, Some(now - 30 * 86_400_000));
    assert_eq!(range_from_index(2, now).since_ms, Some(now - 90 * 86_400_000));
    assert_eq!(range_from_index(3, now).since_ms, None); // All
    assert_eq!(range_from_index(3, now).now_ms, now);
}
