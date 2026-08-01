use crate::traits::ClipboardSnapshot;
use magpie_core::{AppInfo, Content};
use regex::Regex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkipReason { Paused, Concealed, DeniedApp, IgnoredContent, Empty }

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision { Keep, Skip(SkipReason) }

pub struct CapturePolicy {
    pub paused: bool,
    pub app_denylist: Vec<String>,
    pub ignore_regexes: Vec<Regex>,
}

impl Default for CapturePolicy {
    fn default() -> Self { Self::new() }
}

impl CapturePolicy {
    pub fn new() -> Self {
        CapturePolicy { paused: false, app_denylist: Vec::new(), ignore_regexes: Vec::new() }
    }

    pub fn decide(&self, snap: &ClipboardSnapshot, app: Option<&AppInfo>) -> Decision {
        if self.paused {
            return Decision::Skip(SkipReason::Paused);
        }
        let content = match &snap.content {
            Some(c) => c,
            None => return Decision::Skip(SkipReason::Empty),
        };
        if snap.concealed {
            return Decision::Skip(SkipReason::Concealed);
        }
        if let Some(a) = app {
            if self.app_denylist.iter().any(|d| d == &a.identifier) {
                return Decision::Skip(SkipReason::DeniedApp);
            }
        }
        let text = content_text(content);
        if !text.is_empty() && self.ignore_regexes.iter().any(|re| re.is_match(&text)) {
            return Decision::Skip(SkipReason::IgnoredContent);
        }
        Decision::Keep
    }
}

pub fn content_text(c: &Content) -> String {
    match c {
        Content::Text(t) => t.clone(),
        Content::Rich { text, .. } => text.clone(),
        Content::Files(paths) => paths.join("\n"),
        Content::Image { .. } => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use magpie_core::{AppInfo, Content};

    fn snap(content: Option<Content>, concealed: bool) -> ClipboardSnapshot {
        ClipboardSnapshot { content, change_token: 1, concealed }
    }
    fn app(id: &str) -> AppInfo {
        AppInfo { identifier: id.into(), display_name: id.into(), icon_path: None }
    }

    #[test]
    fn paused_skips_everything() {
        let mut p = CapturePolicy::new();
        p.paused = true;
        assert!(matches!(p.decide(&snap(Some(Content::Text("x".into())), false), None),
                         Decision::Skip(SkipReason::Paused)));
    }

    #[test]
    fn empty_content_is_skipped() {
        let p = CapturePolicy::new();
        assert!(matches!(p.decide(&snap(None, false), None), Decision::Skip(SkipReason::Empty)));
    }

    #[test]
    fn concealed_is_skipped() {
        let p = CapturePolicy::new();
        assert!(matches!(p.decide(&snap(Some(Content::Text("secret".into())), true), None),
                         Decision::Skip(SkipReason::Concealed)));
    }

    #[test]
    fn denylisted_app_is_skipped() {
        let mut p = CapturePolicy::new();
        p.app_denylist = vec!["com.1password".into()];
        let d = p.decide(&snap(Some(Content::Text("x".into())), false), Some(&app("com.1password")));
        assert!(matches!(d, Decision::Skip(SkipReason::DeniedApp)));
    }

    #[test]
    fn ignore_regex_skips_matching_content() {
        let mut p = CapturePolicy::new();
        p.ignore_regexes = vec![regex::Regex::new(r"AKIA[0-9A-Z]{16}").unwrap()];
        let d = p.decide(&snap(Some(Content::Text("AKIA1234567890ABCDEF".into())), false), None);
        assert!(matches!(d, Decision::Skip(SkipReason::IgnoredContent)));
    }

    #[test]
    fn normal_text_is_kept() {
        let p = CapturePolicy::new();
        let d = p.decide(&snap(Some(Content::Text("just a note".into())), false), Some(&app("com.ghostty")));
        assert!(matches!(d, Decision::Keep));
    }
}
