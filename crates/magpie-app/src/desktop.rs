//! Desktop file/URL actions. Keep OS-specific launcher flags out of UI callbacks.

use std::path::Path;

pub fn open(target: impl AsRef<std::ffi::OsStr>) -> std::io::Result<()> {
    open::that_detached(target)
}

pub fn open_with(target: impl AsRef<std::ffi::OsStr>, app: &str) -> std::io::Result<()> {
    open::with_detached(target, app)
}

pub fn edit(path: impl AsRef<Path>) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg("-t")
            .arg(path.as_ref())
            .spawn()?;
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        open(path.as_ref())
    }
}

pub fn reveal(path: &Path) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg("-R")
            .arg(path)
            .spawn()?;
        Ok(())
    }
    #[cfg(target_os = "windows")]
    {
        let mut selection = std::ffi::OsString::from("/select,");
        selection.push(path);
        std::process::Command::new("explorer.exe")
            .arg(selection)
            .spawn()?;
        Ok(())
    }
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    {
        open(
            path.parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new(".")),
        )
    }
}
