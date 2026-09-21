//! Native notifications.
//!
//! On macOS a bundled, signed app posts **branded** notifications through
//! `UNUserNotificationCenter` — they appear as "Magpie" with the app icon and
//! honour a real permission grant, so they are not silently dropped. A bare dev
//! binary has no app bundle, and `UNUserNotificationCenter` requires one, so a dev
//! binary can only log the notification (an unbundled `osascript` notification is
//! attributed to Script Editor and useless on click). On Windows and Linux, immediate notifications
//! go through the cross-platform `notify-rust` crate (Linux: D-Bus/libnotify;
//! Windows: WinRT toast). Scheduled "fire when the app is closed" reminders remain
//! macOS-only (`UNTimeIntervalNotificationTrigger`); elsewhere the in-app 60s tick
//! delivers reminders while Magpie runs.

/// Post a notification. Branded via `UNUserNotificationCenter` when running as the
/// packaged `.app`; a dev binary only logs it (macOS can't attribute a notification
/// to an unbundled process).
pub fn notify(title: &str, body: &str) {
    #[cfg(target_os = "macos")]
    {
        if is_bundled() {
            macos_un::post(title, body);
        } else {
            // Unbundled dev binaries CANNOT post an app-attributed notification on
            // macOS: the only unbundled route is `osascript display notification`,
            // which macOS attributes to Script Editor and whose click opens an empty
            // "Untitled" Script Editor window (useless + confusing). So in dev we just
            // log — real, branded, clickable notifications require the packaged
            // `Magpie.app` (UNUserNotificationCenter). Build it with `just package-macos`.
            eprintln!("[magpie notification] {title} — {body}");
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        // Best-effort desktop notification (Linux D-Bus / Windows toast). Failures
        // (e.g. no notification daemon) are swallowed — a missing banner must never
        // disrupt the app.
        let _ = notify_rust::Notification::new()
            .summary(title)
            .body(body)
            .appname("Magpie")
            .show();
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

/// Schedule a notification to fire in `fire_in_secs` from now, even if the app is
/// later quit. Stable `identifier` — re-scheduling with the same id replaces the
/// pending request. macOS bundle only; no-op otherwise or if `fire_in_secs <= 0`.
pub fn schedule_notification(identifier: &str, title: &str, body: &str, fire_in_secs: f64) {
    #[cfg(target_os = "macos")]
    {
        if is_bundled() && fire_in_secs > 0.0 {
            macos_un::schedule(identifier, title, body, fire_in_secs);
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (identifier, title, body, fire_in_secs);
    }
}

/// Remove all pending (scheduled-but-undelivered) notification requests. Immediate
/// notifications already delivered are unaffected. macOS bundle only; no-op otherwise.
pub fn cancel_scheduled_reminders() {
    #[cfg(target_os = "macos")]
    {
        if is_bundled() {
            macos_un::cancel_all();
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

#[cfg(target_os = "macos")]
mod macos_un {
    use block2::RcBlock;
    use objc2::runtime::Bool;
    use objc2_foundation::{NSError, NSString};
    use objc2_user_notifications::{
        UNAuthorizationOptions, UNMutableNotificationContent, UNNotificationRequest,
        UNTimeIntervalNotificationTrigger, UNUserNotificationCenter,
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

    /// Schedule a notification to fire `fire_in_secs` from now via
    /// `UNTimeIntervalNotificationTrigger`. `identifier` is stable so re-scheduling
    /// the same task replaces its pending request rather than duplicating it.
    /// Guards `fire_in_secs` to at least 1.0 (the API's minimum interval).
    pub fn schedule(identifier: &str, title: &str, body: &str, fire_in_secs: f64) {
        let center = UNUserNotificationCenter::currentNotificationCenter();
        let content = UNMutableNotificationContent::new();
        content.setTitle(&NSString::from_str(title));
        content.setBody(&NSString::from_str(body));
        let interval = fire_in_secs.max(1.0);
        let trigger =
            UNTimeIntervalNotificationTrigger::triggerWithTimeInterval_repeats(interval, false);
        let ident = NSString::from_str(identifier);
        let request = UNNotificationRequest::requestWithIdentifier_content_trigger(
            &ident,
            &content,
            Some(&trigger),
        );
        center.addNotificationRequest_withCompletionHandler(&request, None);
    }

    /// Remove all pending (undelivered) notification requests.
    pub fn cancel_all() {
        UNUserNotificationCenter::currentNotificationCenter()
            .removeAllPendingNotificationRequests();
    }
}
