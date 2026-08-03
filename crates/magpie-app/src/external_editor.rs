//! Open an entry's full text in the user's editor.
//!
//! Order of preference: `$VISUAL`, then `$EDITOR`. A GUI editor (code/zed/subl…)
//! is spawned directly. A *terminal* editor (vim/nvim/emacsclient -t/nano/hx…) has
//! no TTY when spawned from a GUI app, so we open it **inside a terminal** —
//! `$TERMINAL`, else the terminal we were launched from (`$TERM_PROGRAM`), else a
//! detected one. If nothing works, fall back to the OS default text handler.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Write `text` to a temp file (named from `key` for stability) and open it.
pub fn open_text(text: &str, key: &str) -> std::io::Result<PathBuf> {
    let path = temp_path(key);
    std::fs::write(&path, text)?;
    launch(&path);
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

/// `$VISUAL` then `$EDITOR`, trimmed; `None` if neither is set/non-empty.
fn env_editor() -> Option<String> {
    for k in ["VISUAL", "EDITOR"] {
        if let Some(v) = std::env::var_os(k) {
            let s = v.to_string_lossy().trim().to_string();
            if !s.is_empty() {
                return Some(s);
            }
        }
    }
    None
}

/// basename of the first token of a command string (`/usr/bin/nvim -p` → `nvim`).
fn command_bin(cmd: &str) -> &str {
    cmd.split_whitespace()
        .next()
        .unwrap_or("")
        .rsplit('/')
        .next()
        .unwrap_or("")
}

/// Heuristic: does this editor command need a terminal? Unknown editors default to
/// terminal — routing a CLI editor to a terminal is correct, and routing a GUI
/// editor there merely leaves a spare terminal window; whereas mis-guessing a CLI
/// editor as GUI (spawning it with no TTY) silently does nothing.
fn is_terminal_editor(cmd: &str) -> bool {
    // Explicit terminal-mode flags (emacs -nw, emacsclient -t, …).
    if cmd.contains("-nw") || cmd.contains("--no-window-system") {
        return true;
    }
    let has_t_flag = cmd
        .split_whitespace()
        .skip(1)
        .any(|t| t == "-t" || t == "-nw");
    if has_t_flag {
        return true;
    }
    const GUI: &[&str] = &[
        "code",
        "code-insiders",
        "codium",
        "zed",
        "subl",
        "sublime_text",
        "cursor",
        "mate",
        "gvim",
        "mvim",
        "gedit",
        "kate",
        "notepad",
        "notepad++",
    ];
    !GUI.contains(&command_bin(cmd))
}

/// The argv to run `inner` inside a terminal, given the terminal's basename.
fn terminal_argv(bin: &str, term: &str, inner: &str) -> Vec<String> {
    let (b, t, i) = (bin, term.to_string(), inner.to_string());
    match b {
        // kitty runs the command directly (no `-e`).
        "kitty" => vec![t, "sh".into(), "-lc".into(), i],
        // wezterm spawns via `start -- <cmd>`.
        "wezterm" => vec![t, "start".into(), "--".into(), "sh".into(), "-lc".into(), i],
        // ghostty, alacritty, xterm, and most others accept `-e <cmd>`.
        _ => vec![t, "-e".into(), "sh".into(), "-lc".into(), i],
    }
}

/// Candidate terminal commands to try, most-preferred first.
fn terminal_candidates() -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if let Some(t) = std::env::var_os("TERMINAL") {
        let s = t.to_string_lossy().to_string();
        if !s.is_empty() {
            out.push(s);
        }
    }
    if let Ok(tp) = std::env::var("TERM_PROGRAM") {
        let mapped = match tp.as_str() {
            "ghostty" | "Ghostty" => "ghostty",
            "WezTerm" => "wezterm",
            "kitty" => "kitty",
            "Alacritty" => "alacritty",
            _ => "",
        };
        if !mapped.is_empty() {
            out.push(mapped.to_string());
        }
    }
    for t in ["ghostty", "kitty", "wezterm", "alacritty"] {
        out.push(t.to_string());
    }
    out
}

fn launch(path: &Path) {
    if let Some(ed) = env_editor() {
        if is_terminal_editor(&ed) {
            let inner = format!("{ed} \"{}\"", path.display());
            for term in terminal_candidates() {
                let bin = term.rsplit('/').next().unwrap_or(&term).to_string();
                let argv = terminal_argv(&bin, &term, &inner);
                let mut cmd = Command::new(&argv[0]);
                cmd.args(&argv[1..]);
                if cmd.spawn().is_ok() {
                    return;
                }
            }
        } else {
            // GUI editor: spawn directly (through a shell so args like `-n` work).
            #[cfg(unix)]
            {
                if Command::new("sh")
                    .arg("-c")
                    .arg(format!("{ed} \"$1\""))
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
                if Command::new(command_bin(&ed)).arg(path).spawn().is_ok() {
                    return;
                }
            }
        }
    }
    os_open(path);
}

/// Last resort: hand the file to the OS default text handler.
fn os_open(path: &Path) {
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
    use super::*;

    #[test]
    fn temp_path_is_sanitized_txt() {
        let p = temp_path("abc/../def 123!");
        let name = p.file_name().unwrap().to_string_lossy().to_string();
        assert!(name.starts_with("magpie-"));
        assert!(name.ends_with(".txt"));
        assert!(!name.contains('/') && !name.contains(' '));
    }

    #[test]
    fn empty_key_falls_back() {
        assert!(temp_path("")
            .file_name()
            .unwrap()
            .to_string_lossy()
            .contains("clip"));
    }

    #[test]
    fn detects_terminal_vs_gui_editors() {
        assert!(is_terminal_editor("nvim"));
        assert!(is_terminal_editor("vim"));
        assert!(is_terminal_editor("nano"));
        assert!(is_terminal_editor("emacsclient -t -a ''"));
        assert!(is_terminal_editor("emacs -nw"));
        assert!(is_terminal_editor("/opt/homebrew/bin/hx"));
        assert!(!is_terminal_editor("code"));
        assert!(!is_terminal_editor("code -n -w"));
        assert!(!is_terminal_editor("zed"));
        assert!(!is_terminal_editor("/usr/local/bin/subl -w"));
    }

    #[test]
    fn terminal_argv_styles() {
        assert_eq!(
            terminal_argv("ghostty", "ghostty", "nvim \"/tmp/x.txt\""),
            vec!["ghostty", "-e", "sh", "-lc", "nvim \"/tmp/x.txt\""]
        );
        assert_eq!(terminal_argv("kitty", "kitty", "vim f")[1], "sh");
        assert_eq!(terminal_argv("wezterm", "wezterm", "vim f")[1], "start");
    }
}
