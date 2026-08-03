//! Bring the app's windows to the front with keyboard focus.
//!
//! A macOS background/agent app (`LSUIElement`) does **not** get keyboard focus
//! just by showing a window — it must activate the process and raise + key the
//! window. This is the mechanism launchers like Raycast/Alfred use so their
//! window appears instantly over whatever is focused, including full-screen
//! spaces. On other platforms the window manager handles focus when the window
//! is shown, so this is a no-op there (Slint/winit already requests focus).

/// Activate this process and raise its windows to the front, taking key focus.
/// Must be called on the main (UI) thread.
#[cfg(target_os = "macos")]
pub fn raise_to_front() {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSWindowCollectionBehavior};

    // Only valid on the main thread; callers invoke us from the Slint event loop.
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let app = NSApplication::sharedApplication(mtm);

    // Foreground the process even though we're an agent app. `activate()` (macOS
    // 14+) will not pull focus from the active app for an accessory app, so we
    // use the older `activateIgnoringOtherApps:` which does exactly that.
    #[allow(deprecated)]
    app.activateIgnoringOtherApps(true);

    // Raise every window, let it join the active Space and float over full-screen
    // apps, and make it the key (focused) window so typing goes to the search box.
    for window in app.windows().iter() {
        window.setCollectionBehavior(
            NSWindowCollectionBehavior::CanJoinAllSpaces
                | NSWindowCollectionBehavior::FullScreenAuxiliary,
        );
        window.makeKeyAndOrderFront(None);
        window.orderFrontRegardless();
    }
}

/// No-op fallback: on Windows/Linux the WM focuses the window on show.
#[cfg(not(target_os = "macos"))]
pub fn raise_to_front() {}
