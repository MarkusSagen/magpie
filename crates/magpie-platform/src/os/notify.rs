//! Native notifications.
//!
//! On macOS a bundled, signed app posts **branded** notifications through
//! `UNUserNotificationCenter` — they appear as "Magpie" with the app icon and
//! honour a real permission grant, so they are not silently dropped. A bare dev
//! binary has no app bundle, and `UNUserNotificationCenter` raises in that case, so
//! `notify` falls back to `osascript` there (attributed to the terminal, but at
//! least visible during development). Other platforms are a best-effort stub for now.

/// Post a notification. Branded via `UNUserNotificationCenter` when running as the
/// packaged `.app`; otherwise `osascript` (dev) so notifications still appear.
pub fn notify(title: &str, body: &str) {
    #[cfg(target_os = "macos")]
    {
        if is_bundled() {
            macos_un::post(title, body);
        } else {
            osascript(title, body);
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (title, body);
    }
}

/// Request notification permission once, at startup. macOS bundle only — `osascript`
/// (dev) needs no grant, and `UNUserNotificationCenter` requires an app bundle. No-op
/// on other platforms.
pub fn request_notification_authorization() {
    #[cfg(target_os = "macos")]
    {
        if is_bundled() {
            macos_un::request_authorization();
        }
    }
}

/// True when running as a real macOS `.app` bundle (`…/Contents/MacOS/…`) rather than
/// a bare `cargo run` binary. `UNUserNotificationCenter` requires the bundle.
#[cfg(target_os = "macos")]
fn is_bundled() -> bool {
    std::env::current_exe()
        .ok()
        .map(|p| p.to_string_lossy().contains("/Contents/MacOS/"))
        .unwrap_or(false)
}

/// Dev fallback: `osascript display notification` (attributed to the script runner).
#[cfg(target_os = "macos")]
fn osascript(title: &str, body: &str) {
    let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
    let script = format!(
        "display notification \"{}\" with title \"{}\"",
        esc(body),
        esc(title)
    );
    let _ = std::process::Command::new("osascript")
        .arg("-e")
        .arg(script)
        .output();
}

#[cfg(target_os = "macos")]
mod macos_un {
    use block2::RcBlock;
    use objc2::runtime::Bool;
    use objc2_foundation::{NSError, NSString};
    use objc2_user_notifications::{
        UNAuthorizationOptions, UNMutableNotificationContent, UNNotificationRequest,
        UNUserNotificationCenter,
    };
    use std::sync::atomic::{AtomicU64, Ordering};

    /// Per-notification identifier counter (UN requires a unique request id).
    static SEQ: AtomicU64 = AtomicU64::new(0);

    pub fn request_authorization() {
        let center = UNUserNotificationCenter::currentNotificationCenter();
        // We don't act on the result; the OS surfaces the prompt and remembers it.
        let handler = RcBlock::new(|_granted: Bool, _err: *mut NSError| {});
        center.requestAuthorizationWithOptions_completionHandler(
            UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound,
            &handler,
        );
    }

    pub fn post(title: &str, body: &str) {
        let center = UNUserNotificationCenter::currentNotificationCenter();
        let content = UNMutableNotificationContent::new();
        content.setTitle(&NSString::from_str(title));
        content.setBody(&NSString::from_str(body));
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        let ident = NSString::from_str(&format!("magpie-{seq}"));
        // A nil trigger delivers the notification immediately.
        let request =
            UNNotificationRequest::requestWithIdentifier_content_trigger(&ident, &content, None);
        center.addNotificationRequest_withCompletionHandler(&request, None);
    }
}
