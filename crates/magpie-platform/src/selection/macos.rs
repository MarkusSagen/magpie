//! AX reads never synthesize Copy. AppKit panels and paste run on the main thread.
use super::*;
use core_foundation::{
    base::{CFGetTypeID, CFRelease, TCFType},
    string::{CFString, CFStringGetTypeID},
};
use global_hotkey::{hotkey::HotKey, GlobalHotKeyManager};
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{define_class, msg_send, sel, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::*;
use objc2_foundation::{
    NSArray, NSData, NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString,
};
use std::sync::atomic::AtomicBool;
use std::{
    ffi::c_void,
    ptr::{null, null_mut, NonNull},
    rc::Rc,
    time::{Duration, Instant},
};
static DIRTY: AtomicBool = AtomicBool::new(true);
static DISMISS: AtomicBool = AtomicBool::new(false);
static MOUSE_HELD: AtomicBool = AtomicBool::new(false);
static RESELECT: AtomicBool = AtomicBool::new(false);
type Ref = *const c_void;
#[repr(C)]
#[derive(Clone, Copy, PartialEq)]
struct Range {
    location: isize,
    length: isize,
}
#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXUIElementCreateApplication(pid: i32) -> Ref;
    fn AXUIElementCopyAttributeValue(e: Ref, a: Ref, out: *mut Ref) -> i32;
    fn AXUIElementCopyParameterizedAttributeValue(e: Ref, a: Ref, p: Ref, out: *mut Ref) -> i32;
    fn AXUIElementIsAttributeSettable(e: Ref, a: Ref, out: *mut u8) -> i32;
    fn AXUIElementSetMessagingTimeout(e: Ref, seconds: f32) -> i32;
    fn AXValueGetValue(v: Ref, t: u32, out: *mut c_void) -> bool;
    fn AXObserverCreate(
        pid: i32,
        callback: extern "C" fn(Ref, Ref, Ref, *mut c_void),
        out: *mut Ref,
    ) -> i32;
    fn AXObserverAddNotification(o: Ref, e: Ref, n: Ref, c: *mut c_void) -> i32;
    fn AXObserverGetRunLoopSource(o: Ref) -> Ref;
}
#[derive(Debug)]
struct Owned(Ref);
impl Drop for Owned {
    fn drop(&mut self) {
        unsafe { CFRelease(self.0) }
    }
}
fn attr(e: Ref, name: &str) -> Option<Owned> {
    let name = CFString::new(name);
    let mut out = null();
    let code =
        unsafe { AXUIElementCopyAttributeValue(e, name.as_concrete_TypeRef().cast(), &mut out) };
    if code == 0 && !out.is_null() {
        Some(Owned(out))
    } else {
        None
    }
}
fn string(value: &Owned) -> Option<String> {
    unsafe {
        (CFGetTypeID(value.0) == CFStringGetTypeID())
            .then(|| CFString::wrap_under_get_rule(value.0.cast()).to_string())
    }
}
fn value_range(value: &Owned) -> Option<Range> {
    let mut r = Range {
        location: 0,
        length: 0,
    };
    unsafe { AXValueGetValue(value.0, 4, (&mut r as *mut Range).cast()).then_some(r) }
}
#[derive(Clone)]
pub struct Target {
    pub text: String,
    pub editable: bool,
    element: Rc<Owned>,
    range: Range,
    pid: i32,
    bounds: NSRect,
}
impl Target {
    pub fn same_as(&self, other: &Self) -> bool {
        self.pid == other.pid
            && self.range == other.range
            && self.text == other.text
            && unsafe { core_foundation::base::CFEqual(self.element.0, other.element.0) != 0 }
            && self.bounds == other.bounds
    }
    fn matches(&self) -> bool {
        if !crate::accessibility_trusted() || crate::secure_input_enabled() {
            return false;
        }
        let app = Owned(unsafe { AXUIElementCreateApplication(self.pid) });
        unsafe {
            AXUIElementSetMessagingTimeout(app.0, 0.12);
        }
        let Some(focused) = attr(app.0, "AXFocusedUIElement") else {
            return false;
        };
        if unsafe { core_foundation::base::CFEqual(focused.0, self.element.0) == 0 } {
            return false;
        }
        let text = attr(self.element.0, "AXSelectedText").and_then(|v| string(&v));
        let range = attr(self.element.0, "AXSelectedTextRange").and_then(|v| value_range(&v));
        text.as_deref() == Some(self.text.as_str()) && range == Some(self.range)
    }
}
extern "C" fn changed(_: Ref, _: Ref, _: Ref, _: *mut c_void) {
    DIRTY.store(true, Ordering::Relaxed);
}
define_class!(
 #[unsafe(super(NSObject))]
 #[thread_kind = MainThreadOnly]
 #[name="MagpieSelectionButtons"]
 struct Buttons;
 unsafe impl NSObjectProtocol for Buttons {}
 impl Buttons {
  #[unsafe(method(choose:))]
  fn choose(&self,sender:&NSButton){let i=sender.tag();if (0..5).contains(&i){queue_action(i as usize, BAR_EPOCH.load(Ordering::Acquire));}}
 }
);
struct ClipboardLease {
    items: Vec<Retained<NSPasteboardItem>>,
    count: isize,
    due: Instant,
}
pub struct Controller {
    panel: Retained<NSPanel>,
    _buttons: Retained<Buttons>,
    buttons: Vec<Retained<NSButton>>,
    monitors: Vec<Retained<objc2::runtime::AnyObject>>,
    observer: Option<Owned>,
    manager: Rc<GlobalHotKeyManager>,
    registered: Vec<HotKey>,
    pid: i32,
    lease: Option<ClipboardLease>,
    cached: Option<Target>,
    ignored: Option<Target>,
    settle: u8,
}
impl Controller {
    pub fn new() -> Option<Self> {
        let mtm = MainThreadMarker::new()?;
        let manager = crate::os::hotkeys::shared_manager().ok()?;
        let panel = NSPanel::initWithContentRect_styleMask_backing_defer(
            NSPanel::alloc(mtm),
            NSRect::new(NSPoint::new(0., 0.), NSSize::new(440., 34.)),
            NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel,
            NSBackingStoreType::Buffered,
            false,
        );
        panel.setTitle(&NSString::from_str("Magpie Selection Actions"));
        panel.setFloatingPanel(true);
        panel.setBecomesKeyOnlyIfNeeded(true);
        panel.setHidesOnDeactivate(false);
        panel.setLevel(NSFloatingWindowLevel);
        panel.setHasShadow(true);
        panel.setCollectionBehavior(
            NSWindowCollectionBehavior::CanJoinAllSpaces
                | NSWindowCollectionBehavior::FullScreenAuxiliary,
        );
        let this = Buttons::alloc(mtm).set_ivars(());
        let target: Retained<Buttons> = unsafe { msg_send![super(this), init] };
        let view = panel.contentView()?;
        let mut buttons = Vec::new();
        for i in 0..5 {
            let button = unsafe {
                NSButton::buttonWithTitle_target_action(
                    &NSString::from_str(""),
                    Some(&target),
                    Some(sel!(choose:)),
                    mtm,
                )
            };
            button.setTag(i);
            button.setFrame(NSRect::new(
                NSPoint::new(i as f64 * 120., 4.),
                NSSize::new(120., 32.),
            ));
            button.setRefusesFirstResponder(true);
            button.setBordered(false);
            button.setFont(Some(&NSFont::systemFontOfSize(12.)));
            view.addSubview(&button);
            buttons.push(button);
        }
        let block = block2::RcBlock::new(|event: NonNull<NSEvent>| {
            let kind = unsafe { event.as_ref() }.r#type();
            if kind == NSEventType::LeftMouseDown {
                MOUSE_HELD.store(true, Ordering::Relaxed);
            }
            if kind == NSEventType::LeftMouseUp {
                MOUSE_HELD.store(false, Ordering::Relaxed);
                RESELECT.store(true, Ordering::Relaxed);
            }
            if kind == NSEventType::KeyUp {
                let event = unsafe { event.as_ref() };
                let flags = event.modifierFlags();
                if (flags.contains(NSEventModifierFlags::Command) && event.keyCode() == 0)
                    || (flags.contains(NSEventModifierFlags::Shift)
                        && (123..=126).contains(&event.keyCode()))
                {
                    RESELECT.store(true, Ordering::Relaxed);
                }
            }

            // Carbon delivers contextual shortcuts separately from this monitor.
            // Keep their registrations alive while the modified key is in flight.
            let typing = kind == NSEventType::KeyDown
                && !unsafe { event.as_ref() }.modifierFlags().intersects(
                    NSEventModifierFlags::Command
                        | NSEventModifierFlags::Control
                        | NSEventModifierFlags::Option,
                );
            if typing || matches!(kind, NSEventType::LeftMouseDown | NSEventType::ScrollWheel) {
                DISMISS.store(true, Ordering::Relaxed);
            }
            DIRTY.store(true, Ordering::Relaxed);
        });
        let monitor = NSEvent::addGlobalMonitorForEventsMatchingMask_handler(
            NSEventMask::LeftMouseUp
                | NSEventMask::KeyDown
                | NSEventMask::KeyUp
                | NSEventMask::ScrollWheel
                | NSEventMask::LeftMouseDown,
            &block,
        )?;
        Some(Self {
            panel,
            _buttons: target,
            buttons,
            monitors: vec![monitor],
            observer: None,
            manager,
            registered: Vec::new(),
            pid: 0,
            lease: None,
            cached: None,
            ignored: None,
            settle: 0,
        })
    }
    pub fn poll(&mut self, apps: &[String]) -> Option<Target> {
        self.restore_clipboard();
        if DISMISS.swap(false, Ordering::Relaxed) {
            self.ignored = self.cached.clone();
            self.dismiss();
        }
        // Mouse-up / selection key-up must win over the corresponding down
        // event when both arrive between timer ticks (including Select All).
        if RESELECT.swap(false, Ordering::Relaxed) {
            self.ignored = None;
        }
        let app = NSWorkspace::sharedWorkspace().frontmostApplication()?;
        let pid = app.processIdentifier();
        let bundle = app.bundleIdentifier()?.to_string();
        if !crate::accessibility_trusted()
            || crate::secure_input_enabled()
            || !apps.contains(&bundle)
        {
            self.pid = 0;
            self.cached = None;
            self.ignored = None;
            self.clear_observer();
            return None;
        }
        if MOUSE_HELD.load(Ordering::Relaxed) {
            return None;
        }
        if pid != self.pid {
            DIRTY.store(true, Ordering::Relaxed);
            self.pid = pid;
        }
        if DIRTY.swap(false, Ordering::Relaxed) {
            self.settle = 3;
        }
        if self.settle > 0 {
            self.settle -= 1;
            self.cached = self.capture(pid);
        }
        let target = self.cached.clone()?;
        if self
            .ignored
            .as_ref()
            .is_some_and(|old| old.same_as(&target))
        {
            return None;
        }
        Some(target)
    }
    fn capture(&mut self, pid: i32) -> Option<Target> {
        let app_element = Owned(unsafe { AXUIElementCreateApplication(pid) });
        unsafe {
            AXUIElementSetMessagingTimeout(app_element.0, 0.12);
        }
        let element = Rc::new(attr(app_element.0, "AXFocusedUIElement")?);
        if attr(element.0, "AXSubrole")
            .and_then(|v| string(&v))
            .as_deref()
            == Some("AXSecureTextField")
        {
            return None;
        }
        self.clear_observer();
        if self.observer.is_none() {
            let mut raw = null();
            if unsafe { AXObserverCreate(pid, changed, &mut raw) } == 0 && !raw.is_null() {
                let o = Owned(raw);
                let n = CFString::new("AXSelectedTextChanged");
                unsafe {
                    AXObserverAddNotification(
                        o.0,
                        element.0,
                        n.as_concrete_TypeRef().cast(),
                        null_mut(),
                    );
                    core_foundation::runloop::CFRunLoopAddSource(
                        core_foundation::runloop::CFRunLoopGetMain(),
                        AXObserverGetRunLoopSource(o.0).cast_mut().cast(),
                        core_foundation::runloop::kCFRunLoopCommonModes,
                    );
                }
                self.observer = Some(o);
            }
        }
        let text = attr(element.0, "AXSelectedText").and_then(|v| string(&v))?;
        if text.is_empty() || text.len() > magpie_core::transforms::MAX_INPUT_BYTES {
            return None;
        }
        let range_value = attr(element.0, "AXSelectedTextRange")?;
        let range = value_range(&range_value)?;
        if range.length <= 0 {
            return None;
        }
        let role = attr(element.0, "AXRole")
            .and_then(|v| string(&v))
            .unwrap_or_default();
        let name = CFString::new("AXValue");
        let mut settable = 0;
        let editable = matches!(role.as_str(), "AXTextArea" | "AXTextField")
            && unsafe {
                AXUIElementIsAttributeSettable(
                    element.0,
                    name.as_concrete_TypeRef().cast(),
                    &mut settable,
                )
            } == 0
            && settable != 0;
        let name = CFString::new("AXBoundsForRange");
        let mut raw = null();
        let mut bounds = NSRect::new(NSPoint::new(0., 0.), NSSize::new(0., 0.));
        if unsafe {
            AXUIElementCopyParameterizedAttributeValue(
                element.0,
                name.as_concrete_TypeRef().cast(),
                range_value.0,
                &mut raw,
            )
        } == 0
            && !raw.is_null()
        {
            let v = Owned(raw);
            unsafe {
                AXValueGetValue(v.0, 3, (&mut bounds as *mut NSRect).cast());
            }
        }
        Some(Target {
            text,
            editable,
            element,
            range,
            pid,
            bounds,
        })
    }
    pub fn show(&mut self, target: &Target, labels: &[String], keys: &[String], style: &BarStyle) {
        if self.panel.isVisible() {
            return;
        }
        let mut bar_width = 8.;
        for (i, button) in self.buttons.iter().enumerate() {
            let key = keys.get(i).cloned().unwrap_or_default();
            let mut hint = String::new();
            if let Ok(spec) = crate::parse_hotkey(&key) {
                if let Ok(hk) = crate::os::hotkeys::to_global_hotkey(&spec) {
                    if self.manager.register(hk).is_ok() {
                        HOTKEYS[i].store(hk.id(), Ordering::Relaxed);
                        self.registered.push(hk);
                        hint = format!("  {}", key.replace("ctrl+shift+", "⌃⇧"));
                    }
                }
            }
            button.setTitle(&NSString::from_str(&format!(
                "{}{hint}",
                labels.get(i).map(String::as_str).unwrap_or("More")
            )));
            button.sizeToFit();
            let width = (button.frame().size.width + 16.).clamp(64., 240.);
            button.setFrame(NSRect::new(
                NSPoint::new(bar_width, 2.),
                NSSize::new(width, 30.),
            ));
            bar_width += width;
            button.setContentTintColor(Some(&NSColor::colorWithSRGBRed_green_blue_alpha(
                style.foreground[0],
                style.foreground[1],
                style.foreground[2],
                style.foreground[3],
            )));
        }
        bar_width += 8.;
        self.panel.setContentSize(NSSize::new(bar_width, 34.));
        self.panel
            .setBackgroundColor(Some(&NSColor::colorWithSRGBRed_green_blue_alpha(
                style.background[0],
                style.background[1],
                style.background[2],
                style.background[3],
            )));
        let mouse = NSEvent::mouseLocation();
        let mtm = MainThreadMarker::new().expect("selection UI main thread");
        let screens = NSScreen::screens(mtm);
        // AX uses a top-left origin on the primary display; Cocoa uses its
        // bottom-left origin. Keyboard selections follow their display even if
        // the pointer was left on another one.
        let primary_height = screens
            .iter()
            .next()
            .map(|s| s.frame().size.height)
            .unwrap_or(0.);
        let has_bounds = target.bounds.size.width > 0.;
        let anchor = if has_bounds {
            NSPoint::new(
                target.bounds.origin.x,
                primary_height - target.bounds.origin.y - 1.,
            )
        } else {
            mouse
        };
        let screen = screens.iter().find(|s| {
            let r = s.frame();
            anchor.x >= r.origin.x
                && anchor.x < r.origin.x + r.size.width
                && anchor.y >= r.origin.y
                && anchor.y < r.origin.y + r.size.height
        });
        if let Some(screen) = screen {
            let r = screen.visibleFrame();
            let p = if has_bounds {
                NSPoint::new(
                    target.bounds.origin.x,
                    primary_height - target.bounds.origin.y + 8.,
                )
            } else {
                NSPoint::new(mouse.x, mouse.y + 20.)
            };
            self.panel.setFrameOrigin(NSPoint::new(
                p.x.clamp(
                    r.origin.x,
                    (r.origin.x + r.size.width - bar_width).max(r.origin.x),
                ),
                p.y.clamp(r.origin.y, r.origin.y + r.size.height - 34.),
            ));
        }
        self.panel.orderFrontRegardless();
    }
    pub fn dismiss(&mut self) {
        self.panel.orderOut(None);
        for key in self.registered.drain(..) {
            let _ = self.manager.unregister(key);
        }
        for id in &HOTKEYS {
            id.store(0, Ordering::Relaxed);
        }
        BAR_EPOCH.fetch_add(1, Ordering::Release);
    }
    fn clear_observer(&mut self) {
        if let Some(o) = self.observer.take() {
            unsafe {
                core_foundation::runloop::CFRunLoopRemoveSource(
                    core_foundation::runloop::CFRunLoopGetMain(),
                    AXObserverGetRunLoopSource(o.0).cast_mut().cast(),
                    core_foundation::runloop::kCFRunLoopCommonModes,
                );
            }
        }
    }
    pub fn finish(&mut self, target: &Target) {
        self.ignored = Some(target.clone());
        self.dismiss();
    }
    pub fn focus(&self, target: &Target) -> Result<(), String> {
        let front = NSWorkspace::sharedWorkspace()
            .frontmostApplication()
            .map(|a| a.processIdentifier());
        if front != Some(target.pid) && front != Some(std::process::id() as i32) {
            return Err("The active app changed; select the text again".into());
        }
        let app = NSRunningApplication::runningApplicationWithProcessIdentifier(target.pid)
            .ok_or("The source app closed")?;
        if !target.matches() {
            return Err("The original selection changed; select the text again".into());
        }
        #[allow(deprecated)]
        app.activateWithOptions(NSApplicationActivationOptions::ActivateIgnoringOtherApps);
        Ok(())
    }
    pub fn valid(&self, target: &Target) -> bool {
        target.matches()
            && NSWorkspace::sharedWorkspace()
                .frontmostApplication()
                .is_some_and(|a| a.processIdentifier() == target.pid)
    }
    pub fn replace(&mut self, target: &Target, text: &str) -> Result<(), String> {
        if !target.editable || !target.matches() {
            return Err(
                "The original selection changed; select the text again or copy the result".into(),
            );
        }
        let app = NSRunningApplication::runningApplicationWithProcessIdentifier(target.pid)
            .ok_or("The source app closed")?;
        let front = NSWorkspace::sharedWorkspace()
            .frontmostApplication()
            .map(|a| a.processIdentifier());
        if front != Some(target.pid) && front != Some(std::process::id() as i32) {
            return Err("The active app changed; select the text again".into());
        }
        self.finish(target);
        self.restore_clipboard_now();
        #[allow(deprecated)]
        app.activateWithOptions(NSApplicationActivationOptions::ActivateIgnoringOtherApps);
        let pb = NSPasteboard::generalPasteboard();
        let original_count = pb.changeCount();
        let mut copies = Vec::new();
        if let Some(items) = pb.pasteboardItems() {
            for item in items.iter() {
                let copy = NSPasteboardItem::new();
                for ty in item.types().iter() {
                    let data = item.dataForType(&ty).ok_or(
                        "The current clipboard could not be preserved; copy the result instead",
                    )?;
                    if !copy.setData_forType(&data, &ty) {
                        return Err(
                            "The current clipboard could not be preserved; copy the result instead"
                                .into(),
                        );
                    }
                }
                copies.push(copy);
            }
        }
        if !self.valid(target) {
            return Err("The original selection changed; select the text again".into());
        }
        if pb.changeCount() != original_count {
            return Err(
                "The clipboard changed; run Replace again to use the current clipboard".into(),
            );
        }
        pb.clearContents();
        let marked = pb.setData_forType(
            Some(&NSData::with_bytes(&[])),
            &NSString::from_str("org.nspasteboard.TransientType"),
        );
        let written = pb.setString_forType(
            &NSString::from_str(text),
            &NSString::from_str("public.utf8-plain-text"),
        );
        self.lease = Some(ClipboardLease {
            items: copies,
            count: pb.changeCount(),
            due: Instant::now() + Duration::from_millis(500),
        });
        if !marked || !written {
            self.restore_clipboard_now();
            return Err("Could not prepare the clipboard for replacement".into());
        }
        use crate::Paster;
        let result = crate::os::paste::EnigoPaster.paste();
        if result.is_err() {
            self.restore_clipboard_now();
        }
        result
    }
    fn restore_clipboard_now(&mut self) {
        if let Some(lease) = self.lease.take() {
            let pb = NSPasteboard::generalPasteboard();
            if pb.changeCount() == lease.count {
                pb.clearContents();
                let objects: Vec<&ProtocolObject<dyn NSPasteboardWriting>> = lease
                    .items
                    .iter()
                    .map(|i| ProtocolObject::from_ref(&**i))
                    .collect();
                if !objects.is_empty() && !pb.writeObjects(&NSArray::from_slice(&objects)) {
                    self.lease = Some(ClipboardLease {
                        count: pb.changeCount(),
                        due: Instant::now() + Duration::from_millis(150),
                        ..lease
                    });
                    return;
                }
                RESTORED_CHANGE.store(pb.changeCount(), Ordering::Release);
            }
        }
    }
    pub fn restore_clipboard(&mut self) {
        if self.lease.as_ref().is_some_and(|l| Instant::now() >= l.due) {
            self.restore_clipboard_now();
        }
    }
}
impl Drop for Controller {
    fn drop(&mut self) {
        self.dismiss();
        self.clear_observer();
        self.restore_clipboard_now();
        for monitor in &self.monitors {
            unsafe {
                NSEvent::removeMonitor(monitor);
            }
        }
    }
}
