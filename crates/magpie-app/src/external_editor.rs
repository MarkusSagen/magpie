//! Open an entry's full text in the user's editor ($VISUAL/$EDITOR) or, failing
//! that, the OS default handler for a text file.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Write `text` to a temp file (named from `key` for stability) and open it.
pub fn open_text(text: &str, key: &str) -> std::io::Result<PathBuf> {
    let path = temp_path(key);
    std::fs::write(&path, text)?;
    open_path(&path);
    Ok(path)
}

fn temp_path(key: &str) -> PathBuf {
    let safe: String = key
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(24)
        .collect();
    let name = if safe.is_empty() {
        "clip".to_string()
    } else {
        safe
    };
    std::env::temp_dir().join(format!("magpie-{name}.txt"))
}

fn open_path(path: &Path) {
    // Prefer $VISUAL then $EDITOR (GUI editors like code/zed/subl work; a purely
    // terminal editor needs a TTY we don't have and will just no-op — that's the
    // user's config choice). Fall back to the OS default text handler.
    let editor = std::env::var_os("VISUAL")
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var_os("EDITOR").filter(|s| !s.is_empty()));
    if let Some(ed) = editor {
        #[cfg(unix)]
        {
            // Through a shell so `$EDITOR` with args (e.g. "code -n") is honored.
            let cmd = format!("{} \"$1\"", ed.to_string_lossy());
            if Command::new("sh")
                .arg("-c")
                .arg(cmd)
                .arg("sh")
                .arg(path)
                .spawn()
                .is_ok()
            {
                return;
            }
        }
        #[cfg(not(unix))]
        {
            if Command::new(ed).arg(path).spawn().is_ok() {
                return;
            }
        }
    }
    #[cfg(target_os = "macos")]
    let _ = Command::new("open").arg(path).spawn();
    #[cfg(target_os = "linux")]
    let _ = Command::new("xdg-open").arg(path).spawn();
    #[cfg(target_os = "windows")]
    let _ = Command::new("cmd")
        .args(["/C", "start", ""])
        .arg(path)
        .spawn();
}

#[cfg(test)]
mod tests {
    use super::temp_path;

    #[test]
    fn temp_path_is_sanitized_txt() {
        let p = temp_path("abc/../def 123!");
        let name = p.file_name().unwrap().to_string_lossy().to_string();
        assert!(name.starts_with("magpie-"));
        assert!(name.ends_with(".txt"));
        assert!(!name.contains('/'));
        assert!(!name.contains(' '));
    }

    #[test]
    fn empty_key_falls_back() {
        let p = temp_path("");
        assert!(p.file_name().unwrap().to_string_lossy().contains("clip"));
    }
}
