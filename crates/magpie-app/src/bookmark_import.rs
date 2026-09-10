//! Import bookmarks from other browsers. Chrome/Chromium store a JSON `Bookmarks`
//! file; Firefox uses `places.sqlite` (read via magpie-core). Safari's binary plist
//! is not yet supported.
use std::path::{Path, PathBuf};

/// Parse a Chrome/Chromium `Bookmarks` JSON string into (title, url) pairs
/// (http(s) only). Never panics; returns empty on malformed input.
pub fn parse_chrome_bookmarks(json: &str) -> Vec<(String, String)> {
    let v: serde_json::Value = match serde_json::from_str(json) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let mut out = Vec::new();
    if let Some(roots) = v.get("roots").and_then(|r| r.as_object()) {
        for node in roots.values() {
            walk(node, &mut out);
        }
    }
    out
}

fn walk(node: &serde_json::Value, out: &mut Vec<(String, String)>) {
    if node.get("type").and_then(|t| t.as_str()) == Some("url") {
        let name = node.get("name").and_then(|x| x.as_str()).unwrap_or("");
        if let Some(url) = node.get("url").and_then(|x| x.as_str()) {
            if url.starts_with("http") {
                out.push((name.to_string(), url.to_string()));
            }
        }
        return;
    }
    if let Some(children) = node.get("children").and_then(|c| c.as_array()) {
        for c in children {
            walk(c, out);
        }
    }
}

/// Default macOS Chrome bookmarks file, if present.
pub fn default_chrome_path() -> Option<PathBuf> {
    let p = dirs::home_dir()?.join("Library/Application Support/Google/Chrome/Default/Bookmarks");
    p.exists().then_some(p)
}

/// Default macOS Firefox `places.sqlite` (first profile that has one), if present.
pub fn default_firefox_places() -> Option<PathBuf> {
    let profiles = dirs::home_dir()?.join("Library/Application Support/Firefox/Profiles");
    let entries = std::fs::read_dir(&profiles).ok()?;
    for e in entries.flatten() {
        let places = e.path().join("places.sqlite");
        if places.exists() {
            return Some(places);
        }
    }
    None
}

/// Resolve a `--import-bookmarks` argument to (title, url) pairs. `which` is a
/// browser name ("chrome"/"firefox"/"safari") or a filesystem path.
pub fn resolve_import(which: &str) -> Result<Vec<(String, String)>, String> {
    match which {
        "chrome" | "chromium" => {
            let p = default_chrome_path().ok_or("Chrome bookmarks not found")?;
            Ok(parse_chrome_bookmarks(
                &std::fs::read_to_string(&p).map_err(|e| e.to_string())?,
            ))
        }
        "firefox" => {
            let p = default_firefox_places().ok_or("Firefox places.sqlite not found")?;
            magpie_core::read_firefox_bookmarks(&p).map_err(|e| e.to_string())
        }
        "safari" => Err("Safari import is not supported yet (binary plist)".to_string()),
        other => {
            let path = Path::new(other);
            if !path.exists() {
                return Err(format!("no such browser or file: {other}"));
            }
            if path.extension().and_then(|e| e.to_str()) == Some("sqlite") {
                magpie_core::read_firefox_bookmarks(path).map_err(|e| e.to_string())
            } else {
                Ok(parse_chrome_bookmarks(
                    &std::fs::read_to_string(path).map_err(|e| e.to_string())?,
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_chrome_tree() {
        let json = r#"{"roots":{"bookmark_bar":{"children":[
            {"type":"url","name":"Rust","url":"https://rust-lang.org"},
            {"type":"folder","name":"Dev","children":[
                {"type":"url","name":"GH","url":"https://github.com"},
                {"type":"url","name":"ftp","url":"ftp://skip"}
            ]}
        ]},"other":{"children":[]}}}"#;
        let got = parse_chrome_bookmarks(json);
        assert_eq!(
            got,
            vec![
                ("Rust".to_string(), "https://rust-lang.org".to_string()),
                ("GH".to_string(), "https://github.com".to_string()),
            ]
        );
        assert!(parse_chrome_bookmarks("not json").is_empty());
    }
}
