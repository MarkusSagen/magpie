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

    // If we previously hid the app to yield focus for a paste, un-hide it first.
    app.unhide(None);

    // Foreground the process even though we're an agent app. `activate()` (macOS
    // 14+) will not pull focus from the active app for an accessory app, so we
    // use the older `activateIgnoringOtherApps:` which does exactly that.
    #[allow(deprecated)]
    app.activateIgnoringOtherApps(true);

    // Raise each ALREADY-VISIBLE window, let it join the active Space and float over
    // full-screen apps, and make it the key (focused) window so typing goes to it.
    // The visibility guard matters now that the app owns two windows (launcher +
    // popover): without it, showing the popover would force-order the never-shown
    // launcher to the front as a collapsed, empty window.
    for window in app.windows().iter() {
        if !window.isVisible() {
            continue;
        }
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

/// Hide Magpie and return keyboard focus to the app that was active before us, so
/// a subsequent paste keystroke lands in that app. On macOS `-[NSApplication
/// hide:]` hides all our windows and activates the next app in line — even when we
/// have no visible window — which is exactly the "paste into the app underneath"
/// behavior. On other platforms hiding the Slint window already yields focus.
#[cfg(target_os = "macos")]
pub fn hide_and_yield_focus() {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSApplication;

    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    NSApplication::sharedApplication(mtm).hide(None);
}

/// No-op fallback: on Windows/Linux hiding the window returns focus to the WM.
#[cfg(not(target_os = "macos"))]
pub fn hide_and_yield_focus() {}

/// Match the native window chrome (titlebar, traffic-light strip) to Magpie's
/// theme. Slint paints the client area, but the OS-drawn titlebar follows the
/// `NSApplication` appearance — so in light mode the titlebar stays dark unless
/// we set the app appearance explicitly. Call on the main (UI) thread whenever
/// the theme changes (and once at startup). No-op if the aqua appearance can't
/// be resolved.
#[cfg(target_os = "macos")]
pub fn set_appearance(dark: bool) {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSAppearance, NSApplication};

    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let name = if dark {
        unsafe { objc2_app_kit::NSAppearanceNameDarkAqua }
    } else {
        unsafe { objc2_app_kit::NSAppearanceNameAqua }
    };
    if let Some(appearance) = NSAppearance::appearanceNamed(name) {
        NSApplication::sharedApplication(mtm).setAppearance(Some(&appearance));
    }
}

/// No-op fallback: Windows/Linux window chrome follows the desktop theme.
#[cfg(not(target_os = "macos"))]
pub fn set_appearance(_dark: bool) {}
