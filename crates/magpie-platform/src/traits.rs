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

/// Lets `Watcher<Box<dyn Clipboard>, _>` and boxed paste targets work — the
/// cfg-selected factory returns a boxed clipboard.
impl Clipboard for Box<dyn Clipboard> {
    fn snapshot(&mut self) -> ClipboardSnapshot {
        (**self).snapshot()
    }
    fn set_text(&mut self, text: &str) -> Result<(), String> {
        (**self).set_text(text)
    }
    fn set_content(&mut self, content: &Content) -> Result<(), String> {
        (**self).set_content(content)
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use magpie_core::Content;

    struct Fake(u64);
    impl Clipboard for Fake {
        fn snapshot(&mut self) -> ClipboardSnapshot {
            ClipboardSnapshot {
                content: None,
                change_token: self.0,
                concealed: false,
            }
        }
        fn set_text(&mut self, _t: &str) -> Result<(), String> {
            Ok(())
        }
        fn set_content(&mut self, _c: &Content) -> Result<(), String> {
            Ok(())
        }
    }

    #[test]
    fn boxed_clipboard_forwards() {
        let mut b: Box<dyn Clipboard> = Box::new(Fake(42));
        assert_eq!(b.snapshot().change_token, 42);
    }
}
