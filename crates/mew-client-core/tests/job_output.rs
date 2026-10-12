use mew_client_core::{ClientEvent, ClientState};
use mew_protocol::ServerMessage;

#[test]
fn job_output_preserves_chunk_identity_offsets_and_incomplete_status() {
    let mut state = ClientState::default();
    let message: ServerMessage = serde_json::from_str(
        r#"{"type":"job_output","session_id":"other-session","job_id":"build","request_id":"read-2","offset":65536,"next_offset":65550,"output":"compile\nfailed","done":true,"incomplete":true}"#,
    )
    .unwrap();
    let events = state.apply_server_message(message);
    assert!(matches!(events.as_slice(), [ClientEvent::JobOutput {
        session_id, job_id, request_id, offset, next_offset, output, done, incomplete
    }] if session_id == "other-session" && job_id == "build" && request_id == "read-2"
        && *offset == 65536 && *next_offset == 65550 && output == "compile\nfailed"
        && *done && *incomplete));
    assert!(state.attached_session.is_none());
    assert!(state.session("other-session").is_none());
}

#[test]
fn job_output_error_preserves_owner_and_request_without_changing_history() {
    let mut state = ClientState::default();
    let message: ServerMessage = serde_json::from_str(
        r#"{"type":"job_output_error","session_id":"other-session","job_id":"gone-job","request_id":"read-3","message":"job output is unavailable"}"#,
    )
    .unwrap();
    let events = state.apply_server_message(message);
    assert!(matches!(events.as_slice(), [ClientEvent::JobOutputError {
        session_id, job_id, request_id, message
    }] if session_id == "other-session" && job_id == "gone-job" && request_id == "read-3"
        && message == "job output is unavailable"));
    assert!(state.attached_session.is_none());
    assert!(state.session("other-session").is_none());
}
