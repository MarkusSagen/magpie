use std::sync::OnceLock;
use regex::Regex;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind { Text, Link, Color, Email, Rtf, Html, Image, File }

impl Kind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Kind::Text => "text",
            Kind::Link => "link",
            Kind::Color => "color",
            Kind::Email => "email",
            Kind::Rtf => "rtf",
            Kind::Html => "html",
            Kind::Image => "image",
            Kind::File => "file",
        }
    }
    pub fn from_str(s: &str) -> Option<Kind> {
        Some(match s {
            "text" => Kind::Text,
            "link" => Kind::Link,
            "color" => Kind::Color,
            "email" => Kind::Email,
            "rtf" => Kind::Rtf,
            "html" => Kind::Html,
            "image" => Kind::Image,
            "file" => Kind::File,
            _ => return None,
        })
    }
}

fn url_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^https?://\S+$").unwrap())
}
fn email_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^[^\s@]+@[^\s@]+\.[^\s@]+$").unwrap())
}
fn color_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)^(#([0-9a-f]{3}|[0-9a-f]{6})|rgb\(\s*\d{1,3}\s*,\s*\d{1,3}\s*,\s*\d{1,3}\s*\)|hsl\(\s*\d{1,3}\s*,\s*\d{1,3}%\s*,\s*\d{1,3}%\s*\))$").unwrap()
    })
}

pub fn detect_text_kind(s: &str) -> Kind {
    let t = s.trim();
    if url_re().is_match(t) { return Kind::Link; }
    if email_re().is_match(t) { return Kind::Email; }
    if color_re().is_match(t) { return Kind::Color; }
    Kind::Text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_string_roundtrip() {
        for k in [Kind::Text, Kind::Link, Kind::Color, Kind::Email,
                  Kind::Rtf, Kind::Html, Kind::Image, Kind::File] {
            assert_eq!(Kind::from_str(k.as_str()), Some(k));
        }
        assert_eq!(Kind::from_str("nope"), None);
    }

    #[test]
    fn detects_url() {
        assert!(matches!(detect_text_kind("https://example.com/x"), Kind::Link));
        assert!(matches!(detect_text_kind("  http://a.b  "), Kind::Link));
    }

    #[test]
    fn detects_email() {
        assert!(matches!(detect_text_kind("me@example.io"), Kind::Email));
    }

    #[test]
    fn detects_hex_and_rgb_color() {
        assert!(matches!(detect_text_kind("#1a2b3c"), Kind::Color));
        assert!(matches!(detect_text_kind("#fff"), Kind::Color));
        assert!(matches!(detect_text_kind("rgb(10, 20, 30)"), Kind::Color));
    }

    #[test]
    fn plain_text_is_text() {
        assert!(matches!(detect_text_kind("just some words"), Kind::Text));
        assert!(matches!(detect_text_kind("not#acolor"), Kind::Text));
    }
}
