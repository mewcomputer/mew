use mew_protocol::{ClientMessage, ServerMessage, SubagentQuestionSource};

#[test]
fn child_question_source_and_scoped_close_survive_wire_roundtrip() {
    let source = SubagentQuestionSource {
        parent_call_id: "task".into(),
        child_session_id: "child".into(),
        name: "worker".into(),
        display_name: Some("Curie".into()),
    };
    let message = ServerMessage::AskUserRequest {
        request_id: "q".into(),
        call_id: "ask".into(),
        questions: vec![],
        subagent: Some(source.clone()),
    };
    let decoded: ServerMessage =
        mew_protocol::decode_json(&mew_protocol::encode_json(&message).unwrap()).unwrap();
    assert!(
        matches!(decoded, ServerMessage::AskUserRequest { subagent: Some(actual), .. } if actual == source)
    );
    let close = ClientMessage::AskUserResponse {
        request_id: "q".into(),
        answers: vec![],
        cancelled: true,
    };
    let decoded: ClientMessage =
        mew_protocol::decode_json(&mew_protocol::encode_json(&close).unwrap()).unwrap();
    assert!(
        matches!(decoded, ClientMessage::AskUserResponse { request_id, cancelled: true, .. } if request_id == "q")
    );
}

#[test]
fn legacy_question_messages_keep_their_top_level_and_answer_semantics() {
    let request: ServerMessage = mew_protocol::decode_json(
        r#"{"type":"ask_user_request","request_id":"q","call_id":"ask","questions":[]}"#,
    )
    .unwrap();
    assert!(matches!(
        request,
        ServerMessage::AskUserRequest { subagent: None, .. }
    ));
    let response: ClientMessage = mew_protocol::decode_json(
        r#"{"type":"ask_user_response","request_id":"q","answers":["answer"]}"#,
    )
    .unwrap();
    assert!(matches!(
        response,
        ClientMessage::AskUserResponse {
            cancelled: false,
            ..
        }
    ));
}
