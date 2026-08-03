//! Accessibility (macOS) permission: auto-paste synthesizes a ⌘V keystroke via
//! `enigo`, which macOS only delivers if the process is trusted for Accessibility
//! (System Settings ▸ Privacy & Security ▸ Accessibility). We check that up front
//! so the UI can guide the user to enable it, and offer a one-click jump to the
//! exact settings pane — the same flow apps like Raycast/Rectangle use.

/// Is this process trusted to synthesize input events? Always `true` off macOS
/// (no equivalent per-app gate for the X11/Windows paste path).
#[cfg(target_os = "macos")]
pub fn accessibility_trusted() -> bool {
    // `AXIsProcessTrusted` (ApplicationServices) reports the current grant without
    // prompting. `Boolean` is an unsigned char; non-zero means trusted.
    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXIsProcessTrusted() -> u8;
    }
    unsafe { AXIsProcessTrusted() != 0 }
}

/// Non-macOS: no per-app accessibility gate for the paste path.
#[cfg(not(target_os = "macos"))]
pub fn accessibility_trusted() -> bool {
    true
}

/// Open the exact OS pane where the user grants Accessibility. macOS deep-links to
/// Privacy ▸ Accessibility; other platforms have no equivalent (no-op).
#[cfg(target_os = "macos")]
pub fn open_accessibility_settings() {
    let _ = std::process::Command::new("open")
        .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")
        .spawn();
}

/// Non-macOS: no-op (no equivalent settings deep-link).
#[cfg(not(target_os = "macos"))]
pub fn open_accessibility_settings() {}
