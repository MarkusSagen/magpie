//! Best-effort page metadata for bookmarks: <title> and og:image. Parsing is pure
//! (tested); fetching uses ureq (short timeout, size cap), best-effort.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LinkMeta {
    pub title: Option<String>,
    pub image: Option<String>,
}

/// Extract <title> and og:image from raw HTML. Case-insensitive tags; minimal entity
/// decode. Best-effort — never panics.
pub fn parse_link_meta(html: &str) -> LinkMeta {
    let lower = html.to_lowercase();
    let title =
        extract_between(&lower, html, "<title", "</title>").map(|t| decode_entities(t.trim()));
    let image = extract_og_image(&lower, html);
    LinkMeta {
        title: title.filter(|s| !s.is_empty()),
        image: image.filter(|s| !s.is_empty()),
    }
}

fn extract_between(lower: &str, orig: &str, open: &str, close: &str) -> Option<String> {
    let o = lower.find(open)?;
    let gt = lower[o..].find('>')? + o + 1;
    let c = lower[gt..].find(close)? + gt;
    Some(orig[gt..c].to_string())
}

fn extract_og_image(lower: &str, orig: &str) -> Option<String> {
    let mut search = 0;
    while let Some(rel) = lower[search..].find("<meta") {
        let start = search + rel;
        let end = lower[start..]
            .find('>')
            .map(|e| start + e + 1)
            .unwrap_or(orig.len());
        let tag_lower = &lower[start..end];
        let tag_orig = &orig[start..end];
        if tag_lower.contains("property=\"og:image\"")
            || tag_lower.contains("property='og:image'")
            || tag_lower.contains("name=\"og:image\"")
        {
            if let Some(v) = attr_value(tag_lower, tag_orig, "content") {
                return Some(decode_entities(v.trim()));
            }
        }
        search = end;
    }
    None
}

fn attr_value(tag_lower: &str, tag_orig: &str, attr: &str) -> Option<String> {
    let key = format!("{attr}=");
    let k = tag_lower.find(&key)? + key.len();
    let quote = *tag_lower.as_bytes().get(k)?;
    if quote != b'"' && quote != b'\'' {
        return None;
    }
    let rest = &tag_lower[k + 1..];
    let end = rest.find(quote as char)? + k + 1;
    Some(tag_orig[k + 1..end].to_string())
}

fn decode_entities(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
}

/// Fetch a page and extract its metadata (best-effort, ~5s timeout, 512 KB cap).
pub fn fetch_link_meta(url: &str) -> LinkMeta {
    let resp = ureq::get(url)
        .timeout(std::time::Duration::from_secs(5))
        .call();
    let Ok(resp) = resp else {
        return LinkMeta::default();
    };
    let mut buf = String::new();
    use std::io::Read;
    let _ = resp.into_reader().take(512 * 1024).read_to_string(&mut buf);
    parse_link_meta(&buf)
}

/// Stable cache filename for a URL's preview image — FNV-1a hex (no crypto dep
/// needed; we just need a deterministic, filesystem-safe name per URL).
pub fn preview_cache_path(dir: &Path, url: &str) -> PathBuf {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in url.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    dir.join(format!("{h:016x}.png"))
}

/// Cache-or-fetch a URL's preview thumbnail. `fetch` is injected so the logic is
/// testable without network (mirror `favicon::ensure_favicon`). Returns the
/// cached PNG path, or `None` on a fetch miss.
pub fn ensure_preview(
    dir: &Path,
    url: &str,
    fetch: impl Fn(&str) -> Option<Vec<u8>>,
) -> Option<PathBuf> {
    let path = preview_cache_path(dir, url);
    if path.exists() {
        return Some(path);
    }
    let bytes = fetch(url)?;
    if bytes.is_empty() {
        return None;
    }
    std::fs::create_dir_all(dir).ok()?;
    std::fs::write(&path, &bytes).ok()?;
    Some(path)
}

/// Real fetch: page OG image → a small PNG thumbnail (longest edge ~400px).
/// Best-effort — never panics, returns `None` on any failure.
pub fn fetch_preview_image(url: &str) -> Option<Vec<u8>> {
    let img_url = fetch_link_meta(url).image?;
    if !img_url.starts_with("http") {
        return None; // absolute only for v1
    }
    let resp = ureq::get(&img_url)
        .timeout(std::time::Duration::from_secs(5))
        .call()
        .ok()?;
    use std::io::Read;
    let mut buf = Vec::new();
    resp.into_reader()
        .take(4 * 1024 * 1024)
        .read_to_end(&mut buf)
        .ok()?;
    if buf.is_empty() {
        return None;
    }
    let img = image::load_from_memory(&buf).ok()?;
    let small = img.thumbnail(400, 400);
    let mut out = std::io::Cursor::new(Vec::new());
    small.write_to(&mut out, image::ImageFormat::Png).ok()?;
    Some(out.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_title_and_og_image() {
        let html = r#"<html><head><TITLE>Hello &amp; Bye</TITLE>
            <meta property="og:image" content="https://x.com/a.png"></head></html>"#;
        let m = parse_link_meta(html);
        assert_eq!(m.title.as_deref(), Some("Hello & Bye"));
        assert_eq!(m.image.as_deref(), Some("https://x.com/a.png"));
    }
    #[test]
    fn missing_is_none() {
        let m = parse_link_meta("<html><body>no head</body></html>");
        assert!(m.title.is_none() && m.image.is_none());
    }

    #[test]
    fn preview_cache_path_is_deterministic_and_differs_per_url() {
        let dir = Path::new("/tmp/magpie-previews");
        let a1 = preview_cache_path(dir, "https://a.example/x");
        let a2 = preview_cache_path(dir, "https://a.example/x");
        let b = preview_cache_path(dir, "https://b.example/y");
        assert_eq!(a1, a2);
        assert_ne!(a1, b);
        assert!(a1.starts_with(dir));
        assert_eq!(a1.extension().and_then(|e| e.to_str()), Some("png"));
    }

    #[test]
    fn ensure_preview_uses_cache_then_writes() {
        let dir = std::env::temp_dir().join(format!("magpie-preview-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let p = ensure_preview(&dir, "https://example.com/a", |_| {
            calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Some(vec![1, 2, 3])
        })
        .unwrap();
        assert!(p.exists());
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        // Second call must be a cache hit — fetch not called again.
        let p2 = ensure_preview(&dir, "https://example.com/a", |_| {
            calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Some(vec![9, 9, 9])
        })
        .unwrap();
        assert_eq!(p, p2);
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn ensure_preview_returns_none_when_fetch_fails() {
        let dir = std::env::temp_dir().join(format!("magpie-preview2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(ensure_preview(&dir, "https://nope.example/x", |_| None).is_none());
        std::fs::remove_dir_all(&dir).ok();
    }
}
