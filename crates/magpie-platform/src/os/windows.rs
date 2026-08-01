use crate::traits::{Clipboard, ClipboardSnapshot};
use magpie_core::Content;
use windows::core::PCWSTR;
use windows::Win32::System::DataExchange::{
    GetClipboardSequenceNumber, IsClipboardFormatAvailable, RegisterClipboardFormatW,
};

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn format_available(name: &str) -> bool {
    unsafe {
        let w = wide(name);
        let id = RegisterClipboardFormatW(PCWSTR(w.as_ptr()));
        id != 0 && IsClipboardFormatAvailable(id).is_ok()
    }
}

pub struct WinClipboard {
    inner: arboard::Clipboard,
}

impl WinClipboard {
    pub fn new() -> Result<Self, String> {
        Ok(WinClipboard { inner: arboard::Clipboard::new().map_err(|e| e.to_string())? })
    }
}

impl Clipboard for WinClipboard {
    fn snapshot(&mut self) -> ClipboardSnapshot {
        let change_token = unsafe { GetClipboardSequenceNumber() } as u64;
        let concealed = format_available("ExcludeClipboardContentFromMonitorProcessing")
            || format_available("CanIncludeInClipboardHistory");
        let content = if let Ok(text) = self.inner.get_text() {
            if text.is_empty() { None } else { Some(Content::Text(text)) }
        } else if let Ok(img) = self.inner.get_image() {
            let mut bytes = Vec::with_capacity(8 + img.bytes.len());
            bytes.extend_from_slice(&(img.width as u32).to_le_bytes());
            bytes.extend_from_slice(&(img.height as u32).to_le_bytes());
            bytes.extend_from_slice(&img.bytes);
            Some(Content::Image { bytes })
        } else {
            None
        };
        ClipboardSnapshot { content, change_token, concealed }
    }

    fn set_text(&mut self, text: &str) -> Result<(), String> {
        self.inner.set_text(text.to_string()).map_err(|e| e.to_string())
    }

    fn set_content(&mut self, content: &Content) -> Result<(), String> {
        match content {
            Content::Text(t) => self.inner.set_text(t.clone()).map_err(|e| e.to_string()),
            Content::Rich { text, .. } => self.inner.set_text(text.clone()).map_err(|e| e.to_string()),
            Content::Files(p) => self.inner.set_text(p.join("\n")).map_err(|e| e.to_string()),
            Content::Image { .. } => Err("image set not supported in v1".into()),
        }
    }
}
