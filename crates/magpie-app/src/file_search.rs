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

/// Folder suggestions for a location being typed (`~/co`, `/Users/x/`, `~/code/hobby/`):
/// subfolders of the fragment's parent whose name starts with the typed prefix
/// (case-insensitive; dot-folders only when the prefix starts with `.`), sorted, shown
/// in the user's own form with a trailing `/` so Tab can keep going deeper.
pub fn suggest_dirs(fragment: &str, limit: usize) -> Vec<String> {
    match dirs::home_dir() {
        Some(home) => suggest_dirs_with_home(fragment, &home, limit),
        None => Vec::new(),
    }
}

pub fn suggest_dirs_with_home(fragment: &str, home: &std::path::Path, limit: usize) -> Vec<String> {
    // Split into the typed parent ("~/code/") and the name prefix ("ho").
    let frag = if fragment.is_empty() || fragment == "~" {
        "~/"
    } else {
        fragment
    };
    let (shown_parent, prefix) = match frag.rfind('/') {
        Some(i) => (&frag[..=i], &frag[i + 1..]),
        None => ("~/", frag),
    };
    let parent = if let Some(rest) = shown_parent.strip_prefix("~/") {
        home.join(rest)
    } else {
        PathBuf::from(shown_parent)
    };
    let want_hidden = prefix.starts_with('.');
    let lower = prefix.to_lowercase();
    let Ok(rd) = std::fs::read_dir(&parent) else {
        return Vec::new();
    };
    let mut names: Vec<String> = rd
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| want_hidden || !n.starts_with('.'))
        .filter(|n| n.to_lowercase().starts_with(&lower))
        .collect();
    names.sort_by_key(|n| n.to_lowercase());
    names
        .into_iter()
        .take(limit)
        .map(|n| format!("{shown_parent}{n}/"))
        .collect()
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

/// Document types the Documents tab hands to `rga` (ripgrep-all).
pub const DOCUMENT_GLOBS: &[&str] = &[
    "*.pdf", "*.doc", "*.docx", "*.odt", "*.rtf", "*.epub", "*.pptx", "*.odp", "*.xlsx", "*.ods",
];

/// `rga` on PATH, if installed.
pub fn rga_path() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join("rga"))
        .find(|p| p.is_file())
}

/// One `rg --json` "match" event → `(path, line)`. rga reports page-prefixed lines
/// ("Page 3: …") and may omit `line_number` (→ 0). The first submatch is highlighted.
pub fn parse_rga_json_line(line: &str) -> Option<(PathBuf, MatchLine)> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    if v.get("type")?.as_str()? != "match" {
        return None;
    }
    let d = v.get("data")?;
    let path = PathBuf::from(d.get("path")?.get("text")?.as_str()?);
    let text = d
        .get("lines")?
        .get("text")?
        .as_str()?
        .trim_end_matches(['\n', '\r'])
        .to_string();
    let line_no = d.get("line_number").and_then(|n| n.as_u64()).unwrap_or(0);
    let (start, end) = d
        .get("submatches")
        .and_then(|s| s.get(0))
        .and_then(|m| {
            Some((
                m.get("start")?.as_u64()? as usize,
                m.get("end")?.as_u64()? as usize,
            ))
        })
        .filter(|(a, b)| {
            a <= b && *b <= text.len() && text.is_char_boundary(*a) && text.is_char_boundary(*b)
        })
        .unwrap_or((0, 0));
    Some((
        path,
        MatchLine {
            line_no,
            text,
            match_start: start,
            match_end: end,
        },
    ))
}

/// Search document files (PDF, Office, e-books) under `locations` with `rga`, reading
/// its JSON stream so a keystroke (`cancelled`) or the caps stop it early (the child is
/// killed). `Err` = rga missing or failed to start.
pub fn search_documents(
    locations: &[String],
    query: &str,
    mode: MatchMode,
    case: CaseMode,
    limits: &Limits,
    cancelled: &dyn Fn() -> bool,
) -> Result<SearchResults, String> {
    use std::io::{BufRead, BufReader};
    let rga =
        rga_path().ok_or("Install ripgrep-all (rga) to search PDFs, Office files and e-books")?;
    let mut cmd = std::process::Command::new(rga);
    cmd.arg("--json")
        .arg("--max-count")
        .arg(limits.max_per_file.to_string());
    cmd.arg(if case.is_insensitive(query) {
        "-i"
    } else {
        "-s"
    });
    if mode == MatchMode::Text {
        cmd.arg("-F");
    }
    for g in DOCUMENT_GLOBS {
        cmd.arg("-g").arg(g);
    }
    let roots: Vec<PathBuf> = locations.iter().map(|l| expand_tilde(l)).collect();
    cmd.arg("--").arg(query).args(&roots);
    cmd.stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("couldn't start rga: {e}"))?;
    let mut results = SearchResults::default();
    let mut index: std::collections::HashMap<PathBuf, usize> = std::collections::HashMap::new();
    if let Some(out) = child.stdout.take() {
        for line in BufReader::new(out).lines().map_while(Result::ok) {
            if cancelled() {
                break;
            }
            let Some((path, m)) = parse_rga_json_line(&line) else {
                continue;
            };
            let i = match index.get(&path) {
                Some(i) => *i,
                None => {
                    if results.files.len() >= limits.max_files {
                        results.truncated = true;
                        break;
                    }
                    let root = roots
                        .iter()
                        .find(|r| path.starts_with(r))
                        .cloned()
                        .unwrap_or_default();
                    results.files.push(FileHit {
                        name: path
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_default(),
                        dir_rel: dir_rel(&path, &root),
                        path: path.clone(),
                        matches: Vec::new(),
                        name_only: false,
                    });
                    index.insert(path, results.files.len() - 1);
                    results.files.len() - 1
                }
            };
            results.files[i].matches.push(m);
            results.total_matches += 1;
            if results.total_matches >= limits.max_total_matches {
                results.truncated = true;
                break;
            }
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    Ok(results)
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
    run_search_streaming(
        locations,
        query,
        mode,
        case,
        tab,
        hidden,
        follow_symlinks,
        limits,
        cancelled,
        &|_| {},
    )
}

/// How often partial results are handed to `on_progress` while a search runs.
pub const PROGRESS_EVERY: std::time::Duration = std::time::Duration::from_millis(200);

/// [`run_search`] that also reports the results found so far to `on_progress`
/// (at most every [`PROGRESS_EVERY`]), so big trees show hits as they're found.
#[allow(clippy::too_many_arguments)]
pub fn run_search_streaming(
    locations: &[String],
    query: &str,
    mode: MatchMode,
    case: CaseMode,
    tab: Tab,
    hidden: bool,
    follow_symlinks: bool,
    limits: &Limits,
    cancelled: &dyn Fn() -> bool,
    on_progress: &dyn Fn(&SearchResults),
) -> SearchResults {
    let mut last_emit = std::time::Instant::now();
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
            if last_emit.elapsed() >= PROGRESS_EVERY {
                on_progress(&results);
                last_emit = std::time::Instant::now();
            }
        }
        if results.truncated {
            break;
        }
    }
    results
}

#[cfg(test)]
mod suggest_tests {
    use super::suggest_dirs_with_home;

    #[test]
    fn suggests_matching_subfolders_in_the_users_form() {
        let home = std::env::temp_dir().join(format!("magpie-suggest-{}", std::process::id()));
        for d in [
            "code/hobby",
            "code/work",
            "Code-old",
            ".config",
            "Documents",
        ] {
            std::fs::create_dir_all(home.join(d)).unwrap();
        }
        std::fs::write(home.join("code/notes.txt"), "x").unwrap(); // files never suggested
        let s = |f: &str| suggest_dirs_with_home(f, &home, 10);
        assert_eq!(s("~/co"), vec!["~/code/", "~/Code-old/"]);
        assert_eq!(s("~/code/"), vec!["~/code/hobby/", "~/code/work/"]);
        assert_eq!(s("~/code/H"), vec!["~/code/hobby/"]);
        assert_eq!(s(""), vec!["~/code/", "~/Code-old/", "~/Documents/"]); // no dot-dirs
        assert_eq!(s("~/.c"), vec!["~/.config/"]);
        let abs = format!("{}/Doc", home.display());
        assert_eq!(s(&abs), vec![format!("{}/Documents/", home.display())]);
        assert!(s("~/nope/").is_empty());
        let _ = std::fs::remove_dir_all(&home);
    }
}

#[cfg(test)]
mod rga_tests {
    use super::parse_rga_json_line;

    #[test]
    fn parses_match_events_only() {
        let m = r#"{"type":"match","data":{"path":{"text":"/d/report.pdf"},"lines":{"text":"Page 3: the quarterly total\n"},"line_number":12,"absolute_offset":0,"submatches":[{"match":{"text":"quarterly"},"start":12,"end":21}]}}"#;
        let (p, l) = parse_rga_json_line(m).unwrap();
        assert_eq!(p.to_str(), Some("/d/report.pdf"));
        assert_eq!(l.text, "Page 3: the quarterly total");
        assert_eq!((l.line_no, l.match_start, l.match_end), (12, 12, 21));
        assert_eq!(&l.text[l.match_start..l.match_end], "quarterly");
        // null line number + bad offsets → 0 / no highlight, never a panic.
        let n = r#"{"type":"match","data":{"path":{"text":"/d/a.docx"},"lines":{"text":"x"},"line_number":null,"submatches":[{"match":{"text":"x"},"start":5,"end":9}]}}"#;
        let (_, l) = parse_rga_json_line(n).unwrap();
        assert_eq!((l.line_no, l.match_start, l.match_end), (0, 0, 0));
        for other in [
            r#"{"type":"begin","data":{"path":{"text":"/d/a.pdf"}}}"#,
            r#"{"type":"summary","data":{}}"#,
            "not json",
        ] {
            assert!(parse_rga_json_line(other).is_none());
        }
    }
}

#[cfg(test)]
mod streaming_tests {
    use super::*;

    #[test]
    fn streaming_matches_the_plain_search() {
        let dir = std::env::temp_dir().join(format!("magpie-stream-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for i in 0..20 {
            std::fs::write(dir.join(format!("f{i}.txt")), "alpha\nneedle here\nomega\n").unwrap();
        }
        let locs = vec![dir.to_string_lossy().to_string()];
        let run = |progress: &dyn Fn(&SearchResults)| {
            run_search_streaming(
                &locs,
                "needle",
                MatchMode::Text,
                CaseMode::Smart,
                Tab::All,
                false,
                false,
                &Limits::default(),
                &|| false,
                progress,
            )
        };
        let calls = std::cell::Cell::new(0usize);
        let streamed = run(&|r| {
            assert!(r.files.len() <= 20);
            calls.set(calls.get() + 1);
        });
        let plain = run_search(
            &locs,
            "needle",
            MatchMode::Text,
            CaseMode::Smart,
            Tab::All,
            false,
            false,
            &Limits::default(),
            &|| false,
        );
        assert_eq!(streamed.files.len(), 20);
        assert_eq!(streamed.total_matches, plain.total_matches);
        let _ = calls.get(); // progress is time-based; a fast run may not emit at all
        let _ = std::fs::remove_dir_all(&dir);
    }
}
