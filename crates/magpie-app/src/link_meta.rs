//! Best-effort page metadata for bookmarks: <title> and og:image. Parsing is pure
//! (tested); fetching uses ureq (short timeout, size cap), best-effort.

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
}
