//! Secrets vault (a lightweight, local 1Password-style store): key→value secrets kept
//! in the same encrypted (SQLCipher) DB as everything else — no separate master
//! password, always available at rest under the DB key.
//!
//! A clipboard entry can be **promoted** to a secret: its plaintext moves into the vault
//! and the history row is scrubbed to a fixed run of asterisks ([`SECRET_MASK`]), so the
//! value can't be shoulder-surfed or searched from clipboard history. The value can still
//! be pasted — the paste path reads it back from the vault via [`Store::secret_for_entry`].
//! Secrets are searchable by key AND value from the Secrets tab.

use crate::store::{Result, Store};
use rusqlite::OptionalExtension;

/// Fixed masked display for a promoted secret. A constant length, so the value's real
/// length is never leaked by the mask.
pub const SECRET_MASK: &str = "********";

/// A stored secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Secret {
    pub id: i64,
    pub key: String,
    pub value: String,
    pub notes: String,
    /// The clipboard entry this was promoted from, if any.
    pub entry_id: Option<i64>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

const SECRET_COLS: &str = "id, key, value, notes, entry_id, created_at_ms, updated_at_ms";

fn row_to_secret(r: &rusqlite::Row) -> rusqlite::Result<Secret> {
    Ok(Secret {
        id: r.get(0)?,
        key: r.get(1)?,
        value: r.get(2)?,
        notes: r.get(3)?,
        entry_id: r.get(4)?,
        created_at_ms: r.get(5)?,
        updated_at_ms: r.get(6)?,
    })
}

/// Escape LIKE metacharacters for a literal substring match.
fn like_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")
}

/// Heuristic: does `text` look like a credential worth offering to save as a secret?
/// Returns a short human label (e.g. "API key") or `None`. Conservative — single-token
/// values only (plus `Bearer …`), so ordinary prose, URLs and notes never trigger a
/// suggestion. Detection only *suggests*; it never moves anything on its own.
pub fn looks_like_secret(text: &str) -> Option<&'static str> {
    let t = text.trim();
    // A bearer header is the one multi-token shape we accept.
    if let Some(tok) = t.strip_prefix("Bearer ").or_else(|| t.strip_prefix("bearer ")) {
        return looks_like_secret(tok).or(Some("bearer token"));
    }
    // Otherwise a secret is a single opaque token — reject anything with whitespace,
    // and anything that is obviously a URL.
    if t.len() < 16 || t.contains(char::is_whitespace) || t.contains("://") {
        return None;
    }
    let lower = t.to_lowercase();
    // Well-known prefixes.
    if lower.starts_with("sk-") || lower.starts_with("pk-") {
        return Some("API key");
    }
    if lower.starts_with("ghp_")
        || lower.starts_with("gho_")
        || lower.starts_with("ghu_")
        || lower.starts_with("ghs_")
        || lower.starts_with("github_pat_")
    {
        return Some("GitHub token");
    }
    if lower.starts_with("xoxb-")
        || lower.starts_with("xoxp-")
        || lower.starts_with("xoxa-")
        || lower.starts_with("xoxr-")
    {
        return Some("Slack token");
    }
    if t.starts_with("AKIA") && t.len() == 20 && t[4..].chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()) {
        return Some("AWS access key");
    }
    // JWT: three base64url segments separated by dots, starting with the `eyJ` header.
    if t.starts_with("eyJ") && t.matches('.').count() == 2 {
        return Some("JWT");
    }
    // Generic high-entropy token: long, only token characters, and a mix of letters and
    // digits (so a long ordinary word or a hyphenated slug isn't flagged).
    let token_chars = t
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '=' | '_' | '-' | '.'));
    let has_alpha = t.chars().any(|c| c.is_ascii_alphabetic());
    let has_digit = t.chars().any(|c| c.is_ascii_digit());
    if t.len() >= 32 && token_chars && has_alpha && has_digit {
        return Some("token");
    }
    None
}

impl Store {
    /// Insert a secret; returns its id.
    pub fn add_secret(
        &self,
        key: &str,
        value: &str,
        notes: &str,
        entry_id: Option<i64>,
        now_ms: i64,
    ) -> Result<i64> {
        self.conn().execute(
            "INSERT INTO secrets (key, value, notes, entry_id, created_at_ms, updated_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
            rusqlite::params![key, value, notes, entry_id, now_ms],
        )?;
        Ok(self.conn().last_insert_rowid())
    }

    pub fn list_secrets(&self) -> Result<Vec<Secret>> {
        let mut stmt = self
            .conn()
            .prepare(&format!("SELECT {SECRET_COLS} FROM secrets ORDER BY key COLLATE NOCASE"))?;
        let rows = stmt.query_map([], row_to_secret)?;
        rows.collect()
    }

    /// Search secrets by key OR value (case-insensitive substring). Empty query lists all.
    pub fn search_secrets(&self, query: &str) -> Result<Vec<Secret>> {
        let q = query.trim();
        if q.is_empty() {
            return self.list_secrets();
        }
        let like = format!("%{}%", like_escape(&q.to_lowercase()));
        let mut stmt = self.conn().prepare(&format!(
            "SELECT {SECRET_COLS} FROM secrets
             WHERE lower(key) LIKE ?1 ESCAPE '\\' OR lower(value) LIKE ?1 ESCAPE '\\'
             ORDER BY key COLLATE NOCASE"
        ))?;
        let rows = stmt.query_map([like], row_to_secret)?;
        rows.collect()
    }

    pub fn get_secret(&self, id: i64) -> Result<Option<Secret>> {
        self.conn()
            .query_row(
                &format!("SELECT {SECRET_COLS} FROM secrets WHERE id = ?1"),
                [id],
                row_to_secret,
            )
            .optional()
    }

    /// The secret a clipboard entry was promoted to, if any (used by the paste path to
    /// substitute the real value for the masked history content).
    pub fn secret_for_entry(&self, entry_id: i64) -> Result<Option<Secret>> {
        self.conn()
            .query_row(
                &format!("SELECT {SECRET_COLS} FROM secrets WHERE entry_id = ?1"),
                [entry_id],
                row_to_secret,
            )
            .optional()
    }

    /// The set of entry ids that are secrets (so the list can badge them). Sorted.
    pub fn secret_entry_ids(&self) -> Result<Vec<i64>> {
        let mut stmt = self
            .conn()
            .prepare("SELECT entry_id FROM secrets WHERE entry_id IS NOT NULL ORDER BY entry_id")?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        rows.collect()
    }

    pub fn update_secret(
        &self,
        id: i64,
        key: &str,
        value: &str,
        notes: &str,
        now_ms: i64,
    ) -> Result<()> {
        self.conn().execute(
            "UPDATE secrets SET key=?2, value=?3, notes=?4, updated_at_ms=?5 WHERE id=?1",
            rusqlite::params![id, key, value, notes, now_ms],
        )?;
        Ok(())
    }

    pub fn delete_secret(&self, id: i64) -> Result<()> {
        self.conn()
            .execute("DELETE FROM secrets WHERE id=?1", [id])?;
        Ok(())
    }

    /// Promote a clipboard entry to a secret: copy its text into the vault under `key`,
    /// then scrub the history row to [`SECRET_MASK`] (so the plaintext leaves `entries`
    /// and its FTS index). Returns the new secret id. No-op error if the entry is missing.
    pub fn promote_entry_to_secret(&self, entry_id: i64, key: &str, now_ms: i64) -> Result<i64> {
        let value: String = self.conn().query_row(
            "SELECT full_text FROM entries WHERE id = ?1",
            [entry_id],
            |r| r.get(0),
        )?;
        let secret_id = self.add_secret(key, &value, "", Some(entry_id), now_ms)?;
        self.conn().execute(
            "UPDATE entries SET full_text = ?2, preview_text = ?2 WHERE id = ?1",
            rusqlite::params![entry_id, SECRET_MASK],
        )?;
        Ok(secret_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{CaptureEvent, Content};
    use crate::store::{open_in_memory, ImageStore};

    struct FakeImages;
    impl ImageStore for FakeImages {
        fn put(&self, hash: &str, _b: &[u8]) -> std::io::Result<String> {
            Ok(format!("/c/{hash}"))
        }
    }

    fn ingest_text(s: &Store, t: &str) -> i64 {
        s.ingest(
            &CaptureEvent {
                content: Content::Text(t.into()),
                source_app: None,
                source_title: String::new(),
                copied_at_ms: 1,
            },
            &FakeImages,
        )
        .unwrap()
        .entry_id
    }

    #[test]
    fn add_search_and_update() {
        let s = open_in_memory().unwrap();
        s.add_secret("STRIPE_KEY", "sk-live-abc", "prod", None, 10).unwrap();
        s.add_secret("DB_PASSWORD", "hunter2", "", None, 11).unwrap();
        // Search matches key OR value, case-insensitive.
        assert_eq!(s.search_secrets("stripe").unwrap().len(), 1);
        assert_eq!(s.search_secrets("hunter").unwrap().len(), 1);
        assert_eq!(s.search_secrets("").unwrap().len(), 2);
        let id = s.search_secrets("stripe").unwrap()[0].id;
        s.update_secret(id, "STRIPE_KEY", "sk-live-xyz", "rotated", 12).unwrap();
        assert_eq!(s.get_secret(id).unwrap().unwrap().value, "sk-live-xyz");
        s.delete_secret(id).unwrap();
        assert!(s.get_secret(id).unwrap().is_none());
    }

    #[test]
    fn promote_moves_value_and_masks_entry() {
        let s = open_in_memory().unwrap();
        let eid = ingest_text(&s, "super-secret-token-value");
        let sid = s.promote_entry_to_secret(eid, "MY_TOKEN", 100).unwrap();
        // The vault holds the real value, linked to the entry.
        let secret = s.get_secret(sid).unwrap().unwrap();
        assert_eq!(secret.value, "super-secret-token-value");
        assert_eq!(secret.entry_id, Some(eid));
        // The history entry is scrubbed to the fixed mask.
        let masked: String = s
            .conn()
            .query_row("SELECT full_text FROM entries WHERE id=?1", [eid], |r| r.get(0))
            .unwrap();
        assert_eq!(masked, SECRET_MASK);
        // Lookups the paste path / list use.
        assert_eq!(s.secret_for_entry(eid).unwrap().unwrap().id, sid);
        assert_eq!(s.secret_entry_ids().unwrap(), vec![eid]);
    }

    #[test]
    fn promoted_value_is_not_searchable_from_history() {
        let s = open_in_memory().unwrap();
        let eid = ingest_text(&s, "topsecretpassphrase");
        // Findable before promotion.
        let mut q = crate::search::default_query();
        q.text = "topsecretpassphrase".into();
        assert_eq!(s.search(&q).unwrap().len(), 1);
        s.promote_entry_to_secret(eid, "PASS", 1).unwrap();
        // Gone from clipboard search after promotion (plaintext left entries + FTS).
        assert_eq!(s.search(&q).unwrap().len(), 0);
        // But still in the vault.
        assert_eq!(s.search_secrets("topsecret").unwrap().len(), 1);
    }

    #[test]
    fn detection_flags_credentials_not_prose() {
        assert_eq!(looks_like_secret("sk-live-1234567890abcdef"), Some("API key"));
        assert_eq!(looks_like_secret("ghp_0123456789abcdefABCDEF0123456789abcd"), Some("GitHub token"));
        assert_eq!(looks_like_secret(concat!("xox", "b-123456789-abcdefghijklmno")), Some("Slack token"));
        assert_eq!(looks_like_secret("AKIAIOSFODNN7EXAMPLE"), Some("AWS access key"));
        assert_eq!(looks_like_secret("Bearer abcDEF123456ghijkl7890"), Some("bearer token"));
        assert_eq!(
            looks_like_secret("eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTYifQ.abc123"),
            Some("JWT")
        );
        // Generic long high-entropy token.
        assert_eq!(looks_like_secret("A1b2C3d4E5f6G7h8I9j0K1l2M3n4O5p6"), Some("token"));
        // Not secrets: prose, URLs, short words, plain words without digits.
        assert!(looks_like_secret("the quick brown fox jumps").is_none());
        assert!(looks_like_secret("https://example.com/very/long/path/here/now").is_none());
        assert!(looks_like_secret("hello").is_none());
        assert!(looks_like_secret("supercalifragilisticexpialidocious").is_none());
    }
}
