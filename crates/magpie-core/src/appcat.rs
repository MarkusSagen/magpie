//! App source **categories** for streamlined clipboard search (e.g. `browser` to
//! narrow to clips copied from any web browser). Pure, name-based classification: a
//! category is a set of case-insensitive substrings matched against an app's name /
//! identifier. No OS calls, no bundle-id lookup — matching by name is robust for the
//! real apps people copy from, and keeps this a plain, testable function the search
//! layer can turn straight into SQL `LIKE` clauses.

/// The canonical categories, each with the substrings that identify a member app.
/// Order is stable but irrelevant to matching. Substrings are lowercase and matched
/// against `lower(display_name || ' ' || identifier)`.
const CATEGORIES: &[(&str, &[&str])] = &[
    (
        "browser",
        &[
            "safari",
            "chrome",
            "chromium",
            "vivaldi",
            "firefox",
            "arc",
            "brave",
            "edge",
            "opera",
            "orion",
            "tor browser",
            "thorium",
            "zen browser",
        ],
    ),
    (
        "terminal",
        &[
            "terminal",
            "iterm",
            "ghostty",
            "alacritty",
            "kitty",
            "wezterm",
            "warp",
            "tabby",
            "hyper",
            "konsole",
            "rio",
        ],
    ),
    (
        "editor",
        &[
            "visual studio code",
            "vscode",
            "code - insiders",
            "cursor",
            "windsurf",
            "sublime text",
            "zed",
            "intellij",
            "pycharm",
            "webstorm",
            "goland",
            "clion",
            "rider",
            "phpstorm",
            "rubymine",
            "datagrip",
            "xcode",
            "android studio",
            "neovim",
            "nvim",
            "emacs",
            "notepad++",
            "textmate",
            "bbedit",
            "jetbrains",
            "fleet",
        ],
    ),
    (
        "chat",
        &[
            "slack",
            "discord",
            "telegram",
            "signal",
            "whatsapp",
            "messages",
            "microsoft teams",
            "teams",
            "zoom",
            "skype",
            "element",
            "mattermost",
            "messenger",
        ],
    ),
    (
        "notes",
        &[
            "notion", "obsidian", "bear", "notes", "logseq", "craft", "roam", "joplin", "typora",
            "anytype",
        ],
    ),
    (
        "design",
        &[
            "figma",
            "sketch",
            "photoshop",
            "illustrator",
            "affinity",
            "canva",
            "framer",
            "pixelmator",
            "gimp",
            "inkscape",
        ],
    ),
];

/// Resolve a user-typed word to a canonical category name, if it names one. Accepts
/// the category itself plus common synonyms/plurals (`browsers`→`browser`,
/// `ide`→`editor`, `term`→`terminal`). Returns `None` for anything else so ordinary
/// search words are never mistaken for a category.
pub fn canonical_category(word: &str) -> Option<&'static str> {
    match word.trim().to_lowercase().as_str() {
        "browser" | "browsers" | "web" => Some("browser"),
        "terminal" | "terminals" | "term" | "shell" | "console" => Some("terminal"),
        "editor" | "editors" | "ide" | "ides" => Some("editor"),
        "chat" | "chats" | "messaging" | "messenger" => Some("chat"),
        "notes" | "note" | "notetaking" => Some("notes"),
        "design" | "designer" => Some("design"),
        _ => None,
    }
}

/// Like [`canonical_category`] but restricted to words that are unambiguous as a
/// **bare** token — strongly app-associated and rare as clipboard content. Used when a
/// category has no `cat:`/`is:` prefix, so a plain search for common words like `note`,
/// `chat`, `web` or `design` is never silently turned into a source filter. Prefix them
/// (`cat:notes`) to filter by those categories.
pub fn canonical_category_bare(word: &str) -> Option<&'static str> {
    match word.trim().to_lowercase().as_str() {
        "browser" | "browsers" => Some("browser"),
        "terminal" | "terminals" => Some("terminal"),
        "editor" | "editors" => Some("editor"),
        _ => None,
    }
}

/// The name substrings that identify apps in `category` (already canonical). Empty
/// slice for an unknown category. The search layer turns each into a `LIKE '%sub%'`.
pub fn category_patterns(category: &str) -> &'static [&'static str] {
    CATEGORIES
        .iter()
        .find(|(c, _)| *c == category)
        .map(|(_, pats)| *pats)
        .unwrap_or(&[])
}

/// Classify an app by name — the category it belongs to, if any. Useful for tagging
/// captured apps or for UI; the search path uses [`category_patterns`] directly in SQL.
pub fn classify(name: &str, identifier: &str) -> Option<&'static str> {
    let hay = format!("{} {}", name, identifier).to_lowercase();
    CATEGORIES
        .iter()
        .find(|(_, pats)| pats.iter().any(|p| hay.contains(p)))
        .map(|(c, _)| *c)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_accepts_synonyms_and_rejects_words() {
        assert_eq!(canonical_category("browser"), Some("browser"));
        assert_eq!(canonical_category("Browsers"), Some("browser"));
        assert_eq!(canonical_category("IDE"), Some("editor"));
        assert_eq!(canonical_category("term"), Some("terminal"));
        assert_eq!(canonical_category("notes"), Some("notes"));
        // Not a category — must stay ordinary search text.
        assert_eq!(canonical_category("enablebanking"), None);
        assert_eq!(canonical_category("todo"), None);
    }

    #[test]
    fn classify_matches_real_apps() {
        assert_eq!(classify("Vivaldi", "Vivaldi"), Some("browser"));
        assert_eq!(
            classify("Google Chrome", "com.google.Chrome"),
            Some("browser")
        );
        assert_eq!(
            classify("Ghostty", "com.mitchellh.ghostty"),
            Some("terminal")
        );
        assert_eq!(classify("Cursor", "Cursor"), Some("editor"));
        assert_eq!(classify("Slack", "Slack"), Some("chat"));
        assert_eq!(classify("Some Unknown App", "x.y.z"), None);
    }

    #[test]
    fn patterns_lookup() {
        assert!(category_patterns("browser").contains(&"vivaldi"));
        assert!(category_patterns("nonsense").is_empty());
    }
}
