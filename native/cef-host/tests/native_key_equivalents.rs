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
    println!("native key equivalents: all responder ownership checks passed");
}

#[cfg(not(target_os = "macos"))]
fn main() {}
