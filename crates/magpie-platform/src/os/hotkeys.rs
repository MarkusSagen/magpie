use crate::hotkey::HotkeySpec;
use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::GlobalHotKeyManager;

pub fn to_global_hotkey(spec: &HotkeySpec) -> Result<HotKey, String> {
    let mut mods = Modifiers::empty();
    if spec.mods.ctrl { mods |= Modifiers::CONTROL; }
    if spec.mods.alt { mods |= Modifiers::ALT; }
    if spec.mods.shift { mods |= Modifiers::SHIFT; }
    if spec.mods.meta { mods |= Modifiers::META; }

    let code = key_to_code(&spec.key)?;
    Ok(HotKey::new(Some(mods), code))
}

fn key_to_code(key: &str) -> Result<Code, String> {
    let c = match key {
        "A" => Code::KeyA, "B" => Code::KeyB, "C" => Code::KeyC, "D" => Code::KeyD,
        "E" => Code::KeyE, "F" => Code::KeyF, "G" => Code::KeyG, "H" => Code::KeyH,
        "I" => Code::KeyI, "J" => Code::KeyJ, "K" => Code::KeyK, "L" => Code::KeyL,
        "M" => Code::KeyM, "N" => Code::KeyN, "O" => Code::KeyO, "P" => Code::KeyP,
        "Q" => Code::KeyQ, "R" => Code::KeyR, "S" => Code::KeyS, "T" => Code::KeyT,
        "U" => Code::KeyU, "V" => Code::KeyV, "W" => Code::KeyW, "X" => Code::KeyX,
        "Y" => Code::KeyY, "Z" => Code::KeyZ,
        "0" => Code::Digit0, "1" => Code::Digit1, "2" => Code::Digit2, "3" => Code::Digit3,
        "4" => Code::Digit4, "5" => Code::Digit5, "6" => Code::Digit6, "7" => Code::Digit7,
        "8" => Code::Digit8, "9" => Code::Digit9,
        other => return Err(format!("unsupported hotkey key: {other}")),
    };
    Ok(c)
}

pub struct Hotkeys {
    manager: GlobalHotKeyManager,
}

impl Hotkeys {
    pub fn new() -> Result<Self, String> {
        Ok(Hotkeys { manager: GlobalHotKeyManager::new().map_err(|e| e.to_string())? })
    }

    pub fn register(&self, spec: &HotkeySpec) -> Result<u32, String> {
        let hk = to_global_hotkey(spec)?;
        self.manager.register(hk).map_err(|e| e.to_string())?;
        Ok(hk.id())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hotkey::parse_hotkey;

    #[test]
    fn maps_letter_and_digit_keys() {
        let v = to_global_hotkey(&parse_hotkey("super+ctrl+v").unwrap()).unwrap();
        let one = to_global_hotkey(&parse_hotkey("super+ctrl+1").unwrap()).unwrap();
        assert_ne!(v.id(), one.id()); // distinct hotkeys
    }

    #[test]
    fn rejects_unsupported_key() {
        assert!(to_global_hotkey(&parse_hotkey("ctrl+f13").unwrap()).is_err());
    }
}
