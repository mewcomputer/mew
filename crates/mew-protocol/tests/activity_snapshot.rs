use mew_protocol::{decode_json, encode_json, JobActivity, ServerMessage, SubagentActivity};

#[test]
fn authoritative_activity_round_trips_complete_identity_and_empty_replacement() {
    let message = ServerMessage::SessionActivitySnapshot {
        session_id: "session".into(),
        subagents: vec![SubagentActivity {
            task_id: "task".into(),
            child_session_id: Some("child".into()),
            name: "worker".into(),
            display_name: Some("worker one".into()),
            status: "checking".into(),
            tool_name: Some("shell".into()),
        }],
        jobs: vec![JobActivity {
            job_id: "job".into(),
            command: "sleep 1".into(),
            state: "failed".into(),
        }],
    };
    let decoded = decode_json::<ServerMessage>(&encode_json(&message).unwrap()).unwrap();
    assert_eq!(
        serde_json::to_value(message).unwrap(),
        serde_json::to_value(decoded).unwrap()
    );
    let empty = ServerMessage::SessionActivitySnapshot {
        session_id: "session".into(),
        subagents: Vec::new(),
        jobs: Vec::new(),
    };
    assert!(
        matches!(decode_json::<ServerMessage>(&encode_json(&empty).unwrap()).unwrap(),ServerMessage::SessionActivitySnapshot {session_id,subagents,jobs} if session_id=="session"&&subagents.is_empty()&&jobs.is_empty())
    );
}
