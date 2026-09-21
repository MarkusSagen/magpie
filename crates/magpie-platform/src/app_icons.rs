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

// ---------------- Windows ----------------
//
// Neither this nor the Linux impl below can be built or run on this (macOS)
// dev machine — they're `#[cfg]`-gated to their target OS, verified only by
// CI's Windows/Linux matrix legs + real hardware. `ensure_app_icon` already
// downscales whatever bytes come back, so each impl only needs to return
// native icon bytes (here: PNG, re-encoded from the raw GDI pixels).

#[cfg(target_os = "windows")]
fn app_icon_png(exe_path: &Path) -> Option<Vec<u8>> {
    windows_icon::extract(exe_path)
}

#[cfg(target_os = "windows")]
mod windows_icon {
    use std::ffi::c_void;
    use std::io::Cursor;
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    use windows::core::PCWSTR;
    use windows::Win32::Graphics::Gdi::{
        DeleteObject, GetDC, GetDIBits, GetObjectW, ReleaseDC, BITMAP, BITMAPINFO,
        BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HBITMAP,
    };
    use windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES;
    use windows::Win32::UI::Shell::{SHGetFileInfoW, SHFILEINFOW, SHGFI_ICON, SHGFI_LARGEICON};
    use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, GetIconInfo, HICON, ICONINFO};

    /// Extract the exe's large icon via `SHGetFileInfoW`, read its 32bpp pixels
    /// via GDI, and re-encode as PNG. Every failure path returns `None` — this
    /// is best-effort UI decoration, never allowed to panic the caller. Every
    /// handle acquired below (`HICON`, two `HBITMAP`s, an `HDC`) is released on
    /// every return path.
    pub fn extract(exe_path: &Path) -> Option<Vec<u8>> {
        extract_inner(exe_path)
    }

    fn extract_inner(exe_path: &Path) -> Option<Vec<u8>> {
        let mut wide: Vec<u16> = exe_path.as_os_str().encode_wide().collect();
        wide.push(0);
        let path = PCWSTR::from_raw(wide.as_ptr());

        let mut shfi = SHFILEINFOW::default();
        let cb = std::mem::size_of::<SHFILEINFOW>() as u32;
        let flags = SHGFI_ICON | SHGFI_LARGEICON;
        // SAFETY: `path` points at `wide`, a NUL-terminated buffer that outlives
        // this call; `shfi` is a valid out-pointer sized by `cb`.
        let ok = unsafe {
            SHGetFileInfoW(
                path,
                FILE_FLAGS_AND_ATTRIBUTES(0),
                Some(&mut shfi),
                cb,
                flags,
            )
        };
        // 0 = failure; SHGFI_ICON otherwise always yields a valid HICON on success.
        if ok == 0 || shfi.hIcon.is_invalid() {
            return None;
        }
        let hicon = shfi.hIcon;

        let png = extract_from_icon(hicon);
        // SAFETY: `hicon` was just returned to us by SHGetFileInfoW(SHGFI_ICON),
        // which hands ownership to the caller — we must destroy it when done.
        let _ = unsafe { DestroyIcon(hicon) };
        png
    }

    fn extract_from_icon(hicon: HICON) -> Option<Vec<u8>> {
        let mut info = ICONINFO::default();
        // SAFETY: `hicon` is the valid handle obtained above; `info` is a valid
        // out-pointer.
        unsafe { GetIconInfo(hicon, &mut info) }.ok()?;
        let hbm_color = info.hbmColor;
        let hbm_mask = info.hbmMask;

        let png = extract_from_bitmap(hbm_color);

        // SAFETY: GetIconInfo allocated both bitmaps for us; its docs require
        // the caller to delete them once done.
        unsafe {
            let _ = DeleteObject(hbm_color.into());
            let _ = DeleteObject(hbm_mask.into());
        }

        png
    }

    fn extract_from_bitmap(hbm: HBITMAP) -> Option<Vec<u8>> {
        let mut bmp = BITMAP::default();
        // SAFETY: `hbm` is valid; `bmp` is sized for the `BITMAP` GDI writes.
        let written = unsafe {
            GetObjectW(
                hbm,
                std::mem::size_of::<BITMAP>() as i32,
                Some(&mut bmp as *mut BITMAP as *mut c_void),
            )
        };
        if written == 0 {
            return None;
        }
        let width = bmp.bmWidth;
        let height = bmp.bmHeight;
        if width <= 0 || height <= 0 {
            return None;
        }

        // SAFETY: `None` requests a DC for the whole screen; released below.
        let hdc = unsafe { GetDC(None) };
        if hdc.is_invalid() {
            return None;
        }

        let mut bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                // Negative height requests a top-down DIB (row 0 = top row),
                // matching the row order `image::RgbaImage` expects.
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };

        let row_bytes = width as usize * 4;
        let mut buf = vec![0u8; row_bytes * height as usize];
        // SAFETY: `hdc`/`hbm` are valid; `buf` holds exactly `height` top-down
        // 32bpp rows matching `bmi`.
        let lines = unsafe {
            GetDIBits(
                hdc,
                hbm,
                0,
                height as u32,
                Some(buf.as_mut_ptr() as *mut c_void),
                &mut bmi,
                DIB_RGB_COLORS,
            )
        };
        // SAFETY: releases the DC obtained from `GetDC(None)` above.
        unsafe {
            let _ = ReleaseDC(None, hdc);
        }

        if lines == 0 {
            return None;
        }

        // GDI hands back BGRA; swap to RGBA. Some icons report an all-zero
        // alpha channel (no real transparency info) — treat that as fully
        // opaque rather than rendering an invisible icon.
        let mut has_alpha = false;
        for px in buf.chunks_exact_mut(4) {
            px.swap(0, 2);
            if px[3] != 0 {
                has_alpha = true;
            }
        }
        if !has_alpha {
            for px in buf.chunks_exact_mut(4) {
                px[3] = 255;
            }
        }

        let img = image::RgbaImage::from_raw(width as u32, height as u32, buf)?;
        let mut out = Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).ok()?;
        Some(out.into_inner())
    }
}

// ---------------- Linux ----------------

#[cfg(target_os = "linux")]
fn app_icon_png(exe_path: &Path) -> Option<Vec<u8>> {
    linux_icon::extract(exe_path)
}

#[cfg(target_os = "linux")]
mod linux_icon {
    use std::path::Path;

    /// Resolve the exe's icon in the freedesktop icon theme and return raster
    /// bytes the `image` crate can decode. Best-effort: many apps' icon name
    /// equals their binary name, but this is a heuristic, not guaranteed —
    /// any miss or non-raster hit (e.g. SVG) falls back to `None`.
    pub fn extract(exe_path: &Path) -> Option<Vec<u8>> {
        let name = exe_path.file_name()?.to_str()?.to_lowercase();
        let path = freedesktop_icons::lookup(&name)
            .with_size(64)
            .with_cache()
            .find()?;
        // `image` can't decode SVG (the common freedesktop icon format); only
        // take the hit if the theme gave us something it can actually read.
        let is_png = path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("png"));
        if !is_png {
            return None;
        }
        std::fs::read(&path).ok()
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
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
