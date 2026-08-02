//! Source-application icon resolution + on-disk cache. OS extraction is
//! platform-specific and best-effort; the cache/reuse logic is pure.

use std::path::{Path, PathBuf};

/// Deterministic cache path for an app identifier: `<dir>/<blake3(id)>.png`.
pub fn icon_cache_path(dir: &Path, app_identifier: &str) -> PathBuf {
    let hash = blake3::hash(app_identifier.as_bytes()).to_hex();
    dir.join(format!("{hash}.png"))
}

/// Cached icon path if present; else extract from `exe_path`, write, and return.
/// `None` if extraction fails (caller falls back to a glyph).
pub fn ensure_app_icon(dir: &Path, app_identifier: &str, exe_path: &Path) -> Option<PathBuf> {
    let path = icon_cache_path(dir, app_identifier);
    if path.exists() {
        return Some(path);
    }
    let raw = app_icon_png(exe_path)?;
    if raw.is_empty() {
        return None;
    }
    // OS extractors return native-resolution icons (macOS can be 1024px / MBs);
    // normalize to a small PNG so on-disk size + per-refresh decode stay cheap.
    let bytes = downscale_png(&raw).unwrap_or(raw);
    std::fs::create_dir_all(dir).ok()?;
    std::fs::write(&path, &bytes).ok()?;
    Some(path)
}

/// Decode any icon bytes (PNG/ICO/TIFF) and re-encode as a ≤64px PNG.
/// Returns `None` if decoding fails (caller keeps the raw bytes).
fn downscale_png(raw: &[u8]) -> Option<Vec<u8>> {
    let img = image::load_from_memory(raw).ok()?;
    let small = img.thumbnail(64, 64);
    let mut out = std::io::Cursor::new(Vec::new());
    small
        .write_to(&mut out, image::ImageFormat::Png)
        .ok()
        .map(|_| out.into_inner())
}

// ---------------- macOS ----------------

#[cfg(target_os = "macos")]
fn app_icon_png(exe_path: &Path) -> Option<Vec<u8>> {
    use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSWorkspace};
    use objc2_foundation::{NSDictionary, NSString};

    let bundle = bundle_path_from_exe(exe_path)?;
    let bundle_str = bundle.to_str()?;
    unsafe {
        let ws = NSWorkspace::sharedWorkspace();
        let ns = NSString::from_str(bundle_str);
        let image = ws.iconForFile(&ns);
        let tiff = image.TIFFRepresentation()?;
        let rep = NSBitmapImageRep::imageRepWithData(&tiff)?;
        let props = NSDictionary::new();
        let png = rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &props)?;
        Some(png.to_vec())
    }
}

/// `…/Foo.app/Contents/MacOS/foo` → `…/Foo.app`; else the exe path itself.
#[cfg(target_os = "macos")]
fn bundle_path_from_exe(exe: &Path) -> Option<PathBuf> {
    let mut cur = exe;
    while let Some(parent) = cur.parent() {
        if parent.extension().and_then(|e| e.to_str()) == Some("app") {
            return Some(parent.to_path_buf());
        }
        cur = parent;
    }
    Some(exe.to_path_buf())
}

// ---------------- non-macOS placeholder (Windows/Linux land in a later task) ----------------

#[cfg(not(target_os = "macos"))]
fn app_icon_png(_exe_path: &Path) -> Option<Vec<u8>> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_path_is_deterministic_and_distinct() {
        let d = Path::new("/tmp/icons");
        assert_eq!(icon_cache_path(d, "Foo"), icon_cache_path(d, "Foo"));
        assert_ne!(icon_cache_path(d, "Foo"), icon_cache_path(d, "Bar"));
    }

    #[test]
    fn ensure_reuses_existing_file_without_extraction() {
        let dir = std::env::temp_dir().join(format!("magpie-icons-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = icon_cache_path(&dir, "Cached");
        std::fs::write(&p, b"x").unwrap();
        // Bogus exe path; extraction must NOT be needed because the file exists.
        let got = ensure_app_icon(&dir, "Cached", Path::new("/nonexistent/bin"));
        assert_eq!(got, Some(p));
        std::fs::remove_dir_all(&dir).ok();
    }
}
