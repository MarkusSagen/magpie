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
