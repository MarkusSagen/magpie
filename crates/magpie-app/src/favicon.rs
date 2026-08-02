//! Best-effort site favicons for link entries. Fetches the site's OWN
//! `/favicon.ico` only — never a third-party favicon aggregator (privacy).

use std::path::{Path, PathBuf};

/// Lowercased host of an http(s) URL, without port. `None` for non-URLs or hosts
/// without a dot (e.g. `localhost`).
pub fn domain_of(url: &str) -> Option<String> {
    let rest = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))?;
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = host.rsplit('@').next().unwrap_or(host); // drop userinfo
    let host = host.split(':').next().unwrap_or(host); // drop port
    if host.is_empty() || !host.contains('.') {
        return None;
    }
    Some(host.to_lowercase())
}

/// `<dir>/<domain>.png`.
pub fn favicon_cache_path(dir: &Path, domain: &str) -> PathBuf {
    dir.join(format!("{domain}.png"))
}

/// Cached favicon path if present; else fetch + write. `fetch` is injected so the
/// logic is testable without network.
pub fn ensure_favicon(
    dir: &Path,
    domain: &str,
    fetch: impl Fn(&str) -> Option<Vec<u8>>,
) -> Option<PathBuf> {
    let path = favicon_cache_path(dir, domain);
    if path.exists() {
        return Some(path);
    }
    let bytes = fetch(domain)?;
    if bytes.is_empty() {
        return None;
    }
    std::fs::create_dir_all(dir).ok()?;
    std::fs::write(&path, &bytes).ok()?;
    Some(path)
}

/// Real fetch: the site's own `/favicon.ico`, short timeout, size-capped.
pub fn fetch_favicon(domain: &str) -> Option<Vec<u8>> {
    let url = format!("https://{domain}/favicon.ico");
    let resp = ureq::get(&url)
        .timeout(std::time::Duration::from_secs(4))
        .call()
        .ok()?;
    use std::io::Read;
    let mut buf = Vec::new();
    resp.into_reader()
        .take(512 * 1024)
        .read_to_end(&mut buf)
        .ok()?;
    if buf.is_empty() {
        None
    } else {
        Some(buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn domain_parsing() {
        assert_eq!(
            domain_of("https://github.com/a/b?x=1"),
            Some("github.com".into())
        );
        assert_eq!(
            domain_of("http://Example.COM:8080/x"),
            Some("example.com".into())
        );
        assert_eq!(domain_of("not a url"), None);
        assert_eq!(domain_of("https://localhost/x"), None); // no dot
    }

    #[test]
    fn ensure_uses_cache_then_writes() {
        let dir = std::env::temp_dir().join(format!("magpie-fav-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let p = ensure_favicon(&dir, "example.com", |_| Some(vec![1, 2, 3])).unwrap();
        assert!(p.exists());
        // second call must NOT fetch (would panic) — file already cached.
        let p2 = ensure_favicon(&dir, "example.com", |_| panic!("should be cached")).unwrap();
        assert_eq!(p, p2);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn ensure_returns_none_when_fetch_fails() {
        let dir = std::env::temp_dir().join(format!("magpie-fav2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(ensure_favicon(&dir, "nope.example", |_| None).is_none());
        std::fs::remove_dir_all(&dir).ok();
    }
}
