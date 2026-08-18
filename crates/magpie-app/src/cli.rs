#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Run,
    InstallAutostart,
    UninstallAutostart,
    Help,
}

pub fn parse_args(args: &[String]) -> Command {
    for a in args {
        match a.as_str() {
            "--install-autostart" => return Command::InstallAutostart,
            "--uninstall-autostart" => return Command::UninstallAutostart,
            "--help" | "-h" => return Command::Help,
            _ => {}
        }
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
    -h, --help              Show this help
";

/// Returns a process exit code, or -1 to signal `main` to launch the GUI.
pub fn run_command(cmd: Command) -> i32 {
    match cmd {
        Command::Help => {
            println!("{USAGE}");
            0
        }
        Command::InstallAutostart => set_autostart(true),
        Command::UninstallAutostart => set_autostart(false),
        Command::Run => -1,
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
        assert_eq!(run_command(Command::Help), 0);
    }
}
