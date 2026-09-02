//! Turning a `Kind::Color` entry's text into an actual RGB triple, so the list
//! and preview can show a swatch instead of only the raw string.
//!
//! The accepted formats mirror `magpie_core::detect`'s colour regex exactly —
//! `#RGB`, `#RRGGBB`, `rgb(r,g,b)`, `hsl(h,s%,l%)`, any case. If the detector
//! calls something a colour, this must be able to draw it.

/// Parse a detected colour string into `(r, g, b)`. Returns `None` for anything
/// it can't render, so callers fall back to plain text.
pub fn parse_color(text: &str) -> Option<(u8, u8, u8)> {
    let t = text.trim();
    if let Some(hex) = t.strip_prefix('#') {
        return parse_hex(hex);
    }
    let lower = t.to_ascii_lowercase();
    if let Some(args) = strip_call(&lower, "rgb") {
        let n = numbers(args)?;
        if n.len() != 3 {
            return None;
        }
        return Some((clamp_u8(n[0]), clamp_u8(n[1]), clamp_u8(n[2])));
    }
    if let Some(args) = strip_call(&lower, "hsl") {
        let n = numbers(args)?;
        if n.len() != 3 {
            return None;
        }
        return Some(hsl_to_rgb(n[0], n[1], n[2]));
    }
    None
}

/// `#RGB` (each digit doubled, the CSS rule) or `#RRGGBB`.
fn parse_hex(hex: &str) -> Option<(u8, u8, u8)> {
    let b = hex.as_bytes();
    match b.len() {
        3 => {
            let d = |i: usize| u8::from_str_radix(&hex[i..i + 1], 16).ok();
            let (r, g, bl) = (d(0)?, d(1)?, d(2)?);
            Some((r * 17, g * 17, bl * 17))
        }
        6 => {
            let d = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
            Some((d(0)?, d(2)?, d(4)?))
        }
        _ => None,
    }
}

/// `"rgb(1, 2, 3)"` + `"rgb"` → `"1, 2, 3"`. Input is already lowercased.
fn strip_call<'a>(s: &'a str, name: &str) -> Option<&'a str> {
    s.strip_prefix(name)?
        .trim()
        .strip_prefix('(')?
        .strip_suffix(')')
}

/// The comma-separated numbers in `"0, 0%, 20%"` (percent signs ignored).
fn numbers(args: &str) -> Option<Vec<f32>> {
    args.split(',')
        .map(|p| p.trim().trim_end_matches('%').trim().parse::<f32>().ok())
        .collect()
}

fn clamp_u8(v: f32) -> u8 {
    v.clamp(0.0, 255.0).round() as u8
}

/// CSS `hsl()` → RGB. `h` in degrees, `s`/`l` in percent.
fn hsl_to_rgb(h: f32, s: f32, l: f32) -> (u8, u8, u8) {
    let h = h.rem_euclid(360.0) / 360.0;
    let s = (s / 100.0).clamp(0.0, 1.0);
    let l = (l / 100.0).clamp(0.0, 1.0);
    if s == 0.0 {
        let v = clamp_u8(l * 255.0);
        return (v, v, v);
    }
    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let p = 2.0 * l - q;
    let f = |mut t: f32| {
        if t < 0.0 {
            t += 1.0;
        }
        if t > 1.0 {
            t -= 1.0;
        }
        let v = if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        };
        clamp_u8(v * 255.0)
    };
    (f(h + 1.0 / 3.0), f(h), f(h - 1.0 / 3.0))
}

/// `1_234_567` → `"1.2 MB"`. Used for image entry sizes.
pub fn human_bytes(n: i64) -> String {
    const KB: f64 = 1024.0;
    let n = n.max(0) as f64;
    if n < KB {
        return format!("{n:.0} B");
    }
    let units = ["KB", "MB", "GB"];
    let mut v = n / KB;
    for (i, u) in units.iter().enumerate() {
        if v < KB || i == units.len() - 1 {
            return if v < 10.0 {
                format!("{v:.1} {u}")
            } else {
                format!("{v:.0} {u}")
            };
        }
        v /= KB;
    }
    unreachable!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_hex_doubles_digits() {
        assert_eq!(parse_color("#333"), Some((0x33, 0x33, 0x33)));
        assert_eq!(parse_color("#f00"), Some((255, 0, 0)));
    }

    #[test]
    fn long_hex() {
        assert_eq!(parse_color("#333333"), Some((0x33, 0x33, 0x33)));
        assert_eq!(parse_color("#FF8800"), Some((255, 0x88, 0)));
    }

    #[test]
    fn rgb_function() {
        assert_eq!(parse_color("rgb(10, 20, 30)"), Some((10, 20, 30)));
        assert_eq!(parse_color("RGB(255,255,255)"), Some((255, 255, 255)));
    }

    #[test]
    fn hsl_function_including_uppercase() {
        // Grey: zero saturation, 20% lightness → 0.2 * 255 ≈ 51 = #333.
        assert_eq!(parse_color("HSL(0, 0%, 20%)"), Some((51, 51, 51)));
        assert_eq!(parse_color("hsl(0, 100%, 50%)"), Some((255, 0, 0)));
        assert_eq!(parse_color("hsl(120, 100%, 50%)"), Some((0, 255, 0)));
        assert_eq!(parse_color("hsl(240, 100%, 50%)"), Some((0, 0, 255)));
    }

    #[test]
    fn rejects_non_colors() {
        assert_eq!(parse_color("hello"), None);
        assert_eq!(parse_color("#12"), None);
        assert_eq!(parse_color("#12345"), None);
        assert_eq!(parse_color("rgb(1,2)"), None);
        assert_eq!(parse_color("rgb(a,b,c)"), None);
        assert_eq!(parse_color(""), None);
    }

    /// Anything `magpie_core::detect` classifies as a colour must be drawable —
    /// a detected colour with no swatch would be a silent hole in the UI.
    #[test]
    fn parses_everything_the_detector_calls_a_color() {
        for s in [
            "#abc",
            "#ABCDEF",
            "rgb(0,0,0)",
            "rgb( 12 , 34 , 56 )",
            "hsl(0,0%,0%)",
            "hsl(359, 50%, 50%)",
        ] {
            assert_eq!(
                magpie_core::detect::detect_text_kind(s),
                magpie_core::Kind::Color,
                "{s} should be detected as a colour"
            );
            assert!(parse_color(s).is_some(), "{s} should parse to a swatch");
        }
    }

    #[test]
    fn human_bytes_scales() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(1024), "1.0 KB");
        assert_eq!(human_bytes(20 * 1024), "20 KB");
        assert_eq!(human_bytes(5 * 1024 * 1024), "5.0 MB");
    }
}
