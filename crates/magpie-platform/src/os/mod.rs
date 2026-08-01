pub mod hotkeys;
pub mod paste;
pub mod source_app;

pub mod linux;

#[cfg(target_os = "macos")]
pub mod autostart;
#[cfg(target_os = "macos")]
pub mod macos;
