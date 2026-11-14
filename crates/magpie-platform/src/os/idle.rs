//! Seconds since the user's last keyboard / mouse input (system-wide), for time-tracking
//! idle detection. macOS: `CGEventSourceSecondsSinceLastEventType` — needs no
//! Accessibility permission and is safe off the main thread. Other OSes: `None`
//! (the feature stays inert).

#[cfg(target_os = "macos")]
pub fn idle_seconds() -> Option<f64> {
    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGEventSourceSecondsSinceLastEventType(state: i32, event_type: u32) -> f64;
    }
    // kCGEventSourceStateCombinedSessionState = 0, kCGAnyInputEventType = ~0.
    let s = unsafe { CGEventSourceSecondsSinceLastEventType(0, !0) };
    s.is_finite().then_some(s.max(0.0))
}

#[cfg(not(target_os = "macos"))]
pub fn idle_seconds() -> Option<f64> {
    None
}
