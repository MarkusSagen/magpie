//! File/media thumbnails for clipboard file entries. On macOS we use **QuickLook**
//! (`qlmanage -t`), which renders a thumbnail for PDFs, images, video (poster frame),
//! audio (album art), epub, and many document types with no extra dependencies — one
//! native mechanism for every file type. Results are cached (keyed by path + mtime) like
//! app icons. Off macOS this is currently a no-op (callers fall back to the kind icon).

use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

/// Deterministic cache filename for `file` at `size` (path + mtime → stable hash, so a
/// changed file re-thumbnails). `DefaultHasher` is seeded fixed, so it's stable across runs.
fn cache_name(file: &Path, size: u32) -> String {
    let mtime = std::fs::metadata(file)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut h = std::collections::hash_map::DefaultHasher::new();
    file.to_string_lossy().hash(&mut h);
    mtime.hash(&mut h);
    size.hash(&mut h);
    format!("{:016x}.png", h.finish())
}

/// Return a cached thumbnail PNG for `file` (generating it if needed), or `None` if one
/// can't be produced. `size` is the max edge in points. Best-effort; never panics.
pub fn ensure_file_thumbnail(cache_dir: &Path, file: &Path, size: u32) -> Option<PathBuf> {
    if !file.exists() || file.is_dir() {
        return None;
    }
    let out = cache_dir.join(cache_name(file, size));
    if out.exists() {
        return Some(out);
    }
    std::fs::create_dir_all(cache_dir).ok()?;
    generate(file, size, &out).then_some(out)
}

/// macOS: `qlmanage -t -s <size> -o <tmp> <file>` writes `<tmp>/<basename>.png`; move it
/// to `out`. Returns true on success.
#[cfg(target_os = "macos")]
fn generate(file: &Path, size: u32, out: &Path) -> bool {
    let tmp = match tempdir() {
        Some(t) => t,
        None => return false,
    };
    let status = std::process::Command::new("qlmanage")
        .arg("-t")
        .arg("-s")
        .arg(size.to_string())
        .arg("-o")
        .arg(&tmp)
        .arg(file)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    if !matches!(status, Ok(s) if s.success()) {
        let _ = std::fs::remove_dir_all(&tmp);
        return false;
    }
    // qlmanage names the output "<file-name>.png" in the out dir; take the first PNG.
    let produced = std::fs::read_dir(&tmp).ok().and_then(|rd| {
        rd.filter_map(|e| e.ok().map(|e| e.path()))
            .find(|p| p.extension().map(|x| x == "png").unwrap_or(false))
    });
    let ok = match produced {
        Some(p) => std::fs::rename(&p, out)
            .or_else(|_| std::fs::copy(&p, out).map(|_| ()))
            .is_ok(),
        None => false,
    };
    let _ = std::fs::remove_dir_all(&tmp);
    ok
}

#[cfg(target_os = "macos")]
fn tempdir() -> Option<PathBuf> {
    let base = std::env::temp_dir().join(format!(
        "magpie-ql-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&base).ok().map(|_| base)
}

#[cfg(not(target_os = "macos"))]
fn generate(_file: &Path, _size: u32, _out: &Path) -> bool {
    // Non-macOS: no QuickLook. Images could be down-scaled via the `image` crate by the
    // app layer; other types fall back to the kind icon. Left as a no-op here.
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_name_is_stable_and_png() {
        let p = Path::new("/nonexistent/file.pdf");
        let a = cache_name(p, 256);
        let b = cache_name(p, 256);
        assert_eq!(a, b);
        assert!(a.ends_with(".png"));
        assert_ne!(cache_name(p, 256), cache_name(p, 512));
    }

    #[test]
    fn missing_file_yields_none() {
        let dir = std::env::temp_dir().join("magpie-thumb-test");
        assert!(ensure_file_thumbnail(&dir, Path::new("/no/such/file.pdf"), 256).is_none());
    }
}
