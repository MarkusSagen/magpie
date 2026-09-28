//! OS scheduler adapters (Phase 21): export a Magpie job to the native scheduler so it
//! fires even when Magpie is closed. The exported job only ever runs
//! `magpie --run-job <id>` — the real command lives in the encrypted DB and is
//! hash-verified at run time, so an edited unit can't run something else. Magpie only
//! ever creates/removes jobs it namespaced (`io.magpie.job.<id>`); it never touches the
//! user's own crontab.

/// One launchd-style calendar interval (key → value), e.g. `[("Minute",0),("Hour",2)]`.
/// An empty dict means "every minute". Produced by `magpie_core::cron::launchd_intervals`.
pub type Interval = Vec<(&'static str, i64)>;

/// The namespaced label/id for a Magpie job.
pub fn job_label(id: i64) -> String {
    format!("io.magpie.job.{id}")
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Build a launchd plist that runs `exe --run-job <id>` on the given intervals.
#[cfg(target_os = "macos")]
fn launchd_plist(id: i64, intervals: &[Interval], exe: &str) -> String {
    let mut cal = String::new();
    for dict in intervals {
        cal.push_str("      <dict>\n");
        for (k, v) in dict {
            cal.push_str(&format!("        <key>{k}</key><integer>{v}</integer>\n"));
        }
        cal.push_str("      </dict>\n");
    }
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>{label}</string>
  <key>ProgramArguments</key>
  <array>
    <string>{exe}</string>
    <string>--run-job</string>
    <string>{id}</string>
  </array>
  <key>StartCalendarInterval</key>
  <array>
{cal}  </array>
  <key>ProcessType</key><string>Background</string>
</dict>
</plist>
"#,
        label = xml_escape(&job_label(id)),
        exe = xml_escape(exe),
    )
}

#[cfg(target_os = "macos")]
fn plist_path(id: i64) -> Option<std::path::PathBuf> {
    dirs_home().map(|h| {
        h.join("Library/LaunchAgents")
            .join(format!("{}.plist", job_label(id)))
    })
}

#[cfg(target_os = "macos")]
fn dirs_home() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME").map(std::path::PathBuf::from)
}

/// Install (or replace) the OS scheduler entry for job `id`, running `exe --run-job
/// <id>` on the given schedule. The caller passes each platform's native schedule
/// (from `magpie_core::cron`): `launchd` intervals (macOS), `systemd` OnCalendar lines
/// (Linux), `schtasks` args (Windows). The one for this OS must be non-empty, else the
/// schedule is too complex to export and this refuses (keep it While open).
// Each `#[cfg]` block ends in `return` because it is not the last statement on every
// platform — only on the one it's compiled for. clippy sees the active block as final.
#[allow(clippy::needless_return)]
pub fn install_job(
    id: i64,
    exe: &std::path::Path,
    launchd: &[Interval],
    systemd: &[String],
    schtasks: &[String],
) -> Result<(), String> {
    let _ = (id, exe, launchd, systemd, schtasks);
    #[cfg(target_os = "macos")]
    {
        if launchd.is_empty() {
            return Err("schedule too complex for launchd — keep it While open".into());
        }
        let path = plist_path(id).ok_or("no HOME")?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::write(&path, launchd_plist(id, launchd, &exe.to_string_lossy()))
            .map_err(|e| e.to_string())?;
        let _ = std::process::Command::new("launchctl")
            .arg("unload")
            .arg(&path)
            .output();
        let out = std::process::Command::new("launchctl")
            .arg("load")
            .arg("-w")
            .arg(&path)
            .output()
            .map_err(|e| e.to_string())?;
        return if out.status.success() {
            Ok(())
        } else {
            Err(String::from_utf8_lossy(&out.stderr).into_owned())
        };
    }
    #[cfg(target_os = "linux")]
    {
        if systemd.is_empty() {
            return Err("schedule too complex for a systemd timer — keep it While open".into());
        }
        let dir = systemd_dir().ok_or("no HOME")?;
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let label = job_label(id);
        let service = format!(
            "[Unit]\nDescription=Magpie job {id}\n\n[Service]\nType=oneshot\nExecStart={} --run-job {id}\n",
            exe.to_string_lossy()
        );
        let cal = systemd
            .iter()
            .map(|c| format!("OnCalendar={c}"))
            .collect::<Vec<_>>()
            .join("\n");
        let timer = format!(
            "[Unit]\nDescription=Magpie job {id} timer\n\n[Timer]\n{cal}\nPersistent=true\n\n[Install]\nWantedBy=timers.target\n"
        );
        std::fs::write(dir.join(format!("{label}.service")), service).map_err(|e| e.to_string())?;
        std::fs::write(dir.join(format!("{label}.timer")), timer).map_err(|e| e.to_string())?;
        let _ = std::process::Command::new("systemctl")
            .args(["--user", "daemon-reload"])
            .output();
        let out = std::process::Command::new("systemctl")
            .args(["--user", "enable", "--now", &format!("{label}.timer")])
            .output()
            .map_err(|e| e.to_string())?;
        return if out.status.success() {
            Ok(())
        } else {
            Err(String::from_utf8_lossy(&out.stderr).into_owned())
        };
    }
    #[cfg(target_os = "windows")]
    {
        if schtasks.is_empty() {
            return Err("schedule too complex for Task Scheduler — keep it While open".into());
        }
        let tr = format!("\"{}\" --run-job {id}", exe.to_string_lossy());
        let out = std::process::Command::new("schtasks")
            .args(["/create", "/f", "/tn", &job_label(id), "/tr", &tr])
            .args(schtasks.iter().map(|s| s.as_str()))
            .output()
            .map_err(|e| e.to_string())?;
        return if out.status.success() {
            Ok(())
        } else {
            Err(String::from_utf8_lossy(&out.stderr).into_owned())
        };
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        Err("OS scheduling isn't supported on this platform — keep it While open".into())
    }
}

#[cfg(target_os = "linux")]
fn systemd_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config/systemd/user"))
}

/// Remove the OS scheduler entry for job `id` (idempotent).
pub fn remove_job(id: i64) -> Result<(), String> {
    let _ = id;
    #[cfg(target_os = "macos")]
    if let Some(path) = plist_path(id) {
        let _ = std::process::Command::new("launchctl")
            .arg("unload")
            .arg(&path)
            .output();
        let _ = std::fs::remove_file(&path);
    }
    #[cfg(target_os = "linux")]
    if let Some(dir) = systemd_dir() {
        let label = job_label(id);
        let _ = std::process::Command::new("systemctl")
            .args(["--user", "disable", "--now", &format!("{label}.timer")])
            .output();
        let _ = std::fs::remove_file(dir.join(format!("{label}.timer")));
        let _ = std::fs::remove_file(dir.join(format!("{label}.service")));
        let _ = std::process::Command::new("systemctl")
            .args(["--user", "daemon-reload"])
            .output();
    }
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("schtasks")
            .args(["/delete", "/f", "/tn", &job_label(id)])
            .output();
    }
    Ok(())
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[test]
    fn plist_runs_only_run_job_and_is_namespaced() {
        let iv = vec![vec![("Minute", 40), ("Hour", 8)]];
        let p = launchd_plist(7, &iv, "/Applications/Magpie.app/Contents/MacOS/magpie");
        assert!(p.contains("<string>io.magpie.job.7</string>"));
        // The only command is `magpie --run-job 7` — never the job's real command.
        assert!(p.contains("<string>--run-job</string>"));
        assert!(p.contains("<string>7</string>"));
        assert!(p.contains("<key>Minute</key><integer>40</integer>"));
        assert!(p.contains("<key>Hour</key><integer>8</integer>"));
    }

    #[test]
    fn install_refuses_too_complex() {
        // Empty launchd intervals on macOS ⇒ refused (keep it While open).
        let exe = std::path::Path::new("/bin/echo");
        assert!(install_job(1, exe, &[], &[], &[]).is_err());
    }
}
