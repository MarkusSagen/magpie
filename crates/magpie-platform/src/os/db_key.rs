//! The SQLCipher key for the notes/clipboard DB. On macOS it lives in the login
//! Keychain (created once); elsewhere in a 0600 key file in the data dir. A
//! `MAGPIE_DB_KEY` env var overrides both (used by tests / headless runs).
use std::path::Path;

#[cfg(target_os = "macos")]
const KC_SERVICE: &str = "io.magpie";
#[cfg(target_os = "macos")]
const KC_ACCOUNT: &str = "magpie-db";

/// Fetch (or create-and-persist) the DB encryption key as a hex string.
#[cfg_attr(target_os = "macos", allow(unused_variables))]
pub fn db_key(data_dir: &Path) -> Result<String, String> {
    if let Ok(k) = std::env::var("MAGPIE_DB_KEY") {
        if !k.is_empty() {
            return Ok(k);
        }
    }
    #[cfg(target_os = "macos")]
    {
        macos_keychain_key()
    }
    #[cfg(not(target_os = "macos"))]
    {
        file_key(data_dir)
    }
}

/// 32 random bytes from /dev/urandom → 64 lowercase hex chars.
fn generate_hex_key() -> Result<String, String> {
    use std::io::Read;
    let mut f = std::fs::File::open("/dev/urandom").map_err(|e| e.to_string())?;
    let mut bytes = [0u8; 32];
    f.read_exact(&mut bytes).map_err(|e| e.to_string())?;
    let mut hex = String::with_capacity(64);
    for b in bytes {
        hex.push_str(&format!("{b:02x}"));
    }
    Ok(hex)
}

#[cfg(target_os = "macos")]
fn macos_keychain_key() -> Result<String, String> {
    use std::process::Command;
    // Try to read an existing item.
    let out = Command::new("security")
        .args([
            "find-generic-password",
            "-s",
            KC_SERVICE,
            "-a",
            KC_ACCOUNT,
            "-w",
        ])
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        let k = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !k.is_empty() {
            return Ok(k);
        }
    }
    // Create one and store it (login keychain). -U update if present, -A allow app
    // access without repeated prompts (local convenience key).
    let key = generate_hex_key()?;
    let st = Command::new("security")
        .args([
            "add-generic-password",
            "-s",
            KC_SERVICE,
            "-a",
            KC_ACCOUNT,
            "-U",
            "-A",
            "-w",
            &key,
        ])
        .status()
        .map_err(|e| e.to_string())?;
    if !st.success() {
        return Err("failed to store DB key in Keychain".into());
    }
    Ok(key)
}

#[cfg(not(target_os = "macos"))]
fn file_key(data_dir: &Path) -> Result<String, String> {
    let path = data_dir.join(".dbkey");
    if let Ok(k) = std::fs::read_to_string(&path) {
        let k = k.trim().to_string();
        if !k.is_empty() {
            return Ok(k);
        }
    }
    std::fs::create_dir_all(data_dir).map_err(|e| e.to_string())?;
    let key = generate_hex_key()?;
    std::fs::write(&path, &key).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_hex_key_is_64_lowercase_hex_and_differs_between_calls() {
        let a = generate_hex_key().unwrap();
        let b = generate_hex_key().unwrap();
        assert_eq!(a.len(), 64);
        assert!(a
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        assert_ne!(a, b, "two calls should not produce the same random key");
    }

    /// All `MAGPIE_DB_KEY` env manipulation is confined to this single test
    /// (env vars are process-global, so a second test touching it could race
    /// under parallel `cargo test` execution), and the previous value is
    /// always restored at the end.
    #[test]
    fn db_key_honors_nonempty_override_and_empty_fallback() {
        let prev = std::env::var("MAGPIE_DB_KEY").ok();
        let dir = std::env::temp_dir().join(format!("magpie-dbkey-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        std::env::set_var("MAGPIE_DB_KEY", "deadbeefcafef00d");
        let got =
            db_key(&dir).expect("a non-empty MAGPIE_DB_KEY must short-circuit before OS storage");
        assert_eq!(got, "deadbeefcafef00d");

        std::env::set_var("MAGPIE_DB_KEY", "");
        #[cfg(not(target_os = "macos"))]
        {
            // An empty override falls through to the plain-file generator on
            // non-macOS. (On macOS the same fallback goes through the real
            // login Keychain via the `security` CLI against the production
            // service/account `io.magpie`/`magpie-db` — deliberately NOT
            // exercised here, since that could hang on a permission prompt or
            // mutate the developer's real keychain entry during `cargo test`.)
            let generated = db_key(&dir).unwrap();
            assert_eq!(generated.len(), 64);
            assert!(generated.chars().all(|c| c.is_ascii_hexdigit()));
            let again = db_key(&dir).unwrap();
            assert_eq!(
                generated, again,
                "second call should reuse the persisted key file"
            );
        }

        std::fs::remove_dir_all(&dir).ok();
        match prev {
            Some(v) => std::env::set_var("MAGPIE_DB_KEY", v),
            None => std::env::remove_var("MAGPIE_DB_KEY"),
        }
    }
}
