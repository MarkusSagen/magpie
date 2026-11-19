//! Inline-answer evaluator (Phase 6): dates / math / units, hand-rolled (no chrono,
//! no parser crate — see `docs/superpowers/specs/2026-09-24-magpie-inline-answers-design.md`).
//!
//! Pure, no I/O: `evaluate` takes an explicit [`ReferenceNow`] rather than reading the
//! OS clock, which is what makes the spec's example table a deterministic test fixture.

#![allow(clippy::too_many_lines)]

// ---------------------------------------------------------------------------
// Calendar helpers — copied from `magpie-app/src/format_time.rs` (Howard
// Hinnant's day-epoch algorithm) so `magpie-core` stays free of a date crate
// and free of a dependency on `magpie-app`.
// ---------------------------------------------------------------------------

fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

pub(crate) fn days_in_month(y: i64, m: i64) -> i64 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if is_leap(y) {
                29
            } else {
                28
            }
        }
        _ => 30,
    }
}

/// Howard Hinnant's days-from-epoch → civil (y, m, d), proleptic Gregorian.
pub(crate) fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Inverse of `civil_from_days`.
pub(crate) fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Day-of-week for a day number, 0 = Sunday (1970-01-01 was a Thursday).
pub(crate) fn weekday(days: i64) -> i64 {
    (days.rem_euclid(7) + 4) % 7
}

pub(crate) fn month_name(m: u32, long: bool) -> &'static str {
    const LONG: [&str; 12] = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    const SHORT: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let idx = ((m - 1) as usize).min(11);
    if long {
        LONG[idx]
    } else {
        SHORT[idx]
    }
}

pub(crate) fn weekday_name(wd: i64, long: bool) -> &'static str {
    const LONG: [&str; 7] = [
        "Sunday",
        "Monday",
        "Tuesday",
        "Wednesday",
        "Thursday",
        "Friday",
        "Saturday",
    ];
    const SHORT: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    let idx = (wd.rem_euclid(7)) as usize;
    if long {
        LONG[idx]
    } else {
        SHORT[idx]
    }
}

/// ISO-8601 week number (Monday-start, week 1 contains the first Thursday).
pub(crate) fn iso_week(day: i64) -> u32 {
    let (y, m, d) = civil_from_days(day);
    // Thursday of this week determines the ISO year.
    let wd_mon0 = (weekday(day) + 6) % 7; // 0=Mon..6=Sun
    let thursday = day - wd_mon0 + 3;
    let (ty, _, _) = civil_from_days(thursday);
    let jan1 = days_from_civil(ty, 1, 1);
    let _ = (y, m, d);
    (((thursday - jan1) / 7) + 1) as u32
}

// ---------------------------------------------------------------------------
// Exact rational arithmetic — stays exact through + - * / (per spec §2.2);
// decays to `f64` only on an irrational/transcendental operation, and decay
// is one-way and sticky from then on.
// ---------------------------------------------------------------------------

fn gcd_i128(a: i128, b: i128) -> i128 {
    let (mut a, mut b) = (a.abs(), b.abs());
    while b != 0 {
        let t = b;
        b = a % b;
        a = t;
    }
    if a == 0 {
        1
    } else {
        a
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rat {
    pub num: i128,
    pub den: i128,
}

impl Rat {
    fn new(num: i128, den: i128) -> Option<Rat> {
        if den == 0 {
            return None;
        }
        let sign: i128 = if den < 0 { -1 } else { 1 };
        let num = num.checked_mul(sign)?;
        let den = den.checked_mul(sign)?;
        let g = gcd_i128(num, den);
        Some(Rat {
            num: num / g,
            den: den / g,
        })
    }

    fn int(n: i128) -> Rat {
        Rat { num: n, den: 1 }
    }

    fn to_f64(self) -> f64 {
        self.num as f64 / self.den as f64
    }

    fn is_integer(self) -> bool {
        self.den == 1
    }

    fn checked_add(self, o: Rat) -> Option<Rat> {
        let num = self
            .num
            .checked_mul(o.den)?
            .checked_add(o.num.checked_mul(self.den)?)?;
        let den = self.den.checked_mul(o.den)?;
        Rat::new(num, den)
    }

    fn checked_sub(self, o: Rat) -> Option<Rat> {
        self.checked_add(Rat {
            num: -o.num,
            den: o.den,
        })
    }

    fn checked_mul(self, o: Rat) -> Option<Rat> {
        let num = self.num.checked_mul(o.num)?;
        let den = self.den.checked_mul(o.den)?;
        Rat::new(num, den)
    }

    fn checked_div(self, o: Rat) -> Option<Rat> {
        let num = self.num.checked_mul(o.den)?;
        let den = self.den.checked_mul(o.num)?;
        Rat::new(num, den)
    }
}

/// A number that is either an exact reduced fraction, a sticky-decayed float,
/// or a huge-factorial magnitude (`mantissa * 10^exponent`, never materialized
/// as a raw integer — see spec §2.4).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Num {
    R(Rat),
    F(f64),
    Mag { mantissa: f64, exponent: i64 },
}

impl Num {
    fn to_f64(self) -> f64 {
        match self {
            Num::R(r) => r.to_f64(),
            Num::F(f) => f,
            Num::Mag { mantissa, exponent } => {
                mantissa * 10f64.powi(exponent.clamp(-300, 300) as i32)
            }
        }
    }

    fn add(a: Num, b: Num) -> Result<Num, String> {
        if let (Num::R(x), Num::R(y)) = (a, b) {
            if let Some(r) = x.checked_add(y) {
                return Ok(Num::R(r));
            }
        }
        Ok(Num::F(a.to_f64() + b.to_f64()))
    }

    fn sub(a: Num, b: Num) -> Result<Num, String> {
        if let (Num::R(x), Num::R(y)) = (a, b) {
            if let Some(r) = x.checked_sub(y) {
                return Ok(Num::R(r));
            }
        }
        Ok(Num::F(a.to_f64() - b.to_f64()))
    }

    fn mul(a: Num, b: Num) -> Result<Num, String> {
        if let (Num::R(x), Num::R(y)) = (a, b) {
            if let Some(r) = x.checked_mul(y) {
                return Ok(Num::R(r));
            }
        }
        Ok(Num::F(a.to_f64() * b.to_f64()))
    }

    fn div(a: Num, b: Num) -> Result<Num, String> {
        if let (Num::R(x), Num::R(y)) = (a, b) {
            if y.num == 0 {
                return Err(if x.num == 0 {
                    "0 \u{f7} 0 is undefined".to_string()
                } else {
                    "can't divide by zero".to_string()
                });
            }
            if let Some(r) = x.checked_div(y) {
                return Ok(Num::R(r));
            }
        }
        let bf = b.to_f64();
        if bf == 0.0 {
            return Err(if a.to_f64() == 0.0 {
                "0 \u{f7} 0 is undefined".to_string()
            } else {
                "can't divide by zero".to_string()
            });
        }
        Ok(Num::F(a.to_f64() / bf))
    }

    fn neg(a: Num) -> Num {
        match a {
            Num::R(r) => Num::R(Rat {
                num: -r.num,
                den: r.den,
            }),
            Num::F(f) => Num::F(-f),
            Num::Mag { mantissa, exponent } => Num::Mag {
                mantissa: -mantissa,
                exponent,
            },
        }
    }

    fn pow(base: Num, exp: Num) -> Result<Num, String> {
        if let (Num::R(b), Num::R(e)) = (base, exp) {
            if e.is_integer() {
                let n = e.num;
                if n >= 0 {
                    let mut acc = Rat::int(1);
                    let mut overflow = false;
                    for _ in 0..n {
                        match acc.checked_mul(b) {
                            Some(r) => acc = r,
                            None => {
                                overflow = true;
                                break;
                            }
                        }
                    }
                    if !overflow {
                        return Ok(Num::R(acc));
                    }
                } else if b.num != 0 {
                    let mut acc = Rat::int(1);
                    let mut overflow = false;
                    for _ in 0..(-n) {
                        match acc.checked_mul(b) {
                            Some(r) => acc = r,
                            None => {
                                overflow = true;
                                break;
                            }
                        }
                    }
                    if !overflow {
                        if let Some(r) = Rat::new(acc.den, acc.num) {
                            return Ok(Num::R(r));
                        }
                    }
                }
            }
        }
        Ok(Num::F(base.to_f64().powf(exp.to_f64())))
    }
}

// ---------------------------------------------------------------------------
// Gamma function (hand-rolled Lanczos approximation) — used for non-integer
// factorial (`Γ(n+1)`) and, in log-space, for the huge-factorial magnitude
// display (spec §2.4/§5). Also a hand-rolled `erf` for `normal.cdf`.
// ---------------------------------------------------------------------------

const LANCZOS_G: f64 = 7.0;
const LANCZOS: [f64; 9] = [
    0.999_999_999_999_809_9,
    676.520_368_121_885_1,
    -1_259.139_216_722_402_8,
    771.323_428_777_653_1,
    -176.615_029_162_140_6,
    12.507_343_278_686_905,
    -0.138_571_095_265_720_12,
    9.984_369_578_019_572e-6,
    1.505_632_735_149_312e-7,
];

/// ln(Γ(x)) for x > 0, via Lanczos + reflection for x < 0.5. Stable for huge
/// `x` (unlike computing Γ(x) directly, which overflows `f64` well before
/// x = 200).
fn ln_gamma(x: f64) -> f64 {
    if x < 0.5 {
        (std::f64::consts::PI / (std::f64::consts::PI * x).sin()).ln() - ln_gamma(1.0 - x)
    } else {
        let x = x - 1.0;
        let mut a = LANCZOS[0];
        let t = x + LANCZOS_G + 0.5;
        for (i, c) in LANCZOS.iter().enumerate().skip(1) {
            a += c / (x + i as f64);
        }
        0.5 * (2.0 * std::f64::consts::PI).ln() + (x + 0.5) * t.ln() - t + a.ln()
    }
}

fn gamma(x: f64) -> f64 {
    if x < 0.5 {
        std::f64::consts::PI / ((std::f64::consts::PI * x).sin() * gamma(1.0 - x))
    } else {
        ln_gamma(x).exp()
    }
}

/// Abramowitz & Stegun 7.1.26 rational approximation, |error| < 1.5e-7.
fn erf(x: f64) -> f64 {
    let sign = if x < 0.0 { -1.0 } else { 1.0 };
    let x = x.abs();
    let a1 = 0.254_829_592;
    let a2 = -0.284_496_736;
    let a3 = 1.421_413_741;
    let a4 = -1.453_152_027;
    let a5 = 1.061_405_429;
    let p = 0.327_591_1;
    let t = 1.0 / (1.0 + p * x);
    let y = 1.0 - (((((a5 * t + a4) * t) + a3) * t + a2) * t + a1) * t * (-x * x).exp();
    sign * y
}

fn factorial_exact_i128(n: i64) -> Option<i128> {
    let mut acc: i128 = 1;
    for i in 2..=n {
        acc = acc.checked_mul(i as i128)?;
    }
    Some(acc)
}

fn factorial_magnitude(n: i64) -> Num {
    let lg = ln_gamma(n as f64 + 1.0);
    let log10 = lg / std::f64::consts::LN_10;
    let exponent = log10.floor() as i64;
    let mantissa = 10f64.powf(log10 - exponent as f64);
    Num::Mag { mantissa, exponent }
}

fn factorial(n: Num) -> Result<Num, String> {
    match n {
        Num::R(r) if r.is_integer() => {
            let i = r.num;
            if i < 0 {
                return Err("undefined (pole of \u{393}) at negative integers".to_string());
            }
            let i: i64 = i.try_into().unwrap_or(i64::MAX);
            match factorial_exact_i128(i) {
                Some(v) => Ok(Num::R(Rat::int(v))),
                None => Ok(factorial_magnitude(i)),
            }
        }
        _ => Ok(Num::F(gamma(n.to_f64() + 1.0))),
    }
}

fn double_factorial(n: Num) -> Result<Num, String> {
    let r = match n {
        Num::R(r) if r.is_integer() => r,
        _ => return Err("!! is only defined for integers".to_string()),
    };
    if r.num == -1 {
        return Ok(Num::R(Rat::int(1)));
    }
    if r.num < -1 {
        return Err("!! is undefined for that negative integer".to_string());
    }
    let n = r.num;
    let mut acc: i128 = 1;
    let mut k = n;
    while k > 0 {
        acc = match acc.checked_mul(k) {
            Some(v) => v,
            None => return Ok(Num::F(f64::INFINITY)),
        };
        k -= 2;
    }
    Ok(Num::R(Rat::int(acc)))
}

// ---------------------------------------------------------------------------
// Lexer
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum TokKind {
    Num(Rat),
    Ordinal(u32),
    Plus,
    Minus,
    Star,
    Slash,
    Caret,
    Bang,
    Percent,
    Comma,
    Semicolon,
    LParen,
    RParen,
    Word(String),
    Iso(i64, u32, u32, Option<(u32, u32)>),
}

#[derive(Debug, Clone, PartialEq)]
struct Token {
    kind: TokKind,
    space_before: bool,
}

fn is_word_char(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '\''
}

/// Whole-input lexer. Returns `None` on an unrecognized character (never
/// silently drops input — that would violate the whole-string parse rule).
fn lex(input: &str) -> Option<Vec<Token>> {
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0usize;
    let mut out = Vec::new();
    let mut space_before = true; // leading position counts as "boundary"
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            space_before = true;
            i += 1;
            continue;
        }
        let sb = space_before;
        space_before = false;

        // ISO date: 4 digits '-' 2 digits '-' 2 digits, optional T HH:MM.
        if c.is_ascii_digit()
            && i + 10 <= chars.len()
            && chars[i + 4] == '-'
            && chars[i + 7] == '-'
            && chars[i..i + 4].iter().all(|c| c.is_ascii_digit())
            && chars[i + 5..i + 7].iter().all(|c| c.is_ascii_digit())
            && chars[i + 8..i + 10].iter().all(|c| c.is_ascii_digit())
        {
            let y: i64 = chars[i..i + 4].iter().collect::<String>().parse().ok()?;
            let m: u32 = chars[i + 5..i + 7]
                .iter()
                .collect::<String>()
                .parse()
                .ok()?;
            let d: u32 = chars[i + 8..i + 10]
                .iter()
                .collect::<String>()
                .parse()
                .ok()?;
            let mut j = i + 10;
            let mut time = None;
            if j < chars.len() && (chars[j] == 'T' || chars[j] == 't') {
                let k = j + 1;
                if k + 5 <= chars.len()
                    && chars[k..k + 2].iter().all(|c| c.is_ascii_digit())
                    && chars[k + 2] == ':'
                    && chars[k + 3..k + 5].iter().all(|c| c.is_ascii_digit())
                {
                    let hh: u32 = chars[k..k + 2].iter().collect::<String>().parse().ok()?;
                    let mm: u32 = chars[k + 3..k + 5]
                        .iter()
                        .collect::<String>()
                        .parse()
                        .ok()?;
                    time = Some((hh, mm));
                    j = k + 5;
                }
            }
            out.push(Token {
                kind: TokKind::Iso(y, m, d, time),
                space_before: sb,
            });
            i = j;
            continue;
        }

        if c.is_ascii_digit() {
            let start = i;
            while i < chars.len() && chars[i].is_ascii_digit() {
                i += 1;
            }
            // Ordinal suffix ("1st", "22nd", ...), only for plain integers.
            if i + 1 < chars.len() {
                let two: String = chars[i..(i + 2).min(chars.len())].iter().collect();
                let low = two.to_ascii_lowercase();
                if matches!(low.as_str(), "st" | "nd" | "rd" | "th") {
                    let boundary = i + 2 >= chars.len() || !chars[i + 2].is_alphanumeric();
                    if boundary {
                        let n: u32 = chars[start..i].iter().collect::<String>().parse().ok()?;
                        out.push(Token {
                            kind: TokKind::Ordinal(n),
                            space_before: sb,
                        });
                        i += 2;
                        continue;
                    }
                }
            }
            let mut has_frac = false;
            if i < chars.len()
                && chars[i] == '.'
                && i + 1 < chars.len()
                && chars[i + 1].is_ascii_digit()
            {
                has_frac = true;
                i += 1;
                while i < chars.len() && chars[i].is_ascii_digit() {
                    i += 1;
                }
            }
            let mut exp: Option<i32> = None;
            if i < chars.len() && (chars[i] == 'e' || chars[i] == 'E') {
                let mut k = i + 1;
                let mut sign = 1i32;
                if k < chars.len() && (chars[k] == '+' || chars[k] == '-') {
                    if chars[k] == '-' {
                        sign = -1;
                    }
                    k += 1;
                }
                if k < chars.len() && chars[k].is_ascii_digit() {
                    let estart = k;
                    while k < chars.len() && chars[k].is_ascii_digit() {
                        k += 1;
                    }
                    let e: i32 = chars[estart..k].iter().collect::<String>().parse().ok()?;
                    exp = Some(sign * e);
                    i = k;
                }
            }
            let text: String = chars[start..i]
                .iter()
                .filter(|c| **c != '.')
                .collect::<String>();
            // Recompute without exponent digits already consumed separately.
            let mantissa_str: String = {
                let end_mantissa = if let Some(_e) = exp {
                    // find position of 'e'/'E' within [start,i)
                    let mut p = start;
                    while p < i && chars[p] != 'e' && chars[p] != 'E' {
                        p += 1;
                    }
                    p
                } else {
                    i
                };
                chars[start..end_mantissa]
                    .iter()
                    .filter(|c| **c != '.')
                    .collect()
            };
            let frac_digits = if has_frac {
                let dot = chars[start..i].iter().position(|c| *c == '.').unwrap();
                let mantissa_end = if let Some(_e) = exp {
                    let mut p = start;
                    while p < i && chars[p] != 'e' && chars[p] != 'E' {
                        p += 1;
                    }
                    p - start
                } else {
                    i - start
                };
                mantissa_end - dot - 1
            } else {
                0
            };
            let mantissa_val: i128 = mantissa_str.parse().ok()?;
            let _ = text;
            let mut rat = Rat::new(mantissa_val, 10i128.pow(frac_digits as u32))?;
            if let Some(e) = exp {
                if e >= 0 {
                    rat = rat.checked_mul(Rat::int(10i128.checked_pow(e as u32)?))?;
                } else {
                    rat = rat.checked_div(Rat::int(10i128.checked_pow((-e) as u32)?))?;
                }
            }
            out.push(Token {
                kind: TokKind::Num(rat),
                space_before: sb,
            });
            continue;
        }

        if is_word_char(c) {
            let start = i;
            i += 1;
            while i < chars.len() && is_word_char(chars[i]) {
                i += 1;
            }
            loop {
                if i < chars.len()
                    && chars[i] == '-'
                    && i + 1 < chars.len()
                    && is_word_char(chars[i + 1])
                {
                    i += 1;
                    while i < chars.len() && is_word_char(chars[i]) {
                        i += 1;
                    }
                    continue;
                }
                if i < chars.len()
                    && chars[i] == '.'
                    && i + 1 < chars.len()
                    && chars[i + 1].is_ascii_alphabetic()
                {
                    i += 1;
                    while i < chars.len() && is_word_char(chars[i]) {
                        i += 1;
                    }
                    continue;
                }
                break;
            }
            let word: String = chars[start..i]
                .iter()
                .collect::<String>()
                .to_ascii_lowercase();
            out.push(Token {
                kind: TokKind::Word(word),
                space_before: sb,
            });
            continue;
        }

        // Degree symbol sugar: treat '°' as its own single-char word "deg".
        if c == '\u{b0}' {
            out.push(Token {
                kind: TokKind::Word("deg".to_string()),
                space_before: sb,
            });
            i += 1;
            continue;
        }
        if c == '\u{3c0}' {
            // π
            out.push(Token {
                kind: TokKind::Word("\u{3c0}".to_string()),
                space_before: sb,
            });
            i += 1;
            continue;
        }

        let kind = match c {
            '+' => TokKind::Plus,
            '-' => TokKind::Minus,
            '*' => TokKind::Star,
            '/' => TokKind::Slash,
            '^' => TokKind::Caret,
            '!' => TokKind::Bang,
            '%' => TokKind::Percent,
            ',' => TokKind::Comma,
            ';' => TokKind::Semicolon,
            '(' => TokKind::LParen,
            ')' => TokKind::RParen,
            _ => return None,
        };
        out.push(Token {
            kind,
            space_before: sb,
        });
        i += 1;
    }
    Some(out)
}

// ---------------------------------------------------------------------------
// Name tables
// ---------------------------------------------------------------------------

fn weekday_from_word(w: &str) -> Option<i64> {
    let w = w.strip_suffix('s').unwrap_or(w);
    Some(match w {
        "sun" | "sunday" => 0,
        "mon" | "monday" => 1,
        "tue" | "tues" | "tuesday" => 2,
        "wed" | "weds" | "wednesday" => 3,
        "thu" | "thur" | "thurs" | "thursday" => 4,
        "fri" | "friday" => 5,
        "sat" | "saturday" => 6,
        _ => return None,
    })
}

fn month_from_word(w: &str) -> Option<u32> {
    Some(match w {
        "jan" | "january" => 1,
        "feb" | "february" => 2,
        "mar" | "march" => 3,
        "apr" | "april" => 4,
        "may" => 5,
        "jun" | "june" => 6,
        "jul" | "july" => 7,
        "aug" | "august" => 8,
        "sep" | "sept" | "september" => 9,
        "oct" | "october" => 10,
        "nov" | "november" => 11,
        "dec" | "december" => 12,
        _ => return None,
    })
}

fn ordinal_word(w: &str) -> Option<u32> {
    Some(match w {
        "first" => 1,
        "second" => 2,
        "third" => 3,
        "fourth" => 4,
        "fifth" => 5,
        "sixth" => 6,
        "seventh" => 7,
        "eighth" => 8,
        "ninth" => 9,
        "tenth" => 10,
        "eleventh" => 11,
        "twelfth" => 12,
        "thirteenth" => 13,
        "fourteenth" => 14,
        "fifteenth" => 15,
        "sixteenth" => 16,
        "seventeenth" => 17,
        "eighteenth" => 18,
        "nineteenth" => 19,
        "twentieth" => 20,
        "twenty-first" => 21,
        "twenty-second" => 22,
        "twenty-third" => 23,
        "twenty-fourth" => 24,
        "twenty-fifth" => 25,
        "twenty-sixth" => 26,
        "twenty-seventh" => 27,
        "twenty-eighth" => 28,
        "twenty-ninth" => 29,
        "thirtieth" => 30,
        "thirty-first" => 31,
        _ => return None,
    })
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum UnitTime {
    Day,
    Week,
    Hour,
    Minute,
    Month,
    Year,
}

fn time_unit_from_word(w: &str) -> Option<UnitTime> {
    Some(match w {
        "d" | "day" | "days" => UnitTime::Day,
        "w" | "week" | "weeks" => UnitTime::Week,
        "h" | "hour" | "hours" => UnitTime::Hour,
        "m" | "min" | "minute" | "minutes" => UnitTime::Minute,
        "mo" | "month" | "months" => UnitTime::Month,
        "y" | "year" | "years" => UnitTime::Year,
        _ => return None,
    })
}

// ---------------------------------------------------------------------------
// Units (length / mass / time / temperature) — spec §3.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
enum UnitDomain {
    Length,
    Mass,
    Time,
    Temp,
    DataSize,
    Speed,
    Area,
    Volume,
    Angle,
    Energy,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct UnitDef {
    domain: UnitDomain,
    /// Exact rational factor to the domain's base unit (meters / grams / seconds).
    /// Unused for `Temp` (affine, handled specially).
    to_base: Rat,
    name: &'static str,
}

fn unit_from_word(w: &str) -> Option<UnitDef> {
    let mm = Rat::new(1, 1000).unwrap();
    let cm = Rat::new(1, 100).unwrap();
    let m = Rat::int(1);
    let km = Rat::int(1000);
    let inch = Rat::new(127, 5000).unwrap(); // 2.54 cm exactly
    let ft = inch.checked_mul(Rat::int(12)).unwrap();
    let yd = ft.checked_mul(Rat::int(3)).unwrap();
    let mi = ft.checked_mul(Rat::int(5280)).unwrap();
    let mg = Rat::new(1, 1000).unwrap();
    let g = Rat::int(1);
    let kg = Rat::int(1000);
    let lb = Rat::new(45_359_237, 100_000).unwrap(); // exact intl avoirdupois pound, grams
    let oz = lb.checked_div(Rat::int(16)).unwrap();
    let sec = Rat::int(1);
    let min = Rat::int(60);
    let hour = Rat::int(3600);
    let day = Rat::int(86_400);
    let week = Rat::int(604_800);
    Some(match w {
        "mm" => UnitDef {
            domain: UnitDomain::Length,
            to_base: mm,
            name: "mm",
        },
        "cm" => UnitDef {
            domain: UnitDomain::Length,
            to_base: cm,
            name: "cm",
        },
        "m" | "meter" | "meters" | "metre" | "metres" => UnitDef {
            domain: UnitDomain::Length,
            to_base: m,
            name: "m",
        },
        "km" => UnitDef {
            domain: UnitDomain::Length,
            to_base: km,
            name: "km",
        },
        "in" | "inch" | "inches" => UnitDef {
            domain: UnitDomain::Length,
            to_base: inch,
            name: "in",
        },
        "ft" | "foot" | "feet" => UnitDef {
            domain: UnitDomain::Length,
            to_base: ft,
            name: "ft",
        },
        "yd" | "yard" | "yards" => UnitDef {
            domain: UnitDomain::Length,
            to_base: yd,
            name: "yd",
        },
        "mi" | "mile" | "miles" => UnitDef {
            domain: UnitDomain::Length,
            to_base: mi,
            name: "mi",
        },
        "mg" => UnitDef {
            domain: UnitDomain::Mass,
            to_base: mg,
            name: "mg",
        },
        "g" | "gram" | "grams" => UnitDef {
            domain: UnitDomain::Mass,
            to_base: g,
            name: "g",
        },
        "kg" => UnitDef {
            domain: UnitDomain::Mass,
            to_base: kg,
            name: "kg",
        },
        "oz" | "ounce" | "ounces" => UnitDef {
            domain: UnitDomain::Mass,
            to_base: oz,
            name: "oz",
        },
        "lb" | "lbs" | "pound" | "pounds" => UnitDef {
            domain: UnitDomain::Mass,
            to_base: lb,
            name: "lb",
        },
        "s" | "sec" | "secs" | "second" | "seconds" => UnitDef {
            domain: UnitDomain::Time,
            to_base: sec,
            name: "s",
        },
        "min" | "mins" | "minute" | "minutes" => UnitDef {
            domain: UnitDomain::Time,
            to_base: min,
            name: "min",
        },
        "h" | "hr" | "hour" | "hours" => UnitDef {
            domain: UnitDomain::Time,
            to_base: hour,
            name: "h",
        },
        "day" | "days" => UnitDef {
            domain: UnitDomain::Time,
            to_base: day,
            name: "day",
        },
        "week" | "weeks" => UnitDef {
            domain: UnitDomain::Time,
            to_base: week,
            name: "week",
        },
        "c" | "celsius" => UnitDef {
            domain: UnitDomain::Temp,
            to_base: Rat::int(0),
            name: "c",
        },
        "f" | "fahrenheit" => UnitDef {
            domain: UnitDomain::Temp,
            to_base: Rat::int(1),
            name: "f",
        },
        "k" | "kelvin" => UnitDef {
            domain: UnitDomain::Temp,
            to_base: Rat::int(2),
            name: "k",
        },

        // ---- Data size (base: bytes). Decimal (10³) vs binary (2¹⁰). ----
        "b" | "byte" | "bytes" => dsize(Rat::int(1), "B"),
        "kb" | "kilobyte" | "kilobytes" => dsize(Rat::int(1000), "KB"),
        "mb" | "megabyte" | "megabytes" => dsize(Rat::int(1_000_000), "MB"),
        "gb" | "gigabyte" | "gigabytes" => dsize(Rat::int(1_000_000_000), "GB"),
        "tb" | "terabyte" | "terabytes" => dsize(Rat::int(1_000_000_000_000), "TB"),
        "kib" | "kibibyte" | "kibibytes" => dsize(Rat::int(1024), "KiB"),
        "mib" | "mebibyte" | "mebibytes" => dsize(Rat::int(1_048_576), "MiB"),
        "gib" | "gibibyte" | "gibibytes" => dsize(Rat::int(1_073_741_824), "GiB"),
        "tib" | "tebibyte" | "tebibytes" => dsize(Rat::int(1_099_511_627_776), "TiB"),

        // ---- Speed (base: m/s). ----
        "m/s" | "mps" => speed(Rat::int(1), "m/s"),
        "km/h" | "kmh" | "kph" => speed(Rat::new(5, 18).unwrap(), "km/h"),
        "mph" | "mi/h" => speed(Rat::new(1397, 3125).unwrap(), "mph"), // 0.44704 m/s exactly
        "km/s" => speed(Rat::int(1000), "km/s"),
        "knot" | "knots" | "kn" => speed(Rat::new(463, 900).unwrap(), "kn"), // 1852/3600
        "ft/s" | "fps" => speed(Rat::new(381, 1250).unwrap(), "ft/s"),       // 0.3048 m/s

        // ---- Area (base: m²). ----
        "m2" | "m²" | "sqm" => area(Rat::int(1), "m²"),
        "cm2" | "cm²" => area(Rat::new(1, 10000).unwrap(), "cm²"),
        "km2" | "km²" => area(Rat::int(1_000_000), "km²"),
        "ha" | "hectare" | "hectares" => area(Rat::int(10_000), "ha"),
        "acre" | "acres" => area(Rat::new(40_468_564_224, 10_000_000).unwrap(), "acre"), // 4046.8564224 m²
        "ft2" | "ft²" | "sqft" => area(Rat::new(9_290_304, 100_000_000).unwrap(), "ft²"), // 0.09290304 m²

        // ---- Volume (base: litre). ----
        "ml" | "milliliter" | "millilitre" => vol(Rat::new(1, 1000).unwrap(), "ml"),
        "l" | "liter" | "litre" | "liters" | "litres" => vol(Rat::int(1), "l"),
        "cl" | "centiliter" => vol(Rat::new(1, 100).unwrap(), "cl"),
        "dl" | "deciliter" => vol(Rat::new(1, 10).unwrap(), "dl"),
        "gal" | "gallon" | "gallons" => vol(Rat::new(454_609, 100_000).unwrap(), "gal"), // UK gallon 4.54609 l
        "usgal" => vol(Rat::new(3_785_411_784, 1_000_000_000).unwrap(), "US gal"),
        "cup" | "cups" => vol(Rat::new(473_176_473, 2_000_000_000).unwrap(), "cup"), // US cup 236.588 ml
        "tbsp" => vol(Rat::new(295_735_296, 20_000_000_000).unwrap(), "tbsp"),
        "tsp" => vol(Rat::new(295_735_296, 60_000_000_000).unwrap(), "tsp"),

        // ---- Angle (base: radian, using a rational π approximation is lossy, so
        //      store degrees as base and let radians decay to float on convert). ----
        "deg" | "degree" | "degrees" | "°" => angle(Rat::int(1), "°"),
        // 1 rad = 180/π ≈ 57.29577951° (rational approximation — π is irrational).
        "rad" | "radian" | "radians" => angle(Rat::new(5_729_577_951, 100_000_000).unwrap(), "rad"),
        "grad" | "gon" => angle(Rat::new(9, 10).unwrap(), "grad"),
        "turn" | "turns" => angle(Rat::int(360), "turn"),

        // ---- Energy (base: joule). ----
        "j" | "joule" | "joules" => energy(Rat::int(1), "J"),
        "kj" | "kilojoule" | "kilojoules" => energy(Rat::int(1000), "kJ"),
        "cal" | "calorie" | "calories" => energy(Rat::new(4184, 1000).unwrap(), "cal"),
        "kcal" | "kilocalorie" | "kilocalories" => energy(Rat::new(4184, 1).unwrap(), "kcal"),
        "wh" | "watthour" => energy(Rat::int(3600), "Wh"),
        "kwh" | "kilowatthour" => energy(Rat::int(3_600_000), "kWh"),

        _ => return None,
    })
}

fn dsize(to_base: Rat, name: &'static str) -> UnitDef {
    UnitDef {
        domain: UnitDomain::DataSize,
        to_base,
        name,
    }
}
fn speed(to_base: Rat, name: &'static str) -> UnitDef {
    UnitDef {
        domain: UnitDomain::Speed,
        to_base,
        name,
    }
}
fn area(to_base: Rat, name: &'static str) -> UnitDef {
    UnitDef {
        domain: UnitDomain::Area,
        to_base,
        name,
    }
}
fn vol(to_base: Rat, name: &'static str) -> UnitDef {
    UnitDef {
        domain: UnitDomain::Volume,
        to_base,
        name,
    }
}
fn angle(to_base: Rat, name: &'static str) -> UnitDef {
    UnitDef {
        domain: UnitDomain::Angle,
        to_base,
        name,
    }
}
fn energy(to_base: Rat, name: &'static str) -> UnitDef {
    UnitDef {
        domain: UnitDomain::Energy,
        to_base,
        name,
    }
}

fn convert_units(value: Num, from: UnitDef, to: UnitDef) -> Result<Num, String> {
    if from.domain != to.domain {
        return Err("can't convert between those units".to_string());
    }
    if from.domain == UnitDomain::Temp {
        // to_base tag: 0=C, 1=F, 2=K. Affine, not a ratio.
        let to_celsius = |v: Num, tag: i128| -> Result<Num, String> {
            match tag {
                0 => Ok(v),
                1 => Num::mul(
                    Num::sub(v, Num::R(Rat::int(32)))?,
                    Num::R(Rat::new(5, 9).unwrap()),
                ),
                2 => Num::sub(v, Num::R(Rat::new(27315, 100).unwrap())),
                _ => unreachable!(),
            }
        };
        let from_celsius = |v: Num, tag: i128| -> Result<Num, String> {
            match tag {
                0 => Ok(v),
                1 => Num::add(
                    Num::mul(v, Num::R(Rat::new(9, 5).unwrap()))?,
                    Num::R(Rat::int(32)),
                ),
                2 => Num::add(v, Num::R(Rat::new(27315, 100).unwrap())),
                _ => unreachable!(),
            }
        };
        let c = to_celsius(value, from.to_base.num)?;
        return from_celsius(c, to.to_base.num);
    }
    let base = Num::mul(value, Num::R(from.to_base))?;
    Num::div(base, Num::R(to.to_base))
}

/// Approximate densities (g/ml) for volume↔mass cooking conversions
/// (`1 cup flour in g`). Kitchen-grade values, not lab constants.
fn density_of(word: &str) -> Option<(&'static str, Rat)> {
    let r = |n, d| Rat::new(n, d).unwrap();
    Some(match word {
        "water" => ("water", Rat::int(1)),
        "milk" => ("milk", r(103, 100)),
        "flour" => ("flour", r(53, 100)),
        "sugar" => ("sugar", r(85, 100)),
        "butter" => ("butter", r(911, 1000)),
        "oil" => ("oil", r(92, 100)),
        "honey" => ("honey", r(142, 100)),
        "rice" => ("rice", r(85, 100)),
        "salt" => ("salt", r(6, 5)),
        "oats" => ("oats", r(41, 100)),
        _ => return None,
    })
}

/// Convert between Volume (base litre) and Mass (base gram) through a density in g/ml.
fn convert_with_density(
    value: Num,
    from: UnitDef,
    to: UnitDef,
    g_per_ml: Rat,
) -> Result<Num, String> {
    let ml_per_l = Num::R(Rat::int(1000));
    let base = Num::mul(value, Num::R(from.to_base))?;
    let out_base = match (from.domain, to.domain) {
        // litres → ml → grams
        (UnitDomain::Volume, UnitDomain::Mass) => {
            Num::mul(Num::mul(base, ml_per_l)?, Num::R(g_per_ml))?
        }
        // grams → ml → litres
        (UnitDomain::Mass, UnitDomain::Volume) => {
            Num::div(Num::div(base, Num::R(g_per_ml))?, ml_per_l)?
        }
        _ => return convert_units(value, from, to),
    };
    Num::div(out_base, Num::R(to.to_base))
}

fn is_volume_mass(a: UnitDomain, b: UnitDomain) -> bool {
    matches!(
        (a, b),
        (UnitDomain::Volume, UnitDomain::Mass) | (UnitDomain::Mass, UnitDomain::Volume)
    )
}

// ---------------------------------------------------------------------------
// Math parser + evaluator (recursive descent). Handles percent forms,
// factorial/double-factorial, constants, implicit multiplication, functions,
// distributions, and unit conversion.
// ---------------------------------------------------------------------------

fn is_func_name(w: &str) -> bool {
    matches!(
        w,
        "sin"
            | "cos"
            | "tan"
            | "asin"
            | "acos"
            | "atan"
            | "log"
            | "ln"
            | "sqrt"
            | "abs"
            | "normal.pdf"
            | "normal.cdf"
            | "binom.pdf"
            | "binom.cdf"
            | "poisson.pdf"
            | "poisson.cdf"
    )
}

/// Display-only snap: a float within 1e-10 of an integer/half-integer snaps
/// to that exact value for legibility (spec §2.5) — never applied to stored
/// values or comparisons, only at the point functions decide their answer.
fn snap(f: f64) -> f64 {
    let r = f.round();
    if (f - r).abs() < 1e-10 {
        return r;
    }
    let h = (f * 2.0).round() / 2.0;
    if (f - h).abs() < 1e-10 {
        return h;
    }
    f
}

fn eval_func(name: &str, args: Vec<Result<Num, String>>) -> Result<Num, String> {
    for a in &args {
        if let Err(e) = a {
            return Err(e.clone());
        }
    }
    let vals: Vec<f64> = args.iter().map(|a| a.clone().unwrap().to_f64()).collect();
    match name {
        "sin" => Ok(Num::F(snap(vals[0].sin()))),
        "cos" => Ok(Num::F(snap(vals[0].cos()))),
        "tan" => Ok(Num::F(snap(vals[0].tan()))),
        "asin" => Ok(Num::F(snap(vals[0].asin()))),
        "acos" => Ok(Num::F(snap(vals[0].acos()))),
        "atan" => Ok(Num::F(snap(vals[0].atan()))),
        "ln" => {
            if vals[0] <= 0.0 {
                return Err("ln of a non-positive number isn't supported".to_string());
            }
            Ok(Num::F(snap(vals[0].ln())))
        }
        "log" => {
            if vals[0] <= 0.0 {
                return Err("log of a non-positive number isn't supported".to_string());
            }
            if vals.len() >= 2 {
                Ok(Num::F(snap(vals[0].log(vals[1]))))
            } else {
                Ok(Num::F(snap(vals[0].log10())))
            }
        }
        "sqrt" => {
            if vals[0] < 0.0 {
                return Err("sqrt of a negative number isn't supported".to_string());
            }
            let v = args[0].clone().unwrap();
            if let Num::R(r) = v {
                let n = (r.num as f64).sqrt().round() as i128;
                let d = (r.den as f64).sqrt().round() as i128;
                if n * n == r.num && d * d == r.den {
                    return Ok(Num::R(Rat::new(n, d).unwrap()));
                }
            }
            Ok(Num::F(snap(vals[0].sqrt())))
        }
        "abs" => {
            let v = args[0].clone().unwrap();
            Ok(match v {
                Num::R(r) => Num::R(Rat {
                    num: r.num.abs(),
                    den: r.den,
                }),
                Num::F(f) => Num::F(f.abs()),
                other => other,
            })
        }
        "normal.pdf" => {
            let (x, mean, sd) = (vals[0], vals[1], vals[2]);
            let z = (x - mean) / sd;
            Ok(Num::F(
                (-0.5 * z * z).exp() / (sd * (2.0 * std::f64::consts::PI).sqrt()),
            ))
        }
        "normal.cdf" => {
            let (x, mean, sd) = (vals[0], vals[1], vals[2]);
            let z = (x - mean) / (sd * std::f64::consts::SQRT_2);
            Ok(Num::F(0.5 * (1.0 + erf(z))))
        }
        "binom.pdf" => {
            let (k, n, p) = (vals[0], vals[1], vals[2]);
            Ok(Num::F(binom_pmf(k as i64, n as i64, p)))
        }
        "binom.cdf" => {
            let (k, n, p) = (vals[0], vals[1], vals[2]);
            let mut s = 0.0;
            for i in 0..=(k as i64) {
                s += binom_pmf(i, n as i64, p);
            }
            Ok(Num::F(s))
        }
        "poisson.pdf" => {
            let (k, lambda) = (vals[0], vals[1]);
            Ok(Num::F(poisson_pmf(k as i64, lambda)))
        }
        "poisson.cdf" => {
            let (k, lambda) = (vals[0], vals[1]);
            let mut s = 0.0;
            for i in 0..=(k as i64) {
                s += poisson_pmf(i, lambda);
            }
            Ok(Num::F(s))
        }
        _ => Err(format!("unknown function {name}")),
    }
}

fn binom_pmf(k: i64, n: i64, p: f64) -> f64 {
    if k < 0 || k > n {
        return 0.0;
    }
    let log_coeff =
        ln_gamma(n as f64 + 1.0) - ln_gamma(k as f64 + 1.0) - ln_gamma((n - k) as f64 + 1.0);
    (log_coeff + (k as f64) * p.ln() + ((n - k) as f64) * (1.0 - p).ln()).exp()
}

fn poisson_pmf(k: i64, lambda: f64) -> f64 {
    if k < 0 {
        return 0.0;
    }
    ((k as f64) * lambda.ln() - lambda - ln_gamma(k as f64 + 1.0)).exp()
}

/// Parse+evaluate the whole token slice as a math expression. Returns `None`
/// if it doesn't parse as one production consuming every token (per the
/// detection rule in spec §4) or if it is a single bare numeric literal with
/// no operator/unit/keyword (also a detection guard).
fn parse_math_expr(toks: &[Token]) -> Option<Result<Num, String>> {
    parse_math_expr_full(toks).map(|(r, _, _)| r)
}

/// As `parse_math_expr`, but also reports the target unit abbreviation when
/// the *whole* expression was exactly one unit conversion (`N unit "in" unit`)
/// — lets `evaluate` render e.g. `72 h` / `30.48 cm` with the unit suffix.
/// `(result, unit tag, note)` — see `MParser2::unit_tag` / `note`.
type MathParse = (Result<Num, String>, Option<&'static str>, Option<String>);

fn parse_math_expr_full(toks: &[Token]) -> Option<MathParse> {
    let mut p = MParser2::new(toks);
    if let Some(r) = p.parse_is_what_percent() {
        if p.pos == toks.len() {
            return Some((r, None, None));
        }
    }
    p = MParser2::new(toks);
    let r = p.parse_add()?;
    if p.pos != toks.len() {
        return None;
    }
    Some((r, p.unit_tag, p.note))
}

// ---------------------------------------------------------------------------
// The parser above referenced `self.pending_unit` / `self.percent_flag`
// which aren't declared on the tuple-based `MParser`. To keep the recursive
// descent readable while still tracking those two bits of side state,
// `MParser2` wraps the same logic with the extra fields.
// ---------------------------------------------------------------------------

struct MParser2<'a> {
    t: &'a [Token],
    pos: usize,
    pending_unit: Option<UnitDef>,
    percent_flag: bool,
    /// Set when the most recently completed sub-expression was exactly a unit
    /// conversion (`N unit "in" unit`) — cleared by any combining operator, so
    /// it only survives to the top level when the *whole* expression was one
    /// conversion, letting `evaluate` render the target unit's abbreviation.
    unit_tag: Option<&'static str>,
    /// Extra explanation for the answer (e.g. the density a volume↔mass used).
    note: Option<String>,
}

impl<'a> MParser2<'a> {
    fn new(t: &'a [Token]) -> Self {
        MParser2 {
            t,
            pos: 0,
            pending_unit: None,
            percent_flag: false,
            unit_tag: None,
            note: None,
        }
    }
    fn kind_at(&self, i: usize) -> Option<&TokKind> {
        self.t.get(i).map(|tok| &tok.kind)
    }
    fn space_at(&self, i: usize) -> bool {
        self.t.get(i).is_some_and(|tok| tok.space_before)
    }
    /// Consume a unit at the cursor: a slash form written without spaces
    /// (`km/h`, `m/s`) when that compound is a known unit, else a single word.
    /// `10 km / 2` (spaced) stays a division.
    fn eat_unit(&mut self) -> Option<UnitDef> {
        let Some(TokKind::Word(a)) = self.peek().cloned() else {
            return None;
        };
        if let (Some(TokKind::Slash), Some(TokKind::Word(b))) =
            (self.kind_at(self.pos + 1), self.kind_at(self.pos + 2))
        {
            if !self.space_at(self.pos + 1) && !self.space_at(self.pos + 2) {
                if let Some(u) = unit_from_word(&format!("{a}/{b}")) {
                    self.pos += 3;
                    return Some(u);
                }
            }
        }
        let u = unit_from_word(&a)?;
        self.pos += 1;
        Some(u)
    }
    fn peek(&self) -> Option<&TokKind> {
        self.t.get(self.pos).map(|tok| &tok.kind)
    }
    fn peek_space(&self) -> bool {
        self.t
            .get(self.pos)
            .map(|tok| tok.space_before)
            .unwrap_or(true)
    }
    fn eat_word(&mut self, w: &str) -> bool {
        if let Some(TokKind::Word(s)) = self.peek() {
            if s == w {
                self.pos += 1;
                return true;
            }
        }
        false
    }

    fn parse_is_what_percent(&mut self) -> Option<Result<Num, String>> {
        let a = self.parse_add()?;
        if self.eat_word("is")
            && self.eat_word("what")
            && matches!(self.peek(), Some(TokKind::Percent))
        {
            self.pos += 1;
            if self.eat_word("of") {
                let b = self.parse_add()?;
                return Some(match (a, b) {
                    (Ok(a), Ok(b)) => Num::div(a, b).map(|r| Num::F(r.to_f64() * 100.0)),
                    (Err(e), _) | (_, Err(e)) => Err(e),
                });
            }
        }
        None
    }

    fn parse_add(&mut self) -> Option<Result<Num, String>> {
        let mut acc = self.parse_mul()?;
        while matches!(self.peek(), Some(TokKind::Plus) | Some(TokKind::Minus)) {
            let neg = matches!(self.peek(), Some(TokKind::Minus));
            if let Some(TokKind::Num(n)) = self.t.get(self.pos + 1).map(|t| &t.kind) {
                let n = *n;
                let is_bare_pct = matches!(
                    self.t.get(self.pos + 2).map(|t| &t.kind),
                    Some(TokKind::Percent)
                ) && !matches!(
                    self.t.get(self.pos + 3).map(|t| &t.kind),
                    Some(TokKind::Star)
                        | Some(TokKind::Slash)
                        | Some(TokKind::Caret)
                        | Some(TokKind::Bang)
                        | Some(TokKind::Percent)
                ) && !matches!(
                    self.t.get(self.pos + 3).map(|t| &t.kind),
                    Some(TokKind::Word(w)) if w == "of"
                );
                if is_bare_pct {
                    self.pos += 3;
                    acc = acc.and_then(|a| percent_add(a, n, neg));
                    self.unit_tag = None;
                    continue;
                }
            }
            self.pos += 1;
            self.unit_tag = None;
            let rhs = self.parse_mul()?;
            acc = match (acc, rhs) {
                (Ok(a), Ok(b)) => {
                    if neg {
                        Num::sub(a, b)
                    } else {
                        Num::add(a, b)
                    }
                }
                (Err(e), _) | (_, Err(e)) => Err(e),
            };
        }
        Some(acc)
    }

    fn parse_mul(&mut self) -> Option<Result<Num, String>> {
        let mut acc = self.parse_unit_convert()?;
        loop {
            match self.peek() {
                Some(TokKind::Star) => {
                    self.pos += 1;
                    self.unit_tag = None;
                    let rhs = self.parse_unit_convert()?;
                    acc = combine(acc, rhs, Num::mul);
                }
                Some(TokKind::Slash) => {
                    self.pos += 1;
                    self.unit_tag = None;
                    let rhs = self.parse_unit_convert()?;
                    acc = combine(acc, rhs, Num::div);
                }
                Some(TokKind::Word(w))
                    if !self.peek_space()
                        && (w == "pi"
                            || w == "\u{3c0}"
                            || w == "e"
                            || w == "tau"
                            || w == "phi"
                            || is_func_name(w)) =>
                {
                    self.unit_tag = None;
                    let rhs = self.parse_unit_convert()?;
                    acc = combine(acc, rhs, Num::mul);
                }
                Some(TokKind::LParen) if !self.peek_space() => {
                    self.unit_tag = None;
                    let rhs = self.parse_unit_convert()?;
                    acc = combine(acc, rhs, Num::mul);
                }
                _ => break,
            }
        }
        Some(acc)
    }

    fn parse_unit_convert(&mut self) -> Option<Result<Num, String>> {
        self.pending_unit = None;
        self.unit_tag = None;
        let val = self.parse_percent_of()?;
        if let Some(fu) = self.pending_unit.take() {
            // Optional substance for volume↔mass: `1 cup [of] flour in g`.
            let save = self.pos;
            let of = self.eat_word("of");
            let substance = match self.peek() {
                Some(TokKind::Word(w)) => density_of(w),
                _ => None,
            };
            match substance {
                Some(_) => self.pos += 1,
                None if of => self.pos = save,
                None => {}
            }
            if self.eat_word("in") || self.eat_word("to") {
                let tu = self.eat_unit()?;
                self.unit_tag = Some(tu.name);
                if is_volume_mass(fu.domain, tu.domain) {
                    let Some((name, d)) = substance else {
                        return Some(Err(
                            "volume ↔ mass needs a substance, e.g. \"1 cup flour in g\""
                                .to_string(),
                        ));
                    };
                    self.note = Some(format!("{name} ≈ {} g/ml", d.to_f64()));
                    return Some(val.and_then(|v| convert_with_density(v, fu, tu, d)));
                }
                return Some(val.and_then(|v| convert_units(v, fu, tu)));
            }
            if substance.is_some() {
                return None; // a substance without a target isn't math
            }
        }
        Some(val)
    }

    fn parse_percent_of(&mut self) -> Option<Result<Num, String>> {
        self.percent_flag = false;
        let val = self.parse_pow()?;
        if self.percent_flag && self.eat_word("of") {
            let rhs = self.parse_pow()?;
            return Some(match (val, rhs) {
                (Ok(a), Ok(b)) => Num::mul(b, a),
                (Err(e), _) | (_, Err(e)) => Err(e),
            });
        }
        Some(val)
    }

    fn parse_pow(&mut self) -> Option<Result<Num, String>> {
        let base = self.parse_unary()?;
        if matches!(self.peek(), Some(TokKind::Caret)) {
            self.pos += 1;
            self.pending_unit = None;
            let exp = self.parse_unary()?;
            return Some(combine(base, exp, Num::pow));
        }
        Some(base)
    }

    fn parse_unary(&mut self) -> Option<Result<Num, String>> {
        match self.peek() {
            Some(TokKind::Minus) => {
                self.pos += 1;
                let v = self.parse_unary()?;
                Some(v.map(Num::neg))
            }
            Some(TokKind::Plus) => {
                self.pos += 1;
                self.parse_unary()
            }
            _ => self.parse_postfix(),
        }
    }

    fn parse_postfix(&mut self) -> Option<Result<Num, String>> {
        let mut v = self.parse_primary()?;
        loop {
            match self.peek() {
                Some(TokKind::Bang) => {
                    self.pos += 1;
                    let double = matches!(self.peek(), Some(TokKind::Bang)) && !self.peek_space();
                    if double {
                        self.pos += 1;
                        v = v.and_then(double_factorial);
                    } else {
                        v = v.and_then(factorial);
                    }
                    self.pending_unit = None;
                }
                Some(TokKind::Percent) => {
                    self.pos += 1;
                    v = v.map(|n| Num::mul(n, Num::R(Rat::new(1, 100).unwrap())).unwrap_or(n));
                    self.percent_flag = true;
                    self.pending_unit = None;
                }
                _ => break,
            }
        }
        Some(v)
    }

    fn parse_primary(&mut self) -> Option<Result<Num, String>> {
        match self.peek().cloned() {
            Some(TokKind::Num(r)) => {
                self.pos += 1;
                if matches!(self.peek(), Some(TokKind::Word(w)) if w == "deg") && !self.peek_space()
                {
                    self.pos += 1;
                    let rad = r.to_f64() * std::f64::consts::PI / 180.0;
                    return Some(Ok(Num::F(rad)));
                }
                if let Some(TokKind::Word(w)) = self.peek().cloned() {
                    if !is_func_name(&w) && !matches!(w.as_str(), "pi" | "e" | "tau" | "phi") {
                        if let Some(u) = self.eat_unit() {
                            self.pending_unit = Some(u);
                            return Some(Ok(Num::R(r)));
                        }
                    }
                }
                Some(Ok(Num::R(r)))
            }
            Some(TokKind::LParen) => {
                self.pos += 1;
                let inner = self.parse_add()?;
                if !matches!(self.peek(), Some(TokKind::RParen)) {
                    return Some(Err("unbalanced parentheses".to_string()));
                }
                self.pos += 1;
                Some(inner)
            }
            Some(TokKind::Word(w)) => match w.as_str() {
                "pi" | "\u{3c0}" => {
                    self.pos += 1;
                    Some(Ok(Num::F(std::f64::consts::PI)))
                }
                "e" => {
                    self.pos += 1;
                    Some(Ok(Num::F(std::f64::consts::E)))
                }
                "tau" => {
                    self.pos += 1;
                    Some(Ok(Num::F(std::f64::consts::TAU)))
                }
                "phi" => {
                    self.pos += 1;
                    Some(Ok(Num::F((1.0 + 5f64.sqrt()) / 2.0)))
                }
                _ if is_func_name(&w) => {
                    self.pos += 1;
                    let args = self.parse_call_args()?;
                    Some(eval_func(&w, args))
                }
                _ => None,
            },
            _ => None,
        }
    }

    fn parse_call_args(&mut self) -> Option<Vec<Result<Num, String>>> {
        if !matches!(self.peek(), Some(TokKind::LParen)) {
            return None;
        }
        self.pos += 1;
        let mut args = Vec::new();
        if matches!(self.peek(), Some(TokKind::RParen)) {
            self.pos += 1;
            return Some(args);
        }
        loop {
            let v = self.parse_add()?;
            args.push(v);
            match self.peek() {
                Some(TokKind::Comma) | Some(TokKind::Semicolon) => {
                    self.pos += 1;
                }
                Some(TokKind::RParen) => {
                    self.pos += 1;
                    break;
                }
                _ => return None,
            }
        }
        Some(args)
    }
}

fn percent_add(a: Num, n: Rat, neg: bool) -> Result<Num, String> {
    let den = n.den.checked_mul(100).ok_or("overflow")?;
    let pct = Rat::new(n.num, den).ok_or("overflow")?;
    let factor = if neg {
        Num::sub(Num::R(Rat::int(1)), Num::R(pct))?
    } else {
        Num::add(Num::R(Rat::int(1)), Num::R(pct))?
    };
    Num::mul(a, factor)
}

fn combine(
    a: Result<Num, String>,
    b: Result<Num, String>,
    f: impl Fn(Num, Num) -> Result<Num, String>,
) -> Result<Num, String> {
    match (a, b) {
        (Ok(a), Ok(b)) => f(a, b),
        (Err(e), _) | (_, Err(e)) => Err(e),
    }
}

// ---------------------------------------------------------------------------
// Date grammar — anchor pipeline (spec §1.3).
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
struct Anchor {
    day: i64,
    secs: i32,
    has_time: bool,
}

enum TermOutcome {
    Point(Anchor),
    Range(i64, i64, String),
}

enum DateResult {
    Point(Anchor),
    Range(i64, i64, String),
    /// A countdown to a specific date: `days` is signed (+future / −past), and
    /// `target` is the day so the card can draw a calendar. `weeks` picks the
    /// headline unit. (Supersedes a bare duration — "days until X" now carries the
    /// target so the answer card can render its calendar.)
    Countdown {
        target: i64,
        days: i64,
        weeks: bool,
    },
}

struct DParser<'a> {
    t: &'a [Token],
    pos: usize,
}

impl<'a> DParser<'a> {
    fn peek(&self) -> Option<&TokKind> {
        self.t.get(self.pos).map(|tok| &tok.kind)
    }
    fn word(&self) -> Option<&str> {
        match self.peek() {
            Some(TokKind::Word(w)) => Some(w.as_str()),
            _ => None,
        }
    }
    fn eat_word(&mut self, w: &str) -> bool {
        if self.word() == Some(w) {
            self.pos += 1;
            true
        } else {
            false
        }
    }
    /// Consume a computed-holiday name (multi-word aware) and return its canonical
    /// key: `easter`, `black friday`, or `new year`. (Christmas is handled inline.)
    fn eat_holiday(&mut self) -> Option<String> {
        if self.eat_word("easter") {
            return Some("easter".into());
        }
        if self.word() == Some("black") {
            if let Some(TokKind::Word(w)) = self.t.get(self.pos + 1).map(|t| &t.kind) {
                if w == "friday" {
                    self.pos += 2;
                    return Some("black friday".into());
                }
            }
        }
        if self.word() == Some("new") {
            if let Some(TokKind::Word(w)) = self.t.get(self.pos + 1).map(|t| &t.kind) {
                if w == "year" || w == "years" {
                    self.pos += 2;
                    return Some("new year".into());
                }
            }
        }
        None
    }
    /// Parse a NUMBER count, or a `(` math-expr `)` count (paren-expr, spec §1.2).
    fn parse_count(&mut self) -> Option<i64> {
        match self.peek().cloned() {
            Some(TokKind::Num(r)) => {
                self.pos += 1;
                Some(r.to_f64().round() as i64)
            }
            Some(TokKind::Ordinal(n)) => {
                self.pos += 1;
                Some(n as i64)
            }
            Some(TokKind::LParen) => {
                let start = self.pos + 1;
                let mut depth = 1i32;
                let mut j = start;
                while j < self.t.len() && depth > 0 {
                    match self.t[j].kind {
                        TokKind::LParen => depth += 1,
                        TokKind::RParen => depth -= 1,
                        _ => {}
                    }
                    if depth == 0 {
                        break;
                    }
                    j += 1;
                }
                if depth != 0 {
                    return None;
                }
                let sub = &self.t[start..j];
                let val = parse_math_expr(sub)?.ok()?;
                self.pos = j + 1;
                Some(val.to_f64().round() as i64)
            }
            _ => None,
        }
    }

    fn parse_ord_or_num(&mut self) -> Option<u32> {
        match self.peek().cloned() {
            Some(TokKind::Ordinal(n)) => {
                self.pos += 1;
                Some(n)
            }
            Some(TokKind::Num(r)) if r.is_integer() => {
                self.pos += 1;
                Some(r.num as u32)
            }
            Some(TokKind::Word(w)) => ordinal_word(&w).inspect(|_| {
                self.pos += 1;
            }),
            _ => None,
        }
    }
}

fn apply_time_offset(anchor: Anchor, unit: UnitTime, n: i64) -> Anchor {
    match unit {
        // Day/week/month/year offsets name a *day*, not a moment — spec's "in 3
        // days" / "3 days ago" rows resolve from `now` but display date-only
        // ("today + 3 days"), so these always drop any time-of-day.
        UnitTime::Day => Anchor {
            day: anchor.day + n,
            secs: 0,
            has_time: false,
        },
        UnitTime::Week => Anchor {
            day: anchor.day + n * 7,
            secs: 0,
            has_time: false,
        },
        UnitTime::Hour | UnitTime::Minute => {
            let delta = if unit == UnitTime::Hour {
                n * 3600
            } else {
                n * 60
            };
            let total = anchor.day * 86_400 + anchor.secs as i64 + delta;
            Anchor {
                day: total.div_euclid(86_400),
                secs: total.rem_euclid(86_400) as i32,
                has_time: true,
            }
        }
        UnitTime::Month => {
            let (y, m, d) = civil_from_days(anchor.day);
            let total = (m as i64 - 1) + n;
            let ny = y + total.div_euclid(12);
            let nm = total.rem_euclid(12) + 1;
            let nd = (d as i64).min(days_in_month(ny, nm));
            Anchor {
                day: days_from_civil(ny, nm, nd),
                secs: 0,
                has_time: false,
            }
        }
        UnitTime::Year => {
            let (y, m, d) = civil_from_days(anchor.day);
            let ny = y + n;
            let nd = (d as i64).min(days_in_month(ny, m as i64));
            Anchor {
                day: days_from_civil(ny, m as i64, nd),
                secs: 0,
                has_time: false,
            }
        }
    }
}

fn nth_weekday_strictly_after(anchor_day: i64, target_wd: i64, n: i64, forward: bool) -> i64 {
    let mut count = 0;
    let mut day = if forward {
        anchor_day + 1
    } else {
        anchor_day - 1
    };
    loop {
        if weekday(day) == target_wd {
            count += 1;
            if count == n {
                return day;
            }
        }
        day += if forward { 1 } else { -1 };
    }
}

fn nth_weekday_of_month(y: i64, m: i64, target_wd: i64, n: i64) -> i64 {
    let first = days_from_civil(y, m, 1);
    let mut count = 0;
    let mut day = first;
    loop {
        if weekday(day) == target_wd {
            count += 1;
            if count == n {
                return day;
            }
        }
        day += 1;
    }
}

fn nth_weekday_of_year(y: i64, target_wd: i64, n: i64) -> i64 {
    let jan1 = days_from_civil(y, 1, 1);
    let mut count = 0;
    let mut day = jan1;
    loop {
        if weekday(day) == target_wd {
            count += 1;
            if count == n {
                return day;
            }
        }
        day += 1;
    }
}

/// The next occurrence of a computed holiday on or after `from_day`. `name` is a
/// key from `DParser::eat_holiday` (`easter` / `black friday` / `new year`).
fn holiday_next_day(name: &str, from_day: i64) -> i64 {
    let (mut y, _, _) = civil_from_days(from_day);
    // Try this year then next — every one of these recurs annually.
    for _ in 0..2 {
        let (m, d) = match name {
            "easter" => crate::holidays::easter(y),
            "black friday" => crate::holidays::black_friday(y),
            "new year" => (1, 1),
            _ => (1, 1),
        };
        let day = days_from_civil(y, m as i64, d as i64);
        if day >= from_day {
            return day;
        }
        y += 1;
    }
    days_from_civil(y, 1, 1)
}

fn parse_one_term(p: &mut DParser, now_anchor: Anchor, anchor: Anchor) -> Option<TermOutcome> {
    // absolute keywords
    if p.eat_word("now") {
        return Some(TermOutcome::Point(now_anchor));
    }
    if p.eat_word("today") {
        return Some(TermOutcome::Point(Anchor {
            day: now_anchor.day,
            secs: 0,
            has_time: false,
        }));
    }
    if p.eat_word("tomorrow") {
        return Some(TermOutcome::Point(Anchor {
            day: now_anchor.day + 1,
            secs: 0,
            has_time: false,
        }));
    }
    if p.eat_word("yesterday") {
        return Some(TermOutcome::Point(Anchor {
            day: now_anchor.day - 1,
            secs: 0,
            has_time: false,
        }));
    }
    if p.eat_word("christmas") {
        let (y, _, _) = civil_from_days(now_anchor.day);
        return Some(TermOutcome::Point(Anchor {
            day: days_from_civil(y, 12, 25),
            secs: 0,
            has_time: false,
        }));
    }
    // Holidays with a computed date → the next occurrence on/after today
    // (easter, black friday, new year). An optional leading `next` is consumed by
    // the directional handler, which also lands here.
    if let Some(name) = p.eat_holiday() {
        return Some(TermOutcome::Point(Anchor {
            day: holiday_next_day(&name, now_anchor.day),
            secs: 0,
            has_time: false,
        }));
    }
    if let Some(TokKind::Iso(y, m, d, time)) = p.peek().cloned() {
        p.pos += 1;
        let day = days_from_civil(y, m as i64, d as i64);
        return Some(TermOutcome::Point(match time {
            Some((hh, mm)) => Anchor {
                day,
                secs: (hh * 3600 + mm * 60) as i32,
                has_time: true,
            },
            None => Anchor {
                day,
                secs: 0,
                has_time: false,
            },
        }));
    }

    // boundary: (end|start) of (month|year|week)
    if p.word() == Some("end") || p.word() == Some("start") {
        let is_end = p.word() == Some("end");
        let save = p.pos;
        p.pos += 1;
        if p.eat_word("of") {
            let (y, m, _) = civil_from_days(anchor.day);
            if p.eat_word("month") {
                let day = if is_end {
                    days_from_civil(y, m as i64, days_in_month(y, m as i64))
                } else {
                    days_from_civil(y, m as i64, 1)
                };
                return Some(TermOutcome::Point(Anchor {
                    day,
                    secs: 0,
                    has_time: false,
                }));
            }
            if p.eat_word("year") {
                let day = if is_end {
                    days_from_civil(y, 12, 31)
                } else {
                    days_from_civil(y, 1, 1)
                };
                return Some(TermOutcome::Point(Anchor {
                    day,
                    secs: 0,
                    has_time: false,
                }));
            }
            if p.eat_word("week") {
                let wd_mon0 = (weekday(anchor.day) + 6) % 7;
                let monday = anchor.day - wd_mon0;
                let day = if is_end { monday + 6 } else { monday };
                return Some(TermOutcome::Point(Anchor {
                    day,
                    secs: 0,
                    has_time: false,
                }));
            }
        }
        p.pos = save;
    }

    // directional / range: (next|last|this) X
    if matches!(
        p.word(),
        Some("next") | Some("last") | Some("this") | Some("current")
    ) {
        let dir = p.word().unwrap().to_string();
        let save = p.pos;
        p.pos += 1;
        // "next/this <holiday>" (easter, black friday, new year).
        {
            let hsave = p.pos;
            if let Some(name) = p.eat_holiday() {
                let from = if dir == "next" {
                    now_anchor.day + 1
                } else {
                    // "this"/"current": this calendar year's occurrence.
                    days_from_civil(civil_from_days(now_anchor.day).0, 1, 1)
                };
                if dir != "last" {
                    return Some(TermOutcome::Point(Anchor {
                        day: holiday_next_day(&name, from),
                        secs: 0,
                        has_time: false,
                    }));
                }
            }
            p.pos = hsave;
        }
        if let Some(w) = p.word() {
            if let Some(target_wd) = weekday_from_word(w) {
                p.pos += 1;
                let cur_wd = weekday(anchor.day);
                let day = match dir.as_str() {
                    "next" => {
                        let mut delta = (target_wd - cur_wd).rem_euclid(7);
                        if delta == 0 {
                            delta = 7;
                        }
                        anchor.day + delta
                    }
                    "last" => {
                        let mut delta = (cur_wd - target_wd).rem_euclid(7);
                        if delta == 0 {
                            delta = 7;
                        }
                        anchor.day - delta
                    }
                    _ => {
                        // "this" — occurrence in the current week (possibly today)
                        let delta = (target_wd - cur_wd).rem_euclid(7);
                        anchor.day + delta
                    }
                };
                return Some(TermOutcome::Point(Anchor {
                    day,
                    secs: 0,
                    has_time: false,
                }));
            }
            if matches!(w, "month" | "year" | "week") {
                let unit = w.to_string();
                p.pos += 1;
                let (y, m, _) = civil_from_days(anchor.day);
                let (start, end, label) = match unit.as_str() {
                    "month" => {
                        let total = (m as i64 - 1)
                            + match dir.as_str() {
                                "next" => 1,
                                "last" => -1,
                                _ => 0,
                            };
                        let ny = y + total.div_euclid(12);
                        let nm = total.rem_euclid(12) + 1;
                        (
                            days_from_civil(ny, nm, 1),
                            days_from_civil(ny, nm, days_in_month(ny, nm)),
                            format!("{} {}", month_name(nm as u32, true), ny),
                        )
                    }
                    "year" => {
                        let ny = y + match dir.as_str() {
                            "next" => 1,
                            "last" => -1,
                            _ => 0,
                        };
                        (
                            days_from_civil(ny, 1, 1),
                            days_from_civil(ny, 12, 31),
                            format!("{ny}"),
                        )
                    }
                    _ => {
                        let wd_mon0 = (weekday(anchor.day) + 6) % 7;
                        let monday = anchor.day
                            + match dir.as_str() {
                                "next" => 7 - wd_mon0,
                                "last" => -7 - wd_mon0,
                                _ => -wd_mon0,
                            };
                        (monday, monday + 6, format!("week {}", iso_week(monday)))
                    }
                };
                return Some(TermOutcome::Range(start, end, label));
            }
        }
        p.pos = save;
    }

    // nth_occurrence: (ORD|NUM) WEEKDAY ["in" (MONTH [NUM] | NUM)]
    {
        let save = p.pos;
        if let Some(n) = p.parse_ord_or_num() {
            if let Some(w) = p.word() {
                if let Some(target_wd) = weekday_from_word(w) {
                    p.pos += 1;
                    if p.eat_word("in") {
                        if let Some(mw) = p.word() {
                            if let Some(mo) = month_from_word(mw) {
                                p.pos += 1;
                                let year = if let Some(TokKind::Num(r)) = p.peek() {
                                    if r.is_integer() {
                                        let y = r.num as i64;
                                        p.pos += 1;
                                        y
                                    } else {
                                        civil_from_days(now_anchor.day).0
                                    }
                                } else {
                                    civil_from_days(now_anchor.day).0
                                };
                                let day =
                                    nth_weekday_of_month(year, mo as i64, target_wd, n as i64);
                                return Some(TermOutcome::Point(Anchor {
                                    day,
                                    secs: 0,
                                    has_time: false,
                                }));
                            }
                        }
                        if let Some(TokKind::Num(r)) = p.peek() {
                            if r.is_integer() {
                                let y = r.num as i64;
                                p.pos += 1;
                                let day = nth_weekday_of_year(y, target_wd, n as i64);
                                return Some(TermOutcome::Point(Anchor {
                                    day,
                                    secs: 0,
                                    has_time: false,
                                }));
                            }
                        }
                        p.pos = save;
                    } else {
                        // re-anchor form: "1 thursday", "4 thursdays"
                        let day = nth_weekday_strictly_after(anchor.day, target_wd, n as i64, true);
                        return Some(TermOutcome::Point(Anchor {
                            day,
                            secs: 0,
                            has_time: false,
                        }));
                    }
                }
            }
        }
        p.pos = save;
    }

    // offset: SIGN NUMBER UNIT (NUMBER UNIT)*  |  "in" NUMBER UNIT  |  NUMBER UNIT "ago"
    {
        let save = p.pos;
        let sign: i64 = if matches!(p.peek(), Some(TokKind::Plus)) {
            p.pos += 1;
            1
        } else if matches!(p.peek(), Some(TokKind::Minus)) {
            p.pos += 1;
            -1
        } else {
            0
        };
        if sign != 0 {
            if let Some(n) = p.parse_count() {
                if let Some(u) = p.word().and_then(time_unit_from_word) {
                    p.pos += 1;
                    let mut a = apply_time_offset(anchor, u, sign * n);
                    loop {
                        let inner_save = p.pos;
                        if let Some(n2) = p.parse_count() {
                            if let Some(u2) = p.word().and_then(time_unit_from_word) {
                                p.pos += 1;
                                a = apply_time_offset(a, u2, sign * n2);
                                continue;
                            }
                        }
                        p.pos = inner_save;
                        break;
                    }
                    return Some(TermOutcome::Point(a));
                }
                // SIGN NUMBER WEEKDAY(S) — nth-occurrence re-anchor form.
                if let Some(w) = p.word() {
                    if let Some(target_wd) = weekday_from_word(w) {
                        p.pos += 1;
                        let day = nth_weekday_strictly_after(anchor.day, target_wd, n, sign > 0);
                        return Some(TermOutcome::Point(Anchor {
                            day,
                            secs: 0,
                            has_time: false,
                        }));
                    }
                }
            }
        }
        p.pos = save;
    }
    if p.eat_word("in") {
        if let Some(n) = p.parse_count() {
            if let Some(u) = p.word().and_then(time_unit_from_word) {
                p.pos += 1;
                return Some(TermOutcome::Point(apply_time_offset(anchor, u, n)));
            }
        }
        p.pos -= 1; // un-consume "in" — not this production
    }
    {
        let save = p.pos;
        if let Some(n) = p.parse_count() {
            if let Some(u) = p.word().and_then(time_unit_from_word) {
                p.pos += 1;
                if p.eat_word("ago") {
                    return Some(TermOutcome::Point(apply_time_offset(anchor, u, -n)));
                }
            }
        }
        p.pos = save;
    }

    // month_day: (ORD|NUM) MONTH [NUM]  |  MONTH (ORD|NUM) [NUM]
    {
        let save = p.pos;
        if let Some(d) = p.parse_ord_or_num() {
            if let Some(w) = p.word() {
                if let Some(mo) = month_from_word(w) {
                    p.pos += 1;
                    let year = if let Some(TokKind::Num(r)) = p.peek() {
                        if r.is_integer() {
                            let y = r.num as i64;
                            p.pos += 1;
                            y
                        } else {
                            civil_from_days(now_anchor.day).0
                        }
                    } else {
                        civil_from_days(now_anchor.day).0
                    };
                    return Some(TermOutcome::Point(Anchor {
                        day: days_from_civil(year, mo as i64, d as i64),
                        secs: 0,
                        has_time: false,
                    }));
                }
            }
        }
        p.pos = save;
        if let Some(w) = p.word() {
            if let Some(mo) = month_from_word(w) {
                p.pos += 1;
                if let Some(d) = p.parse_ord_or_num() {
                    let year = if let Some(TokKind::Num(r)) = p.peek() {
                        if r.is_integer() {
                            let y = r.num as i64;
                            p.pos += 1;
                            y
                        } else {
                            civil_from_days(now_anchor.day).0
                        }
                    } else {
                        civil_from_days(now_anchor.day).0
                    };
                    return Some(TermOutcome::Point(Anchor {
                        day: days_from_civil(year, mo as i64, d as i64),
                        secs: 0,
                        has_time: false,
                    }));
                }
                p.pos = save;
            }
        }
        p.pos = save;
    }

    // EPOCH: bare 10-digit integer.
    if let Some(TokKind::Num(r)) = p.peek().cloned() {
        if r.is_integer() && (1_000_000_000..10_000_000_000).contains(&r.num) {
            p.pos += 1;
            let secs = r.num as i64;
            return Some(TermOutcome::Point(Anchor {
                day: secs.div_euclid(86_400),
                secs: secs.rem_euclid(86_400) as i32,
                has_time: true,
            }));
        }
    }

    None
}

fn parse_date_expr(toks: &[Token], now_anchor: Anchor) -> Option<DateResult> {
    // query: ("days"|"weeks") ("until"|"since"|"til"|"till"|"to") term
    // Carries the TARGET date (not just the count) so the answer card can draw the
    // calendar and offer a region/holiday label — a date-countdown, not a bare
    // duration.
    if toks.len() >= 3 {
        if let TokKind::Word(w0) = &toks[0].kind {
            let weeks = w0 == "weeks";
            if w0 == "days" || weeks {
                if let TokKind::Word(w1) = &toks[1].kind {
                    if matches!(w1.as_str(), "until" | "since" | "til" | "till" | "to") {
                        let mut p = DParser { t: toks, pos: 2 };
                        let outcome = parse_one_term(&mut p, now_anchor, now_anchor)?;
                        if p.pos != toks.len() {
                            return None;
                        }
                        if let TermOutcome::Point(a) = outcome {
                            let signed = if w1 == "since" {
                                now_anchor.day - a.day
                            } else {
                                a.day - now_anchor.day
                            };
                            return Some(DateResult::Countdown {
                                target: a.day,
                                days: signed,
                                weeks,
                            });
                        }
                        return None;
                    }
                }
            }
        }
    }

    let mut p = DParser { t: toks, pos: 0 };
    let mut anchor = now_anchor;
    let mut range: Option<(i64, i64, String)> = None;
    if p.t.is_empty() {
        return None;
    }
    loop {
        if p.pos >= p.t.len() {
            break;
        }
        let cur = if let Some((s, _, _)) = &range {
            Anchor {
                day: *s,
                secs: 0,
                has_time: false,
            }
        } else {
            anchor
        };
        match parse_one_term(&mut p, now_anchor, cur)? {
            TermOutcome::Point(a) => {
                anchor = a;
                range = None;
            }
            TermOutcome::Range(s, e, l) => {
                range = Some((s, e, l));
            }
        }
    }
    if p.pos != p.t.len() {
        return None;
    }
    if let Some((s, e, l)) = range {
        return Some(DateResult::Range(s, e, l));
    }
    Some(DateResult::Point(anchor))
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// The evaluator's injected clock — never read from the OS inside this module
/// (see spec §5). `epoch_secs` is UTC; `utc_offset_secs` is the caller-resolved
/// local offset, applied uniformly (this hand-rolled evaluator does not model
/// DST transition rules for dates other than `now` itself — see module docs
/// in the design spec's Architecture section for why that's out of scope here).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReferenceNow {
    pub epoch_secs: i64,
    pub utc_offset_secs: i32,
}

impl ReferenceNow {
    fn to_anchor(self) -> Anchor {
        let local_total = self.epoch_secs + self.utc_offset_secs as i64;
        Anchor {
            day: local_total.div_euclid(86_400),
            secs: local_total.rem_euclid(86_400) as i32,
            has_time: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum AnswerKind {
    Date,
    DateTime,
    DateRange,
    Duration,
    Number,
    Rational,
    Percent,
    Unit,
    Error,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AnswerValue {
    Date {
        day_epoch: i64,
    },
    DateTime {
        day_epoch: i64,
        secs_of_day: i32,
    },
    DateRange {
        start_day_epoch: i64,
        end_day_epoch: i64,
        label: String,
    },
    Duration {
        days: i64,
        secs: i64,
    },
    Rational {
        num: i128,
        den: i128,
    },
    Float {
        value: f64,
    },
    Percent {
        value: f64,
    },
    Error {
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct FormatChip {
    pub label: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Answer {
    pub kind: AnswerKind,
    pub value: AnswerValue,
    pub display: String,
    pub explanation: String,
    pub formats: Vec<FormatChip>,
}

fn medium_date(day: i64) -> String {
    let (y, m, d) = civil_from_days(day);
    format!("{d} {} {y}", month_name(m, false))
}

fn full_date(day: i64) -> String {
    let (y, m, d) = civil_from_days(day);
    format!(
        "{}, {d} {} {y}",
        weekday_name(weekday(day), true),
        month_name(m, true)
    )
}

fn iso_date(day: i64) -> String {
    let (y, m, d) = civil_from_days(day);
    format!("{y:04}-{m:02}-{d:02}")
}

fn hhmm(secs: i32) -> String {
    format!("{:02}:{:02}", secs / 3600, (secs % 3600) / 60)
}

fn epoch_of(day: i64, secs: i32, utc_offset: i32) -> i64 {
    day * 86_400 + secs as i64 - utc_offset as i64
}

fn date_answer(day: i64, explanation: String, utc_offset: i32) -> Answer {
    let formats = vec![
        FormatChip {
            label: "ISO".into(),
            text: iso_date(day),
        },
        FormatChip {
            label: "Medium".into(),
            text: medium_date(day),
        },
        FormatChip {
            label: "Full".into(),
            text: full_date(day),
        },
        FormatChip {
            label: "Epoch".into(),
            text: epoch_of(day, 0, utc_offset).to_string(),
        },
        FormatChip {
            label: "Week".into(),
            text: format!("week {}", iso_week(day)),
        },
    ];
    Answer {
        kind: AnswerKind::Date,
        value: AnswerValue::Date { day_epoch: day },
        display: full_date(day),
        explanation,
        formats,
    }
}

/// A "days/weeks until X" answer: a **Date** answer (so the card draws a calendar
/// for `target`), but with a countdown headline (`91 days`) and a count chip. The
/// app enriches it with a holiday/region label from `magpie_core::holidays`.
fn countdown_answer(target: i64, days: i64, weeks: bool, now_day: i64, utc_offset: i32) -> Answer {
    let n = days.abs();
    let display = if weeks {
        let w = (n + 3) / 7; // nearest week
        format!("{w} week{}", if w == 1 { "" } else { "s" })
    } else {
        format!("{n} day{}", if n == 1 { "" } else { "s" })
    };
    let count = if weeks { (n + 3) / 7 } else { n };
    let explanation = format!(
        "today = {} \u{2192} {}",
        full_date(now_day),
        medium_date(target)
    );
    let formats = vec![
        FormatChip {
            label: "Count".into(),
            text: count.to_string(),
        },
        FormatChip {
            label: "ISO".into(),
            text: iso_date(target),
        },
        FormatChip {
            label: "Full".into(),
            text: full_date(target),
        },
        FormatChip {
            label: "Week".into(),
            text: format!("week {}", iso_week(target)),
        },
        FormatChip {
            label: "Epoch".into(),
            text: epoch_of(target, 0, utc_offset).to_string(),
        },
    ];
    Answer {
        kind: AnswerKind::Date,
        value: AnswerValue::Date { day_epoch: target },
        display,
        explanation,
        formats,
    }
}

fn datetime_answer(day: i64, secs: i32, explanation: String, utc_offset: i32) -> Answer {
    let mut formats = vec![
        FormatChip {
            label: "ISO".into(),
            text: iso_date(day),
        },
        FormatChip {
            label: "Medium".into(),
            text: medium_date(day),
        },
        FormatChip {
            label: "Full".into(),
            text: format!("{}, {}", full_date(day), hhmm(secs)),
        },
        FormatChip {
            label: "Epoch".into(),
            text: epoch_of(day, secs, utc_offset).to_string(),
        },
    ];
    formats.push(FormatChip {
        label: "Local time".into(),
        text: hhmm(secs),
    });
    Answer {
        kind: AnswerKind::DateTime,
        value: AnswerValue::DateTime {
            day_epoch: day,
            secs_of_day: secs,
        },
        display: format!("{}, {}", full_date(day), hhmm(secs)),
        explanation,
        formats,
    }
}

fn range_answer(start: i64, end: i64, label: String, explanation: String) -> Answer {
    Answer {
        kind: AnswerKind::DateRange,
        value: AnswerValue::DateRange {
            start_day_epoch: start,
            end_day_epoch: end,
            label: label.clone(),
        },
        display: label,
        explanation,
        formats: vec![
            FormatChip {
                label: "Start".into(),
                text: iso_date(start),
            },
            FormatChip {
                label: "End".into(),
                text: iso_date(end),
            },
        ],
    }
}

fn rat_decimal_display(r: Rat) -> String {
    let exact = 1_000_000i128.checked_rem(r.den) == Some(0);
    if exact {
        let scale = 1_000_000 / r.den;
        let scaled = r.num * scale;
        let neg = scaled < 0;
        let scaled = scaled.unsigned_abs();
        let whole = scaled / 1_000_000;
        let frac = scaled % 1_000_000;
        let mut s = format!("{whole}");
        if frac != 0 {
            let mut fs = format!("{frac:06}");
            while fs.ends_with('0') {
                fs.pop();
            }
            s.push('.');
            s.push_str(&fs);
        }
        if neg && (whole != 0 || frac != 0) {
            format!("-{s}")
        } else {
            s
        }
    } else {
        let f = r.to_f64();
        let mut s = format!("{f:.6}");
        while s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
        format!("\u{2248} {s}")
    }
}

fn format_int_scientific(n: i128, sig: usize) -> String {
    let neg = n < 0;
    let s = n.unsigned_abs().to_string();
    let exp = s.len() as i64 - 1;
    let mut digits: Vec<u8> = s.into_bytes();
    while digits.len() < sig {
        digits.push(b'0');
    }
    let mut mantissa = String::new();
    mantissa.push(digits[0] as char);
    mantissa.push('.');
    for &b in &digits[1..sig] {
        mantissa.push(b as char);
    }
    format!("{}{}\u{d7}10^{}", if neg { "-" } else { "" }, mantissa, exp)
}

fn num_display(n: Num) -> (String, AnswerKind, AnswerValue) {
    match n {
        Num::R(r) => {
            let digit_len = r.num.unsigned_abs().to_string().len();
            if r.is_integer() {
                let disp = if digit_len > 15 {
                    format_int_scientific(r.num, 12)
                } else {
                    format!("{}", r.num)
                };
                (
                    disp,
                    AnswerKind::Number,
                    AnswerValue::Rational {
                        num: r.num,
                        den: r.den,
                    },
                )
            } else {
                (
                    format!("{}/{}", r.num, r.den),
                    AnswerKind::Rational,
                    AnswerValue::Rational {
                        num: r.num,
                        den: r.den,
                    },
                )
            }
        }
        Num::F(f) => (
            {
                let mut s = format!("{f:.10}");
                while s.ends_with('0') {
                    s.pop();
                }
                if s.ends_with('.') {
                    s.pop();
                }
                format!("\u{2248} {s}")
            },
            AnswerKind::Number,
            AnswerValue::Float { value: f },
        ),
        Num::Mag { mantissa, exponent } => (
            format!("\u{2248} {mantissa:.4}\u{d7}10^{exponent}"),
            AnswerKind::Number,
            AnswerValue::Float {
                value: f64::INFINITY * mantissa.signum(),
            },
        ),
    }
}

fn math_answer(n: Num, explanation: String, unit_suffix: Option<&str>) -> Answer {
    let (mut display, mut kind, value) = num_display(n);
    let mut formats = vec![FormatChip {
        label: "Decimal".into(),
        text: match n {
            Num::R(r) => rat_decimal_display(r),
            _ => display.clone(),
        },
    }];
    if let Num::R(r) = n {
        if !r.is_integer() {
            formats.push(FormatChip {
                label: "Fraction".into(),
                text: format!("{}/{}", r.num, r.den),
            });
        }
    }
    if let Some(u) = unit_suffix {
        // Units are physical quantities — always show the decimal reading
        // (spec §3: "30.48 cm", "72 h", "≈ 3.10686 mi"), never a raw fraction.
        let decimal = match n {
            Num::R(r) => rat_decimal_display(r),
            _ => display.clone(),
        };
        display = format!("{decimal} {u}");
        kind = AnswerKind::Unit;
        formats.insert(
            0,
            FormatChip {
                label: "Value".into(),
                text: display.clone(),
            },
        );
    }
    Answer {
        kind,
        value,
        display,
        explanation,
        formats,
    }
}

fn error_answer(kind: AnswerKind, message: String, explanation: String) -> Answer {
    Answer {
        kind: AnswerKind::Error,
        value: AnswerValue::Error {
            message: message.clone(),
        },
        display: message,
        explanation: if explanation.is_empty() {
            format!("attempted {kind:?}")
        } else {
            explanation
        },
        formats: vec![],
    }
}

// ---------------------------------------------------------------------------
// Detection (spec §4)
// ---------------------------------------------------------------------------

const KEYWORDS: &[&str] = &[
    "now",
    "today",
    "tomorrow",
    "yesterday",
    "next",
    "last",
    "this",
    "ago",
    "in",
    "of",
    "until",
    "since",
    "til",
    "till",
    "to",
    "end",
    "start",
    "current",
    "christmas",
    "easter",
    "black",
    "friday",
    "days",
    "weeks",
    "week",
    "is",
    "what",
];

fn has_keyword_or_unit(input: &str) -> bool {
    for raw in input.split(|c: char| !c.is_alphanumeric() && c != '\'' && c != '.') {
        let w = raw.to_ascii_lowercase();
        if w.is_empty() {
            continue;
        }
        if KEYWORDS.contains(&w.as_str())
            || weekday_from_word(&w).is_some()
            || month_from_word(&w).is_some()
            || time_unit_from_word(&w).is_some()
            || unit_from_word(&w).is_some()
            || is_func_name(&w)
            || matches!(w.as_str(), "pi" | "tau" | "phi")
        {
            return true;
        }
    }
    false
}

fn prefilter_ok(trimmed: &str) -> bool {
    if trimmed.is_empty() || trimmed.chars().count() > 120 {
        return false;
    }
    if trimmed.eq_ignore_ascii_case("e") {
        return false;
    }
    let has_digit = trimmed.chars().any(|c| c.is_ascii_digit());
    let has_op = trimmed.chars().any(|c| "+-*/^!%".contains(c));
    has_digit || has_op || has_keyword_or_unit(trimmed)
}

/// Cheap-then-real two-stage detector (spec §4): a fast prefilter, then the
/// real tokenizer + parser, requiring a whole-string parse. Used both to gate
/// the UI (don't run the full evaluator on every keystroke of ordinary search
/// text) and internally by `evaluate` itself.
pub fn is_answer_like(input: &str) -> bool {
    let trimmed = input.trim();
    if !prefilter_ok(trimmed) {
        return false;
    }
    let dummy = ReferenceNow {
        epoch_secs: 0,
        utc_offset_secs: 0,
    };
    evaluate(trimmed, dummy).is_some()
}

/// Parse+evaluate `input` against the fixed clock `now`. Returns `None` when
/// the whole string doesn't parse as any recognized production (§4) — never
/// for a recognized-but-semantically-invalid expression, which still returns
/// `Some(Answer{kind: Error, ..})` (see spec §6, Error state).
pub fn evaluate(input: &str, now: ReferenceNow) -> Option<Answer> {
    let trimmed = input.trim();
    if !prefilter_ok(trimmed) {
        return None;
    }
    let toks = lex(trimmed)?;
    if toks.is_empty() {
        return None;
    }
    let now_anchor = now.to_anchor();

    if let Some(result) = parse_date_expr(&toks, now_anchor) {
        return Some(match result {
            DateResult::Point(a) => {
                let expl = format!("parsed as {trimmed}");
                if a.has_time {
                    datetime_answer(a.day, a.secs, expl, now.utc_offset_secs)
                } else {
                    date_answer(a.day, expl, now.utc_offset_secs)
                }
            }
            DateResult::Range(s, e, label) => range_answer(
                s,
                e,
                format!("{label} ({} \u{2013} {})", iso_date(s), iso_date(e)),
                format!("parsed as {trimmed}"),
            ),
            DateResult::Countdown {
                target,
                days,
                weeks,
            } => countdown_answer(target, days, weeks, now_anchor.day, now.utc_offset_secs),
        });
    }

    // No bare-literal answers (spec §4): a lone `NUMBER` token with no
    // operator/unit/function/keyword never produces a card — "42" is far more
    // likely a search for that literal. Scientific notation (`3e2`) is exempt:
    // its shape is distinctive enough, same reasoning as ISO dates below.
    if toks.len() == 1 {
        if let TokKind::Num(_) = toks[0].kind {
            let looks_scientific = trimmed.chars().any(|c| c == 'e' || c == 'E')
                && trimmed.chars().next().is_some_and(|c| c.is_ascii_digit());
            if !looks_scientific {
                return None;
            }
        }
    }

    if let Some((result, unit_tag, note)) = parse_math_expr_full(&toks) {
        let explanation = match note {
            Some(n) => format!("parsed as {trimmed} · {n}"),
            None => format!("parsed as {trimmed}"),
        };
        return Some(match result {
            Ok(n) => math_answer(n, explanation, unit_tag),
            Err(msg) => error_answer(AnswerKind::Error, msg, format!("parsed as {trimmed}")),
        });
    }

    None
}

// ---------------------------------------------------------------------------
// Tests — every row of the spec's example tables (§1.5, §2.2–2.5, §3), written
// against the fixed reference "now": Thu 24 Sep 2026, 14:00 Europe/Stockholm
// (UTC+2 / CEST), plus the §4 detection false-positive guards.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Spec's fixed reference `now`: Thursday, 24 September 2026, 14:00,
    /// Europe/Stockholm (UTC+2 / CEST) — epoch_secs is the UTC instant.
    fn now() -> ReferenceNow {
        let day = days_from_civil(2026, 9, 24);
        ReferenceNow {
            epoch_secs: day * 86_400 + 12 * 3600,
            utc_offset_secs: 7200,
        }
    }

    fn ans(input: &str) -> Answer {
        evaluate(input, now()).unwrap_or_else(|| panic!("expected an answer for {input:?}"))
    }

    // Direct access to the math evaluator (bypasses display formatting).
    fn m(input: &str) -> Num {
        let toks = lex(input).unwrap_or_else(|| panic!("lex failed for {input:?}"));
        parse_math_expr(&toks)
            .unwrap_or_else(|| panic!("no parse for {input:?}"))
            .unwrap_or_else(|e| panic!("{input:?} errored: {e}"))
    }

    fn m_err(input: &str) -> String {
        let toks = lex(input).unwrap_or_else(|| panic!("lex failed for {input:?}"));
        parse_math_expr(&toks)
            .unwrap_or_else(|| panic!("no parse for {input:?}"))
            .unwrap_err()
    }

    fn assert_close(a: f64, b: f64, tol: f64) {
        assert!((a - b).abs() < tol, "{a} vs {b} (tol {tol})");
    }

    // -----------------------------------------------------------------
    // §1.5 — date grammar full example table.
    // -----------------------------------------------------------------

    #[test]
    fn date_now() {
        let a = ans("now");
        assert_eq!(a.display, "Thursday, 24 September 2026, 14:00");
        assert_eq!(a.kind, AnswerKind::DateTime);
    }

    #[test]
    fn date_today() {
        let a = ans("today");
        assert_eq!(a.display, "Thursday, 24 September 2026");
        assert_eq!(a.kind, AnswerKind::Date);
    }

    #[test]
    fn date_tomorrow() {
        assert_eq!(ans("tomorrow").display, "Friday, 25 September 2026");
    }

    #[test]
    fn date_yesterday() {
        assert_eq!(ans("yesterday").display, "Wednesday, 23 September 2026");
    }

    #[test]
    fn date_in_3_days() {
        assert_eq!(ans("in 3 days").display, "Sunday, 27 September 2026");
    }

    #[test]
    fn date_3_days_ago() {
        assert_eq!(ans("3 days ago").display, "Monday, 21 September 2026");
    }

    #[test]
    fn date_next_friday() {
        assert_eq!(ans("next friday").display, "Friday, 25 September 2026");
    }

    #[test]
    fn date_next_monday() {
        assert_eq!(ans("next monday").display, "Monday, 28 September 2026");
    }

    #[test]
    fn date_last_month_is_a_range() {
        let a = ans("last month");
        assert_eq!(a.kind, AnswerKind::DateRange);
        match a.value {
            AnswerValue::DateRange {
                start_day_epoch,
                end_day_epoch,
                ..
            } => {
                assert_eq!(start_day_epoch, days_from_civil(2026, 8, 1));
                assert_eq!(end_day_epoch, days_from_civil(2026, 8, 31));
            }
            _ => panic!("expected DateRange"),
        }
    }

    #[test]
    fn date_end_of_month() {
        assert_eq!(ans("end of month").display, "Wednesday, 30 September 2026");
    }

    #[test]
    fn date_first_monday_in_2027() {
        assert_eq!(
            ans("first monday in 2027").display,
            "Monday, 4 January 2027"
        );
    }

    #[test]
    fn date_1st_thursday_in_nov() {
        assert_eq!(
            ans("1st thursday in nov").display,
            "Thursday, 5 November 2026"
        );
    }

    #[test]
    fn date_iso_plus_90d() {
        assert_eq!(
            ans("2026-09-24 + 90d").display,
            "Wednesday, 23 December 2026"
        );
    }

    #[test]
    fn date_days_until_christmas() {
        // Phase 11: a countdown is now a DATE answer (carries the target so the
        // card can draw a calendar) with a "N days" headline.
        let a = ans("days until christmas");
        assert_eq!(a.kind, AnswerKind::Date);
        assert_eq!(
            a.value,
            AnswerValue::Date {
                day_epoch: days_from_civil(2026, 12, 25)
            }
        );
        assert_eq!(a.display, "92 days");
        // First format chip is the count, for pasting.
        assert_eq!(a.formats[0].text, "92");
    }

    #[test]
    fn date_next_easter_and_days_until_easter() {
        // Easter 2027 = 28 Mar (2026's already passed by our fixed now? No —
        // Easter 2026 was 5 Apr, before 24 Sep, so "next easter" rolls to 2027).
        let e = ans("next easter");
        assert_eq!(e.kind, AnswerKind::Date);
        assert_eq!(
            e.value,
            AnswerValue::Date {
                day_epoch: days_from_civil(2027, 3, 28)
            }
        );
        // "days until easter" counts to that same day.
        let d = ans("days until easter");
        assert_eq!(d.kind, AnswerKind::Date);
        assert_eq!(
            d.value,
            AnswerValue::Date {
                day_epoch: days_from_civil(2027, 3, 28)
            }
        );
    }

    #[test]
    fn date_next_black_friday() {
        // Black Friday 2026 = 27 Nov, still ahead of 24 Sep.
        let a = ans("next black friday");
        assert_eq!(
            a.value,
            AnswerValue::Date {
                day_epoch: days_from_civil(2026, 11, 27)
            }
        );
    }

    #[test]
    fn date_this_week_is_a_range_with_iso_label() {
        let a = ans("this week");
        assert_eq!(a.kind, AnswerKind::DateRange);
        // 24 Sep 2026 is a Thursday → week runs Mon 21 – Sun 27 Sep, ISO week 39.
        match &a.value {
            AnswerValue::DateRange {
                start_day_epoch,
                end_day_epoch,
                label,
            } => {
                assert_eq!(*start_day_epoch, days_from_civil(2026, 9, 21));
                assert_eq!(*end_day_epoch, days_from_civil(2026, 9, 27));
                assert!(label.starts_with("week 39"), "label was {label:?}");
            }
            other => panic!("expected a date range, got {other:?}"),
        }
        // "current week" is an alias.
        assert_eq!(ans("current week").value, a.value);
    }

    #[test]
    fn date_weeks_until_christmas() {
        let a = ans("weeks until christmas");
        assert_eq!(a.kind, AnswerKind::Date);
        assert_eq!(a.display, "13 weeks"); // 92 days ≈ 13 weeks
    }

    #[test]
    fn date_1_sept_plus_1_thursday() {
        assert_eq!(
            ans("1 sept + 1 thursday").display,
            "Thursday, 3 September 2026"
        );
    }

    #[test]
    fn date_yesterday_plus_4_thursdays() {
        assert_eq!(
            ans("yesterday + 4 thursdays").display,
            "Thursday, 15 October 2026"
        );
    }

    #[test]
    fn date_yesterday_next_monday_reanchors() {
        assert_eq!(
            ans("yesterday next monday").display,
            "Monday, 28 September 2026"
        );
    }

    #[test]
    fn date_next_monday_plus_paren_math_offset() {
        assert_eq!(
            ans("next monday + (195-17)d").display,
            "Thursday, 25 March 2027"
        );
    }

    #[test]
    fn date_now_plus_1h30m() {
        assert_eq!(
            ans("now + 1h30m").display,
            "Thursday, 24 September 2026, 15:30"
        );
    }

    // -----------------------------------------------------------------
    // §2.2 — rationals, exact until a float op.
    // -----------------------------------------------------------------

    #[test]
    fn math_eighths_plus_twelfths() {
        assert_eq!(m("3/8 + 5/12"), Num::R(Rat { num: 19, den: 24 }));
    }

    #[test]
    fn math_tenths_stay_exact() {
        // The classic 0.1 + 0.2 float trap never happens: this stays a Rat.
        assert_eq!(m("1/10 + 2/10"), Num::R(Rat { num: 3, den: 10 }));
    }

    #[test]
    fn math_22_over_7_stays_exact() {
        assert_eq!(m("22/7"), Num::R(Rat { num: 22, den: 7 }));
    }

    #[test]
    fn math_integer_power_stays_exact() {
        assert_eq!(m("2^10"), Num::R(Rat::int(1024)));
    }

    #[test]
    fn math_fractional_power_decays() {
        match m("2^0.5") {
            Num::F(f) => assert_close(f, 2f64.powf(0.5), 1e-9),
            other => panic!("expected float decay, got {other:?}"),
        }
    }

    #[test]
    fn math_sqrt_forces_decay() {
        // NB: the design doc's literal "≈ 0.8908708064" does not square back to
        // 19/24 (0.8908708064² ≈ 0.79367, not 0.791667) — an apparent typo in
        // the source spec. Asserting the mathematically correct value instead.
        match m("sqrt(3/8 + 5/12)") {
            Num::F(f) => assert_close(f, (19f64 / 24f64).sqrt(), 1e-9),
            other => panic!("expected float, got {other:?}"),
        }
    }

    #[test]
    fn math_zero_over_zero_is_undefined() {
        assert_eq!(m_err("0/0"), "0 \u{f7} 0 is undefined");
    }

    #[test]
    fn math_div_by_zero_errors() {
        assert_eq!(m_err("5/0"), "can't divide by zero");
    }

    // -----------------------------------------------------------------
    // §2.3 — constants and the `3e2` rule.
    // -----------------------------------------------------------------

    #[test]
    fn math_scientific_notation_is_exact() {
        assert_eq!(m("3e2"), Num::R(Rat::int(300)));
    }

    #[test]
    fn math_implicit_mult_with_constant() {
        match m("2pi") {
            Num::F(f) => assert_close(f, 2.0 * std::f64::consts::PI, 1e-9),
            other => panic!("expected float, got {other:?}"),
        }
    }

    #[test]
    fn math_space_separated_e_does_not_parse() {
        let toks = lex("3 e 2").unwrap();
        assert!(parse_math_expr(&toks).is_none());
        assert!(evaluate("3 e 2", now()).is_none());
    }

    #[test]
    fn math_bare_e_is_not_answer_like() {
        assert!(!is_answer_like("e"));
        assert!(evaluate("e", now()).is_none());
    }

    #[test]
    fn math_tau_over_2_equals_pi_and_is_float() {
        match m("tau/2") {
            Num::F(f) => assert_close(f, std::f64::consts::PI, 1e-9),
            other => panic!("expected float, got {other:?}"),
        }
    }

    // -----------------------------------------------------------------
    // §2.4 — factorial and double factorial.
    // -----------------------------------------------------------------

    #[test]
    fn fact_zero() {
        assert_eq!(m("0!"), Num::R(Rat::int(1)));
    }

    #[test]
    fn fact_negative_one_is_undefined() {
        assert_eq!(
            m_err("(-1)!"),
            "undefined (pole of \u{393}) at negative integers"
        );
    }

    #[test]
    fn fact_seven() {
        assert_eq!(m("7!"), Num::R(Rat::int(5040)));
    }

    #[test]
    fn fact_three_point_five_via_gamma() {
        match m("3.5!") {
            Num::F(f) => assert_close(f, gamma(4.5), 1e-6),
            other => panic!("expected float, got {other:?}"),
        }
        // Sanity vs. the spec's stated value.
        if let Num::F(f) = m("3.5!") {
            assert_close(f, 11.6317, 1e-3);
        }
    }

    #[test]
    fn double_fact_five() {
        assert_eq!(m("5!!"), Num::R(Rat::int(15)));
    }

    #[test]
    fn double_fact_zero_is_empty_product() {
        assert_eq!(m("0!!"), Num::R(Rat::int(1)));
    }

    #[test]
    fn double_fact_negative_one_is_defined() {
        assert_eq!(m("(-1)!!"), Num::R(Rat::int(1)));
    }

    #[test]
    fn fact_33_fits_i128_exactly() {
        let expected: i128 = 8_683_317_618_811_886_495_518_194_401_280_000_000;
        assert_eq!(m("33!"), Num::R(Rat::int(expected)));
    }

    #[test]
    fn fact_34_switches_to_magnitude_display() {
        // 34! has 39 digits, so the correct exponent is 38 — the spec's table
        // shows "10^37", which is off by one from the true value (33! is
        // exponent 36, so 34! cannot also be 37); implemented the correct
        // magnitude here rather than reproducing that apparent typo.
        match m("34!") {
            Num::Mag { mantissa, exponent } => {
                assert_eq!(exponent, 38);
                assert_close(mantissa, 2.9523, 1e-3);
            }
            other => panic!("expected Mag, got {other:?}"),
        }
    }

    #[test]
    fn fact_1000_via_lgamma_magnitude() {
        match m("1000!") {
            Num::Mag { mantissa, exponent } => {
                assert_eq!(exponent, 2567);
                assert_close(mantissa, 4.0239, 1e-3);
            }
            other => panic!("expected Mag, got {other:?}"),
        }
    }

    // -----------------------------------------------------------------
    // §2.5 — functions and distributions.
    // -----------------------------------------------------------------

    #[test]
    fn func_sin_pi_over_2_snaps_to_one() {
        assert_eq!(m("sin(pi/2)"), Num::F(1.0));
    }

    #[test]
    fn func_sqrt_negative_is_domain_error() {
        assert_eq!(
            m_err("sqrt(-1)"),
            "sqrt of a negative number isn't supported"
        );
    }

    #[test]
    fn func_log_base_10_snaps() {
        assert_eq!(m("log(1000)"), Num::F(3.0));
    }

    #[test]
    fn func_ln_e_snaps() {
        assert_eq!(m("ln(e)"), Num::F(1.0));
    }

    #[test]
    fn func_normal_cdf() {
        match m("normal.cdf(1.96; 0, 1)") {
            Num::F(f) => assert_close(f, 0.9750, 1e-3),
            other => panic!("expected float, got {other:?}"),
        }
    }

    #[test]
    fn func_binom_pdf() {
        match m("binom.pdf(3; 10, 0.5)") {
            Num::F(f) => assert_close(f, 0.1172, 1e-3),
            other => panic!("expected float, got {other:?}"),
        }
    }

    #[test]
    fn func_poisson_cdf() {
        match m("poisson.cdf(2; 3)") {
            Num::F(f) => assert_close(f, 0.4232, 1e-3),
            other => panic!("expected float, got {other:?}"),
        }
    }

    // -----------------------------------------------------------------
    // §2.1 — percent forms (concrete instances of the spec's generic forms).
    // -----------------------------------------------------------------

    #[test]
    fn percent_add() {
        assert_eq!(m("200 + 10%"), Num::R(Rat::int(220)));
    }

    #[test]
    fn percent_sub() {
        assert_eq!(m("200 - 10%"), Num::R(Rat::int(180)));
    }

    #[test]
    fn percent_of() {
        assert_eq!(m("10% of 200"), Num::R(Rat::int(20)));
    }

    #[test]
    fn percent_what_percent_of() {
        match m("40 is what % of 200") {
            Num::F(f) => assert_close(f, 20.0, 1e-9),
            other => panic!("expected float, got {other:?}"),
        }
    }

    // -----------------------------------------------------------------
    // §3 — units.
    // -----------------------------------------------------------------

    #[test]
    fn unit_km_to_miles() {
        let a = ans("5 km in miles");
        assert_eq!(a.kind, AnswerKind::Unit);
        assert!(a.display.starts_with("\u{2248} 3.1068"), "{}", a.display);
        assert!(a.display.ends_with("mi"));
    }

    #[test]
    fn unit_inch_to_cm_is_exact() {
        let a = ans("12 in in cm");
        assert_eq!(a.display, "30.48 cm");
    }

    #[test]
    fn unit_lb_to_kg() {
        let a = ans("2 lb in kg");
        assert!(a.display.starts_with("\u{2248} 0.907185"), "{}", a.display);
    }

    #[test]
    fn unit_data_size_decimal_vs_binary() {
        // Decimal: 1 GB = 1000 MB exactly.
        let a = ans("1 gb in mb");
        assert_eq!(a.kind, AnswerKind::Unit);
        assert!(
            a.display.contains("1000") && a.display.ends_with("MB"),
            "{}",
            a.display
        );
        // Binary: 1 GiB = 1024 MiB exactly.
        let b = ans("1 gib in mib");
        assert!(
            b.display.contains("1024") && b.display.ends_with("MiB"),
            "{}",
            b.display
        );
    }

    #[test]
    fn unit_speed_volume_energy_convert() {
        let a = ans("100 kmh in mph");
        assert_eq!(a.kind, AnswerKind::Unit);
        assert!(a.display.contains("62."), "{}", a.display); // ≈ 62.137 mph
        let v = ans("2 l in ml");
        assert!(
            v.display.contains("2000") && v.display.ends_with("ml"),
            "{}",
            v.display
        );
        let e = ans("1 kwh in kj");
        assert!(
            e.display.contains("3600") && e.display.ends_with("kJ"),
            "{}",
            e.display
        );
    }

    #[test]
    fn unit_angle_degrees_to_radians() {
        let a = ans("180 deg in rad");
        assert_eq!(a.kind, AnswerKind::Unit);
        assert!(a.display.contains("3.14"), "{}", a.display);
    }

    #[test]
    fn unit_days_to_hours_is_exact() {
        let a = ans("3 days in hours");
        assert_eq!(a.display, "72 h");
    }

    #[test]
    fn unit_fahrenheit_to_celsius_is_exact() {
        let a = ans("98.6 f in c");
        assert_eq!(a.display, "37 c");
    }

    // -----------------------------------------------------------------
    // §1.6 / §4 — detection: false positives must show no card.
    // -----------------------------------------------------------------

    #[test]
    fn detection_false_positives() {
        for text in [
            "nextcloud",
            "monday.com",
            "next steps meeting",
            "2026 report",
            "march madness bracket",
        ] {
            assert!(
                evaluate(text, now()).is_none(),
                "should not answer: {text:?}"
            );
            assert!(!is_answer_like(text), "should not be answer-like: {text:?}");
        }
    }

    #[test]
    fn detection_bare_number_is_not_answer_like() {
        assert!(!is_answer_like("42"));
        assert!(!is_answer_like("100"));
        assert!(evaluate("42", now()).is_none());
    }

    #[test]
    fn detection_pasted_blob_over_120_chars_rejected() {
        let long = "a".repeat(200);
        assert!(!is_answer_like(&long));
    }

    #[test]
    fn detection_positive_examples_are_answer_like() {
        for text in [
            "next monday",
            "3/8 + 5/12",
            "2026-09-24",
            "5 km in miles",
            "2pi",
            "e^2",
        ] {
            assert!(is_answer_like(text), "should be answer-like: {text:?}");
        }
    }

    // ---- Slash-form units + volume↔mass via density (2026-11-11) ----

    fn num_of(a: &Answer) -> f64 {
        let t: String = a
            .display
            .chars()
            .filter(|c| c.is_ascii_digit() || *c == '.' || *c == '-')
            .collect();
        t.parse()
            .unwrap_or_else(|_| panic!("no number in {:?}", a.display))
    }

    #[test]
    fn slash_units_convert() {
        let a = ans("100 km/h in mph");
        assert_eq!(a.kind, AnswerKind::Unit);
        assert!((num_of(&a) - 62.137).abs() < 0.01, "{}", a.display);
        assert!(a.display.ends_with("mph"), "{}", a.display);
        let b = ans("10 m/s in km/h");
        assert_eq!(b.display, "36 km/h");
        let c = ans("60 mi/h to km/h");
        assert!((num_of(&c) - 96.56).abs() < 0.01, "{}", c.display);
    }

    #[test]
    fn spaced_slash_is_still_division_and_to_converts() {
        assert_eq!(ans("10 km / 2").display, "5");
        let a = ans("5 km to mi");
        assert!((num_of(&a) - 3.107).abs() < 0.001, "{}", a.display);
    }

    #[test]
    fn volume_mass_through_density() {
        let a = ans("1 cup flour in g");
        assert_eq!(a.kind, AnswerKind::Unit);
        assert!((num_of(&a) - 125.39).abs() < 0.1, "{}", a.display);
        assert!(a.explanation.contains("flour"), "{}", a.explanation);
        assert_eq!(ans("500 ml of water in g").display, "500 g");
        let s = ans("100 g sugar in ml");
        assert!((num_of(&s) - 117.65).abs() < 0.05, "{}", s.display);
        assert_eq!(ans("1 l water in kg").display, "1 kg");
        // Same domain: the substance is ignored.
        assert_eq!(ans("2 l milk in ml").display, "2000 ml");
    }

    #[test]
    fn volume_mass_without_substance_is_an_error() {
        let a = ans("1 cup in g");
        assert_eq!(a.kind, AnswerKind::Error, "{}", a.display);
        assert!(a.display.contains("substance"), "{}", a.display);
    }
}
