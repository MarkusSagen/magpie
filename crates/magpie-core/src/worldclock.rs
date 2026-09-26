//! World clock & time-zone answers (Phase 12). The IANA tz database is embedded
//! via `chrono-tz`, so DST and offsets are correct and identical on every OS.
//! Parsing (`parse_worldclock`) and rendering (`compute`) are pure — the caller
//! injects the epoch. No map (deferred); this is the list + 24-hour strip.

use chrono::{Datelike, Offset, TimeZone, Timelike, Utc};
use chrono_tz::Tz;

/// A parsed world-clock query.
#[derive(Debug, Clone, PartialEq)]
pub struct WorldClockRequest {
    /// Base time of day in the source zone (24h). `None` = now.
    pub base_hm: Option<(u32, u32)>,
    /// Source zone the base time is expressed in. `None` = the home zone.
    pub base_zone: Option<String>,
    /// Target zones to show (IANA ids), in order. Never empty on a match.
    pub targets: Vec<String>,
}

/// Resolve a city name, IANA id, or common abbreviation to a `Tz`. Case- and
/// space-insensitive. Returns `None` for anything we don't recognise (so an
/// ordinary search is never mistaken for a world-clock query).
pub fn resolve_zone(name: &str) -> Option<Tz> {
    let key = name.trim().to_lowercase();
    let key = key.trim();
    if key.is_empty() {
        return None;
    }
    // Common abbreviations → a representative zone.
    let abbrev = match key {
        "utc" | "gmt" | "zulu" | "z" => Some("UTC"),
        "pst" | "pdt" | "pt" => Some("America/Los_Angeles"),
        "mst" | "mdt" | "mt" => Some("America/Denver"),
        "cst" | "cdt" | "ct" => Some("America/Chicago"),
        "est" | "edt" | "et" => Some("America/New_York"),
        "cet" | "cest" => Some("Europe/Paris"),
        "eet" | "eest" => Some("Europe/Helsinki"),
        "bst" | "wet" => Some("Europe/London"),
        "jst" => Some("Asia/Tokyo"),
        "kst" => Some("Asia/Seoul"),
        "ist" => Some("Asia/Kolkata"),
        "aest" | "aedt" => Some("Australia/Sydney"),
        "nzst" | "nzdt" => Some("Pacific/Auckland"),
        _ => None,
    };
    if let Some(id) = abbrev {
        return id.parse().ok();
    }
    if let Some(id) = city_iana(key) {
        return id.parse().ok();
    }
    // A bare IANA id, possibly with a space instead of an underscore.
    let iana: String = name.trim().replace(' ', "_");
    iana.parse().ok()
}

/// Curated city → IANA id table (the world-clock starter set plus common cities).
fn city_iana(key: &str) -> Option<&'static str> {
    let id = match key {
        "stockholm" => "Europe/Stockholm",
        "london" => "Europe/London",
        "paris" => "Europe/Paris",
        "berlin" => "Europe/Berlin",
        "madrid" => "Europe/Madrid",
        "rome" => "Europe/Rome",
        "amsterdam" => "Europe/Amsterdam",
        "oslo" => "Europe/Oslo",
        "copenhagen" => "Europe/Copenhagen",
        "helsinki" => "Europe/Helsinki",
        "dublin" => "Europe/Dublin",
        "lisbon" => "Europe/Lisbon",
        "zurich" => "Europe/Zurich",
        "moscow" => "Europe/Moscow",
        "istanbul" => "Europe/Istanbul",
        "athens" => "Europe/Athens",
        "warsaw" => "Europe/Warsaw",
        "new york" | "nyc" | "newyork" => "America/New_York",
        "san francisco" | "sf" | "sanfrancisco" | "bay area" => "America/Los_Angeles",
        "los angeles" | "la" | "losangeles" => "America/Los_Angeles",
        "seattle" => "America/Los_Angeles",
        "chicago" => "America/Chicago",
        "denver" => "America/Denver",
        "boston" => "America/New_York",
        "washington" | "dc" => "America/New_York",
        "toronto" => "America/Toronto",
        "vancouver" => "America/Vancouver",
        "mexico city" | "mexico" => "America/Mexico_City",
        "sao paulo" | "são paulo" => "America/Sao_Paulo",
        "buenos aires" => "America/Argentina/Buenos_Aires",
        "tokyo" => "Asia/Tokyo",
        "osaka" => "Asia/Tokyo",
        "seoul" => "Asia/Seoul",
        "beijing" | "shanghai" | "china" => "Asia/Shanghai",
        "hong kong" | "hongkong" | "hk" => "Asia/Hong_Kong",
        "singapore" => "Asia/Singapore",
        "bangkok" => "Asia/Bangkok",
        "jakarta" => "Asia/Jakarta",
        "mumbai" | "delhi" | "bangalore" | "bengaluru" | "india" => "Asia/Kolkata",
        "dubai" => "Asia/Dubai",
        "tel aviv" | "jerusalem" => "Asia/Jerusalem",
        "sydney" => "Australia/Sydney",
        "melbourne" => "Australia/Melbourne",
        "brisbane" => "Australia/Brisbane",
        "perth" => "Australia/Perth",
        "auckland" => "Pacific/Auckland",
        "honolulu" | "hawaii" => "Pacific/Honolulu",
        "cairo" => "Africa/Cairo",
        "lagos" => "Africa/Lagos",
        "johannesburg" | "cape town" => "Africa/Johannesburg",
        "nairobi" => "Africa/Nairobi",
        _ => return None,
    };
    Some(id)
}

/// A friendly display name for an IANA id (the city part, `_`→space).
pub fn zone_display_name(iana: &str) -> String {
    iana.rsplit('/').next().unwrap_or(iana).replace('_', " ")
}

fn parse_time_token(s: &str) -> Option<(u32, u32)> {
    let s = s.trim().to_lowercase();
    let (body, ampm) = if let Some(b) = s.strip_suffix("am") {
        (b.trim(), Some(false))
    } else if let Some(b) = s.strip_suffix("pm") {
        (b.trim(), Some(true))
    } else {
        (s.as_str(), None)
    };
    let (h, m) = match body.split_once(':') {
        Some((h, m)) => (h.parse::<u32>().ok()?, m.parse::<u32>().ok()?),
        None => (body.parse::<u32>().ok()?, 0),
    };
    if m > 59 {
        return None;
    }
    let h = match ampm {
        Some(true) => {
            if h == 12 {
                12
            } else if h <= 11 {
                h + 12
            } else {
                return None;
            }
        }
        Some(false) => {
            if h == 12 {
                0
            } else {
                h
            }
        }
        None => h,
    };
    if h > 23 {
        return None;
    }
    Some((h, m))
}

/// Parse a world-clock query, or `None` if it isn't one. Forms: `time in tokyo`,
/// `now in london`, `3pm pst to cet`, `15:00 stockholm in new york, sf, tokyo`,
/// `9am in london`. Every target must resolve to a known zone.
pub fn parse_worldclock(query: &str) -> Option<WorldClockRequest> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return None;
    }
    // Split base vs targets on the first " to " or " in " (both 4 bytes).
    let sep = q.find(" to ").or_else(|| q.find(" in "))?;
    let left = q[..sep].trim();
    let right = q[sep + 4..].trim();

    // Targets: comma-separated, each must resolve.
    let mut targets = Vec::new();
    for part in right.split(',') {
        let p = part.trim();
        if p.is_empty() {
            continue;
        }
        let tz = resolve_zone(p)?;
        targets.push(tz.name().to_string());
    }
    if targets.is_empty() {
        return None;
    }

    // Base: optional leading time, optional source zone, or a filler like "time".
    let mut base_hm = None;
    let mut base_zone = None;
    let words: Vec<&str> = left.split_whitespace().collect();
    let mut idx = 0;
    if let Some(w) = words.first() {
        if let Some(hm) = parse_time_token(w) {
            base_hm = Some(hm);
            idx = 1;
        }
    }
    let rest = words[idx..].join(" ");
    let rest = rest.trim();
    if !rest.is_empty()
        && !matches!(
            rest,
            "time" | "the time" | "now" | "current" | "current time"
        )
    {
        base_zone = Some(resolve_zone(rest)?.name().to_string());
    }
    Some(WorldClockRequest {
        base_hm,
        base_zone,
        targets,
    })
}

/// The current hour (0–23) in the home zone at `epoch_secs` — the default chosen
/// column for the 24-hour strip.
pub fn home_current_hour(home: &str, epoch_secs: i64) -> u32 {
    let home_tz: Tz = resolve_zone(home).unwrap_or(chrono_tz::UTC);
    let now_utc = Utc
        .timestamp_opt(epoch_secs, 0)
        .single()
        .unwrap_or_default();
    now_utc.with_timezone(&home_tz).hour()
}

/// One city row in the world-clock view.
#[derive(Debug, Clone, PartialEq)]
pub struct CityClock {
    pub name: String,
    pub iana: String,
    /// Zone abbreviation at the base instant (e.g. `CEST`, `JST`).
    pub abbrev: String,
    /// UTC offset in minutes at the base instant.
    pub offset_minutes: i32,
    /// Offset relative to the home zone in minutes (+ = ahead of home).
    pub rel_minutes: i32,
    /// The local time at the chosen hour column, `HH:MM`.
    pub big_time: String,
    /// Local date at the chosen column, `Fri 25 Sep` (flags a day change).
    pub date_label: String,
    pub is_home: bool,
    /// 24 local-hour labels (0–23), one per home-day hour column.
    pub hours: Vec<u32>,
    /// Shade band per column: 0 night, 1 awake (07–22), 2 work (09–17).
    pub shades: Vec<u8>,
}

/// The computed world-clock view.
#[derive(Debug, Clone, PartialEq)]
pub struct WorldClock {
    pub home_name: String,
    pub chosen_hour: u32,
    pub rows: Vec<CityClock>,
}

fn shade_for(hour: u32) -> u8 {
    if (9..=17).contains(&hour) {
        2
    } else if (7..=22).contains(&hour) {
        1
    } else {
        0
    }
}

fn short_date(dt: &chrono::DateTime<Tz>) -> String {
    const WD: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
    const MO: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let wd = dt.weekday().num_days_from_monday() as usize;
    format!("{} {} {}", WD[wd], dt.day(), MO[(dt.month0()) as usize])
}

/// Compute the world clock. `home`/`cities` are IANA ids (or names `resolve_zone`
/// accepts); `epoch_secs` is the reference instant; `chosen_hour` (0–23) is the
/// home-zone hour highlighted across all rows. Cities that don't resolve are
/// skipped. The home zone is always the first row.
pub fn compute(home: &str, cities: &[String], epoch_secs: i64, chosen_hour: u32) -> WorldClock {
    let home_tz: Tz = resolve_zone(home).unwrap_or(chrono_tz::UTC);
    let now_utc = Utc
        .timestamp_opt(epoch_secs, 0)
        .single()
        .unwrap_or_default();
    let home_now = now_utc.with_timezone(&home_tz);
    let home_date = home_now.date_naive();
    let home_off = home_now.offset().fix().local_minus_utc() / 60;

    // The instant at the chosen home-hour today, as UTC (handles DST folds).
    let chosen_naive = home_date
        .and_hms_opt(chosen_hour.min(23), 0, 0)
        .unwrap_or_else(|| home_date.and_hms_opt(0, 0, 0).unwrap());
    let chosen_utc = home_tz
        .from_local_datetime(&chosen_naive)
        .earliest()
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or(now_utc);

    // Build the ordered id list: home first, then the rest (deduped).
    let mut ids: Vec<String> = vec![home_tz.name().to_string()];
    for c in cities {
        if let Some(tz) = resolve_zone(c) {
            let id = tz.name().to_string();
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
    }

    let rows = ids
        .iter()
        .map(|id| {
            let tz: Tz = id.parse().unwrap_or(chrono_tz::UTC);
            let city_now = now_utc.with_timezone(&tz);
            let off = city_now.offset().fix().local_minus_utc() / 60;
            let city_at_chosen = chosen_utc.with_timezone(&tz);
            let hours: Vec<u32> = (0..24)
                .map(|c| {
                    let naive = home_date.and_hms_opt(c, 0, 0).unwrap();
                    let utc = home_tz
                        .from_local_datetime(&naive)
                        .earliest()
                        .map(|d| d.with_timezone(&Utc))
                        .unwrap_or(now_utc);
                    utc.with_timezone(&tz).hour()
                })
                .collect();
            let shades = hours.iter().map(|h| shade_for(*h)).collect();
            CityClock {
                name: zone_display_name(id),
                iana: id.clone(),
                abbrev: format!("{}", city_now.offset()),
                offset_minutes: off,
                rel_minutes: off - home_off,
                big_time: format!(
                    "{:02}:{:02}",
                    city_at_chosen.hour(),
                    city_at_chosen.minute()
                ),
                date_label: short_date(&city_at_chosen),
                is_home: id == &home_tz.name().to_string(),
                hours,
                shades,
            }
        })
        .collect();

    WorldClock {
        home_name: zone_display_name(home_tz.name()),
        chosen_hour: chosen_hour.min(23),
        rows,
    }
}

/// A one-line summary for pasting, e.g.
/// `Tokyo 22:00 (JST) · London 14:00 (BST) · Stockholm 15:00 (CEST)`.
pub fn summary_line(wc: &WorldClock) -> String {
    wc.rows
        .iter()
        .map(|r| format!("{} {} ({})", r.name, r.big_time, r.abbrev))
        .collect::<Vec<_>>()
        .join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_cities_abbrevs_and_iana() {
        assert_eq!(resolve_zone("tokyo"), Some(chrono_tz::Asia::Tokyo));
        assert_eq!(resolve_zone("New York"), Some(chrono_tz::America::New_York));
        assert_eq!(resolve_zone("pst"), Some(chrono_tz::America::Los_Angeles));
        assert_eq!(resolve_zone("Asia/Kolkata"), Some(chrono_tz::Asia::Kolkata));
        assert_eq!(resolve_zone("not a place"), None);
    }

    #[test]
    fn parse_time_tokens() {
        assert_eq!(parse_time_token("15:00"), Some((15, 0)));
        assert_eq!(parse_time_token("3pm"), Some((15, 0)));
        assert_eq!(parse_time_token("12am"), Some((0, 0)));
        assert_eq!(parse_time_token("12pm"), Some((12, 0)));
        assert_eq!(parse_time_token("9"), Some((9, 0)));
        assert_eq!(parse_time_token("9:30am"), Some((9, 30)));
        assert_eq!(parse_time_token("nope"), None);
    }

    #[test]
    fn parse_queries() {
        assert_eq!(
            parse_worldclock("time in tokyo"),
            Some(WorldClockRequest {
                base_hm: None,
                base_zone: None,
                targets: vec!["Asia/Tokyo".into()],
            })
        );
        assert_eq!(
            parse_worldclock("3pm pst to cet"),
            Some(WorldClockRequest {
                base_hm: Some((15, 0)),
                base_zone: Some("America/Los_Angeles".into()),
                targets: vec!["Europe/Paris".into()],
            })
        );
        assert_eq!(
            parse_worldclock("15:00 stockholm in new york, sf, tokyo"),
            Some(WorldClockRequest {
                base_hm: Some((15, 0)),
                base_zone: Some("Europe/Stockholm".into()),
                targets: vec![
                    "America/New_York".into(),
                    "America/Los_Angeles".into(),
                    "Asia/Tokyo".into(),
                ],
            })
        );
        // Not a world-clock query.
        assert_eq!(parse_worldclock("things to do"), None);
        assert_eq!(parse_worldclock("meeting in progress"), None);
    }

    #[test]
    fn compute_offsets_and_strip() {
        // 2025-01-15 12:00:00 UTC — winter, so no European DST.
        let epoch = 1_736_942_400;
        let cities = vec!["Asia/Tokyo".to_string(), "America/New_York".to_string()];
        let wc = compute("Europe/Stockholm", &cities, epoch, 15);
        assert_eq!(wc.home_name, "Stockholm");
        assert!(wc.rows[0].is_home);
        assert_eq!(wc.rows[0].rel_minutes, 0);
        // Tokyo is 8h ahead of Stockholm in January (UTC+9 vs UTC+1).
        let tokyo = wc.rows.iter().find(|r| r.iana == "Asia/Tokyo").unwrap();
        assert_eq!(tokyo.rel_minutes, 8 * 60);
        assert_eq!(tokyo.hours.len(), 24);
        // At home hour 15, Tokyo shows 23:00.
        assert_eq!(tokyo.big_time, "23:00");
    }

    #[test]
    fn summary_is_joined() {
        let epoch = 1_736_942_400;
        let wc = compute("Europe/Stockholm", &["Asia/Tokyo".to_string()], epoch, 15);
        let s = summary_line(&wc);
        assert!(s.contains("Stockholm 15:00"));
        assert!(s.contains("Tokyo 23:00"));
        assert!(s.contains('·'));
    }
}
