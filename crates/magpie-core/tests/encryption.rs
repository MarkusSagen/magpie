//! Correctness tests for the SQLCipher-encrypted store: keyed open, wrong-key
//! rejection, encryption-at-rest, and the plaintext -> encrypted migration.

use magpie_core::{open, open_encrypted, open_or_migrate_encrypted};
use std::path::{Path, PathBuf};

/// A fresh, empty temp dir for `tag`, removed first if a previous run left it
/// behind. Returns the path to the (not-yet-created) db file inside it.
fn tmp_db(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("magpie-core-enc-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("magpie.sqlite3")
}

fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push(suffix);
    PathBuf::from(s)
}

#[test]
fn encrypt_roundtrip() {
    let path = tmp_db("roundtrip");
    let key1 = "aa".repeat(32); // 64 hex chars

    {
        let s = open_encrypted(&path, &key1).unwrap();
        s.upsert_note_by_name("Roundtrip Note", 1).unwrap();
    } // Store (and its Connection) dropped here.

    let s2 = open_encrypted(&path, &key1).unwrap();
    let note = s2.note_by_name("Roundtrip Note").unwrap();
    assert!(
        note.is_some(),
        "note should survive close+reopen with same key"
    );
}

#[test]
fn wrong_key_fails() {
    let path = tmp_db("wrongkey");
    let key1 = "bb".repeat(32);
    let key2 = "cc".repeat(32);

    {
        let s = open_encrypted(&path, &key1).unwrap();
        s.upsert_note_by_name("Secret", 1).unwrap();
    }

    let result = open_encrypted(&path, &key2);
    assert!(
        result.is_err(),
        "opening with the wrong key must fail, not silently read"
    );
}

#[test]
fn plaintext_is_encrypted_at_rest() {
    let path = tmp_db("atrest");
    let key = "dd".repeat(32);
    let distinctive = "TOTALLY-DISTINCTIVE-PLAINTEXT-MARKER-XYZ";

    {
        let s = open_encrypted(&path, &key).unwrap();
        let note = s.upsert_note_by_name("Marker Note", 1).unwrap();
        s.update_note_body(note.id, distinctive, 2).unwrap();
    }

    let bytes = std::fs::read(&path).unwrap();
    assert!(
        !bytes
            .windows(distinctive.len())
            .any(|w| w == distinctive.as_bytes()),
        "distinctive plaintext must not appear anywhere in the encrypted file"
    );
    assert_ne!(
        &bytes[..16.min(bytes.len())],
        b"SQLite format 3\0",
        "encrypted db must not start with the plaintext SQLite header"
    );
}

#[test]
fn migrates_plaintext_to_encrypted() {
    let path = tmp_db("migrate");
    let key = "ee".repeat(32);

    {
        let s = open(&path).unwrap();
        s.upsert_note_by_name("Legacy", 1).unwrap();
    }

    let migrated = open_or_migrate_encrypted(&path, &key).unwrap();
    let note = migrated.note_by_name("Legacy").unwrap();
    assert!(note.is_some(), "legacy note must survive migration");
    drop(migrated);

    assert!(
        sibling(&path, ".pre-encrypt-backup").exists(),
        "pre-encrypt backup of the original plaintext db must be kept"
    );
    assert!(
        !sibling(&path, ".enc-tmp").exists(),
        "no leftover temp file should remain after the swap"
    );

    // The file at `path` is now encrypted; opening it as plaintext must fail.
    assert!(
        open(&path).is_err(),
        "post-migration file must no longer open as plaintext"
    );
}

#[test]
fn open_creates_fresh_encrypted() {
    let path = tmp_db("fresh");
    let key = "ff".repeat(32);
    assert!(!path.exists());

    let s = open_or_migrate_encrypted(&path, &key).unwrap();
    let note = s.upsert_note_by_name("Fresh Note", 1).unwrap();
    let fetched = s.get_note(note.id).unwrap();
    assert_eq!(fetched.map(|n| n.name), Some("Fresh Note".to_string()));
}
