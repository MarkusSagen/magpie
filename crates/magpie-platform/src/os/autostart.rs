use crate::traits::Autostart;
use std::path::PathBuf;

pub fn launch_agent_plist(label: &str, program: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{label}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{program}</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <dict>
        <key>SuccessfulExit</key>
        <false/>
    </dict>
</dict>
</plist>
"#
    )
}

pub struct MacAutostart {
    pub plist_path: PathBuf,
    pub label: String,
    pub program: String,
}

impl Autostart for MacAutostart {
    fn is_enabled(&self) -> bool {
        self.plist_path.exists()
    }

    fn set_enabled(&self, on: bool) -> Result<(), String> {
        if on {
            if let Some(parent) = self.plist_path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            let xml = launch_agent_plist(&self.label, &self.program);
            std::fs::write(&self.plist_path, xml).map_err(|e| e.to_string())
        } else {
            match std::fs::remove_file(&self.plist_path) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(e.to_string()),
            }
        }
    }

    fn take_effect_now(&self, on: bool) {
        // Best-effort so autostart applies without a re-login. `load`/`unload -w`
        // are deprecated but still work on current macOS and need no uid. Errors
        // are ignored — the plist still governs the next login regardless.
        let sub = if on { "load" } else { "unload" };
        let _ = std::process::Command::new("launchctl")
            .arg(sub)
            .arg("-w")
            .arg(&self.plist_path)
            .status();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plist_contains_label_program_and_runatload() {
        let xml = launch_agent_plist(
            "io.magpie.agent",
            "/Applications/Magpie.app/Contents/MacOS/magpie",
        );
        assert!(xml.contains("io.magpie.agent"));
        assert!(xml.contains("/Applications/Magpie.app/Contents/MacOS/magpie"));
        assert!(xml.contains("RunAtLoad"));
        // Auto-restart on crash, but NOT on a clean quit.
        assert!(xml.contains("KeepAlive"));
        assert!(xml.contains("SuccessfulExit"));
    }

    #[test]
    fn set_enabled_writes_and_removes_plist() {
        let dir =
            std::env::temp_dir().join(format!("magpie-autostart-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let plist = dir.join("io.magpie.agent.plist");
        let a = MacAutostart {
            plist_path: plist.clone(),
            label: "io.magpie.agent".into(),
            program: "/bin/magpie".into(),
        };
        assert!(!a.is_enabled());
        a.set_enabled(true).unwrap();
        assert!(a.is_enabled() && plist.exists());
        a.set_enabled(false).unwrap();
        assert!(!a.is_enabled() && !plist.exists());
        std::fs::remove_dir_all(&dir).ok();
    }
}
