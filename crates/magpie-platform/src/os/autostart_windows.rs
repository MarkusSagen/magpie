use crate::traits::Autostart;
use windows::core::PCWSTR;
use windows::Win32::Foundation::ERROR_FILE_NOT_FOUND;
use windows::Win32::System::Registry::{
    RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW, HKEY,
    HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_SAM_FLAGS, REG_SZ,
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
    fn open(&self, access: REG_SAM_FLAGS) -> Result<HKEY, String> {
        let mut hkey = HKEY::default();
        let sub = wide(RUN_KEY);
        let rc = unsafe {
            RegOpenKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(sub.as_ptr()),
                0,
                access,
                &mut hkey,
            )
        };
        rc.ok().map_err(|e| e.to_string())?;
        Ok(hkey)
    }
}

impl Autostart for WinAutostart {
    fn is_enabled(&self) -> bool {
        let Ok(hkey) = self.open(KEY_READ) else {
            return false;
        };
        let name = wide(&self.value_name);
        let rc = unsafe { RegQueryValueExW(hkey, PCWSTR(name.as_ptr()), None, None, None, None) };
        unsafe {
            let _ = RegCloseKey(hkey);
        }
        rc.is_ok()
    }

    fn set_enabled(&self, on: bool) -> Result<(), String> {
        let hkey = self.open(KEY_WRITE)?;
        let name = wide(&self.value_name);
        let result = if on {
            let val = wide(&self.exe_path);
            let bytes =
                unsafe { std::slice::from_raw_parts(val.as_ptr() as *const u8, val.len() * 2) };
            unsafe { RegSetValueExW(hkey, PCWSTR(name.as_ptr()), 0, REG_SZ, Some(bytes)) }.ok()
        } else {
            // Deleting an absent value returns ERROR_FILE_NOT_FOUND — treat that as
            // success so disabling autostart is idempotent (it's already off).
            let rc = unsafe { RegDeleteValueW(hkey, PCWSTR(name.as_ptr())) };
            if rc == ERROR_FILE_NOT_FOUND {
                Ok(())
            } else {
                rc.ok()
            }
        };
        unsafe {
            let _ = RegCloseKey(hkey);
        }
        result.map_err(|e| e.to_string())
    }
}
