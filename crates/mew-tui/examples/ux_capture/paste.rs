use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use mew_tui::{app::*, events::handle_input_event, harness::Harness};

pub fn run(gallery: &mut super::Gallery) {
    let long = format!(
        "{}\nplease inspect this complete pasted prompt",
        "line of pasted content \n".repeat(100)
    );
    let mut h = Harness::new(80, 24);
    handle_input_event(&mut h.app, Event::Paste(long));
    gallery.capture(&mut h.app, "desktop-paste-confirm", 128, 40);
    handle_input_event(
        &mut h.app,
        Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
    );
    gallery.capture(&mut h.app, "desktop-paste-accepted", 128, 40);
    h.resize(40, 8);
    gallery.capture(&mut h.app, "desktop-paste-short", 40, 8);
    let mut h = Harness::new(80, 24);
    h.app.permission = Some(PermissionState {
        tool_name: "bash".into(),
        call_id: "permission".into(),
        input: "{\"command\":\"cargo test -p mew-tui\"}".parse().unwrap(),
        tx: None,
        selected: 0,
        scroll: 0,
        view: Default::default(),
    });
    h.app.mode = Mode::PermissionPrompt;
    gallery.capture(&mut h.app, "desktop-permission", 128, 40);
    h.resize(30, 14);
    gallery.capture(&mut h.app, "desktop-permission-narrow", 30, 14);
}
