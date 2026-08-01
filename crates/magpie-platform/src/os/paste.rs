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

        enigo.key(modifier, Direction::Press).map_err(|e| e.to_string())?;
        enigo.key(Key::Unicode('v'), Direction::Click).map_err(|e| e.to_string())?;
        enigo.key(modifier, Direction::Release).map_err(|e| e.to_string())?;
        Ok(())
    }
}
