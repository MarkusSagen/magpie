//! Locale-dependent holiday dates (Phase 11). Pure and total: every function
//! takes the year (or a region) and returns civil dates — no clock, no locale
//! reads. **Holidays are locale-dependent; we never guess** — the caller passes a
//! `Region`, and an unknown region is represented by returning `None` for the
//! region-specific parts (the app then says "not set for this region").
//!
//! Easter uses the Anonymous Gregorian computus (Western/Gregorian Easter, which
//! is what SE/US/UK/DE all observe). Black Friday is the US definition — the day
//! after the fourth Thursday of November — and is labelled as such.

use crate::eval::{civil_from_days, days_from_civil, weekday};

/// The regions we ship built-in holiday data for. Extend deliberately (each new
/// region needs its own eve/observance rules).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Region {
    /// Sweden
    Se,
    /// United States
    Us,
    /// United Kingdom
    Uk,
    /// Germany
    De,
}

impl Region {
    /// The stable id stored in config (`holiday_region`).
    pub fn id(self) -> &'static str {
        match self {
            Region::Se => "SE",
            Region::Us => "US",
            Region::Uk => "UK",
            Region::De => "DE",
        }
    }

    /// Parse a stored id (case-insensitive), or `None` if unknown.
    pub fn from_id(s: &str) -> Option<Region> {
        match s.trim().to_uppercase().as_str() {
            "SE" => Some(Region::Se),
            "US" => Some(Region::Us),
            "UK" | "GB" => Some(Region::Uk),
            "DE" => Some(Region::De),
            _ => None,
        }
    }

    /// Best-effort region from an OS locale string (`sv_SE.UTF-8`, `en-GB`, …).
    /// Falls back to Sweden — this is a Swedish-built local-first app, and the
    /// user can always override `holiday_region` in config.
    pub fn from_locale(locale: &str) -> Region {
        let l = locale.to_uppercase();
        // Prefer an explicit country suffix (after `_`/`-`), else scan the string.
        let country = l
            .split(['_', '-', '.'])
            .nth(1)
            .unwrap_or("")
            .trim_matches(|c: char| !c.is_ascii_alphabetic());
        if let Some(r) = Region::from_id(country) {
            return r;
        }
        if l.contains("SE") || l.contains("SV") {
            Region::Se
        } else if l.contains("GB") || l.contains("UK") {
            Region::Uk
        } else if l.contains("DE") {
            Region::De
        } else if l.contains("US") {
            Region::Us
        } else {
            Region::Se
        }
    }

    /// The local name for Christmas Eve where the region celebrates the eve
    /// itself (SE `Julafton`, DE `Heiligabend`); `None` where 25 Dec is the day.
    pub fn christmas_eve_label(self) -> Option<&'static str> {
        match self {
            Region::Se => Some("Julafton"),
            Region::De => Some("Heiligabend"),
            Region::Us | Region::Uk => None,
        }
    }
}

/// Western (Gregorian) Easter Sunday for `year` → `(month, day)`, via the
/// Anonymous Gregorian computus. Valid for all Gregorian years.
pub fn easter(year: i64) -> (u32, u32) {
    let a = year % 19;
    let b = year / 100;
    let c = year % 100;
    let d = b / 4;
    let e = b % 4;
    let f = (b + 8) / 25;
    let g = (b - f + 1) / 3;
    let h = (19 * a + b - d - g + 15) % 30;
    let i = c / 4;
    let k = c % 4;
    let l = (32 + 2 * e + 2 * i - h - k) % 7;
    let m = (a + 11 * h + 22 * l) / 451;
    let month = (h + l - 7 * m + 114) / 31;
    let day = ((h + l - 7 * m + 114) % 31) + 1;
    (month as u32, day as u32)
}

/// US Black Friday for `year` → `(month, day)`: the day after the fourth
/// Thursday of November. `eval::weekday` is 0=Sunday … 4=Thursday … 6=Saturday.
pub fn black_friday(year: i64) -> (u32, u32) {
    let nov1 = days_from_civil(year, 11, 1);
    let first_thu_offset = (4 - weekday(nov1)).rem_euclid(7); // Thursday = 4
    let fourth_thu = nov1 + first_thu_offset + 21;
    let bf = fourth_thu + 1;
    let (_, m, d) = civil_from_days(bf);
    (m, d)
}

/// A holiday recognised on a given day, for the answer-card label + eve switch.
#[derive(Debug, Clone, PartialEq)]
pub struct HolidayInfo {
    /// The primary label, e.g. `Christmas Day`.
    pub label: String,
    /// For regions that celebrate the eve, the eve's epoch-day and its label
    /// (e.g. `24 Dec · Julafton`) so the card can offer a two-option switch.
    pub eve: Option<(i64, String)>,
}

/// If `day` is a holiday we know for `region`, return its label (and eve switch).
/// Region-aware: the Christmas eve option only appears where the region observes
/// the eve. Returns `None` for ordinary days.
pub fn label_for_day(region: Region, day: i64) -> Option<HolidayInfo> {
    let (y, m, d) = civil_from_days(day);
    if m == 12 && d == 25 {
        let eve = region
            .christmas_eve_label()
            .map(|l| (day - 1, format!("24 Dec · {l}")));
        return Some(HolidayInfo {
            label: "Christmas Day".into(),
            eve,
        });
    }
    if m == 1 && d == 1 {
        return Some(HolidayInfo {
            label: "New Year's Day".into(),
            eve: None,
        });
    }
    let (em, ed) = easter(y);
    if m == em && d == ed {
        return Some(HolidayInfo {
            label: "Easter Sunday".into(),
            eve: None,
        });
    }
    let (bm, bd) = black_friday(y);
    if m == bm && d == bd {
        return Some(HolidayInfo {
            label: "Black Friday (US)".into(),
            eve: None,
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_for_day_recognises_holidays_and_eve() {
        let xmas = days_from_civil(2026, 12, 25);
        let se = label_for_day(Region::Se, xmas).unwrap();
        assert_eq!(se.label, "Christmas Day");
        assert_eq!(se.eve.as_ref().unwrap().0, xmas - 1);
        assert!(se.eve.as_ref().unwrap().1.contains("Julafton"));
        // US has no eve.
        assert_eq!(label_for_day(Region::Us, xmas).unwrap().eve, None);
        // Easter Sunday 2026 = 5 Apr.
        let easter26 = days_from_civil(2026, 4, 5);
        assert_eq!(
            label_for_day(Region::Se, easter26).unwrap().label,
            "Easter Sunday"
        );
        // An ordinary day.
        assert_eq!(label_for_day(Region::Se, days_from_civil(2026, 7, 1)), None);
    }

    #[test]
    fn easter_known_values() {
        // Well-known Western Easter Sundays.
        assert_eq!(easter(2024), (3, 31));
        assert_eq!(easter(2025), (4, 20));
        assert_eq!(easter(2026), (4, 5));
        assert_eq!(easter(2027), (3, 28));
    }

    #[test]
    fn black_friday_is_day_after_fourth_thursday() {
        // 2026: Thanksgiving 26 Nov (4th Thu) → Black Friday 27 Nov.
        assert_eq!(black_friday(2026), (11, 27));
        // 2024: Thanksgiving 28 Nov → Black Friday 29 Nov.
        assert_eq!(black_friday(2024), (11, 29));
    }

    #[test]
    fn region_ids_and_locale() {
        assert_eq!(Region::from_id("se"), Some(Region::Se));
        assert_eq!(Region::from_id("gb"), Some(Region::Uk));
        assert_eq!(Region::from_id("xx"), None);
        assert_eq!(Region::from_locale("sv_SE.UTF-8"), Region::Se);
        assert_eq!(Region::from_locale("en-GB"), Region::Uk);
        assert_eq!(Region::from_locale("de_DE"), Region::De);
        assert_eq!(Region::from_locale("en_US"), Region::Us);
        assert_eq!(Region::from_locale("weird"), Region::Se); // fallback
    }

    #[test]
    fn eve_labels() {
        assert_eq!(Region::Se.christmas_eve_label(), Some("Julafton"));
        assert_eq!(Region::De.christmas_eve_label(), Some("Heiligabend"));
        assert_eq!(Region::Us.christmas_eve_label(), None);
    }
}
