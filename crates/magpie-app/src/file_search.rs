//! File-search engine (Phase 15): walks the chosen locations with ripgrep's
//! `ignore` crate (respecting `.gitignore`, the global excludes, and Magpie's own
//! `search/.ignore`) and matches file names and contents with `grep-searcher` +
//! `grep-regex`. Runs synchronously with hard caps; the caller runs it on a
//! background thread and cancels stale runs via a generation counter.

use grep_matcher::Matcher;
use grep_regex::{RegexMatcher, RegexMatcherBuilder};
use grep_searcher::sinks::UTF8;
use grep_searcher::{BinaryDetection, SearcherBuilder};
use ignore::WalkBuilder;
use magpie_core::filesearch::{parse_sed, CaseMode, MatchMode};
use std::path::{Path, PathBuf};

/// One matching line within a file.
#[derive(Debug, Clone)]
pub struct MatchLine {
    pub line_no: u64,
    pub text: String,
    /// Byte range of the (first) match within `text`, for highlighting.
    pub match_start: usize,
    pub match_end: usize,
}

/// One file with its matches (content matches, or a name-only hit).
#[derive(Debug, Clone)]
pub struct FileHit {
    pub path: PathBuf,
    pub name: String,
    /// Directory shown next to the name, relative to its search root when possible.
    pub dir_rel: String,
    pub matches: Vec<MatchLine>,
    pub name_only: bool,
}

/// The full result set for one search.
#[derive(Debug, Clone, Default)]
pub struct SearchResults {
    pub files: Vec<FileHit>,
    pub total_matches: usize,
    pub truncated: bool,
}

/// Which result set to build.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    All,
    Names,
    Contents,
    Documents,
}

/// Caps so a huge tree can't hang the UI.
pub struct Limits {
    pub max_files: usize,
    pub max_total_matches: usize,
    pub max_per_file: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_files: 300,
            max_total_matches: 2000,
            max_per_file: 50,
        }
    }
}

/// Expand a leading `~` to the home directory.
pub fn expand_tilde(p: &str) -> PathBuf {
    if let Some(rest) = p.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    if p == "~" {
        if let Some(home) = dirs::home_dir() {
            return home;
        }
    }
    PathBuf::from(p)
}

/// Path to Magpie's own ignore file (same location on every OS).
pub fn magpie_ignore_path() -> PathBuf {
    let base = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| dirs::home_dir().unwrap_or_default().join(".config"));
    base.join("magpie/search/.ignore")
}

fn build_matcher(query: &str, mode: MatchMode, case: CaseMode) -> Option<RegexMatcher> {
    let (pattern, insensitive) = match mode {
        MatchMode::Text => (regex::escape(query), case.is_insensitive(query)),
        MatchMode::Regex => (query.to_string(), case.is_insensitive(query)),
        MatchMode::Sed => {
            let s = parse_sed(query)?;
            (
                s.pattern.clone(),
                s.case_insensitive || case.is_insensitive(&s.pattern),
            )
        }
    };
    if pattern.is_empty() {
        return None;
    }
    RegexMatcherBuilder::new()
        .case_insensitive(insensitive)
        .build(&pattern)
        .ok()
}

fn dir_rel(path: &Path, root: &Path) -> String {
    let parent = path.parent().unwrap_or(path);
    parent
        .strip_prefix(root)
        .ok()
        .map(|p| p.to_string_lossy().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| parent.to_string_lossy().to_string())
}

/// Run a search across `locations` for `query`. `hidden`/`follow_symlinks`/`binary`
/// toggle the walker; ignore files are always honoured.
#[allow(clippy::too_many_arguments)]
pub fn run_search(
    locations: &[String],
    query: &str,
    mode: MatchMode,
    case: CaseMode,
    tab: Tab,
    hidden: bool,
    follow_symlinks: bool,
    limits: &Limits,
    cancelled: &dyn Fn() -> bool,
) -> SearchResults {
    let mut results = SearchResults::default();
    if query.is_empty() && mode != MatchMode::Sed {
        return results;
    }
    let matcher = match build_matcher(query, mode, case) {
        Some(m) => m,
        None => return results,
    };
    // A separate name matcher (same pattern) for the Names tab.
    let name_re = matcher.clone();

    let roots: Vec<PathBuf> = locations.iter().map(|l| expand_tilde(l)).collect();
    let want_names = matches!(tab, Tab::All | Tab::Names);
    let want_contents = matches!(tab, Tab::All | Tab::Contents | Tab::Documents);

    for root in &roots {
        if !root.exists() {
            continue;
        }
        let mut builder = WalkBuilder::new(root);
        builder
            .hidden(!hidden)
            .follow_links(follow_symlinks)
            .git_ignore(true)
            .git_global(true)
            .git_exclude(true);
        builder.add_custom_ignore_filename(".ignore");
        let mig = magpie_ignore_path();
        if mig.exists() {
            let _ = builder.add_ignore(&mig);
        }
        let mut searcher = SearcherBuilder::new()
            .binary_detection(BinaryDetection::quit(0))
            .line_number(true)
            .build();

        for dent in builder.build() {
            if cancelled()
                || results.files.len() >= limits.max_files
                || results.total_matches >= limits.max_total_matches
            {
                results.truncated = true;
                break;
            }
            let dent = match dent {
                Ok(d) => d,
                Err(_) => continue,
            };
            if !dent.file_type().map(|t| t.is_file()).unwrap_or(false) {
                continue;
            }
            let path = dent.path().to_path_buf();
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();

            // Names tab: match the file name.
            let name_hit = want_names && name_re.is_match(name.as_bytes()).unwrap_or(false);

            // Contents.
            let mut matches: Vec<MatchLine> = Vec::new();
            if want_contents {
                let per_file_cap = limits.max_per_file;
                let mut collected: Vec<MatchLine> = Vec::new();
                let m2 = matcher.clone();
                let _ = searcher.search_path(
                    &matcher,
                    &path,
                    UTF8(|line_no, line| {
                        if collected.len() >= per_file_cap {
                            return Ok(false);
                        }
                        let text = line.trim_end_matches(['\n', '\r']).to_string();
                        let (mut s, mut e) = (0usize, 0usize);
                        if let Ok(Some(m)) = m2.find(text.as_bytes()) {
                            s = m.start();
                            e = m.end();
                        }
                        collected.push(MatchLine {
                            line_no,
                            text,
                            match_start: s,
                            match_end: e,
                        });
                        Ok(true)
                    }),
                );
                matches = collected;
            }

            if matches.is_empty() && !name_hit {
                continue;
            }
            let count = matches.len().max(if name_hit { 1 } else { 0 });
            results.total_matches += matches.len();
            results.files.push(FileHit {
                path: path.clone(),
                name,
                dir_rel: dir_rel(&path, root),
                matches,
                name_only: name_hit && count > 0 && want_names && !want_contents,
            });
        }
        if results.truncated {
            break;
        }
    }
    results
}
