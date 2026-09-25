//! Pure calendar layout for the date answer card (Phase 11, PNG 15). Given a
//! target day, an optional range, and today, it produces month grids (Mon-first)
//! and a 53×7 year heatmap — all as plain data with epoch-day + flags per cell.
//! The app renders these as Slint rects; no drawing logic lives here.

use crate::eval::{civil_from_days, days_from_civil, weekday};

/// One day cell in a month grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DayCell {
    pub day_epoch: i64,
    pub dom: u32,
    /// Inside the highlighted span (today..=target, inclusive).
    pub in_range: bool,
    /// The exact target day (solid accent).
    pub is_target: bool,
    /// Today (1px ring).
    pub is_today: bool,
    /// Saturday or Sunday.
    pub is_weekend: bool,
}

/// A month grid: 6 rows × 7 columns (Mon-first). Leading/trailing blanks are
/// `None`, so a row of 7 always aligns under the M T W T F S S header.
#[derive(Debug, Clone, PartialEq)]
pub struct MonthGrid {
    pub year: i64,
    pub month: u32,
    /// 42 slots (6×7), Mon-first, row-major.
    pub cells: Vec<Option<DayCell>>,
}

/// One cell of the year heatmap (column = ISO-ish week, row = weekday Mon-first).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct YearCell {
    pub day_epoch: i64,
    pub month: u32,
    pub in_range: bool,
    pub is_target: bool,
    pub is_today: bool,
}

/// A 53-column × 7-row heatmap for one calendar year (column-major weeks).
#[derive(Debug, Clone, PartialEq)]
pub struct YearStrip {
    pub year: i64,
    pub cols: usize,
    /// `cols * 7` slots, column-major (col 0 rows 0..7, then col 1 …). `None`
    /// where the week column falls outside the year.
    pub cells: Vec<Option<YearCell>>,
    /// The column index at which each month's first day lands, for labels.
    pub month_label_cols: Vec<(u32, usize)>,
}

/// The full layout for the answer card's calendar area.
#[derive(Debug, Clone, PartialEq)]
pub struct CalendarLayout {
    /// Up to `MAX_MONTHS` month grids from the range's start month to its end
    /// month. Empty when the span exceeds `MAX_MONTHS` (year strip carries it).
    pub months: Vec<MonthGrid>,
    pub year_strip: YearStrip,
}

/// The most month grids we render before falling back to the year strip only.
pub const MAX_MONTHS: usize = 4;

fn is_weekend(day: i64) -> bool {
    let wd = weekday(day); // 0 = Sunday, 6 = Saturday
    wd == 0 || wd == 6
}

/// Monday-first weekday index (0 = Monday … 6 = Sunday).
fn mon0(day: i64) -> i64 {
    (weekday(day) + 6) % 7
}

fn build_month(year: i64, month: u32, lo: i64, hi: i64, target: i64, today: i64) -> MonthGrid {
    let first = days_from_civil(year, month as i64, 1);
    let lead = mon0(first) as usize;
    let dim = crate::eval::days_in_month(year, month as i64) as usize;
    let mut cells: Vec<Option<DayCell>> = vec![None; 42];
    for d in 0..dim {
        let day = first + d as i64;
        let (_, _, dom) = civil_from_days(day);
        cells[lead + d] = Some(DayCell {
            day_epoch: day,
            dom,
            in_range: day >= lo && day <= hi,
            is_target: day == target,
            is_today: day == today,
            is_weekend: is_weekend(day),
        });
    }
    MonthGrid { year, month, cells }
}

fn build_year_strip(year: i64, lo: i64, hi: i64, target: i64, today: i64) -> YearStrip {
    let jan1 = days_from_civil(year, 1, 1);
    let dec31 = days_from_civil(year, 12, 31);
    // Column 0 starts on the Monday on/before Jan 1.
    let grid_start = jan1 - mon0(jan1);
    let total_days = (dec31 - grid_start + 1) as usize;
    let cols = total_days.div_ceil(7);
    let mut cells: Vec<Option<YearCell>> = vec![None; cols * 7];
    let mut month_label_cols: Vec<(u32, usize)> = Vec::new();
    let mut seen_month = 0u32;
    for col in 0..cols {
        for row in 0..7 {
            let day = grid_start + (col * 7 + row) as i64;
            if day < jan1 || day > dec31 {
                continue;
            }
            let (_, m, dom) = civil_from_days(day);
            if m != seen_month && dom <= 7 {
                month_label_cols.push((m, col));
                seen_month = m;
            }
            cells[col * 7 + row] = Some(YearCell {
                day_epoch: day,
                month: m,
                in_range: day >= lo && day <= hi,
                is_target: day == target,
                is_today: day == today,
            });
        }
    }
    YearStrip {
        year,
        cols,
        cells,
        month_label_cols,
    }
}

/// Compute the calendar layout. `start_day`/`end_day` bound the highlighted span
/// (order-independent); `target_day` is the solid accent day; `today` gets a ring.
/// Month grids span the range's months (≤ `MAX_MONTHS`, else empty). The year
/// strip always covers the target day's year.
pub fn layout(start_day: i64, end_day: i64, target_day: i64, today: i64) -> CalendarLayout {
    let lo = start_day.min(end_day);
    let hi = start_day.max(end_day);
    let (ly, lm, _) = civil_from_days(lo);
    let (hy, hm, _) = civil_from_days(hi);
    let month_span = ((hy - ly) * 12 + (hm as i64 - lm as i64) + 1).max(1);

    let mut months = Vec::new();
    if month_span as usize <= MAX_MONTHS {
        let mut y = ly;
        let mut m = lm as i64;
        for _ in 0..month_span {
            months.push(build_month(y, m as u32, lo, hi, target_day, today));
            m += 1;
            if m > 12 {
                m = 1;
                y += 1;
            }
        }
    }

    let (ty, _, _) = civil_from_days(target_day);
    let year_strip = build_year_strip(ty, lo, hi, target_day, today);
    CalendarLayout { months, year_strip }
}

// ---------------------------------------------------------------------------
// Public formatting helpers for the app's date answer card.
// ---------------------------------------------------------------------------

/// `(year, month, day-of-month)` for an epoch-day.
pub fn ymd(day: i64) -> (i64, u32, u32) {
    civil_from_days(day)
}

/// Uppercased 3-letter month abbreviation for the date tile (`DEC`).
pub fn month_abbrev_upper(day: i64) -> String {
    let (_, m, _) = civil_from_days(day);
    crate::eval::month_name(m, false).to_uppercase()
}

/// Long weekday name (`Friday`).
pub fn weekday_long(day: i64) -> String {
    crate::eval::weekday_name(crate::eval::weekday(day), true).to_string()
}

/// Full human date (`Friday, 25 December 2026`).
pub fn full_date(day: i64) -> String {
    let (y, m, _) = civil_from_days(day);
    let (_, _, d) = civil_from_days(day);
    format!(
        "{}, {d} {} {y}",
        crate::eval::weekday_name(crate::eval::weekday(day), true),
        crate::eval::month_name(m, true)
    )
}

/// Short human date (`25 Dec`).
pub fn short_date(day: i64) -> String {
    let (_, m, d) = civil_from_days(day);
    format!("{d} {}", crate::eval::month_name(m, false))
}

/// ISO-8601 week number.
pub fn iso_week(day: i64) -> u32 {
    crate::eval::iso_week(day)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(y: i64, m: i64, d: i64) -> i64 {
        days_from_civil(y, m, d)
    }

    #[test]
    fn formatting_helpers() {
        let xmas = days_from_civil(2026, 12, 25);
        assert_eq!(month_abbrev_upper(xmas), "DEC");
        assert_eq!(weekday_long(xmas), "Friday");
        assert_eq!(short_date(xmas), "25 Dec");
        assert_eq!(full_date(xmas), "Friday, 25 December 2026");
        assert_eq!(iso_week(xmas), 52);
    }

    #[test]
    fn christmas_countdown_spans_four_months() {
        // today 2026-09-25 → target 2026-12-25.
        let today = day(2026, 9, 25);
        let target = day(2026, 12, 25);
        let l = layout(today, target, target, today);
        // Sep, Oct, Nov, Dec.
        assert_eq!(l.months.len(), 4);
        assert_eq!((l.months[0].year, l.months[0].month), (2026, 9));
        assert_eq!((l.months[3].year, l.months[3].month), (2026, 12));
        // The target cell is flagged in December.
        let dec = &l.months[3];
        let tgt = dec
            .cells
            .iter()
            .flatten()
            .find(|c| c.is_target)
            .expect("target in December");
        assert_eq!(tgt.dom, 25);
        assert!(tgt.in_range);
        // Today is flagged in September and in range.
        let sep = &l.months[0];
        let td = sep.cells.iter().flatten().find(|c| c.is_today).unwrap();
        assert_eq!(td.dom, 25);
    }

    #[test]
    fn month_grid_is_mon_first_and_42_slots() {
        // September 2026 starts on a Tuesday → one leading blank (Mon).
        let today = day(2026, 9, 1);
        let l = layout(today, today, today, today);
        let sep = &l.months[0];
        assert_eq!(sep.cells.len(), 42);
        assert!(
            sep.cells[0].is_none(),
            "Monday slot before the 1st is blank"
        );
        assert_eq!(sep.cells[1].unwrap().dom, 1); // Tuesday the 1st
    }

    #[test]
    fn wide_span_drops_month_grids() {
        // Over 4 months → only the year strip.
        let today = day(2026, 1, 1);
        let target = day(2026, 12, 31);
        let l = layout(today, target, target, today);
        assert!(l.months.is_empty());
        assert_eq!(l.year_strip.year, 2026);
    }

    #[test]
    fn year_strip_has_target_and_month_labels() {
        let today = day(2026, 9, 25);
        let target = day(2026, 12, 25);
        let l = layout(today, target, target, today);
        let ys = &l.year_strip;
        assert!(ys.cols >= 52 && ys.cols <= 54);
        assert_eq!(ys.cells.len(), ys.cols * 7);
        assert!(ys.cells.iter().flatten().any(|c| c.is_target));
        assert_eq!(ys.month_label_cols.len(), 12); // one per month
        assert_eq!(ys.month_label_cols[0].0, 1); // January first
    }
}
