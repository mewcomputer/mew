//! Pin the window-scoped navigation separately from global conversation history.
//! Regenerate after inspecting the rendered frames:
//! `MEW_UPDATE_GOLDEN=1 cargo test -p mew-tui --test window_golden`

use mew_client_core::ClientState;
use mew_protocol::{AlertKind, ServerMessage, SessionInfo};
use mew_tui::harness::Harness;

fn apply(harness: &mut Harness, state: &mut ClientState, message: ServerMessage) {
    let events = state.apply_server_message(message);
    harness.app.apply_client_update(&events, state, None);
}

fn ready(harness: &mut Harness, state: &mut ClientState, id: &str) {
    apply(
        harness,
        state,
        ServerMessage::SessionReady {
            session_id: id.into(),
            cwd: Some("/tmp/mew".into()),
            model: None,
            provider: None,
            permission_mode: None,
        },
    );
}

fn window(multiple: bool) -> Harness {
    let mut harness = Harness::new(120, 32);
    let mut state = ClientState::default();
    harness.app.daemon_mode = true;
    harness.app.git_branch = None;
    harness.app.git_branch_resolved = true;
    ready(&mut harness, &mut state, "browser");
    let sessions: Vec<SessionInfo> = [
        ("browser", "Native browser lifecycle", "/tmp/mew"),
        ("tui", "TUI interaction pass", "/tmp/mew"),
        ("site", "Project homepage", "/tmp/site"),
        ("elsewhere", "Another terminal's work", "/tmp/other"),
    ]
    .into_iter()
    .map(|(id, title, cwd)| {
        serde_json::from_value(serde_json::json!({
            "session_id": id, "state": "idle", "created_at": 0,
            "client_count": 1, "summary": title, "cwd": cwd
        }))
        .unwrap()
    })
    .collect();
    apply(
        &mut harness,
        &mut state,
        ServerMessage::SessionList { sessions },
    );
    if multiple {
        for id in ["tui", "site", "browser"] {
            ready(&mut harness, &mut state, id);
        }
        apply(
            &mut harness,
            &mut state,
            ServerMessage::SessionAttentionChanged {
                session_id: "tui".into(),
                pending_permissions: 0,
                pending_questions: 1,
            },
        );
        apply(
            &mut harness,
            &mut state,
            ServerMessage::SessionAlert {
                session_id: "site".into(),
                kind: AlertKind::TurnComplete,
                title: "Project homepage".into(),
                detail: None,
            },
        );
    }
    // Global broadcasts must not make another window's work appear here.
    apply(
        &mut harness,
        &mut state,
        ServerMessage::SessionAttentionChanged {
            session_id: "elsewhere".into(),
            pending_permissions: 2,
            pending_questions: 0,
        },
    );
    harness.app.alert = None;
    harness.app.toasts.clear();
    harness
        .app
        .push_user("check the browser lifecycle".into(), vec![]);
    harness.say("Each terminal chooses which conversations to keep open. Closing a view preserves its draft and running work.");
    harness.app.input = "include reconnects too".into();
    harness.app.cursor = harness.app.input.len();
    harness
}

fn golden(name: &str, harness: &mut Harness) -> String {
    let actual = harness.render();
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(format!("{name}.frame"));
    if std::env::var("MEW_UPDATE_GOLDEN").as_deref() == Ok("1") {
        std::fs::write(&path, &actual).unwrap();
    }
    let expected = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("missing golden frame: {}", path.display()));
    assert_eq!(actual.trim_end(), expected.trim_end(), "{name}");
    actual
}

#[test]
fn window_single_quiet() {
    let frame = golden("window_single_quiet", &mut window(false));
    assert!(!frame.contains("Open here"));
    assert!(!frame.contains("Another terminal's work"));
}

#[test]
fn window_multiple_open() {
    let frame = golden("window_multiple_open", &mut window(true));
    assert!(frame.contains("Open here"));
    assert!(frame.contains("1 needs you"));
    assert!(frame.contains("1 result"));
    assert!(!frame.contains("Another terminal's work"));
}

#[test]
fn window_notifications() {
    let mut harness = window(true);
    harness.app.open_attention_picker();
    let frame = golden("window_notifications", &mut harness);
    assert!(frame.contains("TUI interaction pass"));
    assert!(frame.contains("Project homepage"));
    assert!(!frame.contains("Another terminal's work"));
}

#[test]
fn window_global_history() {
    let mut harness = window(true);
    harness.app.open_session_picker();
    let frame = golden("window_global_history", &mut harness);
    assert!(frame.contains("Another terminal's work"));
    assert!(frame.contains("Ctrl+Alt+O open here"));
}
