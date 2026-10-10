use mew_client_core::{ActionKind, ClientState};
use mew_protocol::{ServerMessage, SubagentQuestionSource};

#[test]
fn child_question_replay_retains_identity_and_resolution_preserves_parent_request() {
    let mut state = ClientState::default();
    state.apply_server_message(ServerMessage::SessionReady {
        session_id: "s".into(),
        cwd: None,
        model: None,
        provider: None,
        permission_mode: None,
    });
    let source = SubagentQuestionSource {
        parent_call_id: "task".into(),
        child_session_id: "child".into(),
        name: "worker".into(),
        display_name: Some("Curie".into()),
    };
    let child = ServerMessage::AskUserRequest {
        request_id: "child-q".into(),
        call_id: "ask".into(),
        questions: vec![],
        subagent: Some(source.clone()),
    };
    state.apply_server_message(child.clone());
    state.apply_server_message(child);
    state.apply_server_message(ServerMessage::AskUserRequest {
        request_id: "parent-q".into(),
        call_id: "parent-ask".into(),
        questions: vec![],
        subagent: None,
    });
    let actions = &state.session("s").unwrap().pending_actions;
    assert_eq!(actions.len(), 2);
    assert!(
        matches!(&actions[0].kind, ActionKind::AskUser { subagent: Some(actual), .. } if actual == &source)
    );
    state.apply_server_message(ServerMessage::RequestResolved {
        request_id: "child-q".into(),
    });
    let actions = &state.session("s").unwrap().pending_actions;
    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0].request_id, "parent-q");
}
