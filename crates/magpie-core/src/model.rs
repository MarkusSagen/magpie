use crate::detect::Kind;

#[derive(Debug, Clone)]
pub struct AppInfo {
    pub identifier: String,
    pub display_name: String,
    pub icon_path: Option<String>,
}

#[derive(Debug, Clone)]
pub enum Content {
    Text(String),
    Rich { text: String, html: Option<String>, rtf: Option<String> },
    Image { bytes: Vec<u8> },
    Files(Vec<String>),
}

#[derive(Debug, Clone)]
pub struct CaptureEvent {
    pub content: Content,
    pub source_app: Option<AppInfo>,
    pub copied_at_ms: i64,
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub id: i64,
    pub content_hash: String,
    pub kind: Kind,
    pub preview_text: String,
    pub full_text: String,
    pub image_path: Option<String>,
    pub byte_size: i64,
    pub char_count: i64,
    pub word_count: i64,
    pub line_count: i64,
    pub first_copied_at_ms: i64,
    pub last_copied_at_ms: i64,
    pub copy_count: i64,
    pub pinned: bool,
    pub source_app_id: Option<i64>,
}

pub fn content_hash(content: &Content) -> String {
    let mut h = blake3::Hasher::new();
    match content {
        Content::Text(t) => { h.update(b"text\0"); h.update(t.as_bytes()); }
        Content::Rich { text, .. } => { h.update(b"text\0"); h.update(text.as_bytes()); }
        Content::Image { bytes } => { h.update(b"image\0"); h.update(bytes); }
        Content::Files(paths) => {
            h.update(b"files\0");
            for p in paths { h.update(p.as_bytes()); h.update(b"\0"); }
        }
    }
    h.finalize().to_hex().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_text_hashes_equal() {
        let a = content_hash(&Content::Text("hello".into()));
        let b = content_hash(&Content::Text("hello".into()));
        assert_eq!(a, b);
    }

    #[test]
    fn different_text_hashes_differ() {
        assert_ne!(
            content_hash(&Content::Text("a".into())),
            content_hash(&Content::Text("b".into())),
        );
    }

    #[test]
    fn same_string_different_variant_does_not_collide() {
        assert_ne!(
            content_hash(&Content::Text("x".into())),
            content_hash(&Content::Files(vec!["x".into()])),
        );
    }

    #[test]
    fn image_hashes_by_bytes() {
        assert_eq!(
            content_hash(&Content::Image { bytes: vec![1, 2, 3] }),
            content_hash(&Content::Image { bytes: vec![1, 2, 3] }),
        );
        assert_ne!(
            content_hash(&Content::Image { bytes: vec![1, 2, 3] }),
            content_hash(&Content::Image { bytes: vec![9] }),
        );
    }
}
