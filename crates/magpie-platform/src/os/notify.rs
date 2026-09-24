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

use std::sync::OnceLock;

/// Process-global handler invoked (on the main thread) when the user clicks a
/// reminder notification, with the note_id parsed from the notification
/// identifier (or -1 when none is encoded). Set once via
/// [`set_notification_click_handler`]; read by the macOS notification-center
/// delegate installed by [`install_notification_delegate`].
static CLICK_HANDLER: OnceLock<Box<dyn Fn(i64) + Send + Sync>> = OnceLock::new();

/// Register the handler invoked when the user clicks a reminder notification,
/// with the note_id parsed from the notification identifier (or -1 if none).
/// Invoked on the main thread by AppKit. Call once, at startup. macOS bundle
/// only in effect — on other platforms the handler is simply never called (their
/// notifications route clicks via the OS default).
pub fn set_notification_click_handler(f: impl Fn(i64) + Send + Sync + 'static) {
    let _ = CLICK_HANDLER.set(Box::new(f));
}

/// Install the `UNUserNotificationCenter` delegate that routes notification
/// clicks to the handler registered via [`set_notification_click_handler`], and
/// keeps reminders visible even while Magpie is foreground. macOS bundle only;
/// no-op otherwise.
pub fn install_notification_delegate() {
    #[cfg(target_os = "macos")]
    {
        if is_bundled() {
            macos_un::install_delegate();
        }
    }
}

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
    use block2::{DynBlock, RcBlock};
    use objc2::rc::Retained;
    use objc2::runtime::{Bool, NSObject, NSObjectProtocol, ProtocolObject};
    use objc2::{define_class, msg_send, AnyThread};
    use objc2_foundation::{NSError, NSString};
    use objc2_user_notifications::{
        UNAuthorizationOptions, UNMutableNotificationContent, UNNotification,
        UNNotificationPresentationOptions, UNNotificationRequest, UNNotificationResponse,
        UNTimeIntervalNotificationTrigger, UNUserNotificationCenter,
        UNUserNotificationCenterDelegate,
    };
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::OnceLock;

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

    /// Recover the note id encoded in a scheduled reminder's identifier
    /// (`magpie-task-<note_id>-<fingerprint>`): strip the prefix, then read the
    /// leading integer up to the next `-`. The explicit `<note_id>-` prefix means
    /// this stays correct even though the fingerprint itself contains `-` (e.g. a
    /// `-1` due-time sentinel) and `|`. Returns -1 when the identifier isn't one
    /// of ours or carries no parseable id.
    fn note_id_from_identifier(identifier: &str) -> i64 {
        identifier
            .strip_prefix("magpie-task-")
            .and_then(|rest| rest.split('-').next())
            .and_then(|head| head.parse::<i64>().ok())
            .unwrap_or(-1)
    }

    define_class!(
        // A minimal (no-ivar) NSObject subclass conforming to
        // `UNUserNotificationCenterDelegate`. AppKit calls its methods on the
        // main thread, so the registered click handler runs there too.
        #[unsafe(super(NSObject))]
        #[name = "MagpieNotificationDelegate"]
        struct NotificationDelegate;

        unsafe impl NSObjectProtocol for NotificationDelegate {}

        unsafe impl UNUserNotificationCenterDelegate for NotificationDelegate {
            // The user clicked (or otherwise responded to) a notification.
            #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
            fn did_receive_response(
                &self,
                _center: &UNUserNotificationCenter,
                response: &UNNotificationResponse,
                completion_handler: &DynBlock<dyn Fn()>,
            ) {
                let identifier = response.notification().request().identifier();
                let note_id = note_id_from_identifier(&identifier.to_string());
                if let Some(handler) = super::CLICK_HANDLER.get() {
                    handler(note_id);
                }
                // macOS logs an error unless the completion handler is called.
                completion_handler.call(());
            }

            // A notification arrived while Magpie is in the foreground — still
            // show the banner and play the sound (default is to suppress it).
            #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
            fn will_present(
                &self,
                _center: &UNUserNotificationCenter,
                _notification: &UNNotification,
                completion_handler: &DynBlock<dyn Fn(UNNotificationPresentationOptions)>,
            ) {
                completion_handler.call((UNNotificationPresentationOptions::Banner
                    | UNNotificationPresentationOptions::Sound,));
            }
        }
    );

    /// Keeps the delegate alive for the process lifetime — `setDelegate:` holds
    /// only a weak reference, so a dropped delegate would silently stop routing
    /// clicks. Send + Sync because it is created once on the main thread at
    /// startup and thereafter only read by AppKit on the main thread.
    struct DelegateHolder(#[allow(dead_code)] Retained<NotificationDelegate>);
    // SAFETY: installed once on the main thread; the delegate is never mutated or
    // moved across threads afterwards.
    unsafe impl Send for DelegateHolder {}
    unsafe impl Sync for DelegateHolder {}

    static DELEGATE: OnceLock<DelegateHolder> = OnceLock::new();

    /// Create the delegate and register it with the current notification center.
    /// Idempotent: a second call is a no-op.
    pub fn install_delegate() {
        if DELEGATE.get().is_some() {
            return;
        }
        let this = NotificationDelegate::alloc().set_ivars(());
        let delegate: Retained<NotificationDelegate> = unsafe { msg_send![super(this), init] };
        let center = UNUserNotificationCenter::currentNotificationCenter();
        center.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        let _ = DELEGATE.set(DelegateHolder(delegate));
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn extracts_note_id_from_well_formed_identifier() {
            assert_eq!(note_id_from_identifier("magpie-task-42-abcdef"), 42);
        }

        #[test]
        fn fingerprint_containing_dashes_and_sentinel_does_not_confuse_parsing() {
            // The fingerprint itself may contain `-` (e.g. a `-1` due-time
            // sentinel) and `|`; the explicit `<note_id>-` prefix must still
            // isolate just the note id, not e.g. return -1 from the sentinel.
            assert_eq!(
                note_id_from_identifier("magpie-task-7-1699999999|-1|foo"),
                7
            );
        }

        #[test]
        fn missing_task_prefix_returns_sentinel() {
            assert_eq!(note_id_from_identifier("magpie-5"), -1);
        }

        #[test]
        fn empty_id_after_prefix_returns_sentinel() {
            assert_eq!(note_id_from_identifier("magpie-task-"), -1);
        }

        #[test]
        fn non_numeric_id_returns_sentinel() {
            assert_eq!(note_id_from_identifier("magpie-task-x-y"), -1);
        }

        #[test]
        fn unrelated_identifier_returns_sentinel() {
            assert_eq!(note_id_from_identifier("com.apple.whatever"), -1);
        }
    }
}
