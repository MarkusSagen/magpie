#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Run,
    InstallAutostart,
    UninstallAutostart,
    Help,
    Export(std::path::PathBuf),
    Backup(std::path::PathBuf),
    Restore(std::path::PathBuf),
    ImportBookmarks(String),
    SyncVault(Option<std::path::PathBuf>),
}

pub fn parse_args(args: &[String]) -> Command {
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--install-autostart" => return Command::InstallAutostart,
            "--uninstall-autostart" => return Command::UninstallAutostart,
            "--help" | "-h" => return Command::Help,
            "--export" => {
                return match args.get(i + 1) {
                    Some(p) => Command::Export(std::path::PathBuf::from(p)),
                    None => Command::Run,
                };
            }
            "--backup" => {
                return match args.get(i + 1) {
                    Some(p) => Command::Backup(std::path::PathBuf::from(p)),
                    None => Command::Run,
                };
            }
            "--restore" => {
                return match args.get(i + 1) {
                    Some(p) => Command::Restore(std::path::PathBuf::from(p)),
                    None => Command::Run,
                };
            }
            "--import-bookmarks" => {
                return match args.get(i + 1) {
                    Some(w) => Command::ImportBookmarks(w.clone()),
                    None => Command::Run,
                };
            }
            "--sync-vault" => {
                let path = match args.get(i + 1) {
                    Some(p) if !p.starts_with("--") => Some(std::path::PathBuf::from(p)),
                    _ => None,
                };
                return Command::SyncVault(path);
            }
            _ => {}
        }
        i += 1;
    }
    Command::Run
}

const USAGE: &str = "\
Magpie — cross-platform clipboard manager

USAGE:
    magpie [FLAGS]

FLAGS:
    (no flags)              Run the tray app
    --install-autostart     Start Magpie automatically at login
    --uninstall-autostart   Remove login autostart
    --export <dir>          Export notes (Markdown) + clipboard (JSONL) to <dir>
    --backup <dir>          Write a full backup (DB snapshot + assets) to <dir>
    --restore <dir>         Restore from a backup dir (quit Magpie first)
    --import-bookmarks <chrome|firefox|path>   Import bookmarks into Magpie
    --sync-vault <dir>      Sync notes with a folder of Markdown files (newest wins)
    -h, --help              Show this help
";

/// Returns a process exit code, or -1 to signal `main` to launch the GUI.
pub fn run_command(cmd: Command, data_dir: &std::path::Path) -> i32 {
    match cmd {
        Command::Help => {
            println!("{USAGE}");
            0
        }
        Command::InstallAutostart => set_autostart(true),
        Command::UninstallAutostart => set_autostart(false),
        Command::Run => -1,
        Command::Export(out) => export_data(data_dir, &out),
        Command::Backup(dest) => backup_data(data_dir, &dest),
        Command::Restore(src) => restore_data(data_dir, &src),
        Command::ImportBookmarks(which) => import_bookmarks(data_dir, &which),
        Command::SyncVault(p) => sync_vault_cmd(data_dir, p),
    }
}

fn open_store(data_dir: &std::path::Path) -> Result<magpie_core::Store, String> {
    magpie_core::open(&data_dir.join("magpie.sqlite3")).map_err(|e| e.to_string())
}

fn export_data(data_dir: &std::path::Path, out: &std::path::Path) -> i32 {
    let store = match open_store(data_dir) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("export: {e}");
            return 1;
        }
    };
    if let Err(e) = std::fs::create_dir_all(out) {
        eprintln!("export: {e}");
        return 1;
    }
    let notes = match magpie_core::export_markdown(&store, out) {
        Ok(n) => n,
        Err(e) => {
            eprintln!("export: {e}");
            return 1;
        }
    };
    let clips = match magpie_core::export_clipboard_jsonl(&store, &out.join("clipboard.jsonl")) {
        Ok(n) => n,
        Err(e) => {
            eprintln!("export: {e}");
            return 1;
        }
    };
    println!(
        "Exported {notes} notes (Markdown) and {clips} clipboard entries to {}",
        out.display()
    );
    0
}

fn copy_dir_all(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    if !src.exists() {
        return Ok(());
    }
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let p = entry.path();
        let d = dst.join(entry.file_name());
        if p.is_dir() {
            copy_dir_all(&p, &d)?;
        } else {
            std::fs::copy(&p, &d)?;
        }
    }
    Ok(())
}

fn backup_data(data_dir: &std::path::Path, dest: &std::path::Path) -> i32 {
    let store = match open_store(data_dir) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("backup: {e}");
            return 1;
        }
    };
    if let Err(e) = std::fs::create_dir_all(dest) {
        eprintln!("backup: {e}");
        return 1;
    }
    if let Err(e) = magpie_core::backup_db(&store, &dest.join("magpie.sqlite3")) {
        eprintln!("backup: {e}");
        return 1;
    }
    for dir in ["favicons", "app_icons"] {
        if let Err(e) = copy_dir_all(&data_dir.join(dir), &dest.join(dir)) {
            eprintln!("backup ({dir}): {e}");
            return 1;
        }
    }
    let cfg = data_dir.join("config.toml");
    if cfg.exists() {
        if let Err(e) = std::fs::copy(&cfg, dest.join("config.toml")) {
            eprintln!("backup (config): {e}");
            return 1;
        }
    }
    println!("Backed up to {}", dest.display());
    0
}

fn restore_data(data_dir: &std::path::Path, src: &std::path::Path) -> i32 {
    eprintln!("Restore: quit Magpie first if it is running, or the database may be corrupted.");
    let db = src.join("magpie.sqlite3");
    if !db.exists() {
        eprintln!("restore: no magpie.sqlite3 in {}", src.display());
        return 1;
    }
    if let Err(e) = std::fs::create_dir_all(data_dir) {
        eprintln!("restore: {e}");
        return 1;
    }
    if let Err(e) = std::fs::copy(&db, data_dir.join("magpie.sqlite3")) {
        eprintln!("restore: {e}");
        return 1;
    }
    // Drop stale WAL/SHM so the restored DB isn't shadowed.
    let _ = std::fs::remove_file(data_dir.join("magpie.sqlite3-wal"));
    let _ = std::fs::remove_file(data_dir.join("magpie.sqlite3-shm"));
    for dir in ["favicons", "app_icons"] {
        if let Err(e) = copy_dir_all(&src.join(dir), &data_dir.join(dir)) {
            eprintln!("restore ({dir}): {e}");
            return 1;
        }
    }
    let cfg = src.join("config.toml");
    if cfg.exists() {
        let _ = std::fs::copy(&cfg, data_dir.join("config.toml"));
    }
    println!("Restored from {} — restart Magpie.", src.display());
    0
}

fn import_bookmarks(data_dir: &std::path::Path, which: &str) -> i32 {
    let pairs = match crate::bookmark_import::resolve_import(which) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("import: {e}");
            return 1;
        }
    };
    let store = match magpie_core::open(&data_dir.join("magpie.sqlite3")) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("import: {e}");
            return 1;
        }
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let mut n = 0;
    for (title, url) in &pairs {
        let domain = crate::favicon::domain_of(url).unwrap_or_default();
        if store.add_bookmark(url, title, &domain, now).is_ok() {
            n += 1;
        }
    }
    println!("Imported {n} bookmarks from {which}");
    0
}

fn sync_vault_cmd(data_dir: &std::path::Path, path: Option<std::path::PathBuf>) -> i32 {
    let dir = match path.or_else(|| {
        crate::config::load_or_default(&data_dir.join("config.toml"))
            .vault_path
            .map(std::path::PathBuf::from)
    }) {
        Some(d) => d,
        None => {
            eprintln!(
                "sync-vault: no vault path given and none configured (set vault_path in config.toml)"
            );
            return 1;
        }
    };
    let store = match open_store(data_dir) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("sync-vault: {e}");
            return 1;
        }
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    match magpie_core::sync_vault(&store, &dir, now) {
        Ok(r) => {
            println!(
                "Vault sync: {} imported, {} updated, {} exported, {} unchanged ({})",
                r.imported,
                r.updated,
                r.exported,
                r.skipped,
                dir.display()
            );
            0
        }
        Err(e) => {
            eprintln!("sync-vault: {e}");
            1
        }
    }
}

fn set_autostart(on: bool) -> i32 {
    let exe = match std::env::current_exe() {
        Ok(p) => p.to_string_lossy().into_owned(),
        Err(e) => {
            eprintln!("cannot resolve executable path: {e}");
            return 1;
        }
    };
    let auto = magpie_platform::platform_autostart(&exe);
    match auto.set_enabled(on) {
        Ok(()) => {
            // Apply immediately so it works without a re-login (best-effort).
            auto.take_effect_now(on);
            println!("autostart {}", if on { "enabled" } else { "disabled" });
            0
        }
        Err(e) => {
            eprintln!("autostart change failed: {e}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_known_flags() {
        assert!(matches!(
            parse_args(&v(&["--install-autostart"])),
            Command::InstallAutostart
        ));
        assert!(matches!(
            parse_args(&v(&["--uninstall-autostart"])),
            Command::UninstallAutostart
        ));
        assert!(matches!(parse_args(&v(&["--help"])), Command::Help));
        assert!(matches!(parse_args(&v(&["-h"])), Command::Help));
    }

    #[test]
    fn empty_or_unknown_is_run() {
        assert!(matches!(parse_args(&v(&[])), Command::Run));
        assert!(matches!(parse_args(&v(&["--nonsense"])), Command::Run));
    }

    #[test]
    fn help_command_returns_zero() {
        assert_eq!(
            run_command(Command::Help, &std::path::PathBuf::from("/tmp")),
            0
        );
    }

    #[test]
    fn parses_export_backup_restore() {
        assert!(matches!(
            parse_args(&v(&["--export", "/tmp/x"])),
            Command::Export(p) if p.as_path() == std::path::Path::new("/tmp/x")
        ));
        assert!(matches!(
            parse_args(&v(&["--backup", "/tmp/b"])),
            Command::Backup(_)
        ));
        assert!(matches!(
            parse_args(&v(&["--restore", "/tmp/r"])),
            Command::Restore(_)
        ));
        // missing path → Run
        assert!(matches!(parse_args(&v(&["--export"])), Command::Run));
    }

    #[test]
    fn parses_import_bookmarks() {
        assert!(matches!(
            parse_args(&v(&["--import-bookmarks", "chrome"])),
            Command::ImportBookmarks(w) if w == "chrome"
        ));
        assert!(matches!(
            parse_args(&v(&["--import-bookmarks"])),
            Command::Run
        ));
    }

    #[test]
    fn parses_sync_vault() {
        assert!(matches!(
            parse_args(&v(&["--sync-vault", "/tmp/v"])),
            Command::SyncVault(Some(p)) if p.as_path() == std::path::Path::new("/tmp/v")
        ));
        assert!(matches!(
            parse_args(&v(&["--sync-vault"])),
            Command::SyncVault(None)
        ));
    }
}
