//! Never fail silently. Logging, panic surfacing, and abnormal-exit detection so a
//! background crash is loud and recoverable instead of an invisible death.
//!
//! The logic (log append/rotate, session state, crash-loop tracking) is pure and
//! cross-platform + unit-tested. Only three functions are per-OS: [`alert`],
//! [`notify`], [`reveal`] — macOS uses `osascript`/`open`; Windows/Linux are
//! documented follow-ups (see the install spec).

use std::io::Write;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

/// Rotate the log once it passes this size, so it never grows unbounded.
const LOG_CAP_BYTES: u64 = 512 * 1024;

/// Crash-loop window / threshold: N crashes within the window ⇒ a loop.
pub const CRASH_WINDOW_MS: i64 = 60_000;
pub const CRASH_THRESHOLD: usize = 3;

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Append a timestamped line to `log_path` (creating parents), rotating first if
/// the file has grown past the cap.
pub fn log_line(log_path: &Path, msg: &str) {
    if let Some(parent) = log_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(meta) = std::fs::metadata(log_path) {
        if meta.len() > LOG_CAP_BYTES {
            let _ = std::fs::rename(log_path, log_path.with_extension("log.1"));
        }
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
    {
        let _ = writeln!(f, "[{}] {}", now_ms(), msg);
    }
}

/// How the previous run ended, read from the session-state file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrevExit {
    /// No state file — first run (or fresh data dir).
    FirstRun,
    /// Previous run wrote "clean" on a real quit.
    Clean,
    /// State was still "running" ⇒ the previous run died abnormally.
    Crashed,
}

pub fn read_prev_exit(state_path: &Path) -> PrevExit {
    match std::fs::read_to_string(state_path) {
        Ok(s) if s.trim() == "clean" => PrevExit::Clean,
        Ok(_) => PrevExit::Crashed, // "running" (or anything unexpected) = abnormal
        Err(_) => PrevExit::FirstRun,
    }
}

/// Mark the session live (call at startup, after reading the previous state).
pub fn mark_running(state_path: &Path) {
    if let Some(parent) = state_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(state_path, b"running");
}

/// Mark a clean shutdown (call on a real quit, before exiting the event loop).
pub fn mark_clean(state_path: &Path) {
    let _ = std::fs::write(state_path, b"clean");
}

/// Append `at_ms` to the crash-timestamp log, keeping only the most recent 10.
pub fn record_crash(crashes_path: &Path, at_ms: i64) {
    let mut times = read_crash_times(crashes_path);
    times.push(at_ms);
    let start = times.len().saturating_sub(10);
    let kept: Vec<String> = times[start..].iter().map(|t| t.to_string()).collect();
    if let Some(parent) = crashes_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(crashes_path, kept.join("\n"));
}

pub fn read_crash_times(crashes_path: &Path) -> Vec<i64> {
    std::fs::read_to_string(crashes_path)
        .map(|s| {
            s.lines()
                .filter_map(|l| l.trim().parse::<i64>().ok())
                .collect()
        })
        .unwrap_or_default()
}

/// True when ≥`threshold` of `times` fall within `window_ms` before `now_ms`.
pub fn is_crash_loop(times: &[i64], now_ms: i64, window_ms: i64, threshold: usize) -> bool {
    times.iter().filter(|&&t| now_ms - t <= window_ms).count() >= threshold
}

/// Convenience: record this crash now and report whether we're in a loop.
pub fn record_and_check_loop(crashes_path: &Path) -> bool {
    let now = now_ms();
    record_crash(crashes_path, now);
    let times = read_crash_times(crashes_path);
    is_crash_loop(&times, now, CRASH_WINDOW_MS, CRASH_THRESHOLD)
}

// ---- per-OS surfacing (macOS implemented; win/linux are follow-ups) ----

/// Escape a string for embedding in an AppleScript double-quoted literal.
#[cfg(target_os = "macos")]
fn as_applescript_str(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Show a blocking native alert with `buttons`; returns the pressed button label.
/// The last button is the default. macOS: `osascript display dialog`.
#[cfg(target_os = "macos")]
pub fn alert(title: &str, message: &str, buttons: &[&str]) -> Option<String> {
    let btns = buttons
        .iter()
        .map(|b| as_applescript_str(b))
        .collect::<Vec<_>>()
        .join(", ");
    let default = buttons.last().copied().unwrap_or("OK");
    let script = format!(
        "display dialog {msg} with title {title} buttons {{{btns}}} default button {def} with icon caution giving up after 120",
        msg = as_applescript_str(message),
        title = as_applescript_str(title),
        def = as_applescript_str(default),
    );
    let out = std::process::Command::new("osascript")
        .arg("-e")
        .arg(script)
        .output()
        .ok()?;
    let s = String::from_utf8_lossy(&out.stdout);
    s.split("button returned:")
        .nth(1)
        .map(|r| r.split(',').next().unwrap_or("").trim().to_string())
}

/// Post a non-blocking notification. macOS: `osascript display notification`.
#[cfg(target_os = "macos")]
pub fn notify(title: &str, message: &str) {
    let script = format!(
        "display notification {msg} with title {title}",
        msg = as_applescript_str(message),
        title = as_applescript_str(title),
    );
    let _ = std::process::Command::new("osascript")
        .arg("-e")
        .arg(script)
        .spawn();
}

/// Reveal a file/path in the OS. macOS: `open -R` (reveal in Finder).
#[cfg(target_os = "macos")]
pub fn reveal(path: &Path) {
    let _ = std::process::Command::new("open")
        .arg("-R")
        .arg(path)
        .spawn();
}

// Non-macOS stubs — behavior implemented per-OS later (see the install spec). We
// still log; the alert falls back to stderr so nothing is truly silent in dev.
#[cfg(not(target_os = "macos"))]
pub fn alert(title: &str, message: &str, _buttons: &[&str]) -> Option<String> {
    eprintln!("[magpie] ALERT: {title} — {message}");
    None
}
#[cfg(not(target_os = "macos"))]
pub fn notify(title: &str, message: &str) {
    eprintln!("[magpie] NOTIFY: {title} — {message}");
}
#[cfg(not(target_os = "macos"))]
pub fn reveal(_path: &Path) {}

/// Rate-limit gate for the panic alert: returns true at most once per 30s.
fn alert_allowed() -> bool {
    use std::sync::atomic::{AtomicI64, Ordering};
    static LAST_ALERT_MS: AtomicI64 = AtomicI64::new(0);
    let now = now_ms();
    let last = LAST_ALERT_MS.load(Ordering::Relaxed);
    if now - last >= 30_000 {
        LAST_ALERT_MS.store(now, Ordering::Relaxed);
        true
    } else {
        false
    }
}

/// Install a panic hook that logs the panic (+ backtrace), leaves the session
/// marked "running" (so the next launch also notices), and surfaces a native
/// alert. Fires for panics on any thread.
pub fn install_panic_hook(log_path: std::path::PathBuf) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let loc = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_else(|| "?".into());
        let payload = if let Some(s) = info.payload().downcast_ref::<&str>() {
            (*s).to_string()
        } else if let Some(s) = info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            "<non-string panic>".to_string()
        };
        let thread = std::thread::current()
            .name()
            .unwrap_or("unnamed")
            .to_string();
        let msg = format!("PANIC [{thread}] at {loc}: {payload}");
        log_line(&log_path, &msg);
        let bt = std::backtrace::Backtrace::force_capture();
        log_line(&log_path, &format!("backtrace:\n{bt}"));
        // Always log; rate-limit the blocking alert so a repeating background-thread
        // panic can't spam modal dialogs (min 30s between alerts).
        if alert_allowed() {
            let chosen = alert(
                "Magpie stopped working",
                &format!("{msg}\n\nA log was saved to:\n{}", log_path.display()),
                &["View Log", "Quit"],
            );
            if chosen.as_deref() == Some("View Log") {
                reveal(&log_path);
            }
        }
        previous(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    // Unique dir per test — tests run in parallel and each `remove_dir_all`s, so a
    // shared dir would let one test delete another's files mid-run.
    fn tmp(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("magpie-diag-{}-{}", std::process::id(), name))
    }

    #[test]
    fn prev_exit_states() {
        let dir = tmp("prev-exit");
        std::fs::create_dir_all(&dir).unwrap();
        let state = dir.join("session");
        std::fs::remove_file(&state).ok();
        assert_eq!(read_prev_exit(&state), PrevExit::FirstRun);
        mark_running(&state);
        assert_eq!(read_prev_exit(&state), PrevExit::Crashed); // still "running" = abnormal
        mark_clean(&state);
        assert_eq!(read_prev_exit(&state), PrevExit::Clean);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn crash_loop_detection() {
        // 3 crashes within 60s at now=100_000 → loop; spread out → not.
        let tight = [70_000_i64, 80_000, 95_000];
        assert!(is_crash_loop(
            &tight,
            100_000,
            CRASH_WINDOW_MS,
            CRASH_THRESHOLD
        ));
        let sparse = [1_000_i64, 40_000, 95_000];
        assert!(!is_crash_loop(
            &sparse,
            100_000,
            CRASH_WINDOW_MS,
            CRASH_THRESHOLD
        ));
    }

    #[test]
    fn record_crash_keeps_last_ten() {
        let dir = tmp("record-crash");
        std::fs::create_dir_all(&dir).unwrap();
        let c = dir.join("crashes");
        std::fs::remove_file(&c).ok();
        for i in 0..15 {
            record_crash(&c, i);
        }
        let times = read_crash_times(&c);
        assert_eq!(times.len(), 10);
        assert_eq!(times.first().copied(), Some(5)); // oldest kept is the 6th (index 5)
        assert_eq!(times.last().copied(), Some(14));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn log_rotates_when_large() {
        let dir = tmp("log-rotate");
        std::fs::create_dir_all(&dir).unwrap();
        let log = dir.join("magpie.log");
        std::fs::write(&log, vec![b'x'; (LOG_CAP_BYTES + 1) as usize]).unwrap();
        log_line(&log, "after-rotate");
        assert!(log.with_extension("log.1").exists());
        let fresh = std::fs::read_to_string(&log).unwrap();
        assert!(fresh.contains("after-rotate") && fresh.len() < 200);
        std::fs::remove_dir_all(&dir).ok();
    }
}
