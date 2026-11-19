//! Axis ticks for the Data chart (screen 14): "nice" 1/2/5×10ⁿ steps on linear
//! axes, powers of ten on log axes, and compact human labels (`1,000`, `0.25`).
//! Pure — the app maps each tick's `frac` (0..1 along the plotted span) to pixels.

/// One tick: its position along the axis (0 = min, 1 = max) and its label.
#[derive(Debug, Clone, PartialEq)]
pub struct AxisTick {
    pub frac: f64,
    pub label: String,
}

/// Tick values inside `[min, max]` on a 1/2/5×10ⁿ step, aiming for ~`target` ticks.
/// A degenerate span (`min == max`) yields that single value.
pub fn nice_ticks(min: f64, max: f64, target: usize) -> Vec<f64> {
    if !min.is_finite() || !max.is_finite() || max < min {
        return Vec::new();
    }
    if max == min {
        return vec![min];
    }
    let raw = (max - min) / target.max(1) as f64;
    let mag = 10f64.powf(raw.log10().floor());
    // The 1/2/5×10ⁿ step closest to the raw step (log distance), so a span
    // never collapses to just two ticks nor explodes past ~2× the target.
    let step = [1.0, 2.0, 5.0, 10.0]
        .iter()
        .map(|m| m * mag)
        .min_by(|a, b| {
            let da = (a / raw).ln().abs();
            let db = (b / raw).ln().abs();
            da.total_cmp(&db)
        })
        .unwrap_or(mag);
    // Epsilon so float error doesn't drop an end tick (0.35 / 0.05 = 6.999…).
    let first = (min / step - 1e-9).ceil() as i64;
    let last = (max / step + 1e-9).floor() as i64;
    // Round to the step's precision to kill noise like 0.30000000000000004.
    let scale = 10f64.powi((-step.log10().floor()).max(0.0) as i32 + 1);
    (first..=last)
        .map(|k| {
            let v = (k as f64 * step * scale).round() / scale;
            if v == 0.0 {
                0.0 // no -0
            } else {
                v
            }
        })
        .collect()
}

/// Ticks for an axis whose data spans `[min, max]`. When `log` is set, `min`/`max`
/// are log10 values: ticks fall on whole decades (or nice exponents for a span
/// under one decade) and are labelled with the real value `10^v`.
pub fn axis_ticks(min: f64, max: f64, log: bool, target: usize) -> Vec<AxisTick> {
    let span = if max > min { max - min } else { 1.0 };
    let frac = |v: f64| if max > min { (v - min) / span } else { 0.5 };
    let values = if log {
        let decades: Vec<f64> = (min.ceil() as i64..=max.floor() as i64)
            .map(|e| e as f64)
            .collect();
        if decades.len() >= 2 {
            decades
        } else {
            nice_ticks(min, max, target)
        }
    } else {
        nice_ticks(min, max, target)
    };
    values
        .into_iter()
        .map(|v| AxisTick {
            frac: frac(v),
            label: tick_label(if log { 10f64.powf(v) } else { v }),
        })
        .collect()
}

/// A compact label: thousands separators, no trailing zeros, scientific above 1e6
/// or below 1e-3. `1000` → `1,000`; `0.25` → `0.25`; `2.50` → `2.5`; `1.2e7`.
pub fn tick_label(v: f64) -> String {
    if !v.is_finite() {
        return String::new();
    }
    let a = v.abs();
    if a != 0.0 && !(1e-3..1e6).contains(&a) {
        let s = format!("{v:.1e}");
        return s.replace(".0e", "e");
    }
    // Up to 4 decimals, then trim.
    let mut s = format!("{v:.4}");
    if s.contains('.') {
        s = s.trim_end_matches('0').trim_end_matches('.').to_string();
    }
    if s == "-0" {
        s = "0".into();
    }
    let (sign, rest) = s.strip_prefix('-').map_or(("", s.as_str()), |r| ("-", r));
    let (int, frac) = rest
        .split_once('.')
        .map_or((rest, None), |(i, f)| (i, Some(f)));
    let mut grouped = String::new();
    for (i, c) in int.chars().enumerate() {
        if i > 0 && (int.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(c);
    }
    match frac {
        Some(f) => format!("{sign}{grouped}.{f}"),
        None => format!("{sign}{grouped}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nice_steps_table() {
        assert_eq!(
            nice_ticks(0.0, 10.0, 5),
            vec![0.0, 2.0, 4.0, 6.0, 8.0, 10.0]
        );
        assert_eq!(nice_ticks(0.33, 5.97, 4), vec![1.0, 2.0, 3.0, 4.0, 5.0]);
        assert_eq!(nice_ticks(4879.0, 142984.0, 4), vec![50000.0, 100000.0]);
        assert_eq!(
            nice_ticks(-40.0, 40.0, 4),
            vec![-40.0, -20.0, 0.0, 20.0, 40.0]
        );
        assert_eq!(
            nice_ticks(0.1, 0.35, 5),
            vec![0.1, 0.15, 0.2, 0.25, 0.3, 0.35]
        );
        assert_eq!(nice_ticks(7.0, 7.0, 5), vec![7.0]);
        assert!(nice_ticks(f64::NAN, 1.0, 5).is_empty());
    }

    #[test]
    fn ticks_lie_inside_the_span() {
        for (lo, hi) in [(0.0, 1.0), (3.2, 9.9), (-1e5, 2e5), (0.001, 0.0042)] {
            let t = axis_ticks(lo, hi, false, 5);
            assert!(!t.is_empty(), "{lo}..{hi}");
            assert!(t.iter().all(|k| (-1e-9..=1.0 + 1e-9).contains(&k.frac)));
        }
    }

    #[test]
    fn log_axis_uses_decades_with_real_labels() {
        // Data 57.9 .. 4515 → log10 1.76 .. 3.65 → decades 2, 3.
        let t = axis_ticks(57.9f64.log10(), 4515f64.log10(), true, 5);
        let labels: Vec<&str> = t.iter().map(|k| k.label.as_str()).collect();
        assert_eq!(labels, vec!["100", "1,000"]);
        assert!(t[0].frac > 0.0 && t[1].frac < 1.0);
        // Under one decade: nice exponents, still real-valued labels.
        let t = axis_ticks(2f64.log10(), 8f64.log10(), true, 4);
        assert!(t.len() >= 2);
    }

    #[test]
    fn label_formatting_table() {
        for (v, want) in [
            (0.0, "0"),
            (1000.0, "1,000"),
            (142984.0, "142,984"),
            (-1234.5, "-1,234.5"),
            (0.25, "0.25"),
            (2.5, "2.5"),
            (100.0, "100"),
            (12000000.0, "1.2e7"),
            (0.0001, "1e-4"),
        ] {
            assert_eq!(tick_label(v), want, "{v}");
        }
    }
}
