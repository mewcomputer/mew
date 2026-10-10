use mew_protocol::{decode_json, encode_json, ClientMessage, PromptStatus, ServerMessage};

#[test]
fn acknowledged_prompts_and_receipts_keep_their_session_and_request_identity() {
    for command in [
        ClientMessage::SubmitPrompt {
            session_id: "a".into(),
            request_id: "request".into(),
            text: "hello".into(),
            attachments: Vec::new(),
        },
        ClientMessage::ReadPromptReceipt {
            session_id: "a".into(),
            request_id: "request".into(),
        },
    ] {
        let value = serde_json::to_value(
            decode_json::<ClientMessage>(&encode_json(&command).unwrap()).unwrap(),
        )
        .unwrap();
        assert_eq!(value["session_id"], "a");
        assert_eq!(value["request_id"], "request");
    }
    for status in [
        PromptStatus::Accepted,
        PromptStatus::Rejected,
        PromptStatus::Unknown,
    ] {
        let reply = ServerMessage::PromptReceipt {
            session_id: "a".into(),
            request_id: "request".into(),
            status,
            message: Some("reason".into()),
        };
        let decoded = decode_json::<ServerMessage>(&encode_json(&reply).unwrap()).unwrap();
        assert!(
            matches!(decoded,ServerMessage::PromptReceipt {session_id,request_id,status:actual,message} if session_id=="a"&&request_id=="request"&&actual==status&&message.as_deref()==Some("reason"))
        );
    }
}

#[test]
fn user_echo_request_identity_is_optional_and_round_trips() {
    let legacy = decode_json::<ServerMessage>(r#"{"type":"user_message","text":"hello"}"#).unwrap();
    assert!(matches!(
        legacy,
        ServerMessage::UserMessage {
            request_id: None,
            ..
        }
    ));
    let value = serde_json::to_value(legacy).unwrap();
    assert!(value.get("request_id").is_none());
    let echo = ServerMessage::UserMessage {
        text: "hello".into(),
        request_id: Some("request".into()),
    };
    assert!(
        matches!(decode_json::<ServerMessage>(&encode_json(&echo).unwrap()).unwrap(),
        ServerMessage::UserMessage {request_id:Some(request),..} if request=="request")
    );
}
