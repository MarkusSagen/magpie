//! Display-only masking for screensharing / sensitive sources. Never changes
//! stored or pasted text — only how an entry is rendered.

use regex::Regex;

/// First `visible` chars of `text` then capped bullets. Text no longer than
/// `visible` is fully masked (min one bullet). Unicode-safe (operates on chars).
pub fn mask_render(text: &str, visible: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    if n <= visible {
        return "•".repeat(n.max(1));
    }
    let head: String = chars[..visible].iter().collect();
    let bullets = "•".repeat((n - visible).min(12));
    format!("{head}{bullets}")
}

/// Compiled masking rules for one refresh. Invalid regexes are dropped.
pub struct MaskRules {
    pub screenshare: bool,
    pub apps: Vec<String>,
    pub patterns: Vec<Regex>,
}

impl MaskRules {
    pub fn build(apps: &[String], patterns: &[String], screenshare: bool) -> MaskRules {
        let patterns = patterns.iter().filter_map(|p| Regex::new(p).ok()).collect();
        MaskRules {
            screenshare,
            apps: apps.to_vec(),
            patterns,
        }
    }
}

/// An entry is masked if screenshare is on, its source app is in `apps`, or its
/// text matches any pattern.
pub fn should_mask(rules: &MaskRules, app_name: Option<&str>, full_text: &str) -> bool {
    if rules.screenshare {
        return true;
    }
    if let Some(a) = app_name {
        if rules.apps.iter().any(|x| x == a) {
            return true;
        }
    }
    rules.patterns.iter().any(|re| re.is_match(full_text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_prefix_then_bullets() {
        assert_eq!(
            mask_render("sk-live-abcdef", 3),
            format!("sk-{}", "•".repeat(11))
        );
    }

    #[test]
    fn render_short_text_fully_masked() {
        assert_eq!(mask_render("hi", 3), "••");
        assert_eq!(mask_render("", 3), "•");
    }

    #[test]
    fn render_visible_zero_all_bullets() {
        assert_eq!(mask_render("hello", 0), "•••••");
    }

    #[test]
    fn render_caps_bullets_at_12() {
        let out = mask_render(&"x".repeat(100), 2);
        assert_eq!(out.chars().filter(|&c| c == '•').count(), 12);
    }

    #[test]
    fn render_unicode_by_char() {
        assert_eq!(mask_render("café…", 3), "caf••");
    }

    #[test]
    fn should_mask_screenshare_always() {
        let r = MaskRules::build(&[], &[], true);
        assert!(should_mask(&r, None, "anything"));
    }

    #[test]
    fn should_mask_by_app() {
        let r = MaskRules::build(&["Slack".into()], &[], false);
        assert!(should_mask(&r, Some("Slack"), "x"));
        assert!(!should_mask(&r, Some("Terminal"), "x"));
    }

    #[test]
    fn should_mask_by_pattern_and_skips_invalid() {
        let r = MaskRules::build(&[], &["secret".into(), "(".into()], false);
        assert!(should_mask(&r, None, "a secret value"));
        assert!(!should_mask(&r, None, "nothing here"));
    }

    #[test]
    fn should_not_mask_when_no_rule_hits() {
        let r = MaskRules::build(&[], &[], false);
        assert!(!should_mask(&r, Some("Terminal"), "plain"));
    }
}
