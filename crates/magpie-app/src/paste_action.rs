use magpie_core::{Content, Entry};
use magpie_platform::{Clipboard, Paster};

pub fn resolve_quick_paste(recent: &[Entry], slot: usize) -> Option<&Entry> {
    if slot == 0 {
        return None;
    }
    recent.get(slot - 1)
}

/// Pick the slot entry if assigned, else the `slot`-th most-recent (1-based).
pub fn resolve_slot_or_recent(
    slot_entry: Option<Entry>,
    recent: &[Entry],
    slot: usize,
) -> Option<Entry> {
    match slot_entry {
        Some(e) => Some(e),
        None => resolve_quick_paste(recent, slot).cloned(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasteKind {
    Formatted,
    PlainText,
}

pub fn perform_paste(
    clip: &mut dyn Clipboard,
    paster: &dyn Paster,
    entry: &Entry,
    _kind: PasteKind,
    auto: bool,
) -> Result<(), String> {
    // v1 stores text for all non-image kinds; both paste kinds emit full_text.
    clip.set_content(&Content::Text(entry.full_text.clone()))?;
    if auto {
        paster.paste()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use magpie_core::{Content, Entry, Kind};
    use std::cell::Cell;

    fn entry(text: &str) -> Entry {
        Entry {
            id: 1,
            content_hash: "h".into(),
            kind: Kind::Text,
            preview_text: text.into(),
            full_text: text.into(),
            image_path: None,
            byte_size: 0,
            char_count: 0,
            word_count: 0,
            line_count: 0,
            first_copied_at_ms: 0,
            last_copied_at_ms: 0,
            copy_count: 1,
            pinned: false,
            source_app_id: None,
        }
    }

    struct FakeClip {
        last: std::cell::RefCell<String>,
    }
    impl magpie_platform::Clipboard for FakeClip {
        fn snapshot(&mut self) -> magpie_platform::ClipboardSnapshot {
            magpie_platform::ClipboardSnapshot {
                content: None,
                change_token: 0,
                concealed: false,
            }
        }
        fn set_text(&mut self, t: &str) -> Result<(), String> {
            *self.last.borrow_mut() = t.into();
            Ok(())
        }
        fn set_content(&mut self, c: &Content) -> Result<(), String> {
            if let Content::Text(t) = c {
                *self.last.borrow_mut() = t.clone();
            }
            Ok(())
        }
    }
    struct FakePaster {
        pasted: Cell<bool>,
    }
    impl magpie_platform::Paster for FakePaster {
        fn paste(&self) -> Result<(), String> {
            self.pasted.set(true);
            Ok(())
        }
    }

    #[test]
    fn resolve_is_one_based_and_bounded() {
        let r = vec![entry("a"), entry("b")];
        assert_eq!(resolve_quick_paste(&r, 1).unwrap().full_text, "a");
        assert_eq!(resolve_quick_paste(&r, 2).unwrap().full_text, "b");
        assert!(resolve_quick_paste(&r, 0).is_none());
        assert!(resolve_quick_paste(&r, 3).is_none());
    }

    #[test]
    fn resolve_slot_or_recent_prefers_assigned_slot() {
        let recent = vec![entry("r1"), entry("r2")];
        // An assigned slot entry wins regardless of what's in `recent`.
        assert_eq!(
            resolve_slot_or_recent(Some(entry("pinned")), &recent, 1)
                .unwrap()
                .full_text,
            "pinned"
        );
        // No slot entry: fall through to the slot-th most-recent (1-based).
        assert_eq!(
            resolve_slot_or_recent(None, &recent, 2).unwrap().full_text,
            "r2"
        );
        // No slot entry and out-of-range / zero slot: nothing.
        assert!(resolve_slot_or_recent(None, &recent, 0).is_none());
        assert!(resolve_slot_or_recent(None, &recent, 3).is_none());
    }

    #[test]
    fn perform_paste_sets_clipboard_and_auto_pastes() {
        let mut clip = FakeClip {
            last: std::cell::RefCell::new(String::new()),
        };
        let paster = FakePaster {
            pasted: Cell::new(false),
        };
        perform_paste(
            &mut clip,
            &paster,
            &entry("hello"),
            PasteKind::PlainText,
            true,
        )
        .unwrap();
        assert_eq!(*clip.last.borrow(), "hello");
        assert!(paster.pasted.get());
    }

    #[test]
    fn perform_paste_clipboard_only_when_not_auto() {
        let mut clip = FakeClip {
            last: std::cell::RefCell::new(String::new()),
        };
        let paster = FakePaster {
            pasted: Cell::new(false),
        };
        perform_paste(
            &mut clip,
            &paster,
            &entry("hi"),
            PasteKind::Formatted,
            false,
        )
        .unwrap();
        assert_eq!(*clip.last.borrow(), "hi");
        assert!(!paster.pasted.get());
    }
}
