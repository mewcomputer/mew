use cef::application_mac::{CefAppProtocol, CrAppControlProtocol, CrAppProtocol};
use objc2::{
    ClassType, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, extern_methods,
    msg_send,
    rc::{Retained, Weak},
    runtime::{AnyObject, Bool, Imp, NSObject, NSObjectProtocol, ProtocolObject, Sel},
    sel,
};
use objc2_app_kit::{
    NSApp, NSApplication, NSApplicationDelegate, NSApplicationTerminateReply, NSBackspaceCharacter,
    NSDeleteCharacter, NSDeleteFunctionKey, NSDownArrowFunctionKey, NSEvent, NSEventModifierFlags,
    NSLeftArrowFunctionKey, NSRightArrowFunctionKey, NSUpArrowFunctionKey, NSView, NSWindow,
};
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Default)]
struct NativeKeyEquivalents {
    original_handlers: BTreeMap<usize, Imp>,
    browser_views: Vec<Weak<NSView>>,
}

thread_local! {
    static NATIVE_KEY_EQUIVALENTS: RefCell<NativeKeyEquivalents> = RefCell::default();
}

fn native_key_equivalent_imp() -> Imp {
    unsafe {
        std::mem::transmute(native_key_equivalent as unsafe extern "C-unwind" fn(_, _, _) -> _)
    }
}

/// Give a focused CEF child its editing shortcuts before the host view.
#[allow(dead_code)]
pub fn register_browser_view(view: &NSView) {
    let Some(window) = view.window() else { return };
    let class = window.class();
    let selector = sel!(performKeyEquivalent:);
    let method = class.instance_method(selector).unwrap();
    let original = method.implementation();
    let replacement = native_key_equivalent_imp();
    NATIVE_KEY_EQUIVALENTS.with_borrow_mut(|state| {
        if !std::ptr::fn_addr_eq(original, replacement) {
            state
                .original_handlers
                .insert(class as *const _ as usize, original);
            unsafe {
                objc2::ffi::class_replaceMethod(
                    class as *const _ as *mut _,
                    selector,
                    replacement,
                    objc2::ffi::method_getTypeEncoding(method),
                );
            }
        }
        state.browser_views.retain(|view| view.load().is_some());
        if !state
            .browser_views
            .iter()
            .filter_map(Weak::load)
            .any(|known| std::ptr::eq(&*known, view))
        {
            state.browser_views.push(Weak::new(view));
        }
    });
}

unsafe extern "C-unwind" fn native_key_equivalent(
    window: &NSWindow,
    selector: Sel,
    event: &NSEvent,
) -> Bool {
    let flags = event.modifierFlags();
    let editing = flags.contains(NSEventModifierFlags::Command)
        && !flags.contains(NSEventModifierFlags::Control)
        && event.charactersIgnoringModifiers().is_some_and(|text| {
            if text.length() != 1 {
                return false;
            }
            let key = text.characterAtIndex(0);
            [
                NSUpArrowFunctionKey,
                NSDownArrowFunctionKey,
                NSLeftArrowFunctionKey,
                NSRightArrowFunctionKey,
                NSBackspaceCharacter,
                NSDeleteCharacter,
                NSDeleteFunctionKey,
            ]
            .contains(&u32::from(key))
                || (!flags.contains(NSEventModifierFlags::Function)
                    && u8::try_from(key).is_ok_and(|key| {
                        matches!(key.to_ascii_lowercase(), b'a' | b'c' | b'v' | b'x' | b'z')
                    }))
        });
    if editing
        && let Some(responder) = window.firstResponder()
        && let Some(view) = responder.downcast_ref::<NSView>()
        && !view.isHiddenOrHasHiddenAncestor()
        && NATIVE_KEY_EQUIVALENTS.with_borrow(|state| {
            state
                .browser_views
                .iter()
                .filter_map(Weak::load)
                .any(|browser| view.isDescendantOf(&browser))
        })
        && responder.performKeyEquivalent(event)
    {
        return Bool::YES;
    }

    let original = NATIVE_KEY_EQUIVALENTS.with_borrow(|state| {
        let mut class = Some(window.class());
        while let Some(current) = class {
            if let Some(handler) = state.original_handlers.get(&(current as *const _ as usize)) {
                return *handler;
            }
            class = current.superclass();
        }
        unreachable!("native window handler must retain its original implementation")
    });
    let original: unsafe extern "C-unwind" fn(&NSWindow, Sel, &NSEvent) -> Bool =
        unsafe { std::mem::transmute(original) };
    unsafe { original(window, selector, event) }
}

static EXISTING_APPLICATION_HANDLING_SEND_EVENT: AtomicBool = AtomicBool::new(false);

unsafe extern "C-unwind" fn existing_application_is_handling_send_event(
    _this: &AnyObject,
    _cmd: Sel,
) -> Bool {
    Bool::from(EXISTING_APPLICATION_HANDLING_SEND_EVENT.load(Ordering::Acquire))
}

unsafe extern "C-unwind" fn existing_application_set_handling_send_event(
    _this: &AnyObject,
    _cmd: Sel,
    value: Bool,
) {
    EXISTING_APPLICATION_HANDLING_SEND_EVENT.store(value.as_bool(), Ordering::Release);
}

define_class! {
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    pub struct MewCefAppDelegate;

    unsafe impl NSObjectProtocol for MewCefAppDelegate {}

    unsafe impl NSApplicationDelegate for MewCefAppDelegate {
        #[unsafe(method(applicationShouldTerminate:))]
        unsafe fn application_should_terminate(&self, _sender: &NSApplication) -> NSApplicationTerminateReply {
            NSApplicationTerminateReply::TerminateNow
        }

        #[unsafe(method(applicationSupportsSecureRestorableState:))]
        unsafe fn application_supports_secure_restorable_state(&self, _sender: &NSApplication) -> Bool {
            Bool::YES
        }
    }
}

impl MewCefAppDelegate {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = MewCefAppDelegate::alloc(mtm).set_ivars(());
        unsafe { msg_send![super(this), init] }
    }
}

#[derive(Default)]
pub struct MewCefApplicationIvars {
    handling_send_event: Cell<Bool>,
}

define_class! {
    #[unsafe(super(NSApplication))]
    #[ivars = MewCefApplicationIvars]
    pub struct MewCefApplication;

    impl MewCefApplication {
        #[unsafe(method(sendEvent:))]
        unsafe fn send_event(&self, event: &NSEvent) {
            let was_sending_event = self.ivars().handling_send_event.get();
            if !was_sending_event.as_bool() {
                self.ivars().handling_send_event.set(Bool::YES);
            }
            let _: () = msg_send![super(self), sendEvent:event];
            if !was_sending_event.as_bool() {
                self.ivars().handling_send_event.set(Bool::NO);
            }
        }
    }

    unsafe impl CrAppControlProtocol for MewCefApplication {
        #[unsafe(method(setHandlingSendEvent:))]
        unsafe fn _set_handling_send_event(&self, value: Bool) {
            self.ivars().handling_send_event.set(value);
        }
    }

    unsafe impl CrAppProtocol for MewCefApplication {
        #[unsafe(method(isHandlingSendEvent))]
        unsafe fn _is_handling_send_event(&self) -> Bool {
            self.ivars().handling_send_event.get()
        }
    }

    unsafe impl CefAppProtocol for MewCefApplication {}
}

impl MewCefApplication {
    extern_methods! {
        #[unsafe(method(sharedApplication))]
        fn shared_application() -> Retained<Self>;
    }
}

pub fn setup_application() {
    let mtm = MainThreadMarker::new().expect("CEF must initialize on the main thread");
    let _ = MewCefApplication::shared_application();
    assert!(NSApp(mtm).isKindOfClass(MewCefApplication::class()));
}

/// GPUI owns the already-created NSApplication instance. Do not replace its
/// class or delegate, because that would break the app event loop.
#[allow(dead_code)]
pub fn setup_existing_application() {
    let mtm = MainThreadMarker::new().expect("CEF must initialize on the main thread");
    let app = NSApp(mtm);
    let class = (app.as_ref() as &AnyObject).class();
    let class = class as *const _ as *mut _;
    unsafe {
        let get_imp: Imp = std::mem::transmute(
            existing_application_is_handling_send_event as unsafe extern "C-unwind" fn(_, _) -> _,
        );
        let set_imp: Imp = std::mem::transmute(
            existing_application_set_handling_send_event as unsafe extern "C-unwind" fn(_, _, _),
        );
        // CEF's macOS message loop asks NSApp for these two methods. GPUI
        // owns the application instance, so add the narrow protocol methods
        // to its concrete class instead of replacing the application class.
        let _ =
            objc2::ffi::class_addMethod(class, sel!(isHandlingSendEvent), get_imp, c"B@:".as_ptr());
        let _ = objc2::ffi::class_addMethod(
            class,
            sel!(setHandlingSendEvent:),
            set_imp,
            c"v@:B".as_ptr(),
        );
    }
}

pub fn setup_application_delegate() -> Retained<MewCefAppDelegate> {
    let mtm = MainThreadMarker::new().expect("CEF must initialize on the main thread");
    let delegate = MewCefAppDelegate::new(mtm);
    let protocol = ProtocolObject::<dyn NSApplicationDelegate>::from_retained(delegate.clone());
    NSApp(mtm).setDelegate(Some(&protocol));
    delegate
}
