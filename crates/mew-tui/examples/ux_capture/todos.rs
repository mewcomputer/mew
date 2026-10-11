use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use mew_tui::{app::*, harness::Harness};
fn key(h: &mut Harness, code: KeyCode, modifiers: KeyModifiers) {
    mew_tui::events::handle_input_event(&mut h.app, Event::Key(KeyEvent::new(code, modifiers)));
}

pub fn run(gallery: &mut super::Gallery) {
    let mut h = Harness::new(80, 26);
    h.say("Inspect and edit the current work without losing your conversation draft.");
    h.app.input = "Keep this chat draft while inspecting work".into();
    h.app.cursor = h.app.input.len();
    h.app.status.session_id = "example".into();
    h.app.todos = vec![
        mew_agent::Todo {
            id: 1,
            content: "Make the composer reliable for long pasted prompts".into(),
            status: mew_agent::TodoStatus::InProgress,
            depends_on: vec![],
        },
        mew_agent::Todo {
            id: 2,
            content: "Verify narrow popovers and sidebar click targets".into(),
            status: mew_agent::TodoStatus::Blocked,
            depends_on: vec![1],
        },
        mew_agent::Todo {
            id: 3,
            content: "Preserve the playful personality".into(),
            status: mew_agent::TodoStatus::Done,
            depends_on: vec![],
        },
    ];
    h.app.open_todos(Some(2));
    gallery.capture(&mut h.app, "desktop-todos-list", 128, 40);
    key(&mut h, KeyCode::Enter, KeyModifiers::NONE);
    gallery.capture(&mut h.app, "desktop-todo-editor", 128, 40);
    h.resize(30, 14);
    gallery.capture(&mut h.app, "desktop-todo-editor-narrow", 30, 14);
    h.resize(80, 26);
    h.app.finish_todo_save(Err(
        "cannot complete todo 2: depends on #1 which is not done".into(),
    ));
    gallery.capture(&mut h.app, "desktop-todo-save-error", 128, 40);
    h.app.todo_editor = None;
    h.app.open_command_palette();
    gallery.capture(&mut h.app, "desktop-command-palette", 128, 40);
    h.resize(40, 24);
    gallery.capture(&mut h.app, "desktop-command-palette-narrow", 40, 24);
    h.app.close_picker();
    h.resize(80, 26);
    h.app.mode = Mode::Help;
    gallery.capture(&mut h.app, "desktop-help", 128, 40);
    h.app.mode = Mode::Normal;
    h.app
        .open_goal_compose("Improve the TUI task flow while keeping its personality".into());
    gallery.capture(&mut h.app, "desktop-goal-editor", 128, 40);
    h.app.goal_compose = None;
    h.app.request_persona_switch_confirm(
        PersonaSummary {
            name: "researcher".into(),
            description: "Inspect source and gather evidence before making changes.".into(),
            tools: Some(vec!["read".into(), "search".into()]),
            skills: Some(vec!["code-reviewer".into()]),
            ..Default::default()
        },
        None,
    );
    gallery.capture(&mut h.app, "desktop-persona-switch", 128, 40);
    h.app.persona_switch_confirm = None;
    h.app.mode = Mode::Settings;
    h.app.settings = Some(mew_tui::settings::SettingsState::new(
        mew_config::Config::default(),
        vec![],
    ));
    gallery.capture(&mut h.app, "desktop-settings", 128, 40);
}
