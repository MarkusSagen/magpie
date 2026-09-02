use magpie_core::ImageStore;
use std::path::PathBuf;

pub struct FsImageStore {
    pub dir: PathBuf,
}

impl FsImageStore {
    fn ensure_dir(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.dir)
    }

    /// Downscale RGBA to a PNG thumbnail (longest edge = `max_edge`) at `<hash>.thumb.png`.
    pub fn write_thumbnail(
        &self,
        hash: &str,
        w: u32,
        h: u32,
        rgba: &[u8],
        max_edge: u32,
    ) -> std::io::Result<String> {
        self.ensure_dir()?;
        let img = image::RgbaImage::from_raw(w, h, rgba.to_vec())
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "bad rgba dims"))?;
        let (tw, th) = fit_within(w, h, max_edge);
        let thumb = image::imageops::thumbnail(&img, tw, th);
        let path = self.dir.join(format!("{hash}.thumb.png"));
        thumb.save(&path).map_err(std::io::Error::other)?;
        Ok(path.to_string_lossy().into_owned())
    }

    pub fn thumbnail_path(&self, hash: &str) -> PathBuf {
        self.dir.join(format!("{hash}.thumb.png"))
    }

    /// Path to a **displayable** (`.png`) thumbnail for a stored image, creating
    /// it on first use.
    ///
    /// Originals are stored as `<hash>.bin` holding a tagged-RGBA blob (see
    /// [`parse_tagged_rgba`]) — not an encoded image file. Slint's
    /// `Image::load_from_path` needs a real image with a known extension, so
    /// without this every image entry rendered blank. This is the "real PNG
    /// re-encoding + thumbnailing happens in the app's image cache" step that
    /// `magpie_platform::os::macos::tagged_rgba` documents but nothing performed.
    pub fn ensure_thumbnail(&self, bin_path: &str, hash: &str, max_edge: u32) -> Option<String> {
        let thumb = self.thumbnail_path(hash);
        if thumb.exists() {
            return Some(thumb.to_string_lossy().into_owned());
        }
        let bytes = std::fs::read(bin_path).ok()?;
        let (w, h, rgba) = parse_tagged_rgba(&bytes)?;
        self.write_thumbnail(hash, w, h, rgba, max_edge).ok()
    }

    /// Pixel dimensions of a stored original, from its 8-byte header — no decode.
    pub fn dimensions(&self, bin_path: &str) -> Option<(u32, u32)> {
        use std::io::Read;
        let mut f = std::fs::File::open(bin_path).ok()?;
        let len = f.metadata().ok()?.len();
        let mut head = [0u8; RGBA_HEADER];
        f.read_exact(&mut head).ok()?;
        let (w, h) = header_dims(&head);
        // Only trust the header if the file length matches it exactly.
        (expected_len(w, h) == Some(len)).then_some((w, h))
    }
}

/// Byte length of the tagged-RGBA header: two little-endian `u32`s.
const RGBA_HEADER: usize = 8;

fn header_dims(head: &[u8; RGBA_HEADER]) -> (u32, u32) {
    (
        u32::from_le_bytes([head[0], head[1], head[2], head[3]]),
        u32::from_le_bytes([head[4], head[5], head[6], head[7]]),
    )
}

fn expected_len(w: u32, h: u32) -> Option<u64> {
    (w as u64)
        .checked_mul(h as u64)?
        .checked_mul(4)?
        .checked_add(RGBA_HEADER as u64)
}

/// Parse `[u32 LE width][u32 LE height][RGBA bytes…]` — the container that every
/// `magpie-platform` clipboard adapter wraps `arboard`'s raw pixels in (macOS,
/// Linux and Windows all build it identically; see `os::macos::tagged_rgba`).
/// Clipboard images arrive as raw pixels, not as a PNG, which is why sniffing an
/// image format off these bytes fails.
///
/// Returns `None` unless the length matches `width * height * 4` exactly, so a
/// truncated or foreign blob is ignored rather than rendered as garbage.
pub fn parse_tagged_rgba(bytes: &[u8]) -> Option<(u32, u32, &[u8])> {
    if bytes.len() < RGBA_HEADER {
        return None;
    }
    let head: [u8; RGBA_HEADER] = bytes[..RGBA_HEADER].try_into().ok()?;
    let (w, h) = header_dims(&head);
    if w == 0 || h == 0 || expected_len(w, h) != Some(bytes.len() as u64) {
        return None;
    }
    Some((w, h, &bytes[RGBA_HEADER..]))
}

/// Scale `w`×`h` down so the longest edge is at most `max_edge`, preserving the
/// aspect ratio. Clamping each edge independently (the previous behaviour)
/// squashed anything non-square.
fn fit_within(w: u32, h: u32, max_edge: u32) -> (u32, u32) {
    let longest = w.max(h);
    if longest <= max_edge || longest == 0 {
        return (w.max(1), h.max(1));
    }
    let scale = max_edge as f64 / longest as f64;
    (
        ((w as f64 * scale).round() as u32).max(1),
        ((h as f64 * scale).round() as u32).max(1),
    )
}

impl FsImageStore {
    /// Best-effort removal of each image file plus its thumbnail sibling.
    pub fn remove_paths(&self, paths: &[String]) {
        for p in paths {
            let _ = std::fs::remove_file(p);
            if let Some(stem) = p.strip_suffix(".bin") {
                let _ = std::fs::remove_file(format!("{stem}.thumb.png"));
            }
        }
    }
}

impl ImageStore for FsImageStore {
    fn put(&self, hash: &str, bytes: &[u8]) -> std::io::Result<String> {
        self.ensure_dir()?;
        let path = self.dir.join(format!("{hash}.bin"));
        if !path.exists() {
            std::fs::write(&path, bytes)?;
        }
        Ok(path.to_string_lossy().into_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use magpie_core::ImageStore;

    #[test]
    fn put_is_content_addressed_and_idempotent() {
        let dir = std::env::temp_dir().join(format!("magpie-img-{}", std::process::id()));
        let store = FsImageStore { dir: dir.clone() };
        let p1 = store.put("abc123", &[1, 2, 3]).unwrap();
        let p2 = store.put("abc123", &[1, 2, 3]).unwrap();
        assert_eq!(p1, p2);
        assert!(std::path::Path::new(&p1).exists());
        assert!(p1.ends_with("abc123.bin"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn remove_paths_deletes_bin_and_thumbnail_and_ignores_missing() {
        let dir = std::env::temp_dir().join(format!("magpie-rm-{}", std::process::id()));
        let store = FsImageStore { dir: dir.clone() };
        let bin = store.put("abcd", &[1, 2, 3]).unwrap();
        let rgba = vec![255, 0, 0, 255];
        let thumb = store.write_thumbnail("abcd", 1, 1, &rgba, 8).unwrap();
        assert!(std::path::Path::new(&bin).exists());
        assert!(std::path::Path::new(&thumb).exists());

        store.remove_paths(&[bin.clone(), "/no/such/file.bin".to_string()]);
        assert!(!std::path::Path::new(&bin).exists());
        assert!(!std::path::Path::new(&thumb).exists()); // sibling removed too
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn fit_within_preserves_aspect_ratio() {
        // Was the bug: each edge clamped independently squashed wide images.
        assert_eq!(super::fit_within(1600, 400, 640), (640, 160));
        assert_eq!(super::fit_within(400, 1600, 640), (160, 640));
        assert_eq!(super::fit_within(100, 50, 640), (100, 50)); // no upscaling
        assert_eq!(super::fit_within(0, 0, 640), (1, 1)); // never zero-sized
    }

    /// Exactly the blob the platform clipboards build: LE width, LE height, RGBA.
    fn tagged(w: u32, h: u32) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&w.to_le_bytes());
        v.extend_from_slice(&h.to_le_bytes());
        v.extend(std::iter::repeat_n(200u8, (w * h * 4) as usize));
        v
    }

    #[test]
    fn parse_tagged_rgba_reads_the_platform_container() {
        let blob = tagged(4, 2);
        let (w, h, rgba) = super::parse_tagged_rgba(&blob).unwrap();
        assert_eq!((w, h), (4, 2));
        assert_eq!(rgba.len(), 4 * 2 * 4);
    }

    #[test]
    fn parse_tagged_rgba_rejects_bad_blobs() {
        assert!(super::parse_tagged_rgba(b"").is_none());
        assert!(super::parse_tagged_rgba(b"hello world").is_none());
        assert!(super::parse_tagged_rgba(&tagged(4, 2)[..20]).is_none()); // truncated
        let mut zero = tagged(1, 1);
        zero[0..4].copy_from_slice(&0u32.to_le_bytes()); // zero width
        assert!(super::parse_tagged_rgba(&zero).is_none());
    }

    /// The end-to-end fix: a stored `.bin` original becomes a loadable `.png`.
    #[test]
    fn ensure_thumbnail_turns_a_bin_original_into_a_png() {
        let dir = std::env::temp_dir().join(format!("magpie-ensure-{}", std::process::id()));
        let store = FsImageStore { dir: dir.clone() };
        let bin = store.put("feedface", &tagged(80, 40)).unwrap();
        assert!(bin.ends_with(".bin"));

        let thumb = store.ensure_thumbnail(&bin, "feedface", 20).unwrap();
        assert!(thumb.ends_with("feedface.thumb.png"));
        assert!(std::path::Path::new(&thumb).exists());
        // Aspect ratio preserved on the way down: 80x40 capped at 20 -> 20x10.
        assert_eq!(image::image_dimensions(&thumb).unwrap(), (20, 10));
        // Second call is a cache hit returning the same path.
        assert_eq!(store.ensure_thumbnail(&bin, "feedface", 20).unwrap(), thumb);
        // Dimensions come from the header, and describe the ORIGINAL.
        assert_eq!(store.dimensions(&bin), Some((80, 40)));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn ensure_thumbnail_gives_up_on_non_image_bytes() {
        let dir = std::env::temp_dir().join(format!("magpie-nonimg-{}", std::process::id()));
        let store = FsImageStore { dir: dir.clone() };
        let bin = store.put("notanimage", b"hello world").unwrap();
        assert!(store.ensure_thumbnail(&bin, "notanimage", 64).is_none());
        assert_eq!(store.dimensions(&bin), None);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn write_thumbnail_produces_png() {
        let dir = std::env::temp_dir().join(format!("magpie-thumb-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = FsImageStore { dir: dir.clone() };
        // 2x2 RGBA red image
        let rgba = vec![
            255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255,
        ];
        let path = store.write_thumbnail("deadbeef", 2, 2, &rgba, 64).unwrap();
        assert!(std::path::Path::new(&path).exists());
        assert!(path.ends_with("deadbeef.thumb.png"));
        std::fs::remove_dir_all(&dir).ok();
    }
}
