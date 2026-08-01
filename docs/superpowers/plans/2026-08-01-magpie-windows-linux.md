# magpie Windows + Linux Adapters Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement the Windows and Linux clipboard + autostart adapters for `magpie-platform`, and a cfg-selected factory so `magpie-app` runs on all three OSes with no other code change.

**Architecture:** Source-app (`ActiveWinSource`), paste (`EnigoPaster`), and hotkeys (`Hotkeys`) from Plan 2 are already cross-platform — this plan adds only what's OS-specific: a Windows clipboard adapter (change via `GetClipboardSequenceNumber`, concealed via clipboard-format probing), a Linux clipboard adapter (change via content-hash polling, concealed deferred to denylist/regex), Windows autostart (registry `Run` key), and Linux autostart (XDG `.desktop`). A `platform_clipboard()` factory returns the right `Clipboard` per `cfg`.

**Tech Stack:** Rust, `magpie-core`, `magpie-platform`, `arboard`, `blake3` (Linux change token). Windows-only: `windows` crate (Win32 clipboard + registry). Linux autostart is plain file IO. OS adapters are manual-verify; the pure `.desktop`/token logic is TDD'd.

## Global Constraints

- **Implements existing traits** from Plan 2 (`Clipboard`, `Autostart`) — no new trait shapes.
- **Same privacy semantics** as macOS: `concealed` true when the OS/app marks the clipboard sensitive. Where a platform can't expose that cheaply (Linux v1), `concealed` is `false` and secrecy relies on the denylist + content-regex (documented, not silent).
- **`ActiveWinSource`, `EnigoPaster`, `Hotkeys` are reused unchanged** — do not fork them.
- OS adapters are `#[cfg(target_os = "…")]` with `## Manual verification` blocks.
- **Minimal dependencies:** add only the `windows` crate (Windows-gated) and `blake3` (already in the workspace).
- **Commit style:** conventional commits, one per task.

---

### Task 1: Linux clipboard adapter (content-hash change token)

**Files:**
- Create: `crates/magpie-platform/src/os/linux.rs`
- Modify: `crates/magpie-platform/src/os/mod.rs`
- Modify: `crates/magpie-platform/Cargo.toml` (add `blake3` to deps if not present)
- Test: `linux.rs` tests module (the token logic is pure and IS testable).

**Interfaces:**
- Produces:
  - `pub fn content_token(text: Option<&str>, image_len: Option<usize>) -> u64` — a stable hash-derived token (blake3 → first 8 bytes as u64) so the watcher detects changes without a native sequence number; `None/None` → `0`.
  - `#[cfg(target_os = "linux")] pub struct LinuxClipboard { inner: arboard::Clipboard, }` implementing `Clipboard`: `snapshot` reads text (else image) via `arboard`, computes `change_token = content_token(...)`, sets `concealed = false` (v1). `set_text`/`set_content` via `arboard`.

- [ ] **Step 1: Add to `os/mod.rs`**

```rust
#[cfg(target_os = "linux")]
pub mod linux;
```

- [ ] **Step 2: Write failing tests in `linux.rs`** (guard the pure fn so it compiles on all OSes for the test)

```rust
#[cfg(test)]
mod tests {
    use super::content_token;

    #[test]
    fn same_text_same_token_diff_text_diff_token() {
        assert_eq!(content_token(Some("hello"), None), content_token(Some("hello"), None));
        assert_ne!(content_token(Some("a"), None), content_token(Some("b"), None));
    }

    #[test]
    fn empty_is_zero() {
        assert_eq!(content_token(None, None), 0);
    }
}
```

- [ ] **Step 3: Run to verify fail**

Run: `cargo test -p magpie-platform content_token`
Expected: FAIL.

- [ ] **Step 4: Implement `linux.rs`** (pure fn unguarded; the struct is cfg-gated)

```rust
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
            Ok(LinuxClipboard { inner: arboard::Clipboard::new().map_err(|e| e.to_string())? })
        }
    }

    impl Clipboard for LinuxClipboard {
        fn snapshot(&mut self) -> ClipboardSnapshot {
            if let Ok(text) = self.inner.get_text() {
                if !text.is_empty() {
                    let token = content_token(Some(&text), None);
                    return ClipboardSnapshot { content: Some(Content::Text(text)), change_token: token, concealed: false };
                }
            }
            if let Ok(img) = self.inner.get_image() {
                let mut bytes = Vec::with_capacity(8 + img.bytes.len());
                bytes.extend_from_slice(&(img.width as u32).to_le_bytes());
                bytes.extend_from_slice(&(img.height as u32).to_le_bytes());
                bytes.extend_from_slice(&img.bytes);
                let token = content_token(None, Some(bytes.len()));
                return ClipboardSnapshot { content: Some(Content::Image { bytes }), change_token: token, concealed: false };
            }
            ClipboardSnapshot { content: None, change_token: 0, concealed: false }
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
}

#[cfg(target_os = "linux")]
pub use imp::LinuxClipboard;
```

- [ ] **Step 5: Run to verify pass**

Run: `cargo test -p magpie-platform content_token`
Expected: PASS (2 tests).

- [ ] **Step 6: Manual verification** (on a Linux box): a `clip_probe`-style example printing `snapshot()` — copy text, confirm `change_token` changes and content is captured. Note whether you're on X11 or Wayland and whether `ActiveWinSource` returns an app or `None`. Record in report.

- [ ] **Step 7: Commit**

```bash
git add crates/magpie-platform/src/os/linux.rs crates/magpie-platform/src/os/mod.rs crates/magpie-platform/Cargo.toml
git commit -m "feat(platform): Linux clipboard adapter (content-hash token)"
```

---

### Task 2: Linux autostart (XDG .desktop)

**Files:**
- Create: `crates/magpie-platform/src/os/autostart_linux.rs`
- Modify: `crates/magpie-platform/src/os/mod.rs`
- Test: `autostart_linux.rs` tests module (pure `.desktop` generation + temp-dir file IO).

**Interfaces:**
- Produces:
  - `pub fn desktop_entry(name: &str, exec: &str) -> String` — an XDG autostart `.desktop` with `X-GNOME-Autostart-enabled=true` (pure, tested).
  - `pub struct LinuxAutostart { pub desktop_path: std::path::PathBuf, pub name: String, pub exec: String }` implementing `Autostart` (exists / write / remove), same shape as `MacAutostart`.

- [ ] **Step 1: Add `#[cfg(target_os = "linux")] pub mod autostart_linux;` to `os/mod.rs`.** (Keep the pure `desktop_entry` fn compilable on all OSes by not cfg-gating the module's test; see Step 2.)

Actually gate the whole module on Linux; run its tests on Linux. On non-Linux the tests are skipped — acceptable since the logic is trivial file IO. (If you want CI coverage on macOS too, leave the module ungated and only gate the `Autostart` impl; either is fine.)

- [ ] **Step 2: Write failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_entry_has_exec_and_autostart_flag() {
        let d = desktop_entry("Magpie", "/usr/bin/magpie");
        assert!(d.contains("Exec=/usr/bin/magpie"));
        assert!(d.contains("X-GNOME-Autostart-enabled=true"));
        assert!(d.contains("[Desktop Entry]"));
    }

    #[test]
    fn set_enabled_writes_and_removes() {
        let dir = std::env::temp_dir().join(format!("magpie-xdg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("magpie.desktop");
        let a = LinuxAutostart { desktop_path: path.clone(), name: "Magpie".into(), exec: "/usr/bin/magpie".into() };
        assert!(!a.is_enabled());
        a.set_enabled(true).unwrap();
        assert!(a.is_enabled() && path.exists());
        a.set_enabled(false).unwrap();
        assert!(!path.exists());
        std::fs::remove_dir_all(&dir).ok();
    }
}
```

- [ ] **Step 3: Run to verify fail**

Run: `cargo test -p magpie-platform autostart_linux` (on Linux; on macOS this module is gated out — run on the target OS or temporarily ungate for local check).
Expected: FAIL.

- [ ] **Step 4: Implement `autostart_linux.rs`**

```rust
use crate::traits::Autostart;
use std::path::PathBuf;

pub fn desktop_entry(name: &str, exec: &str) -> String {
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name={name}\n\
         Exec={exec}\n\
         X-GNOME-Autostart-enabled=true\n\
         Terminal=false\n"
    )
}

pub struct LinuxAutostart {
    pub desktop_path: PathBuf,
    pub name: String,
    pub exec: String,
}

impl Autostart for LinuxAutostart {
    fn is_enabled(&self) -> bool {
        self.desktop_path.exists()
    }

    fn set_enabled(&self, on: bool) -> Result<(), String> {
        if on {
            if let Some(parent) = self.desktop_path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            std::fs::write(&self.desktop_path, desktop_entry(&self.name, &self.exec)).map_err(|e| e.to_string())
        } else {
            match std::fs::remove_file(&self.desktop_path) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(e.to_string()),
            }
        }
    }
}
```

- [ ] **Step 5: Run to verify pass**

Run: `cargo test -p magpie-platform autostart_linux`
Expected: PASS (2 tests).

- [ ] **Step 6: Commit**

```bash
git add crates/magpie-platform/src/os/autostart_linux.rs crates/magpie-platform/src/os/mod.rs
git commit -m "feat(platform): Linux XDG autostart"
```

---

### Task 3: Windows clipboard adapter (sequence number + exclusion probe)

**Files:**
- Create: `crates/magpie-platform/src/os/windows.rs`
- Modify: `crates/magpie-platform/src/os/mod.rs`
- Modify: `crates/magpie-platform/Cargo.toml` (Windows-gated `windows` dep)

**Interfaces:**
- Produces: `#[cfg(target_os = "windows")] pub struct WinClipboard { inner: arboard::Clipboard }` implementing `Clipboard`: `change_token = GetClipboardSequenceNumber()`; `concealed` = the clipboard advertises the `ExcludeClipboardContentFromMonitorProcessing` or `CanIncludeInClipboardHistory=0` format (probe registered clipboard-format presence). Content via `arboard`.

> Windows OS glue — **manual-verify only**. The `windows` API names below are the intended shape; confirm against the pinned crate version.

- [ ] **Step 1: Add Windows deps to `Cargo.toml`**

```toml
[target.'cfg(target_os = "windows")'.dependencies]
windows = { version = "0.58", features = [
  "Win32_System_DataExchange",
  "Win32_System_Ole",
  "Win32_Foundation",
] }
```

- [ ] **Step 2: Add to `os/mod.rs`**

```rust
#[cfg(target_os = "windows")]
pub mod windows;
```

- [ ] **Step 3: Implement `os/windows.rs`**

```rust
use crate::traits::{Clipboard, ClipboardSnapshot};
use magpie_core::Content;
use windows::Win32::System::DataExchange::{
    GetClipboardSequenceNumber, RegisterClipboardFormatW, IsClipboardFormatAvailable,
};
use windows::core::PCWSTR;

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
            || format_available("CanIncludeInClipboardHistory"); // presence of the opt-out marker
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
```

> Note: `CanIncludeInClipboardHistory` is technically a DWORD-valued format; probing presence is a coarse v1 heuristic. Precise value-reading is a documented follow-up.

- [ ] **Step 4: Compile on Windows**

Run: `cargo build -p magpie-platform` (on Windows)
Expected: compiles. Adjust `windows` API names/features if the compiler disagrees.

- [ ] **Step 5: Manual verification** (on Windows): a probe example — copy text, confirm `change_token` (sequence number) increases and content is captured; confirm frontmost app is named via `ActiveWinSource`. Record in report.

- [ ] **Step 6: Commit**

```bash
git add crates/magpie-platform/src/os/windows.rs crates/magpie-platform/src/os/mod.rs crates/magpie-platform/Cargo.toml
git commit -m "feat(platform): Windows clipboard adapter (seq number + exclusion probe)"
```

---

### Task 4: Windows autostart (registry Run key)

**Files:**
- Create: `crates/magpie-platform/src/os/autostart_windows.rs`
- Modify: `crates/magpie-platform/src/os/mod.rs`

**Interfaces:**
- Produces: `#[cfg(target_os = "windows")] pub struct WinAutostart { pub value_name: String, pub exe_path: String }` implementing `Autostart` against `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`: `is_enabled` = value exists; `set_enabled(true)` writes `value_name = exe_path`; `set_enabled(false)` deletes it.

> Windows registry glue — **manual-verify only** (writes to the real HKCU Run key).

- [ ] **Step 1: Add to `os/mod.rs`**

```rust
#[cfg(target_os = "windows")]
pub mod autostart_windows;
```

- [ ] **Step 2: Implement `autostart_windows.rs`**

```rust
use crate::traits::Autostart;
use windows::core::PCWSTR;
use windows::Win32::System::Registry::{
    RegOpenKeyExW, RegSetValueExW, RegDeleteValueW, RegQueryValueExW, RegCloseKey,
    HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_SZ,
};

const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub struct WinAutostart {
    pub value_name: String,
    pub exe_path: String,
}

impl WinAutostart {
    fn open(&self, access: windows::Win32::System::Registry::REG_SAM_FLAGS) -> Result<HKEY, String> {
        let mut hkey = HKEY::default();
        let sub = wide(RUN_KEY);
        let rc = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, PCWSTR(sub.as_ptr()), 0, access, &mut hkey) };
        rc.ok().map_err(|e| e.to_string())?;
        Ok(hkey)
    }
}

impl Autostart for WinAutostart {
    fn is_enabled(&self) -> bool {
        let Ok(hkey) = self.open(KEY_READ) else { return false };
        let name = wide(&self.value_name);
        let mut ty = 0u32;
        let mut len = 0u32;
        let rc = unsafe {
            RegQueryValueExW(hkey, PCWSTR(name.as_ptr()), None, Some(&mut ty as *mut u32 as *mut _), None, Some(&mut len))
        };
        unsafe { RegCloseKey(hkey); }
        rc.is_ok()
    }

    fn set_enabled(&self, on: bool) -> Result<(), String> {
        let hkey = self.open(KEY_WRITE)?;
        let name = wide(&self.value_name);
        let result = if on {
            let val = wide(&self.exe_path);
            let bytes = unsafe {
                std::slice::from_raw_parts(val.as_ptr() as *const u8, val.len() * 2)
            };
            unsafe { RegSetValueExW(hkey, PCWSTR(name.as_ptr()), 0, REG_SZ, Some(bytes)) }.ok()
        } else {
            unsafe { RegDeleteValueW(hkey, PCWSTR(name.as_ptr())) }.ok()
        };
        unsafe { RegCloseKey(hkey); }
        result.map_err(|e| e.to_string())
    }
}
```

- [ ] **Step 3: Compile on Windows**

Run: `cargo build -p magpie-platform` (on Windows). Fix `windows` API names/features per the compiler; add `"Win32_System_Registry"` to the Windows feature list in `Cargo.toml`.

- [ ] **Step 4: Manual verification**: call `set_enabled(true)`, confirm the value appears under `HKCU\...\Run` (via `regedit` or `reg query`), then `set_enabled(false)` and confirm it's gone. Record in report.

- [ ] **Step 5: Commit**

```bash
git add crates/magpie-platform/src/os/autostart_windows.rs crates/magpie-platform/src/os/mod.rs crates/magpie-platform/Cargo.toml
git commit -m "feat(platform): Windows registry autostart"
```

---

### Task 5: cfg-selected platform factory + app runtime hookup

**Files:**
- Create: `crates/magpie-platform/src/os/factory.rs`
- Modify: `crates/magpie-platform/src/os/mod.rs`, `crates/magpie-platform/src/lib.rs`
- Modify: `crates/magpie-app/src/runtime.rs` (use the factory instead of `MacClipboard` directly)

**Interfaces:**
- Produces:
  - `pub fn platform_clipboard() -> Result<Box<dyn Clipboard>, String>` — returns `MacClipboard`/`WinClipboard`/`LinuxClipboard` per `cfg(target_os)`; error on unsupported OS.
  - `pub fn platform_autostart(exe_path: &str) -> Box<dyn Autostart>` — returns the per-OS `Autostart` with sensible default paths (macOS `~/Library/LaunchAgents/io.magpie.agent.plist`; Linux `~/.config/autostart/magpie.desktop`; Windows `WinAutostart{ value_name: "Magpie", exe_path }`).
- `runtime.rs`: `spawn_watcher` and the quick-paste handler call `platform_clipboard()` rather than `MacClipboard::new()`, so the app is OS-agnostic.

- [ ] **Step 1: Add `pub mod factory;` to `os/mod.rs`; re-export from `lib.rs`: `pub use os::factory::{platform_clipboard, platform_autostart};`**

- [ ] **Step 2: Implement `os/factory.rs`**

```rust
use crate::traits::{Autostart, Clipboard};

pub fn platform_clipboard() -> Result<Box<dyn Clipboard>, String> {
    #[cfg(target_os = "macos")]
    { Ok(Box::new(super::macos::MacClipboard::new()?)) }
    #[cfg(target_os = "windows")]
    { Ok(Box::new(super::windows::WinClipboard::new()?)) }
    #[cfg(target_os = "linux")]
    { Ok(Box::new(super::linux::LinuxClipboard::new()?)) }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    { Err("unsupported OS".into()) }
}

pub fn platform_autostart(exe_path: &str) -> Box<dyn Autostart> {
    #[cfg(target_os = "macos")]
    {
        let path = dirs_next_home().join("Library/LaunchAgents/io.magpie.agent.plist");
        Box::new(super::autostart::MacAutostart {
            plist_path: path, label: "io.magpie.agent".into(), program: exe_path.to_string(),
        })
    }
    #[cfg(target_os = "linux")]
    {
        let path = dirs_next_home().join(".config/autostart/magpie.desktop");
        Box::new(super::autostart_linux::LinuxAutostart {
            desktop_path: path, name: "Magpie".into(), exec: exe_path.to_string(),
        })
    }
    #[cfg(target_os = "windows")]
    {
        Box::new(super::autostart_windows::WinAutostart {
            value_name: "Magpie".into(), exe_path: exe_path.to_string(),
        })
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn dirs_next_home() -> std::path::PathBuf {
    std::env::var_os("HOME").map(std::path::PathBuf::from).unwrap_or_else(|| std::path::PathBuf::from("."))
}
```

> Note: `platform_clipboard` returns `Box<dyn Clipboard>`. Update `Watcher` usage in the app to accept a boxed clipboard — `Watcher<Box<dyn Clipboard>, ActiveWinSource>` works because `Box<dyn Clipboard>` implements `Clipboard` if you add a blanket `impl Clipboard for Box<dyn Clipboard>` (add it to `traits.rs`), OR change `Watcher` to hold `Box<dyn Clipboard>`. Prefer the blanket impl:
>
> ```rust
> impl Clipboard for Box<dyn Clipboard> {
>     fn snapshot(&mut self) -> ClipboardSnapshot { (**self).snapshot() }
>     fn set_text(&mut self, t: &str) -> Result<(), String> { (**self).set_text(t) }
>     fn set_content(&mut self, c: &magpie_core::Content) -> Result<(), String> { (**self).set_content(c) }
> }
> ```

- [ ] **Step 3: Add the blanket `impl Clipboard for Box<dyn Clipboard>` to `traits.rs`; add a unit test** in `traits.rs` using a fake to prove the blanket impl forwards:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use magpie_core::Content;

    struct Fake(u64);
    impl Clipboard for Fake {
        fn snapshot(&mut self) -> ClipboardSnapshot { ClipboardSnapshot { content: None, change_token: self.0, concealed: false } }
        fn set_text(&mut self, _t: &str) -> Result<(), String> { Ok(()) }
        fn set_content(&mut self, _c: &Content) -> Result<(), String> { Ok(()) }
    }

    #[test]
    fn boxed_clipboard_forwards() {
        let mut b: Box<dyn Clipboard> = Box::new(Fake(42));
        assert_eq!(b.snapshot().change_token, 42);
    }
}
```

- [ ] **Step 4: Update `runtime.rs`** — replace `MacClipboard::new()` calls with `magpie_platform::platform_clipboard()?`, and `Watcher::new(clip, ActiveWinSource, policy)` now takes the boxed clipboard.

- [ ] **Step 5: Run tests + build**

Run: `cargo test -p magpie-platform && cargo build -p magpie-app`
Expected: PASS + compiles (on your host OS).

- [ ] **Step 6: Commit**

```bash
git add crates/magpie-platform/src crates/magpie-app/src/runtime.rs
git commit -m "feat(platform): cfg-selected clipboard/autostart factory + boxed Clipboard"
```

---

## Self-Review

**Spec coverage:**
- Windows clipboard capture + change detection (sequence number) + exclusion markers → Task 3. ✅
- Linux clipboard capture + change detection (content hash); Wayland concealed deferred to denylist/regex (documented) → Task 1. ✅
- Windows autostart (registry) → Task 4. Linux autostart (XDG) → Task 2. ✅
- Source-app / paste / hotkeys reused cross-platform from Plan 2 (no new work). ✅
- OS-agnostic app via factory → Task 5. ✅

**Placeholder scan:** OS glue (Tasks 1 struct, 3, 4) is manual-verify with concrete reference code; pure logic (`content_token`, `desktop_entry`, factory forwarding, blanket impl) is TDD'd. The `CanIncludeInClipboardHistory` coarse-probe and Linux-concealed=false are documented limitations, not silent gaps.

**Type consistency:** `Clipboard`/`Autostart` trait signatures match Plan 2; `WinClipboard`/`LinuxClipboard`/`MacClipboard`, `WinAutostart`/`LinuxAutostart`/`MacAutostart`, `platform_clipboard`/`platform_autostart` consistent. The blanket `impl Clipboard for Box<dyn Clipboard>` keeps `Watcher` generic bound satisfied.

## Next Plan

- **Plan 5** builds release binaries and installs autostart via `platform_autostart`.
