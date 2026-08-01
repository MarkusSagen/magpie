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
    pub fn write_thumbnail(&self, hash: &str, w: u32, h: u32, rgba: &[u8], max_edge: u32) -> std::io::Result<String> {
        self.ensure_dir()?;
        let img = image::RgbaImage::from_raw(w, h, rgba.to_vec())
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "bad rgba dims"))?;
        let thumb = image::imageops::thumbnail(&img, max_edge.min(w), max_edge.min(h));
        let path = self.dir.join(format!("{hash}.thumb.png"));
        thumb.save(&path).map_err(std::io::Error::other)?;
        Ok(path.to_string_lossy().into_owned())
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
    fn write_thumbnail_produces_png() {
        let dir = std::env::temp_dir().join(format!("magpie-thumb-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = FsImageStore { dir: dir.clone() };
        // 2x2 RGBA red image
        let rgba = vec![255, 0, 0, 255,  255, 0, 0, 255,  255, 0, 0, 255,  255, 0, 0, 255];
        let path = store.write_thumbnail("deadbeef", 2, 2, &rgba, 64).unwrap();
        assert!(std::path::Path::new(&path).exists());
        assert!(path.ends_with("deadbeef.thumb.png"));
        std::fs::remove_dir_all(&dir).ok();
    }
}
