//! Import bookmarks from other browsers. Chrome/Chromium store a JSON `Bookmarks`
//! file; Firefox uses `places.sqlite` (read via magpie-core); Safari stores a
//! binary (or XML) plist, read via the `plist` crate. Reading Safari's file may
//! require granting Full Disk Access to the terminal/Magpie in System Settings.
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

/// Parse a Safari `Bookmarks.plist` (binary or xml) into (title, url) pairs, http(s)
/// only. Never panics; returns Err on unreadable/invalid input.
pub fn parse_safari_bookmarks(path: &Path) -> Result<Vec<(String, String)>, String> {
    let root = plist::Value::from_file(path).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    walk_safari(&root, &mut out);
    Ok(out)
}

fn walk_safari(node: &plist::Value, out: &mut Vec<(String, String)>) {
    if let Some(dict) = node.as_dictionary() {
        let is_leaf =
            dict.get("WebBookmarkType").and_then(|t| t.as_string()) == Some("WebBookmarkTypeLeaf");
        if is_leaf {
            if let Some(url) = dict.get("URLString").and_then(|u| u.as_string()) {
                if url.starts_with("http") {
                    let title = dict
                        .get("URIDictionary")
                        .and_then(|d| d.as_dictionary())
                        .and_then(|d| d.get("title"))
                        .and_then(|t| t.as_string())
                        .unwrap_or(url)
                        .to_string();
                    out.push((title, url.to_string()));
                }
            }
        }
        if let Some(children) = dict.get("Children").and_then(|c| c.as_array()) {
            for c in children {
                walk_safari(c, out);
            }
        }
    }
}

/// Default macOS Safari bookmarks file, if present.
pub fn default_safari_bookmarks() -> Option<PathBuf> {
    let p = dirs::home_dir()?.join("Library/Safari/Bookmarks.plist");
    p.exists().then_some(p)
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
        "safari" => {
            let p = default_safari_bookmarks().ok_or(
                "Safari bookmarks not found (Reading Safari's file may require granting Full \
                 Disk Access to your terminal/Magpie in System Settings › Privacy & Security)",
            )?;
            parse_safari_bookmarks(&p)
        }
        other => {
            let path = Path::new(other);
            if !path.exists() {
                return Err(format!("no such browser or file: {other}"));
            }
            match path.extension().and_then(|e| e.to_str()) {
                Some("sqlite") => {
                    magpie_core::read_firefox_bookmarks(path).map_err(|e| e.to_string())
                }
                Some("plist") => parse_safari_bookmarks(path),
                _ => Ok(parse_chrome_bookmarks(
                    &std::fs::read_to_string(path).map_err(|e| e.to_string())?,
                )),
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

    fn safari_leaf(title: &str, url: &str) -> plist::Value {
        let mut uri_dict = plist::Dictionary::new();
        uri_dict.insert("title".to_string(), plist::Value::String(title.to_string()));
        let mut leaf = plist::Dictionary::new();
        leaf.insert(
            "WebBookmarkType".to_string(),
            plist::Value::String("WebBookmarkTypeLeaf".to_string()),
        );
        leaf.insert(
            "URLString".to_string(),
            plist::Value::String(url.to_string()),
        );
        leaf.insert(
            "URIDictionary".to_string(),
            plist::Value::Dictionary(uri_dict),
        );
        plist::Value::Dictionary(leaf)
    }

    fn safari_folder(children: Vec<plist::Value>) -> plist::Value {
        let mut folder = plist::Dictionary::new();
        folder.insert(
            "WebBookmarkType".to_string(),
            plist::Value::String("WebBookmarkTypeList".to_string()),
        );
        folder.insert("Children".to_string(), plist::Value::Array(children));
        plist::Value::Dictionary(folder)
    }

    #[test]
    fn parses_safari_tree() {
        let tmp =
            std::env::temp_dir().join(format!("magpie-safari-test-{}.plist", std::process::id()));

        let root = safari_folder(vec![
            safari_leaf("Rust", "https://rust-lang.org"),
            safari_folder(vec![
                safari_leaf("GH", "https://github.com"),
                safari_leaf("skip-me", "ftp://skip"),
            ]),
        ]);
        root.to_file_binary(&tmp).expect("write fixture plist");

        let got = parse_safari_bookmarks(&tmp).expect("parse fixture plist");
        assert_eq!(
            got,
            vec![
                ("Rust".to_string(), "https://rust-lang.org".to_string()),
                ("GH".to_string(), "https://github.com".to_string()),
            ]
        );

        assert!(parse_safari_bookmarks(Path::new("/no/such/Bookmarks.plist")).is_err());

        let _ = std::fs::remove_file(&tmp);
    }
}
