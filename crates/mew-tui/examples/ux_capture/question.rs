use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use mew_tui::{app::*, events::handle_input_event};

pub fn run(gallery: &mut super::Gallery) {
    let mut app = App::new();
    app.push_synthetic_message("the session reconnect checks pass. before the next change, i need your preference on task flow.".into());
    app.user_question = Some(UserQuestionState {
        call_id: "question-capture".into(),
        subagent: None,
        questions: vec![
            mew_agent::AskUserQuestion {
                prompt: "which surface should we start with?".into(),
                options: vec![],
            },
            mew_agent::AskUserQuestion {
                prompt: "what matters most in the next ux pass?".into(),
                options: vec![
                    mew_agent::QuestionOption {
                        label: "task progress".into(),
                        description: "make active work, blockers, and follow-ups easy to find"
                            .into(),
                    },
                    mew_agent::QuestionOption {
                        label: "browser sessions".into(),
                        description: "keep the retained page and conversation in sync".into(),
                    },
                ],
            },
        ],
        answers: vec![
            "tui with the native app as a companion".into(),
            String::new(),
        ],
        page: 1,
        selected: 2,
        freeform_text: String::new(),
        freeform_cursor: 0,
        freeform_undo: Default::default(),
        review: false,
        review_selected: 0,
        tx: None,
    });
    app.mode = Mode::UserQuestion;
    handle_input_event(&mut app,Event::Paste("i want the tui and native app to share a clear task flow. keep the personality, make active work easy to find, and put unreliable session behavior first. the browser should stay attached to the right conversation when i switch tasks. long pasted prompts and answers should stay visible while editing, even on a smaller terminal.".into()));
    gallery.capture(&mut app, "desktop-question-freeform", 128, 40);
    handle_input_event(
        &mut app,
        Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
    );
    gallery.capture(&mut app, "desktop-question-review", 128, 40);
}
