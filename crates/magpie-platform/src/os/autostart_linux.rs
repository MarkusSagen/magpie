use crate::traits::Autostart;
use std::path::PathBuf;

pub fn desktop_entry(name: &str, exec: &str) -> String {
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name={name}\n\
         Exec={exec}\n\
         X-GNOME-Autostart-enabled=true\n\
         Terminal=false\n"
    )
}

pub struct LinuxAutostart {
    pub desktop_path: PathBuf,
    pub name: String,
    pub exec: String,
}

impl Autostart for LinuxAutostart {
    fn is_enabled(&self) -> bool {
        self.desktop_path.exists()
    }

    fn set_enabled(&self, on: bool) -> Result<(), String> {
        if on {
            if let Some(parent) = self.desktop_path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            std::fs::write(&self.desktop_path, desktop_entry(&self.name, &self.exec))
                .map_err(|e| e.to_string())
        } else {
            match std::fs::remove_file(&self.desktop_path) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(e.to_string()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_entry_has_exec_and_autostart_flag() {
        let d = desktop_entry("Magpie", "/usr/bin/magpie");
        assert!(d.contains("Exec=/usr/bin/magpie"));
        assert!(d.contains("X-GNOME-Autostart-enabled=true"));
        assert!(d.contains("[Desktop Entry]"));
    }

    #[test]
    fn set_enabled_writes_and_removes() {
        let dir = std::env::temp_dir().join(format!("magpie-xdg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("magpie.desktop");
        let a = LinuxAutostart {
            desktop_path: path.clone(),
            name: "Magpie".into(),
            exec: "/usr/bin/magpie".into(),
        };
        assert!(!a.is_enabled());
        a.set_enabled(true).unwrap();
        assert!(a.is_enabled() && path.exists());
        a.set_enabled(false).unwrap();
        assert!(!path.exists());
        std::fs::remove_dir_all(&dir).ok();
    }
}
