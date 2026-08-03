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

/// Register this process with the Accessibility system and show macOS's own
/// "wants to control your computer" prompt. This is what makes the app **appear
/// in the Accessibility list** — a bare `AXIsProcessTrusted()` check never
/// registers it. Returns the current trust state. macOS only; no-op elsewhere.
///
/// `AXIsProcessTrustedWithOptions({ kAXTrustedCheckOptionPrompt: true })`.
#[cfg(target_os = "macos")]
pub fn prompt_accessibility() -> bool {
    use std::ffi::c_void;
    type Ref = *const c_void;

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        static kCFBooleanTrue: Ref;
        static kCFTypeDictionaryKeyCallBacks: c_void;
        static kCFTypeDictionaryValueCallBacks: c_void;
        fn CFDictionaryCreate(
            allocator: Ref,
            keys: *const Ref,
            values: *const Ref,
            num_values: isize,
            key_cbs: *const c_void,
            value_cbs: *const c_void,
        ) -> Ref;
        fn CFRelease(cf: Ref);
    }
    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        static kAXTrustedCheckOptionPrompt: Ref;
        fn AXIsProcessTrustedWithOptions(options: Ref) -> u8;
    }

    unsafe {
        let keys = [kAXTrustedCheckOptionPrompt];
        let values = [kCFBooleanTrue];
        let options = CFDictionaryCreate(
            std::ptr::null(),
            keys.as_ptr(),
            values.as_ptr(),
            1,
            std::ptr::addr_of!(kCFTypeDictionaryKeyCallBacks),
            std::ptr::addr_of!(kCFTypeDictionaryValueCallBacks),
        );
        let trusted = AXIsProcessTrustedWithOptions(options) != 0;
        if !options.is_null() {
            CFRelease(options);
        }
        trusted
    }
}

/// Non-macOS: nothing to register.
#[cfg(not(target_os = "macos"))]
pub fn prompt_accessibility() -> bool {
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
