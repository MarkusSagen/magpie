pub fn content_token(text: Option<&str>, image_len: Option<usize>) -> u64 {
    if text.is_none() && image_len.is_none() {
        return 0;
    }
    let mut h = blake3::Hasher::new();
    if let Some(t) = text {
        h.update(b"t");
        h.update(t.as_bytes());
    }
    if let Some(n) = image_len {
        h.update(b"i");
        h.update(&(n as u64).to_le_bytes());
    }
    let hash = h.finalize();
    let mut b = [0u8; 8];
    b.copy_from_slice(&hash.as_bytes()[..8]);
    u64::from_le_bytes(b)
}

#[cfg(target_os = "linux")]
mod imp {
    use super::content_token;
    use crate::traits::{Clipboard, ClipboardSnapshot};
    use magpie_core::Content;

    pub struct LinuxClipboard {
        inner: arboard::Clipboard,
    }

    impl LinuxClipboard {
        pub fn new() -> Result<Self, String> {
            Ok(LinuxClipboard {
                inner: arboard::Clipboard::new().map_err(|e| e.to_string())?,
            })
        }
    }

    impl Clipboard for LinuxClipboard {
        fn snapshot(&mut self) -> ClipboardSnapshot {
            if let Ok(text) = self.inner.get_text() {
                if !text.is_empty() {
                    let token = content_token(Some(&text), None);
                    return ClipboardSnapshot {
                        content: Some(Content::Text(text)),
                        change_token: token,
                        concealed: false,
                    };
                }
            }
            if let Ok(img) = self.inner.get_image() {
                let mut bytes = Vec::with_capacity(8 + img.bytes.len());
                bytes.extend_from_slice(&(img.width as u32).to_le_bytes());
                bytes.extend_from_slice(&(img.height as u32).to_le_bytes());
                bytes.extend_from_slice(&img.bytes);
                let token = content_token(None, Some(bytes.len()));
                return ClipboardSnapshot {
                    content: Some(Content::Image { bytes }),
                    change_token: token,
                    concealed: false,
                };
            }
            ClipboardSnapshot {
                content: None,
                change_token: 0,
                concealed: false,
            }
        }

        fn set_text(&mut self, text: &str) -> Result<(), String> {
            self.inner
                .set_text(text.to_string())
                .map_err(|e| e.to_string())
        }

        fn set_content(&mut self, content: &Content) -> Result<(), String> {
            match content {
                Content::Text(t) => self.inner.set_text(t.clone()).map_err(|e| e.to_string()),
                Content::Rich { text, .. } => {
                    self.inner.set_text(text.clone()).map_err(|e| e.to_string())
                }
                Content::Files(p) => self.inner.set_text(p.join("\n")).map_err(|e| e.to_string()),
                Content::Image { .. } => Err("image set not supported in v1".into()),
            }
        }
    }
}

#[cfg(target_os = "linux")]
pub use imp::LinuxClipboard;

#[cfg(test)]
mod tests {
    use super::content_token;

    #[test]
    fn same_text_same_token_diff_text_diff_token() {
        assert_eq!(
            content_token(Some("hello"), None),
            content_token(Some("hello"), None)
        );
        assert_ne!(
            content_token(Some("a"), None),
            content_token(Some("b"), None)
        );
    }

    #[test]
    fn empty_is_zero() {
        assert_eq!(content_token(None, None), 0);
    }
}
