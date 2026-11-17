#[cfg(target_os = "macos")]
fn main() {
    use objc2::{MainThreadMarker, MainThreadOnly};
    use objc2_app_kit::{
        NSApplication, NSBackingStoreType, NSButton, NSButtonType, NSEvent, NSEventModifierFlags,
        NSEventType, NSView, NSWindow, NSWindowStyleMask,
    };
    use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

    if std::env::var_os("MEW_NATIVE_APPKIT_TESTS").is_none() {
        println!("native key equivalents: ignored; set MEW_NATIVE_APPKIT_TESTS=1 to run");
        return;
    }
    let mtm = MainThreadMarker::new().unwrap();
    let _app = NSApplication::sharedApplication(mtm);
    let bounds = NSRect::new(NSPoint::ZERO, NSSize::new(200., 200.));
    let window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            bounds,
            NSWindowStyleMask::Borderless,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    // Stock AppKit controls reproduce the host consuming a key equivalent
    // before a focused native descendant. No test doubles or clipboard writes.
    let host = NSButton::new(mtm);
    host.setButtonType(NSButtonType::Switch);
    host.setFrame(bounds);
    window.setContentView(Some(&host));
    let browser = NSView::new(mtm);
    browser.setFrame(bounds);
    host.addSubview(&browser);
    let native = NSButton::new(mtm);
    native.setButtonType(NSButtonType::Switch);
    native.setFrame(bounds);
    browser.addSubview(&native);
    let command = NSEventModifierFlags::Command;
    let configure = |key: &str, modifiers| {
        for button in [&host, &native] {
            button.setState(0);
            button.setKeyEquivalent(&NSString::from_str(key));
            button.setKeyEquivalentModifierMask(modifiers);
        }
    };
    let dispatch = |key: &str, modifiers| {
        let text = NSString::from_str(key);
        let event = NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
            NSEventType::KeyDown,
            NSPoint::ZERO,
            modifiers,
            0.,
            window.windowNumber(),
            None,
            &text,
            &text,
            false,
            0,
        ).unwrap();
        assert!(window.performKeyEquivalent(&event));
    };
    mew_cef_host::mac::register_browser_view(&browser);
    mew_cef_host::mac::register_browser_view(&browser);

    for key in ["a", "c", "v", "x", "z"] {
        configure(key, command);
        assert!(window.makeFirstResponder(Some(&native)));
        dispatch(key, command);
        assert_eq!(
            native.state(),
            1,
            "focused native child must receive Cmd-{key}"
        );
        assert_eq!(host.state(), 0, "host must not consume native Cmd-{key}");
    }
    let shifted = command | NSEventModifierFlags::Shift;
    configure("v", shifted);
    dispatch("v", shifted);
    assert_eq!((native.state(), host.state()), (1, 0));

    for (key, name) in [
        ("\u{f700}", "up"),
        ("\u{f701}", "down"),
        ("\u{f702}", "left"),
        ("\u{f703}", "right"),
        ("\u{8}", "backspace"),
        ("\u{7f}", "delete"),
        ("\u{f728}", "forward delete"),
    ] {
        for modifiers in [
            command,
            shifted,
            command | NSEventModifierFlags::Function,
            shifted | NSEventModifierFlags::Function,
        ] {
            configure(key, modifiers);
            dispatch(key, modifiers);
            assert_eq!(
                (native.state(), host.state()),
                (1, 0),
                "native Cmd-{name} with {modifiers:?}"
            );
        }
        let control_command = command | NSEventModifierFlags::Control;
        configure(key, control_command);
        dispatch(key, control_command);
        assert_eq!((native.state(), host.state()), (0, 1));
    }

    configure("v", command);
    assert!(window.makeFirstResponder(Some(&host)));
    dispatch("v", command);
    assert_eq!((native.state(), host.state()), (0, 1));

    configure("v", command);
    assert!(window.makeFirstResponder(Some(&native)));
    browser.setHidden(true);
    dispatch("v", command);
    assert_eq!((native.state(), host.state()), (0, 1));
    browser.setHidden(false);

    let unrelated = NSButton::new(mtm);
    unrelated.setButtonType(NSButtonType::Switch);
    unrelated.setKeyEquivalent(&NSString::from_str("v"));
    unrelated.setKeyEquivalentModifierMask(command);
    host.addSubview(&unrelated);
    configure("v", command);
    assert!(window.makeFirstResponder(Some(&unrelated)));
    dispatch("v", command);
    assert_eq!((unrelated.state(), host.state()), (0, 1));

    configure("v", command);
    native.setKeyEquivalent(&NSString::from_str("c"));
    assert!(window.makeFirstResponder(Some(&native)));
    dispatch("v", command);
    assert_eq!((native.state(), host.state()), (0, 1));

    for key in ["q", "n", "w", "b", "j", "k", "y"] {
        configure(key, command);
        dispatch(key, command);
        assert_eq!(
            (native.state(), host.state()),
            (0, 1),
            "Cmd-{key} stays with host"
        );
    }
    for extra in [
        NSEventModifierFlags::Control,
        NSEventModifierFlags::Function,
    ] {
        configure("v", command | extra);
        dispatch("v", command | extra);
        assert_eq!((native.state(), host.state()), (0, 1));
    }
    native_browser_accessibility(mtm);
    native_app_edit_menu(mtm);
    native_app_draggable_regions(mtm);
    native_browser_input_guard(mtm);
    println!("native editing and browser accessibility: all checks passed");
}

#[cfg(target_os = "macos")]
fn native_browser_input_guard(mtm: objc2::MainThreadMarker) {
    use objc2::{MainThreadOnly, class, msg_send, rc::Retained};
    use objc2_app_kit::{NSBackingStoreType, NSButton, NSView, NSWindow, NSWindowStyleMask};
    use objc2_foundation::{NSPoint, NSRect, NSSize};

    let bounds = NSRect::new(NSPoint::ZERO, NSSize::new(300., 200.));
    let window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            bounds,
            NSWindowStyleMask::Borderless,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    let parent = NSView::new(mtm);
    window.setContentView(Some(&parent));
    let browser = NSView::new(mtm);
    browser.setFrame(bounds);
    parent.addSubview(&browser);
    let button = NSButton::new(mtm);
    button.setFrame(bounds);
    browser.addSubview(&button);
    let shield = mew_cef_host::mac::BrowserInputShield::attach(&browser);
    let target = browser.hitTest(NSPoint::new(100., 100.)).unwrap();
    let button_view: &NSView = &button;
    assert!(
        std::ptr::eq(&*target, button_view),
        "new pages must accept input by default"
    );

    let editor: Retained<NSView> = unsafe { msg_send![class!(NSTextView), new] };
    browser.addSubview(&editor);
    assert!(window.makeFirstResponder(Some(&editor)));
    shield.set_interactive(false);
    let target = browser.hitTest(NSPoint::new(100., 100.)).unwrap();
    let shield_view: &NSView = &shield;
    assert!(
        std::ptr::eq(&*target, shield_view),
        "closing pages must consume pointer input without hiding their content"
    );
    assert!(!browser.isHidden());
    assert_eq!(browser.frame(), bounds);
    assert!(!std::ptr::eq(
        &*window.firstResponder().unwrap(),
        &*editor as &objc2_app_kit::NSResponder
    ));
    browser.setFrame(NSRect::new(NSPoint::ZERO, NSSize::new(400., 300.)));
    let target = browser.hitTest(NSPoint::new(350., 250.)).unwrap();
    assert!(
        std::ptr::eq(&*target, shield_view),
        "animated bounds must remain covered by the input guard"
    );

    let other_editor: Retained<NSView> = unsafe { msg_send![class!(NSTextView), new] };
    parent.addSubview(&other_editor);
    assert!(window.makeFirstResponder(Some(&other_editor)));
    shield.set_interactive(false);
    assert!(
        std::ptr::eq(
            &*window.firstResponder().unwrap(),
            &*other_editor as &objc2_app_kit::NSResponder
        ),
        "background page updates must preserve another page's focus"
    );
    other_editor.removeFromSuperview();
    editor.removeFromSuperview();
    shield.set_interactive(true);
    let target = browser.hitTest(NSPoint::new(100., 100.)).unwrap();
    assert!(
        std::ptr::eq(&*target, button_view),
        "reopening must restore the same retained page's controls"
    );
}

#[cfg(target_os = "macos")]
fn native_app_draggable_regions(mtm: objc2::MainThreadMarker) {
    use cef::{DraggableRegion, Rect};
    use objc2::MainThreadOnly;
    use objc2_app_kit::{NSBackingStoreType, NSButton, NSView, NSWindow, NSWindowStyleMask};
    use objc2_foundation::{NSPoint, NSRect, NSSize};

    let window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            NSRect::new(NSPoint::ZERO, NSSize::new(500., 400.)),
            NSWindowStyleMask::Borderless,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    let parent = NSView::new(mtm);
    window.setContentView(Some(&parent));
    let browser = NSView::new(mtm);
    browser.setFrame(NSRect::new(NSPoint::new(20., 30.), NSSize::new(300., 200.)));
    parent.addSubview(&browser);
    let button = NSButton::new(mtm);
    button.setFrame(NSRect::new(NSPoint::new(240., 164.), NSSize::new(60., 36.)));
    browser.addSubview(&button);
    let overlay = mew_cef_host::mac::AppDragView::attach(&browser);
    let region = |x, y, width, height, draggable| DraggableRegion {
        bounds: Rect {
            x,
            y,
            width,
            height,
        },
        draggable,
    };
    overlay.set_regions(&[region(0, 0, 300, 36, 1), region(240, 0, 60, 36, 0)]);
    assert!(
        overlay.hitTest(NSPoint::new(100., 180.)).is_some(),
        "CSS header coordinates must flip into AppKit coordinates"
    );
    let drag_target = browser.hitTest(NSPoint::new(120., 210.)).unwrap();
    let overlay_view: &NSView = &overlay;
    assert!(
        std::ptr::eq(&*drag_target, overlay_view),
        "draggable chrome must intercept the actual native hit test"
    );
    assert!(
        overlay.hitTest(NSPoint::new(270., 180.)).is_none(),
        "interactive no-drag controls must pass through"
    );
    let target = browser.hitTest(NSPoint::new(290., 210.)).unwrap();
    let button_view: &NSView = &button;
    assert!(
        std::ptr::eq(&*target, button_view),
        "native control must remain the actual hit target"
    );
    assert!(overlay.hitTest(NSPoint::new(100., 100.)).is_none());
    assert!(
        overlay.hitTest(NSPoint::new(100., 164.)).is_none(),
        "bottom edge is exclusive"
    );
    assert!(overlay.hitTest(NSPoint::new(-1., 180.)).is_none());
    let new_content = NSView::new(mtm);
    new_content.setFrame(browser.bounds());
    browser.addSubview(&new_content);
    overlay.set_regions(&[region(0, 0, 300, 36, 1), region(240, 0, 60, 36, 0)]);
    let drag_target = browser.hitTest(NSPoint::new(120., 210.)).unwrap();
    assert!(
        std::ptr::eq(&*drag_target, overlay_view),
        "region updates must keep drag handling above new browser content"
    );
    new_content.removeFromSuperview();
    overlay.set_regions(&[
        region(0, 0, 300, 36, 1),
        region(240, 0, 60, 36, 0),
        region(250, 5, 10, 10, 1),
    ]);
    assert!(
        overlay.hitTest(NSPoint::new(255., 190.)).is_some(),
        "regions must respect CEF's ordered union and subtraction"
    );
    browser.setFrameSize(NSSize::new(400., 250.));
    assert!(
        overlay.hitTest(NSPoint::new(100., 238.)).is_some(),
        "overlay must resize with the app view"
    );
    browser.setHidden(true);
    assert!(overlay.hitTest(NSPoint::new(100., 238.)).is_none());
    browser.setHidden(false);
    overlay.set_regions(&[]);
    assert!(
        overlay.hitTest(NSPoint::new(100., 238.)).is_none(),
        "empty updates must release every drag hit target"
    );
}

#[cfg(target_os = "macos")]
fn native_app_edit_menu(mtm: objc2::MainThreadMarker) {
    use objc2::MainThreadOnly;
    use objc2::{
        class, msg_send,
        rc::{Allocated, Retained},
        runtime::{AnyObject, NSObject, Sel},
        sel,
    };
    use objc2_app_kit::{NSApp, NSBackingStoreType, NSView, NSWindow, NSWindowStyleMask};
    use objc2_foundation::{NSPoint, NSRange, NSRect, NSSize, NSString};

    let app = NSApp(mtm);
    let original: Option<Retained<NSObject>> = unsafe { msg_send![&*app, mainMenu] };
    let main: Retained<NSObject> = unsafe { msg_send![class!(NSMenu), new] };
    let edit: Retained<NSObject> = unsafe { msg_send![class!(NSMenu), new] };
    let create = |title: &str, key: &str, action: Sel| -> Retained<NSObject> {
        unsafe {
            let item: Allocated<NSObject> = msg_send![class!(NSMenuItem), alloc];
            msg_send![item, initWithTitle: &*NSString::from_str(title), action: action, keyEquivalent: &*NSString::from_str(key)]
        }
    };
    let parent = create("Edit", "", sel!(handleGPUIMenuItem:));
    let undo = create("Undo", "z", sel!(handleGPUIMenuItem:));
    let redo = create("Redo", "Z", sel!(handleGPUIMenuItem:));
    let copy = create("Copy", "c", sel!(copy:));
    unsafe {
        let _: () = msg_send![&*main, addItem: &*parent];
        let _: () = msg_send![&*parent, setSubmenu: &*edit];
        for item in [&undo, &redo, &copy] {
            let _: () = msg_send![&*edit, addItem: &**item];
        }
        let _: () = msg_send![&*app, setMainMenu: &*main];
    }
    let action = |item: &NSObject| -> Option<Sel> { unsafe { msg_send![item, action] } };
    assert_eq!(action(&undo), Some(sel!(handleGPUIMenuItem:)));
    assert_eq!(action(&redo), Some(sel!(handleGPUIMenuItem:)));
    mew_cef_host::mac::configure_app_edit_menu();
    assert_eq!(action(&undo), Some(sel!(undo:)));
    assert_eq!(action(&redo), Some(sel!(redo:)));
    assert_eq!(action(&copy), Some(sel!(copy:)));
    for item in [&undo, &redo] {
        let target: Option<Retained<AnyObject>> = unsafe { msg_send![&**item, target] };
        assert!(
            target.is_none(),
            "native editing must use the first-responder chain"
        );
    }
    let window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            NSRect::new(NSPoint::ZERO, NSSize::new(200., 200.)),
            NSWindowStyleMask::Borderless,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    let editor: Retained<NSView> = unsafe { msg_send![class!(NSTextView), new] };
    window.setContentView(Some(&editor));
    assert!(window.makeFirstResponder(Some(&editor)));
    unsafe {
        let _: () = msg_send![&*editor, setAllowsUndo: true];
        let _: () = msg_send![&*editor, insertText: &*NSString::from_str("native edit"), replacementRange: NSRange::new(0, 0)];
        let _: () = msg_send![&*editor, breakUndoCoalescing];
    }
    let text = || -> String {
        let text: Retained<NSString> = unsafe { msg_send![&*editor, string] };
        text.to_string()
    };
    assert_eq!(text(), "native edit");
    assert!(unsafe { editor.tryToPerform_with(action(&undo).unwrap(), Some(&undo)) });
    assert_eq!(text(), "");
    assert!(unsafe { editor.tryToPerform_with(action(&redo).unwrap(), Some(&redo)) });
    assert_eq!(text(), "native edit");
    unsafe {
        let _: () = msg_send![&*app, setMainMenu: original.as_deref()];
    }
}

#[cfg(target_os = "macos")]
fn native_browser_accessibility(mtm: objc2::MainThreadMarker) {
    use objc2::{MainThreadOnly, msg_send, rc::Retained, runtime::NSObject};
    use objc2_app_kit::{NSBackingStoreType, NSButton, NSView, NSWindow, NSWindowStyleMask};
    use objc2_foundation::{NSArray, NSPoint, NSRect, NSSize};

    let bounds = NSRect::new(NSPoint::ZERO, NSSize::new(200., 200.));
    let window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            bounds,
            NSWindowStyleMask::Borderless,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    let host = NSView::new(mtm);
    host.setFrame(bounds);
    window.setContentView(Some(&host));
    let chrome = NSButton::new(mtm);
    host.addSubview(&chrome);
    // AppKit's explicit children list represents a host that owns its AX tree.
    let declared = NSArray::<NSObject>::from_slice(&[&chrome]);
    unsafe {
        let _: () = msg_send![&*host, setAccessibilityChildren: &*declared];
    }
    let children =
        || -> Retained<NSArray<NSObject>> { unsafe { msg_send![&*host, accessibilityChildren] } };
    assert!(children().containsObject(&chrome));

    let app = NSView::new(mtm);
    app.setFrame(bounds);
    host.addSubview(&app);
    let composer = NSButton::new(mtm);
    composer.setFrame(bounds);
    app.addSubview(&composer);
    let composer_node: Retained<NSObject> = unsafe { msg_send![&*composer, cell] };
    assert!(!children().containsObject(&composer_node));
    mew_cef_host::mac::register_browser_view(&app);
    mew_cef_host::mac::register_browser_view(&app);
    assert!(children().containsObject(&chrome));
    assert!(
        children().containsObject(&composer_node),
        "native app controls must join the host AX tree"
    );

    let external = NSView::new(mtm);
    external.setFrame(bounds);
    host.addSubview(&external);
    let page_control = NSButton::new(mtm);
    page_control.setFrame(bounds);
    external.addSubview(&page_control);
    let page_node: Retained<NSObject> = unsafe { msg_send![&*page_control, cell] };
    mew_cef_host::mac::register_browser_view(&external);
    assert_eq!(
        children().count(),
        3,
        "app and browser siblings must both be accessible, without duplicates"
    );
    let shield = mew_cef_host::mac::BrowserInputShield::attach(&external);
    shield.set_interactive(false);
    assert!(
        !children().containsObject(&page_node),
        "closing pages must leave the AX tree before their exit animation finishes"
    );
    assert!(children().containsObject(&composer_node));
    shield.set_interactive(true);
    assert!(
        children().containsObject(&page_node),
        "reopening must restore the retained page's AX controls"
    );
    external.setHidden(true);
    assert!(children().containsObject(&composer_node));
    assert!(
        !children().containsObject(&page_node),
        "hidden native pages must leave the AX tree"
    );
    app.removeFromSuperview();
    assert_eq!(
        children().count(),
        1,
        "detached pages must leave the AX tree even while retained"
    );
    assert!(children().containsObject(&chrome));
}

#[cfg(not(target_os = "macos"))]
fn main() {}
