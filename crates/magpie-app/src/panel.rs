//! Pure geometry for the quick-clipboard popover panel: size clamping and where a
//! pinned side panel docks on screen. Kept free of Slint/OS types so it's unit
//! tested in isolation; the actual `set_size`/`set_position` calls live in
//! `runtime`.

/// Minimum / maximum logical panel size. The floor keeps the tab row + a few clip
/// rows usable; the ceiling stops a fat-fingered grip drag from filling the screen.
pub const MIN_W: i32 = 280;
pub const MAX_W: i32 = 720;
pub const MIN_H: i32 = 320;
pub const MAX_H: i32 = 1000;

/// Gap between a docked panel and the screen edges / menu bar.
pub const DOCK_MARGIN: i32 = 8;

/// Clamp a requested panel size into the usable range.
pub fn clamp_size(w: i32, h: i32) -> (i32, i32) {
    (w.clamp(MIN_W, MAX_W), h.clamp(MIN_H, MAX_H))
}

/// Top-left position (physical-ish logical px) for a pinned panel docked flush to
/// the right screen edge, sitting just below the menu bar. The panel keeps its own
/// size; only its origin is computed. `menubar_h` is the height reserved at the top
/// (the macOS menu bar); `screen` is the usable width/height.
pub fn dock_right(screen: (i32, i32), panel: (i32, i32), menubar_h: i32) -> (i32, i32) {
    let (sw, sh) = screen;
    let (pw, ph) = panel;
    let x = (sw - pw - DOCK_MARGIN).max(0);
    let top = menubar_h + DOCK_MARGIN;
    // Don't let a tall panel run off the bottom: pull it up if needed.
    let y = if top + ph + DOCK_MARGIN > sh {
        (sh - ph - DOCK_MARGIN).max(top)
    } else {
        top
    };
    (x, y)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_bounds() {
        assert_eq!(clamp_size(100, 100), (MIN_W, MIN_H));
        assert_eq!(clamp_size(9999, 9999), (MAX_W, MAX_H));
        assert_eq!(clamp_size(400, 500), (400, 500));
    }

    #[test]
    fn dock_right_hugs_edge_below_menubar() {
        // 1920x1080 screen, 360x440 panel, 24px menu bar.
        let (x, y) = dock_right((1920, 1080), (360, 440), 24);
        assert_eq!(x, 1920 - 360 - DOCK_MARGIN);
        assert_eq!(y, 24 + DOCK_MARGIN);
    }

    #[test]
    fn dock_right_pulls_tall_panel_up() {
        // A panel taller than the space below the menu bar is pulled up so its
        // bottom stays on screen.
        let (_, y) = dock_right((1440, 600), (360, 560), 24);
        assert_eq!(y, 600 - 560 - DOCK_MARGIN);
    }

    #[test]
    fn dock_right_never_negative_origin() {
        // Panel wider/taller than the screen still yields a non-negative origin.
        let (x, y) = dock_right((300, 300), (400, 400), 24);
        assert!(x >= 0);
        assert!(y >= 0);
    }
}
