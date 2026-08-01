//! Autostart adapters write/remove real files. LinuxAutostart is portable and
//! always tested; MacAutostart is tested on the macOS host.

use magpie_platform::os::autostart_linux::{desktop_entry, LinuxAutostart};
use magpie_platform::Autostart;

fn tmp(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("magpie-autostart-it-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn linux_desktop_entry_content_is_valid() {
    let d = desktop_entry("Magpie", "/opt/magpie/magpie");
    assert!(d.starts_with("[Desktop Entry]"));
    assert!(d.contains("Type=Application"));
    assert!(d.contains("Exec=/opt/magpie/magpie"));
    assert!(d.contains("X-GNOME-Autostart-enabled=true"));
}

#[test]
fn linux_autostart_enable_disable_is_idempotent() {
    let dir = tmp("linux");
    let path = dir.join("magpie.desktop");
    let a = LinuxAutostart { desktop_path: path.clone(), name: "Magpie".into(), exec: "/bin/magpie".into() };

    assert!(!a.is_enabled());
    a.set_enabled(true).unwrap();
    a.set_enabled(true).unwrap(); // idempotent enable
    assert!(a.is_enabled() && path.exists());
    let written = std::fs::read_to_string(&path).unwrap();
    assert!(written.contains("Exec=/bin/magpie"));

    a.set_enabled(false).unwrap();
    a.set_enabled(false).unwrap(); // idempotent disable (no error when already gone)
    assert!(!a.is_enabled() && !path.exists());

    std::fs::remove_dir_all(&dir).ok();
}

#[cfg(target_os = "macos")]
#[test]
fn macos_launchagent_plist_write_remove() {
    use magpie_platform::os::autostart::{launch_agent_plist, MacAutostart};

    let xml = launch_agent_plist("io.magpie.agent", "/Applications/Magpie.app/Contents/MacOS/magpie");
    assert!(xml.contains("<key>RunAtLoad</key>"));
    assert!(xml.contains("io.magpie.agent"));

    let dir = tmp("macos");
    let plist = dir.join("io.magpie.agent.plist");
    let a = MacAutostart { plist_path: plist.clone(), label: "io.magpie.agent".into(), program: "/bin/magpie".into() };
    assert!(!a.is_enabled());
    a.set_enabled(true).unwrap();
    assert!(a.is_enabled() && plist.exists());
    let body = std::fs::read_to_string(&plist).unwrap();
    assert!(body.contains("/bin/magpie") && body.contains("RunAtLoad"));
    a.set_enabled(false).unwrap();
    assert!(!plist.exists());
    std::fs::remove_dir_all(&dir).ok();
}
