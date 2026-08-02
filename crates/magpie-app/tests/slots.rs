use magpie_app::paste_action::resolve_slot_or_recent;
use magpie_core::{Entry, Kind};

fn entry(id: i64, text: &str) -> Entry {
    Entry {
        id,
        content_hash: format!("h{id}"),
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

#[test]
fn slot_entry_takes_precedence() {
    let recent = vec![entry(1, "recent1"), entry(2, "recent2")];
    let slotted = entry(99, "slotted");
    let got = resolve_slot_or_recent(Some(slotted), &recent, 1).unwrap();
    assert_eq!(got.full_text, "slotted");
}

#[test]
fn falls_back_to_nth_recent_when_slot_empty() {
    let recent = vec![entry(1, "recent1"), entry(2, "recent2")];
    let got = resolve_slot_or_recent(None, &recent, 2).unwrap();
    assert_eq!(got.full_text, "recent2");
}

#[test]
fn none_when_slot_empty_and_out_of_range() {
    let recent = vec![entry(1, "recent1")];
    assert!(resolve_slot_or_recent(None, &recent, 0).is_none());
    assert!(resolve_slot_or_recent(None, &recent, 5).is_none());
}
