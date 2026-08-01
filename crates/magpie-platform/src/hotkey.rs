#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Mods {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub meta: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HotkeySpec {
    pub mods: Mods,
    pub key: String,
}

pub fn parse_hotkey(s: &str) -> Result<HotkeySpec, String> {
    let mut mods = Mods::default();
    let mut key: Option<String> = None;
    for raw in s.split('+') {
        let tok = raw.trim().to_lowercase();
        if tok.is_empty() {
            return Err(format!("empty token in '{s}'"));
        }
        match tok.as_str() {
            "ctrl" | "control" => mods.ctrl = true,
            "alt" | "opt" | "option" => mods.alt = true,
            "shift" => mods.shift = true,
            "super" | "cmd" | "command" | "win" | "meta" => mods.meta = true,
            _ => {
                if key.is_some() {
                    return Err(format!("more than one key in '{s}'"));
                }
                key = Some(tok.to_uppercase());
            }
        }
    }
    let key = key.ok_or_else(|| format!("no key in '{s}'"))?;
    if !(mods.ctrl || mods.alt || mods.shift || mods.meta) {
        return Err(format!("no modifier in '{s}'"));
    }
    Ok(HotkeySpec { mods, key })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_super_ctrl_1() {
        let h = parse_hotkey("super+ctrl+1").unwrap();
        assert!(h.mods.meta && h.mods.ctrl && !h.mods.alt && !h.mods.shift);
        assert_eq!(h.key, "1");
    }

    #[test]
    fn aliases_and_case_insensitive() {
        let h = parse_hotkey("Cmd+Opt+Shift+v").unwrap();
        assert!(h.mods.meta && h.mods.alt && h.mods.shift);
        assert_eq!(h.key, "V");
    }

    #[test]
    fn rejects_no_modifier() {
        assert!(parse_hotkey("v").is_err());
    }

    #[test]
    fn rejects_no_or_multiple_keys() {
        assert!(parse_hotkey("ctrl").is_err());
        assert!(parse_hotkey("ctrl+a+b").is_err());
    }
}
