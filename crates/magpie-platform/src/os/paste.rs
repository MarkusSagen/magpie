use crate::traits::Paster;
use enigo::{Direction, Enigo, Key, Keyboard, Settings};

pub struct EnigoPaster;

impl Paster for EnigoPaster {
    fn paste(&self) -> Result<(), String> {
        let mut enigo = Enigo::new(&Settings::default()).map_err(|e| e.to_string())?;
        #[cfg(target_os = "macos")]
        let modifier = Key::Meta; // Cmd
        #[cfg(not(target_os = "macos"))]
        let modifier = Key::Control;

        // Defensive: clear the modifier first, in case a previous *interrupted*
        // paste (e.g. a crash between press and release) left it logically held
        // down at the OS level — which makes every keystroke act as Cmd+<key>
        // system-wide and breaks pasting everywhere. A key-up on an already-up key
        // is harmless. This lets the next paste self-heal a stuck modifier.
        let _ = enigo.key(modifier, Direction::Release);

        enigo
            .key(modifier, Direction::Press)
            .map_err(|e| e.to_string())?;
        let click = enigo.key(Key::Unicode('v'), Direction::Click);
        // ALWAYS release the modifier, even if the 'v' click failed — never leave
        // Cmd/Ctrl stuck down.
        let release = enigo.key(modifier, Direction::Release);
        click.map_err(|e| e.to_string())?;
        release.map_err(|e| e.to_string())?;
        Ok(())
    }
}
