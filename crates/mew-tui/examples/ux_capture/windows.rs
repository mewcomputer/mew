use mew_client_core::{ClientState, ConnectionStatus};
use mew_protocol::{AlertKind, ServerMessage, SessionInfo};
use mew_tui::{harness::Harness, Mode};

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
            cwd: Some("~/code/mew".into()),
            model: None,
            provider: None,
            permission_mode: None,
        },
    );
}

fn window(multiple: bool) -> Harness {
    let mut harness = Harness::new(128, 40);
    let mut state = ClientState::default();
    harness.app.daemon_mode = true;
    harness.app.git_branch = None;
    harness.app.git_branch_resolved = true;
    ready(&mut harness, &mut state, "browser");
    let sessions: Vec<SessionInfo> = [
        ("browser", "Native browser lifecycle", "~/code/mew"),
        ("tui", "TUI interaction pass", "~/code/mew"),
        ("site", "Project homepage", "~/code/site"),
        ("elsewhere", "Another terminal's work", "~/code/other"),
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

pub fn run(gallery: &mut super::Gallery) {
    let mut single = window(false);
    gallery.capture(&mut single.app, "desktop-single-conversation", 128, 40);
    gallery.capture(&mut single.app, "short-single-conversation", 60, 22);
    let mut multiple = window(true);
    gallery.capture(&mut multiple.app, "desktop-open-conversations", 128, 40);
    multiple.app.open_window_picker();
    gallery.capture(&mut multiple.app, "desktop-open-picker", 128, 40);
    multiple.app.open_attention_picker();
    gallery.capture(&mut multiple.app, "desktop-attention-bell", 128, 40);
    gallery.capture(&mut multiple.app, "short-notifications", 60, 22);
    multiple.app.open_session_picker();
    gallery.capture(&mut multiple.app, "desktop-conversation-picker", 128, 40);
    gallery.capture(&mut multiple.app, "short-conversation-projects", 40, 16);
    multiple.app.picker = None;
    multiple.app.mode = Mode::Normal;
    let theme = multiple.app.theme.clone();
    gallery.capture(&mut multiple.app, "desktop-user-surfaces", 128, 40);
    multiple.app.theme = mew_tui::theme::Theme::light();
    multiple.app.rendered_md_cache.clear();
    multiple.app.mark_chat_dirty();
    gallery.capture(&mut multiple.app, "desktop-user-surfaces-light", 128, 40);
    multiple.app.theme = theme;
    multiple.app.rendered_md_cache.clear();
    multiple.app.mark_chat_dirty();
    gallery.capture(&mut multiple.app, "short-user-surfaces", 40, 16);
    multiple.app.connection = Some(ConnectionStatus::Backoff {
        attempt: 2,
        error: "daemon connection closed".into(),
    });
    multiple.app.attaching_session = true;
    gallery.capture(&mut multiple.app, "desktop-reconnect-status", 128, 40);
}
