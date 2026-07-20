use cef::application_mac::{CefAppProtocol, CrAppControlProtocol, CrAppProtocol};
use objc2::{
    ClassType, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, extern_methods,
    msg_send,
    rc::Retained,
    runtime::{AnyObject, Bool, Imp, NSObject, NSObjectProtocol, ProtocolObject, Sel},
    sel,
};
use objc2_app_kit::{
    NSApp, NSApplication, NSApplicationDelegate, NSApplicationTerminateReply, NSEvent,
};
use std::cell::Cell;
use std::sync::atomic::{AtomicBool, Ordering};

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

/// Tauri owns the already-created NSApplication instance. Do not replace its
/// class or delegate, because that would break Tauri's event loop.
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
        // CEF's macOS message loop asks NSApp for these two methods. Tauri
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
