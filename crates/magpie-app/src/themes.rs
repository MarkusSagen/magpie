//! The colour-theme catalogue. This is the Rust half of the theme system; the
//! matching palettes (colours) live in `Theme.palettes` in `ui/launcher.slint`.
//!
//! **The order here is the contract**: index N in `THEMES` must be palette N in
//! the Slint `palettes` array. The UI selects a palette by index; Rust persists
//! the stable string `id` and derives the native-window darkness from `dark`.

/// One theme's stable identity. `id` is what's written to `config.toml` (so
/// reordering the list never changes a user's saved choice — only the id does);
/// `name` is the human label (kept in sync with the Slint palette `name`);
/// `dark` drives the macOS titlebar appearance.
pub struct ThemeDef {
    pub id: &'static str,
    pub name: &'static str,
    pub dark: bool,
}

/// The catalogue, in palette-index order (see the module docs).
pub const THEMES: &[ThemeDef] = &[
    ThemeDef {
        id: "dark",
        name: "Dark",
        dark: true,
    },
    ThemeDef {
        id: "light",
        name: "Light",
        dark: false,
    },
    ThemeDef {
        id: "catppuccin-mocha",
        name: "Catppuccin Mocha",
        dark: true,
    },
    ThemeDef {
        id: "catppuccin-latte",
        name: "Catppuccin Latte",
        dark: false,
    },
    ThemeDef {
        id: "tokyo-night",
        name: "Tokyo Night",
        dark: true,
    },
    ThemeDef {
        id: "atom-one-dark",
        name: "Atom One Dark",
        dark: true,
    },
    ThemeDef {
        id: "dracula",
        name: "Dracula",
        dark: true,
    },
    ThemeDef {
        id: "nord",
        name: "Nord",
        dark: true,
    },
    ThemeDef {
        id: "gruvbox-dark",
        name: "Gruvbox Dark",
        dark: true,
    },
];

/// Catalogue index for a theme `id`; falls back to 0 (Dark) for an unknown id
/// (e.g. a config written by a newer build, or a typo).
pub fn index_of(id: &str) -> usize {
    THEMES.iter().position(|t| t.id == id).unwrap_or(0)
}

/// The stable `id` for a catalogue index; falls back to "dark" out of range.
pub fn id_of(index: usize) -> &'static str {
    THEMES.get(index).map(|t| t.id).unwrap_or("dark")
}

/// Whether the theme at `index` uses dark window chrome; defaults to true.
pub fn is_dark(index: usize) -> bool {
    THEMES.get(index).map(|t| t.dark).unwrap_or(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_id_and_index() {
        for (i, t) in THEMES.iter().enumerate() {
            assert_eq!(index_of(t.id), i, "index_of({})", t.id);
            assert_eq!(id_of(i), t.id);
            assert_eq!(is_dark(i), t.dark);
        }
    }

    #[test]
    fn unknown_id_falls_back_to_dark() {
        assert_eq!(index_of("no-such-theme"), 0);
        assert_eq!(id_of(9999), "dark");
        assert!(is_dark(9999));
    }

    #[test]
    fn first_two_are_the_legacy_dark_and_light_defaults() {
        // The `theme_dark` bool migration relies on these two positions.
        assert_eq!(THEMES[0].id, "dark");
        assert!(THEMES[0].dark);
        assert_eq!(THEMES[1].id, "light");
        assert!(!THEMES[1].dark);
    }
}
