//! Cron parsing, next-N-runs, and natural-language → cron (Phase 21). Pure and
//! timezone-aware (via `chrono-tz`); the job store, scheduler and OS adapters build on
//! top. Fields are the classic five: `minute hour day-of-month month day-of-week`
//! (dow 0 = Sunday … 6 = Saturday). Each field supports `*`, `n`, `a-b`, `a,b,c`,
//! `*/n`, and `a-b/n`. day-of-month vs day-of-week use Vixie's OR rule (see `day_ok`).

use chrono::{Datelike, LocalResult, TimeZone};
use chrono_tz::Tz;

/// One parsed cron field: which values are allowed, and whether it was restricted
/// (anything other than a bare `*`) — the latter drives the dom/dow OR rule.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Field {
    allow: Vec<bool>, // indexed by value - min
    min: u32,
    restricted: bool,
}

impl Field {
    fn has(&self, v: u32) -> bool {
        v.checked_sub(self.min)
            .and_then(|i| self.allow.get(i as usize).copied())
            .unwrap_or(false)
    }
}

/// A parsed cron expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cron {
    minute: Field,
    hour: Field,
    dom: Field,
    month: Field,
    dow: Field,
}

fn name_to_num(tok: &str, names: &[&str], base: u32) -> Option<u32> {
    let t = tok.to_ascii_lowercase();
    names.iter().position(|n| *n == t).map(|i| i as u32 + base)
}

/// Parse one field into a `Field` over `[min, max]`, resolving 3-letter `names`.
fn parse_field(spec: &str, min: u32, max: u32, names: &[&str]) -> Result<Field, String> {
    let span = (max - min + 1) as usize;
    let mut allow = vec![false; span];
    let restricted = spec.trim() != "*";
    let num = |tok: &str| -> Result<u32, String> {
        if let Ok(n) = tok.parse::<u32>() {
            return Ok(n);
        }
        name_to_num(tok, names, min).ok_or_else(|| format!("bad value '{tok}'"))
    };
    for part in spec.split(',') {
        let part = part.trim();
        if part.is_empty() {
            return Err("empty field part".into());
        }
        // Split an optional `/step`.
        let (range, step) = match part.split_once('/') {
            Some((r, s)) => (r, s.parse::<u32>().map_err(|_| format!("bad step '{s}'"))?),
            None => (part, 1),
        };
        if step == 0 {
            return Err("step 0".into());
        }
        let (lo, hi) = if range == "*" {
            (min, max)
        } else if let Some((a, b)) = range.split_once('-') {
            (num(a)?, num(b)?)
        } else {
            let v = num(range)?;
            (v, v)
        };
        if lo < min || hi > max || lo > hi {
            return Err(format!("value out of range in '{part}'"));
        }
        let mut v = lo;
        while v <= hi {
            allow[(v - min) as usize] = true;
            v += step;
        }
    }
    Ok(Field {
        allow,
        min,
        restricted,
    })
}

const DOW_NAMES: &[&str] = &["sun", "mon", "tue", "wed", "thu", "fri", "sat"];
const MON_NAMES: &[&str] = &[
    "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
];

/// Parse a 5-field cron expression.
pub fn parse_cron(expr: &str) -> Result<Cron, String> {
    let f: Vec<&str> = expr.split_whitespace().collect();
    if f.len() != 5 {
        return Err(format!("expected 5 fields, got {}", f.len()));
    }
    let mut dow = parse_field(f[4], 0, 7, DOW_NAMES)?; // allow 7 = Sunday, fold into 0
    if dow.has(7) {
        dow.allow[0] = true;
    }
    dow.allow.truncate(7); // keep 0..=6
    Ok(Cron {
        minute: parse_field(f[0], 0, 59, &[])?,
        hour: parse_field(f[1], 0, 23, &[])?,
        dom: parse_field(f[2], 1, 31, &[])?,
        month: parse_field(f[3], 1, 12, MON_NAMES)?,
        dow,
    })
}

impl Cron {
    /// Whether a given (day-of-month, day-of-week 0=Sun) is a firing day, applying
    /// Vixie's rule: if BOTH dom and dow are restricted, the day matches if EITHER
    /// matches; otherwise both (a `*` field always matches) must hold.
    fn day_ok(&self, dom: u32, dow: u32) -> bool {
        let dom_m = self.dom.has(dom);
        let dow_m = self.dow.has(dow);
        if self.dom.restricted && self.dow.restricted {
            dom_m || dow_m
        } else {
            dom_m && dow_m
        }
    }

    /// The matching (hour, minute) firing times on a day, ascending.
    fn day_times(&self) -> Vec<(u32, u32)> {
        let mut out = Vec::new();
        for h in 0..24 {
            if !self.hour.has(h) {
                continue;
            }
            for m in 0..60 {
                if self.minute.has(m) {
                    out.push((h, m));
                }
            }
        }
        out
    }
}

/// Resolve a wall-clock time in `tz` to an epoch-seconds instant, applying the DST
/// rules: fall-back ambiguity → the first (earlier) occurrence; spring-forward gap
/// (the time doesn't exist) → the first valid instant after the gap that day
/// ("fire at gap end"). `None` only if no valid instant exists that day.
fn resolve(tz: Tz, y: i32, mo: u32, d: u32, h: u32, mi: u32) -> Option<i64> {
    match tz.with_ymd_and_hms(y, mo, d, h, mi, 0) {
        LocalResult::Single(dt) => Some(dt.timestamp()),
        LocalResult::Ambiguous(a, _b) => Some(a.timestamp()),
        LocalResult::None => {
            // Gap: bump minute-by-minute to the first existing wall time this day.
            let mut total = h * 60 + mi + 1;
            while total < 24 * 60 {
                let (nh, nm) = (total / 60, total % 60);
                if let LocalResult::Single(dt) = tz.with_ymd_and_hms(y, mo, d, nh, nm, 0) {
                    return Some(dt.timestamp());
                }
                total += 1;
            }
            None
        }
    }
}

/// The next `n` firing instants (epoch seconds) strictly after `from_epoch`, in `tz`.
pub fn next_runs(cron: &Cron, from_epoch: i64, tz: Tz, n: usize) -> Vec<i64> {
    let mut out = Vec::new();
    if n == 0 {
        return out;
    }
    let from = tz.timestamp_opt(from_epoch, 0).single();
    let Some(from) = from else {
        return out;
    };
    let times = cron.day_times();
    if times.is_empty() {
        return out;
    }
    // Walk days from `from`'s local date, up to ~5 years (covers Feb-29 leap gaps).
    let mut day = from.date_naive();
    for _ in 0..(366 * 5 + 10) {
        let (y, mo, d) = (day.year(), day.month(), day.day());
        if cron.month.has(mo) {
            // chrono weekday: Mon=0..Sun=6 → cron dow Sun=0..Sat=6.
            let dow = day.weekday().num_days_from_sunday();
            if cron.day_ok(d, dow) {
                for (h, mi) in &times {
                    if let Some(ts) = resolve(tz, y, mo, d, *h, *mi) {
                        if ts > from_epoch {
                            out.push(ts);
                            if out.len() == n {
                                return out;
                            }
                        }
                    }
                }
            }
        }
        day = match day.succ_opt() {
            Some(nx) => nx,
            None => break,
        };
    }
    out
}

/// Like [`next_runs`], but takes an IANA tz name (falling back to UTC) so callers
/// don't need a `chrono-tz` dependency.
pub fn next_runs_in_tz(cron: &Cron, from_epoch: i64, tz_name: &str, n: usize) -> Vec<i64> {
    let tz: Tz = tz_name.parse().unwrap_or(chrono_tz::UTC);
    next_runs(cron, from_epoch, tz, n)
}

/// Best-effort natural-language → cron. Returns `None` if it can't confidently parse
/// (the UI then lets the user type/edit the cron directly — never a wrong guess).
pub fn natural_to_cron(text: &str) -> Option<String> {
    let t = text.trim().to_ascii_lowercase();
    if t.is_empty() {
        return None;
    }

    // "every N minutes/hours"
    if let Some(rest) = t.strip_prefix("every ") {
        if let Some(mins) = rest
            .strip_suffix(" minutes")
            .or_else(|| rest.strip_suffix(" mins"))
        {
            if let Ok(nn) = mins.trim().parse::<u32>() {
                if (1..=59).contains(&nn) {
                    return Some(format!("*/{nn} * * * *"));
                }
            }
        }
        if let Some(hrs) = rest.strip_suffix(" hours") {
            if let Ok(nn) = hrs.trim().parse::<u32>() {
                if (1..=23).contains(&nn) {
                    return Some(format!("0 */{nn} * * *"));
                }
            }
        }
    }

    let (hh, mm) = parse_time(&t).unwrap_or((9, 0)); // default 9:00 when a day is named

    // Weekday sets.
    if t.contains("weekday") {
        return Some(format!("{mm} {hh} * * 1-5"));
    }
    if t.contains("weekend") {
        return Some(format!("{mm} {hh} * * 0,6"));
    }
    for (i, day) in [
        "sunday",
        "monday",
        "tuesday",
        "wednesday",
        "thursday",
        "friday",
        "saturday",
    ]
    .iter()
    .enumerate()
    {
        if t.contains(day) {
            return Some(format!("{mm} {hh} * * {i}"));
        }
    }

    // "Nth of month"
    if let Some(dom) = parse_month_day(&t) {
        return Some(format!("{mm} {hh} {dom} * *"));
    }

    if t.contains("daily") || t.contains("every day") || parse_time(&t).is_some() {
        return Some(format!("{mm} {hh} * * *"));
    }
    None
}

/// Extract an `H`/`H:MM` (12/24h, optional am/pm) from text → (hour, minute).
fn parse_time(t: &str) -> Option<(u32, u32)> {
    // find a token like 8, 8:40, 08:40, with optional am/pm suffix nearby
    let re = regex::Regex::new(r"(?:\bat\s+)?(\d{1,2})(?::(\d{2}))?\s*(am|pm)?").ok()?;
    for cap in re.captures_iter(t) {
        let h: u32 = cap.get(1)?.as_str().parse().ok()?;
        let m: u32 = cap
            .get(2)
            .and_then(|x| x.as_str().parse().ok())
            .unwrap_or(0);
        let ampm = cap.get(3).map(|x| x.as_str());
        // Skip matches that are clearly a "Nth"/"N minutes" number, not a time.
        let mut h = h;
        match ampm {
            Some("pm") if h < 12 => h += 12,
            Some("am") if h == 12 => h = 0,
            _ => {}
        }
        if h < 24 && m < 60 && (cap.get(2).is_some() || ampm.is_some() || t.contains("at ")) {
            return Some((h, m));
        }
    }
    None
}

/// Extract "1st/2nd/… of month" or "1st of the month" → day-of-month.
fn parse_month_day(t: &str) -> Option<u32> {
    if !t.contains("month") {
        return None;
    }
    let re = regex::Regex::new(r"(\d{1,2})(?:st|nd|rd|th)?\s+of").ok()?;
    let cap = re.captures(t)?;
    let d: u32 = cap.get(1)?.as_str().parse().ok()?;
    (1..=31).contains(&d).then_some(d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn sthlm() -> Tz {
        "Europe/Stockholm".parse().unwrap()
    }
    fn utc() -> Tz {
        chrono_tz::UTC
    }
    /// Epoch seconds for a wall-clock time in a tz.
    fn at(tz: Tz, y: i32, mo: u32, d: u32, h: u32, mi: u32) -> i64 {
        tz.with_ymd_and_hms(y, mo, d, h, mi, 0).unwrap().timestamp()
    }

    // ---- A. parsing ----
    #[test]
    fn a_parse_fields() {
        let c = parse_cron("*/15 * * * *").unwrap();
        assert!(c.minute.has(0) && c.minute.has(15) && c.minute.has(45));
        assert!(!c.minute.has(10));
        let c = parse_cron("40 8 * * 1-5").unwrap();
        assert!(c.hour.has(8) && c.minute.has(40) && c.dow.has(1) && c.dow.has(5) && !c.dow.has(0));
        assert!(parse_cron("60 * * * *").is_err());
        assert!(parse_cron("* * * * 8").is_err());
        assert!(parse_cron("* * * *").is_err());
        assert!(parse_cron("").is_err());
        // Sunday as 7 folds to 0.
        assert!(parse_cron("0 0 * * 7").unwrap().dow.has(0));
        // Names.
        assert!(parse_cron("0 0 * jan mon").unwrap().month.has(1));
    }

    // ---- B. next-runs basic ----
    #[test]
    fn b_next_runs_basic() {
        let c = parse_cron("*/15 * * * *").unwrap();
        let from = at(utc(), 2026, 1, 1, 10, 7);
        let got = next_runs(&c, from, utc(), 4);
        let want: Vec<i64> = [(10, 15), (10, 30), (10, 45), (11, 0)]
            .iter()
            .map(|(h, m)| at(utc(), 2026, 1, 1, *h, *m))
            .collect();
        assert_eq!(got, want);
    }

    #[test]
    fn b_strictly_after_from() {
        let c = parse_cron("0 9 * * *").unwrap();
        let from = at(utc(), 2026, 1, 1, 9, 0); // exactly a firing instant
        let got = next_runs(&c, from, utc(), 1);
        assert_eq!(
            got,
            vec![at(utc(), 2026, 1, 2, 9, 0)],
            "excludes `from` itself"
        );
    }

    // ---- C. calendar edges ----
    #[test]
    fn c_31st_skips_short_months() {
        let c = parse_cron("0 0 31 * *").unwrap();
        let from = at(utc(), 2026, 1, 31, 12, 0);
        let got = next_runs(&c, from, utc(), 1);
        assert_eq!(got, vec![at(utc(), 2026, 3, 31, 0, 0)]);
    }

    #[test]
    fn c_feb_29_leap_only() {
        let c = parse_cron("0 0 29 2 *").unwrap();
        let from = at(utc(), 2025, 3, 1, 0, 0);
        let got = next_runs(&c, from, utc(), 1);
        assert_eq!(got, vec![at(utc(), 2028, 2, 29, 0, 0)]);
    }

    #[test]
    fn c_dom_or_dow_vixie() {
        // 1st OR Monday.
        let c = parse_cron("0 0 1 * 1").unwrap();
        // From Thu 2026-01-01 (which is the 1st) → next is Mon 2026-01-05.
        let from = at(utc(), 2026, 1, 1, 12, 0);
        let got = next_runs(&c, from, utc(), 2);
        assert_eq!(
            got,
            vec![at(utc(), 2026, 1, 5, 0, 0), at(utc(), 2026, 1, 12, 0, 0)]
        );
    }

    // ---- D. DST (Europe/Stockholm: spring 2026-03-29 02:00→03:00; fall 2026-10-25 03:00→02:00) ----
    #[test]
    fn d_spring_forward_fires_at_gap_end() {
        let c = parse_cron("30 2 * * *").unwrap();
        let from = at(sthlm(), 2026, 3, 29, 0, 0);
        let got = next_runs(&c, from, sthlm(), 1);
        // 02:30 doesn't exist that day → first valid instant is 03:00 local.
        assert_eq!(got, vec![at(sthlm(), 2026, 3, 29, 3, 0)]);
    }

    #[test]
    fn d_fall_back_fires_once() {
        let c = parse_cron("30 1 * * *").unwrap();
        let from = at(sthlm(), 2026, 10, 25, 0, 0);
        let got = next_runs(&c, from, sthlm(), 1);
        // 01:30 happens twice; we take the first occurrence, and only once.
        let first = sthlm()
            .with_ymd_and_hms(2026, 10, 25, 1, 30, 0)
            .earliest()
            .unwrap()
            .timestamp();
        assert_eq!(got, vec![first]);
        // The next run is the following day (not the second 01:30).
        let got2 = next_runs(&c, from, sthlm(), 2);
        assert_eq!(got2[1], at(sthlm(), 2026, 10, 26, 1, 30));
    }

    // ---- E. natural language ----
    #[test]
    fn e_natural_language() {
        assert_eq!(
            natural_to_cron("every weekday at 8:40").as_deref(),
            Some("40 8 * * 1-5")
        );
        assert_eq!(
            natural_to_cron("1st of month 10:00").as_deref(),
            Some("0 10 1 * *")
        );
        assert_eq!(
            natural_to_cron("every 15 minutes").as_deref(),
            Some("*/15 * * * *")
        );
        assert_eq!(natural_to_cron("daily at 9").as_deref(), Some("0 9 * * *"));
        assert_eq!(
            natural_to_cron("sundays at 18:00").as_deref(),
            Some("0 18 * * 0")
        );
        assert_eq!(
            natural_to_cron("every 3 hours").as_deref(),
            Some("0 */3 * * *")
        );
        assert_eq!(natural_to_cron("gibberish blah").as_deref(), None);
        // Round-trips through the parser.
        for txt in [
            "every weekday at 8:40",
            "1st of month 10:00",
            "every 15 minutes",
        ] {
            assert!(parse_cron(&natural_to_cron(txt).unwrap()).is_ok());
        }
    }
}
