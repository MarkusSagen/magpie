use crate::traits::{Autostart, Clipboard};

pub fn platform_clipboard() -> Result<Box<dyn Clipboard>, String> {
    #[cfg(target_os = "macos")]
    {
        Ok(Box::new(super::macos::MacClipboard::new()?))
    }
    #[cfg(target_os = "windows")]
    {
        Ok(Box::new(super::windows::WinClipboard::new()?))
    }
    #[cfg(target_os = "linux")]
    {
        Ok(Box::new(super::linux::LinuxClipboard::new()?))
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        Err("unsupported OS".into())
    }
}

pub fn platform_autostart(exe_path: &str) -> Box<dyn Autostart> {
    #[cfg(target_os = "macos")]
    {
        let path = home_dir().join("Library/LaunchAgents/io.magpie.agent.plist");
        Box::new(super::autostart::MacAutostart {
            plist_path: path,
            label: "io.magpie.agent".into(),
            program: exe_path.to_string(),
        })
    }
    #[cfg(target_os = "linux")]
    {
        let path = home_dir().join(".config/autostart/magpie.desktop");
        Box::new(super::autostart_linux::LinuxAutostart {
            desktop_path: path,
            name: "Magpie".into(),
            exec: exe_path.to_string(),
        })
    }
    #[cfg(target_os = "windows")]
    {
        Box::new(super::autostart_windows::WinAutostart {
            value_name: "Magpie".into(),
            exe_path: exe_path.to_string(),
        })
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        let _ = exe_path;
        panic!("unsupported OS")
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn home_dir() -> std::path::PathBuf {
    std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("."))
}
