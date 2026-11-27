//! Selection targets are opaque and main-thread owned. Unsupported backends are
//! explicit; the text engine itself has no native dependencies.
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
static ACTION: AtomicUsize = AtomicUsize::new(usize::MAX);
#[cfg(target_os = "macos")]
static BAR_EPOCH: AtomicUsize = AtomicUsize::new(0);
static HOTKEYS: [AtomicU32; 5] = [const { AtomicU32::new(0) }; 5];
pub fn route_hotkey(id: u32) -> bool {
    #[cfg(target_os = "macos")]
    let epoch = BAR_EPOCH.load(Ordering::Acquire);
    if let Some(i) = HOTKEYS
        .iter()
        .position(|k| k.load(Ordering::Relaxed) != 0 && k.load(Ordering::Relaxed) == id)
    {
        if std::env::var_os("MAGPIE_SELECTION_DEBUG").is_some() {
            eprintln!("selection: hotkey slot={i}");
        }
        #[cfg(target_os = "macos")]
        queue_action(i, epoch);
        #[cfg(not(target_os = "macos"))]
        ACTION.store(i, Ordering::Relaxed);
        true
    } else {
        false
    }
}
// Bind queued shortcuts to the panel that owned them. The hotkey listener runs
// off the UI thread and can finish routing after a selection has changed.
#[cfg(target_os = "macos")]
fn queue_action(slot: usize, epoch: usize) {
    ACTION.store((epoch << 3) | slot, Ordering::Release);
}

pub fn take_action() -> Option<usize> {
    let action = ACTION.swap(usize::MAX, Ordering::AcqRel);
    #[cfg(target_os = "macos")]
    if action >> 3 != BAR_EPOCH.load(Ordering::Acquire) {
        return None;
    }
    (action != usize::MAX).then_some(action & 7)
}
pub static RESTORED_CHANGE: std::sync::atomic::AtomicIsize =
    std::sync::atomic::AtomicIsize::new(-1);
pub fn supported() -> bool {
    cfg!(target_os = "macos")
}
#[derive(Clone, Debug)]
pub struct BarStyle {
    pub background: [f64; 4],
    pub foreground: [f64; 4],
}
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::{Controller, Target};
#[cfg(not(target_os = "macos"))]
#[derive(Clone)]
pub struct Target {
    pub text: String,
    pub editable: bool,
}
#[cfg(not(target_os = "macos"))]
impl Target {
    pub fn same_as(&self, other: &Self) -> bool {
        self.text == other.text
    }
}
#[cfg(not(target_os = "macos"))]
pub struct Controller;
#[cfg(not(target_os = "macos"))]
impl Controller {
    pub fn new() -> Option<Self> {
        None
    }
    pub fn poll(&mut self, _apps: &[String]) -> Option<Target> {
        None
    }
    pub fn show(
        &mut self,
        _target: &Target,
        _labels: &[String],
        _keys: &[String],
        _style: &BarStyle,
    ) {
    }
    pub fn dismiss(&mut self) {}
    pub fn finish(&mut self, _target: &Target) {}
    pub fn focus(&self, _target: &Target) -> Result<(), String> {
        Err("Unsupported platform".into())
    }
    pub fn valid(&self, _target: &Target) -> bool {
        false
    }
    pub fn replace(&mut self, _target: &Target, _text: &str) -> Result<(), String> {
        Err("Selection replacement is not implemented on this platform".into())
    }
    pub fn restore_clipboard(&mut self) {}
}
