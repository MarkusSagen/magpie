//! Pick a color from anywhere on screen (spec §2 "Pick from screen", ⌘⇧C).
//! macOS: the system eyedropper, `NSColorSampler`. Other OSes: unsupported (`false`).

/// Whether a screen color picker exists on this OS (hide the action otherwise).
pub fn color_picker_supported() -> bool {
    cfg!(target_os = "macos")
}

/// Show the system eyedropper. `on_done` runs on the main thread with the picked sRGB
/// color as `(r, g, b)` (0–255), or `None` if the user cancelled. Must be called on
/// the main thread. Returns `false` if no picker is available.
#[cfg(target_os = "macos")]
pub fn pick_screen_color(on_done: impl Fn(Option<(u8, u8, u8)>) + 'static) -> bool {
    use block2::RcBlock;
    use objc2_app_kit::{NSColor, NSColorSampler, NSColorSpace};

    let handler = RcBlock::new(move |c: *mut NSColor| {
        // SAFETY: AppKit passes a valid (possibly nil) NSColor for the call's duration.
        let picked = unsafe { c.as_ref() }
            .and_then(|c| c.colorUsingColorSpace(&NSColorSpace::sRGBColorSpace()))
            .map(|c| {
                let to8 = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
                (
                    to8(c.redComponent()),
                    to8(c.greenComponent()),
                    to8(c.blueComponent()),
                )
            });
        on_done(picked);
    });
    let sampler = NSColorSampler::new();
    // SAFETY: called on the main thread; the session retains the sampler until done.
    unsafe { sampler.showSamplerWithSelectionHandler(&handler) };
    true
}

#[cfg(not(target_os = "macos"))]
pub fn pick_screen_color(_on_done: impl Fn(Option<(u8, u8, u8)>) + 'static) -> bool {
    false
}
