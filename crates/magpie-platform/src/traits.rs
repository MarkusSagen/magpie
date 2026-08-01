use magpie_core::{AppInfo, Content};

pub struct ClipboardSnapshot {
    pub content: Option<Content>,
    pub change_token: u64,
    pub concealed: bool,
}

pub trait Clipboard: Send {
    fn snapshot(&mut self) -> ClipboardSnapshot;
    fn set_text(&mut self, text: &str) -> Result<(), String>;
    fn set_content(&mut self, content: &Content) -> Result<(), String>;
}

pub trait SourceApp: Send {
    fn frontmost(&self) -> Option<AppInfo>;
}

pub trait Paster: Send {
    fn paste(&self) -> Result<(), String>;
}

pub trait Autostart {
    fn is_enabled(&self) -> bool;
    fn set_enabled(&self, on: bool) -> Result<(), String>;
}
