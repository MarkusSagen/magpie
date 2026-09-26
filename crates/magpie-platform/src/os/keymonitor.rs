//! Expand-as-you-type keyboard monitor (Phase 16b). A **listen-only** global key
//! observer used only when the user opts in. It never modifies the event stream,
//! keeps no persistent state here (the caller holds a small in-memory rolling
//! buffer), and is trivially stoppable. macOS uses a `CGEventTap`; other platforms
//! return `None` (unsupported for now — Wayland is impossible).
//!
//! SECURITY: the caller must pause expansion while secure input is active and in
//! denylisted apps; this module only reports decoded characters.

/// A running key monitor. Dropping or calling `stop` tears down the tap.
pub struct KeyMonitor {
    #[cfg(target_os = "macos")]
    inner: macos::MacKeyMonitor,
    #[cfg(not(target_os = "macos"))]
    _priv: (),
}

/// Whether macOS secure input is active (a password field has focus). Expansion
/// must pause while this is true. Always `false` off macOS.
pub fn secure_input_enabled() -> bool {
    #[cfg(target_os = "macos")]
    {
        extern "C" {
            fn IsSecureEventInputEnabled() -> bool;
        }
        unsafe { IsSecureEventInputEnabled() }
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

impl KeyMonitor {
    /// Start observing keystrokes, invoking `on_char` for each decoded character
    /// (and `'\u{8}'` for Backspace, `'\n'` for Return). Returns `None` when the
    /// platform is unsupported or the OS denied the tap (no Input Monitoring).
    pub fn start(on_char: impl Fn(char) + Send + 'static) -> Option<KeyMonitor> {
        #[cfg(target_os = "macos")]
        {
            macos::MacKeyMonitor::start(Box::new(on_char)).map(|inner| KeyMonitor { inner })
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = on_char;
            None
        }
    }

    /// Stop and tear down the monitor.
    pub fn stop(self) {
        #[cfg(target_os = "macos")]
        self.inner.stop();
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use core_foundation::runloop::{
        kCFRunLoopCommonModes, CFRunLoop, CFRunLoopAddSource, CFRunLoopRef, CFRunLoopRun,
    };
    use core_graphics::event::{CGEventField, EventField};
    use core_graphics::event::{
        CGEventTap, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement, CGEventType,
    };
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    pub struct MacKeyMonitor {
        run_loop: Arc<RunLoopHandle>,
        stopped: Arc<AtomicBool>,
    }

    /// A `Send` wrapper around the tap thread's `CFRunLoopRef` so we can stop it.
    struct RunLoopHandle(std::sync::Mutex<Option<CFRunLoopRef>>);
    unsafe impl Send for RunLoopHandle {}
    unsafe impl Sync for RunLoopHandle {}

    impl MacKeyMonitor {
        pub fn start(on_char: Box<dyn Fn(char) + Send>) -> Option<Self> {
            let run_loop = Arc::new(RunLoopHandle(std::sync::Mutex::new(None)));
            let stopped = Arc::new(AtomicBool::new(false));
            let (tx, rx) = std::sync::mpsc::channel::<bool>();

            let rl = run_loop.clone();
            std::thread::Builder::new()
                .name("magpie-keymonitor".into())
                .spawn(move || {
                    let tap = CGEventTap::new(
                        CGEventTapLocation::Session,
                        CGEventTapPlacement::HeadInsertEventTap,
                        CGEventTapOptions::ListenOnly, // observe only — never modify
                        vec![CGEventType::KeyDown],
                        |_p, _t, event| {
                            let keycode = event
                                .get_integer_value_field(EventField::KEYBOARD_EVENT_KEYCODE)
                                as u16;
                            let flags = event.get_flags();
                            let shift = flags
                                .contains(core_graphics::event::CGEventFlags::CGEventFlagShift);
                            if let Some(c) = keycode_to_char(keycode, shift) {
                                on_char(c);
                            }
                            None
                        },
                    );
                    let tap = match tap {
                        Ok(t) => t,
                        Err(_) => {
                            let _ = tx.send(false);
                            return;
                        }
                    };
                    let current = CFRunLoop::get_current();
                    unsafe {
                        let source = tap.mach_port.create_runloop_source(0);
                        match source {
                            Ok(source) => {
                                CFRunLoopAddSource(
                                    current.as_concrete_TypeRef(),
                                    source.as_concrete_TypeRef(),
                                    kCFRunLoopCommonModes,
                                );
                                tap.enable();
                                if let Ok(mut g) = rl.0.lock() {
                                    *g = Some(current.as_concrete_TypeRef());
                                }
                                let _ = tx.send(true);
                                CFRunLoopRun();
                            }
                            Err(_) => {
                                let _ = tx.send(false);
                            }
                        }
                    }
                    let _ = &tap; // keep the tap alive for the run loop's lifetime
                })
                .ok()?;

            // Wait for the tap thread to report whether it started.
            match rx.recv_timeout(std::time::Duration::from_secs(2)) {
                Ok(true) => Some(MacKeyMonitor { run_loop, stopped }),
                _ => None,
            }
        }

        pub fn stop(self) {
            self.stopped.store(true, Ordering::SeqCst);
            if let Ok(g) = self.run_loop.0.lock() {
                if let Some(rl) = *g {
                    unsafe {
                        core_foundation::runloop::CFRunLoopStop(rl);
                    }
                }
            }
        }
    }

    /// US-QWERTY keycode → character (enough for ASCII snippet keywords like
    /// `;meet`). Non-US layouts would need `CGEventKeyboardGetUnicodeString`; this
    /// experimental feature documents that limitation.
    fn keycode_to_char(code: u16, shift: bool) -> Option<char> {
        let base: Option<(char, char)> = match code {
            0 => Some(('a', 'A')),
            1 => Some(('s', 'S')),
            2 => Some(('d', 'D')),
            3 => Some(('f', 'F')),
            4 => Some(('h', 'H')),
            5 => Some(('g', 'G')),
            6 => Some(('z', 'Z')),
            7 => Some(('x', 'X')),
            8 => Some(('c', 'C')),
            9 => Some(('v', 'V')),
            11 => Some(('b', 'B')),
            12 => Some(('q', 'Q')),
            13 => Some(('w', 'W')),
            14 => Some(('e', 'E')),
            15 => Some(('r', 'R')),
            16 => Some(('y', 'Y')),
            17 => Some(('t', 'T')),
            18 => Some(('1', '!')),
            19 => Some(('2', '@')),
            20 => Some(('3', '#')),
            21 => Some(('4', '$')),
            22 => Some(('6', '^')),
            23 => Some(('5', '%')),
            24 => Some(('=', '+')),
            25 => Some(('9', '(')),
            26 => Some(('7', '&')),
            27 => Some(('-', '_')),
            28 => Some(('8', '*')),
            29 => Some(('0', ')')),
            31 => Some(('o', 'O')),
            32 => Some(('u', 'U')),
            34 => Some(('i', 'I')),
            35 => Some(('p', 'P')),
            37 => Some(('l', 'L')),
            38 => Some(('j', 'J')),
            40 => Some(('k', 'K')),
            41 => Some((';', ':')),
            39 => Some(('\'', '"')),
            43 => Some((',', '<')),
            44 => Some(('/', '?')),
            45 => Some(('n', 'N')),
            46 => Some(('m', 'M')),
            47 => Some(('.', '>')),
            50 => Some(('`', '~')),
            30 => Some((']', '}')),
            33 => Some(('[', '{')),
            42 => Some(('\\', '|')),
            _ => None,
        };
        match code {
            49 => Some(' '),
            36 | 76 => Some('\n'),
            51 => Some('\u{8}'), // Backspace
            _ => base.map(|(lo, hi)| if shift { hi } else { lo }),
        }
    }

    // Bring the trait for `get_integer_value_field` / `get_flags` into scope.
    #[allow(unused_imports)]
    use core_foundation::base::TCFType;
    #[allow(unused_imports)]
    use core_graphics::event::CGEvent;
    #[allow(unused_imports)]
    type _Field = CGEventField;
}
