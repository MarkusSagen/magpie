pub mod accessibility;
pub mod hotkeys;
pub mod paste;
pub mod source_app;
pub mod window;

pub mod autostart_linux;
pub mod factory;
pub mod linux;

#[cfg(target_os = "windows")]
pub mod autostart_windows;
#[cfg(target_os = "windows")]
pub mod windows;

#[cfg(target_os = "macos")]
pub mod autostart;
#[cfg(target_os = "macos")]
pub mod macos;
