//! Colour answers (Phase 13): parse `#hex` / `rgb()` / `hsl()` / `oklch()`, convert
//! between spaces, and derive an OKLCH tint/shade scale plus complement / analogous
//! / triad harmonies. OKLCH is used for the scale so the steps are perceptually even.
//! Pure maths — table-driven tested; no dependencies.

/// An sRGB colour with straight alpha (all channels 0..=1 for maths; `to_hex`
/// etc. quantise to 8-bit).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Color {
    pub r: f64,
    pub g: f64,
    pub b: f64,
    pub a: f64,
}

impl Color {
    fn clamp(self) -> Color {
        Color {
            r: self.r.clamp(0.0, 1.0),
            g: self.g.clamp(0.0, 1.0),
            b: self.b.clamp(0.0, 1.0),
            a: self.a.clamp(0.0, 1.0),
        }
    }

    pub fn to_hex(self) -> String {
        let c = self.clamp();
        let q = |x: f64| (x * 255.0).round() as u8;
        if c.a >= 1.0 {
            format!("#{:02x}{:02x}{:02x}", q(c.r), q(c.g), q(c.b))
        } else {
            format!("#{:02x}{:02x}{:02x}{:02x}", q(c.r), q(c.g), q(c.b), q(c.a))
        }
    }

    pub fn to_rgb_string(self) -> String {
        let c = self.clamp();
        let q = |x: f64| (x * 255.0).round() as u8;
        if c.a >= 1.0 {
            format!("rgb({}, {}, {})", q(c.r), q(c.g), q(c.b))
        } else {
            format!("rgba({}, {}, {}, {:.2})", q(c.r), q(c.g), q(c.b), c.a)
        }
    }

    pub fn to_hsl_string(self) -> String {
        let (h, s, l) = self.to_hsl();
        format!("hsl({:.0}, {:.0}%, {:.0}%)", h, s * 100.0, l * 100.0)
    }

    pub fn to_oklch_string(self) -> String {
        let (l, c, h) = self.to_oklch();
        format!("oklch({:.3} {:.3} {:.1})", l, c, h)
    }

    /// 8-bit RGB tuple for the UI swatch.
    pub fn rgb_u8(self) -> (u8, u8, u8) {
        let c = self.clamp();
        (
            (c.r * 255.0).round() as u8,
            (c.g * 255.0).round() as u8,
            (c.b * 255.0).round() as u8,
        )
    }

    // ---- HSL ----
    pub fn to_hsl(self) -> (f64, f64, f64) {
        let (r, g, b) = (self.r, self.g, self.b);
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let l = (max + min) / 2.0;
        if (max - min).abs() < 1e-9 {
            return (0.0, 0.0, l);
        }
        let d = max - min;
        let s = if l > 0.5 {
            d / (2.0 - max - min)
        } else {
            d / (max + min)
        };
        let h = if max == r {
            ((g - b) / d).rem_euclid(6.0)
        } else if max == g {
            (b - r) / d + 2.0
        } else {
            (r - g) / d + 4.0
        } * 60.0;
        (h.rem_euclid(360.0), s, l)
    }

    pub fn from_hsl(h: f64, s: f64, l: f64, a: f64) -> Color {
        let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
        let hp = h.rem_euclid(360.0) / 60.0;
        let x = c * (1.0 - (hp.rem_euclid(2.0) - 1.0).abs());
        let (r1, g1, b1) = match hp as i32 {
            0 => (c, x, 0.0),
            1 => (x, c, 0.0),
            2 => (0.0, c, x),
            3 => (0.0, x, c),
            4 => (x, 0.0, c),
            _ => (c, 0.0, x),
        };
        let m = l - c / 2.0;
        Color {
            r: r1 + m,
            g: g1 + m,
            b: b1 + m,
            a,
        }
    }

    // ---- OKLCH (via OKLab) ----
    pub fn to_oklch(self) -> (f64, f64, f64) {
        let lin = |c: f64| {
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        let (r, g, b) = (lin(self.r), lin(self.g), lin(self.b));
        let l = 0.412_221_470_8 * r + 0.536_332_536_3 * g + 0.051_445_992_9 * b;
        let m = 0.211_903_498_2 * r + 0.680_699_545_1 * g + 0.107_396_956_6 * b;
        let s = 0.088_302_461_9 * r + 0.281_718_837_6 * g + 0.629_978_700_5 * b;
        let (l_, m_, s_) = (l.cbrt(), m.cbrt(), s.cbrt());
        let ok_l = 0.210_454_255_3 * l_ + 0.793_617_785_0 * m_ - 0.004_072_046_8 * s_;
        let ok_a = 1.977_998_495_1 * l_ - 2.428_592_205_0 * m_ + 0.450_593_709_9 * s_;
        let ok_b = 0.025_904_037_1 * l_ + 0.782_771_766_2 * m_ - 0.808_675_766_0 * s_;
        let c = (ok_a * ok_a + ok_b * ok_b).sqrt();
        let h = ok_b.atan2(ok_a).to_degrees().rem_euclid(360.0);
        (ok_l, c, h)
    }

    pub fn from_oklch(ok_l: f64, c: f64, h: f64, a: f64) -> Color {
        let hr = h.to_radians();
        let ok_a = c * hr.cos();
        let ok_b = c * hr.sin();
        let l_ = ok_l + 0.396_337_777_4 * ok_a + 0.215_803_757_3 * ok_b;
        let m_ = ok_l - 0.105_561_345_8 * ok_a - 0.063_854_172_8 * ok_b;
        let s_ = ok_l - 0.089_484_177_5 * ok_a - 1.291_485_548_0 * ok_b;
        let (l, m, s) = (l_ * l_ * l_, m_ * m_ * m_, s_ * s_ * s_);
        let r = 4.076_741_662_1 * l - 3.307_711_591_3 * m + 0.230_969_929_2 * s;
        let g = -1.268_438_004_6 * l + 2.609_757_401_1 * m - 0.341_319_396_5 * s;
        let b = -0.004_196_086_3 * l - 0.703_418_614_7 * m + 1.707_614_701_0 * s;
        let gam = |c: f64| {
            let c = c.clamp(0.0, 1.0);
            if c <= 0.003_130_8 {
                12.92 * c
            } else {
                1.055 * c.powf(1.0 / 2.4) - 0.055
            }
        };
        Color {
            r: gam(r),
            g: gam(g),
            b: gam(b),
            a,
        }
        .clamp()
    }

    /// Rotate the hue by `deg` (in OKLCH) — the basis for harmonies.
    pub fn rotate_hue(self, deg: f64) -> Color {
        let (l, c, h) = self.to_oklch();
        Color::from_oklch(l, c, h + deg, self.a)
    }
}

/// Parse `#hex` (3/4/6/8), `rgb()/rgba()`, `hsl()/hsla()` or `oklch()`. Returns
/// `None` if it isn't a recognizable colour.
pub fn parse(input: &str) -> Option<Color> {
    let s = input.trim();
    if let Some(hex) = s.strip_prefix('#') {
        return parse_hex(hex);
    }
    let lower = s.to_lowercase();
    if let Some(inner) = fn_args(&lower, "rgb") {
        return parse_rgb(inner);
    }
    if let Some(inner) = fn_args(&lower, "hsl") {
        return parse_hsl(inner);
    }
    if let Some(inner) = fn_args(&lower, "oklch") {
        return parse_oklch(inner);
    }
    None
}

fn fn_args<'a>(s: &'a str, name: &str) -> Option<&'a str> {
    // Accept both `rgb(` and `rgba(` etc.
    for pfx in [name.to_string(), format!("{name}a")] {
        if let Some(rest) = s.strip_prefix(&pfx) {
            let rest = rest.trim_start();
            if let Some(inner) = rest.strip_prefix('(').and_then(|r| r.strip_suffix(')')) {
                return Some(inner);
            }
        }
    }
    None
}

fn parse_hex(hex: &str) -> Option<Color> {
    let h = hex.trim();
    let expand = |c: char| c.to_string().repeat(2);
    let full = match h.len() {
        3 => h.chars().map(expand).collect::<String>(),
        4 => h.chars().map(expand).collect::<String>(),
        6 | 8 => h.to_string(),
        _ => return None,
    };
    let byte = |i: usize| u8::from_str_radix(&full[i..i + 2], 16).ok();
    let (r, g, b) = (byte(0)?, byte(2)?, byte(4)?);
    let a = if full.len() == 8 { byte(6)? } else { 255 };
    Some(Color {
        r: r as f64 / 255.0,
        g: g as f64 / 255.0,
        b: b as f64 / 255.0,
        a: a as f64 / 255.0,
    })
}

fn split_nums(inner: &str) -> Vec<f64> {
    inner
        .split([',', '/', ' '])
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .filter_map(|t| t.trim_end_matches('%').parse::<f64>().ok())
        .collect()
}

fn parse_rgb(inner: &str) -> Option<Color> {
    let n = split_nums(inner);
    if n.len() < 3 {
        return None;
    }
    Some(Color {
        r: n[0] / 255.0,
        g: n[1] / 255.0,
        b: n[2] / 255.0,
        a: n.get(3).copied().unwrap_or(1.0),
    })
}

fn parse_hsl(inner: &str) -> Option<Color> {
    let n = split_nums(inner);
    if n.len() < 3 {
        return None;
    }
    Some(Color::from_hsl(
        n[0],
        n[1] / 100.0,
        n[2] / 100.0,
        n.get(3).copied().unwrap_or(1.0),
    ))
}

fn parse_oklch(inner: &str) -> Option<Color> {
    let n = split_nums(inner);
    if n.len() < 3 {
        return None;
    }
    // L may be given as 0..1 or as a percentage (0..100).
    let l = if n[0] > 1.5 { n[0] / 100.0 } else { n[0] };
    Some(Color::from_oklch(
        l,
        n[1],
        n[2],
        n.get(3).copied().unwrap_or(1.0),
    ))
}

/// An 8-step tint→shade scale (light to dark) in OKLCH — even lightness steps
/// keeping the chroma/hue, so it reads as one hue ramp.
pub fn scale(base: Color) -> Vec<Color> {
    let (_, c, h) = base.to_oklch();
    (0..8)
        .map(|i| {
            let l = 0.95 - (i as f64) * (0.80 / 7.0); // 0.95 → 0.15
            Color::from_oklch(l, c, h, base.a)
        })
        .collect()
}

/// Complement (180°).
pub fn complement(base: Color) -> Color {
    base.rotate_hue(180.0)
}

/// Analogous pair (±30°).
pub fn analogous(base: Color) -> (Color, Color) {
    (base.rotate_hue(-30.0), base.rotate_hue(30.0))
}

/// Triad (±120°).
pub fn triad(base: Color) -> (Color, Color) {
    (base.rotate_hue(120.0), base.rotate_hue(240.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f64, b: f64) -> bool {
        (a - b).abs() < 0.02
    }

    #[test]
    fn parses_all_formats() {
        assert_eq!(parse("#89b4fa").unwrap().rgb_u8(), (0x89, 0xb4, 0xfa));
        assert_eq!(parse("#fff").unwrap().rgb_u8(), (255, 255, 255));
        assert_eq!(
            parse("rgb(137, 180, 250)").unwrap().rgb_u8(),
            (137, 180, 250)
        );
        assert_eq!(parse("rgba(0,0,0,0.5)").unwrap().a, 0.5);
        let hsl = parse("hsl(217, 92%, 76%)").unwrap();
        assert!(approx(hsl.r, 137.0 / 255.0) && approx(hsl.b, 250.0 / 255.0));
        assert!(parse("not a color").is_none());
    }

    #[test]
    fn hex_roundtrips() {
        let c = parse("#89b4fa").unwrap();
        assert_eq!(c.to_hex(), "#89b4fa");
    }

    #[test]
    fn oklch_roundtrip_is_stable() {
        let c = parse("#89b4fa").unwrap();
        let (l, ch, h) = c.to_oklch();
        let back = Color::from_oklch(l, ch, h, 1.0);
        assert!(approx(c.r, back.r) && approx(c.g, back.g) && approx(c.b, back.b));
    }

    #[test]
    fn scale_has_eight_steps_light_to_dark() {
        let s = scale(parse("#89b4fa").unwrap());
        assert_eq!(s.len(), 8);
        let l0 = s[0].to_oklch().0;
        let l7 = s[7].to_oklch().0;
        assert!(l0 > l7); // lighter first
    }

    #[test]
    fn harmonies_rotate_hue() {
        let base = parse("#ff0000").unwrap();
        let comp = complement(base);
        let (bh, ch) = (base.to_oklch().2, comp.to_oklch().2);
        assert!(approx(((ch - bh).rem_euclid(360.0) - 180.0).abs(), 0.0) || (ch - bh).abs() > 90.0);
        let (a1, a2) = analogous(base);
        assert_ne!(a1.to_hex(), a2.to_hex());
        let _ = triad(base);
    }
}
