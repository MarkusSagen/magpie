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

/// Bring the menu-bar **popover** to the front WITHOUT un-hiding the rest of the app.
/// Unlike [`raise_to_front`], this never calls `unhide`, so an app-hidden main launcher
/// window is NOT resurrected alongside the popover — clicking the tray icon then behaves
/// like a normal macOS menu-bar dropdown (one surface), not "popover + full window".
/// Only already-visible windows (the popover we just showed) are ordered front.
#[cfg(target_os = "macos")]
pub fn raise_popover_front() {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSWindowCollectionBehavior};

    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let app = NSApplication::sharedApplication(mtm);
    // NB: deliberately no `app.unhide(None)` here — that is what pulled the hidden main
    // window back. Order only the currently-visible windows to the front; an app-hidden
    // launcher reports `!isVisible` and is skipped.
    for window in app.windows().iter() {
        if !window.isVisible() {
            continue;
        }
        window.setCollectionBehavior(
            NSWindowCollectionBehavior::CanJoinAllSpaces
                | NSWindowCollectionBehavior::FullScreenAuxiliary,
        );
        window.orderFrontRegardless();
    }
    // Foreground the process so the popover takes clicks/keys, but only after the
    // visibility guard above decided what to raise (so the hidden launcher isn't lifted).
    #[allow(deprecated)]
    app.activateIgnoringOtherApps(true);
}

/// No-op fallback: on Windows/Linux the WM focuses the window on show.
#[cfg(not(target_os = "macos"))]
pub fn raise_popover_front() {}

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

/// Usable size of the main screen in logical points, and the height reserved at the
/// top by the menu bar — `(width, height, menubar_height)`. Used to dock the pinned
/// clipboard side panel to the right edge below the menu bar. Call on the main
/// thread. Returns `None` if no screen can be resolved.
#[cfg(target_os = "macos")]
pub fn main_screen_size() -> Option<(i32, i32, i32)> {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSScreen;

    let mtm = MainThreadMarker::new()?;
    let screen = NSScreen::mainScreen(mtm)?;
    // `frame` is the full screen; `visibleFrame` excludes the menu bar (and Dock).
    // The menu bar height is the gap between the two at the top.
    let frame = screen.frame();
    let visible = screen.visibleFrame();
    let w = frame.size.width as i32;
    let h = frame.size.height as i32;
    // In Cocoa's bottom-left origin, the menu bar eats the top: its height is the
    // full height minus the visible frame's top edge.
    let menubar = (frame.size.height - (visible.origin.y + visible.size.height)).max(0.0) as i32;
    Some((w, h, menubar))
}

/// No-op fallback: other platforms don't dock the panel to the menu-bar edge.
#[cfg(not(target_os = "macos"))]
pub fn main_screen_size() -> Option<(i32, i32, i32)> {
    None
}
