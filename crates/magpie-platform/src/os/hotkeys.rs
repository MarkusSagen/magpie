use crate::hotkey::HotkeySpec;
use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::GlobalHotKeyManager;

pub fn to_global_hotkey(spec: &HotkeySpec) -> Result<HotKey, String> {
    let mut mods = Modifiers::empty();
    if spec.mods.ctrl {
        mods |= Modifiers::CONTROL;
    }
    if spec.mods.alt {
        mods |= Modifiers::ALT;
    }
    if spec.mods.shift {
        mods |= Modifiers::SHIFT;
    }
    if spec.mods.meta {
        mods |= Modifiers::META;
    }

    let code = key_to_code(&spec.key)?;
    Ok(HotKey::new(Some(mods), code))
}

fn key_to_code(key: &str) -> Result<Code, String> {
    let c = match key {
        "A" => Code::KeyA,
        "B" => Code::KeyB,
        "C" => Code::KeyC,
        "D" => Code::KeyD,
        "E" => Code::KeyE,
        "F" => Code::KeyF,
        "G" => Code::KeyG,
        "H" => Code::KeyH,
        "I" => Code::KeyI,
        "J" => Code::KeyJ,
        "K" => Code::KeyK,
        "L" => Code::KeyL,
        "M" => Code::KeyM,
        "N" => Code::KeyN,
        "O" => Code::KeyO,
        "P" => Code::KeyP,
        "Q" => Code::KeyQ,
        "R" => Code::KeyR,
        "S" => Code::KeyS,
        "T" => Code::KeyT,
        "U" => Code::KeyU,
        "V" => Code::KeyV,
        "W" => Code::KeyW,
        "X" => Code::KeyX,
        "Y" => Code::KeyY,
        "Z" => Code::KeyZ,
        "0" => Code::Digit0,
        "1" => Code::Digit1,
        "2" => Code::Digit2,
        "3" => Code::Digit3,
        "4" => Code::Digit4,
        "5" => Code::Digit5,
        "6" => Code::Digit6,
        "7" => Code::Digit7,
        "8" => Code::Digit8,
        "9" => Code::Digit9,
        // Common non-alphanumeric keys (Space is required for the default
        // summon binding Cmd/Ctrl+Shift+Space).
        "SPACE" => Code::Space,
        "ENTER" | "RETURN" => Code::Enter,
        "TAB" => Code::Tab,
        "ESC" | "ESCAPE" => Code::Escape,
        "COMMA" => Code::Comma,
        "PERIOD" | "DOT" => Code::Period,
        "SLASH" => Code::Slash,
        "BACKQUOTE" | "GRAVE" => Code::Backquote,
        other => return Err(format!("unsupported hotkey key: {other}")),
    };
    Ok(c)
}

pub struct Hotkeys {
    manager: GlobalHotKeyManager,
}

impl Hotkeys {
    pub fn new() -> Result<Self, String> {
        Ok(Hotkeys {
            manager: GlobalHotKeyManager::new().map_err(|e| e.to_string())?,
        })
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

    #[test]
    fn maps_space_for_summon_default() {
        // The default summon binding must register (Space was previously
        // unsupported, so Cmd/Ctrl+Shift+Space silently failed).
        let hk = to_global_hotkey(&parse_hotkey("super+shift+space").unwrap()).unwrap();
        let ctrl_variant = to_global_hotkey(&parse_hotkey("ctrl+shift+space").unwrap()).unwrap();
        assert_ne!(hk.id(), ctrl_variant.id());
    }

    #[test]
    fn maps_other_common_keys() {
        for combo in [
            "ctrl+enter",
            "ctrl+tab",
            "ctrl+comma",
            "ctrl+period",
            "ctrl+slash",
        ] {
            assert!(
                to_global_hotkey(&parse_hotkey(combo).unwrap()).is_ok(),
                "{combo} should map"
            );
        }
    }

    #[test]
    fn maps_grave_dot_escape_return_aliases() {
        for combo in ["ctrl+grave", "ctrl+dot", "ctrl+escape", "ctrl+return"] {
            assert!(
                to_global_hotkey(&parse_hotkey(combo).unwrap()).is_ok(),
                "{combo} should map"
            );
        }
    }

    #[test]
    fn aliases_map_to_the_same_code_as_their_canonical_spelling() {
        assert_eq!(key_to_code("DOT").unwrap(), key_to_code("PERIOD").unwrap());
        assert_eq!(
            key_to_code("RETURN").unwrap(),
            key_to_code("ENTER").unwrap()
        );
        assert_eq!(key_to_code("ESC").unwrap(), key_to_code("ESCAPE").unwrap());
        assert_eq!(
            key_to_code("GRAVE").unwrap(),
            key_to_code("BACKQUOTE").unwrap()
        );
    }
}
