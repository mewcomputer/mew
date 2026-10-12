use mew_tui::harness::Harness;
use std::time::{Duration, Instant};

fn trace_messages() -> Vec<mew_message::Message> {
    serde_json::from_value(serde_json::json!([
        {
            "id": "01ARZ3NDEKTSV4RRFFQ69G5FA1",
            "session_id": "01ARZ3NDEKTSV4RRFFQ69G5FAV",
            "role": "user", "time": {"created": 0},
            "parts": [{"type": "text", "id": "01ARZ3NDEKTSV4RRFFQ69G5FA2", "message_id": "01ARZ3NDEKTSV4RRFFQ69G5FA1", "session_id": "01ARZ3NDEKTSV4RRFFQ69G5FAV", "text": "check the attachment lifecycle"}]
        },
        {
            "id": "01ARZ3NDEKTSV4RRFFQ69G5FA3",
            "session_id": "01ARZ3NDEKTSV4RRFFQ69G5FAV",
            "role": "assistant", "time": {"created": 1},
            "parts": [
                {"type": "text", "id": "01ARZ3NDEKTSV4RRFFQ69G5FA4", "message_id": "01ARZ3NDEKTSV4RRFFQ69G5FA3", "session_id": "01ARZ3NDEKTSV4RRFFQ69G5FAV", "text": "checking the session owner"},
                {"type": "tool_call", "id": "01ARZ3NDEKTSV4RRFFQ69G5FA5", "message_id": "01ARZ3NDEKTSV4RRFFQ69G5FA3", "session_id": "01ARZ3NDEKTSV4RRFFQ69G5FAV", "tool_name": "read", "call_id": "read-owner", "state": {"status": "completed", "input": {"path": "session.rs"}, "output": "owner is retained", "time": {"start": 1, "end": 2}}},
                {"type": "tool_call", "id": "01ARZ3NDEKTSV4RRFFQ69G5FA6", "message_id": "01ARZ3NDEKTSV4RRFFQ69G5FA3", "session_id": "01ARZ3NDEKTSV4RRFFQ69G5FAV", "tool_name": "bash", "call_id": "test-owner", "state": {"status": "error", "input": {"command": "cargo test"}, "error": "fixture unavailable", "time": {"start": 3, "end": 4}}}
            ]
        }
    ])).unwrap()
}

pub fn run(gallery: &mut super::Gallery) {
    let mut h = Harness::new(128, 40);
    h.say("The task is running. Click a todo, agent, or background job to inspect it.");
    h.app.streaming = true;
    h.app.status.session_id = "capturetask".into();
    h.app
        .session_titles
        .insert("capturetask".into(), "Improve TUI task flow".into());
    h.app
        .handle_agent_event(mew_agent::AgentEvent::SubagentStart {
            parent_call_id: "review".into(),
            name: "reviewer".into(),
            child_session_id: "01ARZ3NDEKTSV4RRFFQ69G5FAV".into(),
            display_name: Some("Curie".into()),
        });
    h.app.subagents[0].started_at = Instant::now() - Duration::from_secs(127);
    h.app.open_subagent("review".into());
    let mew_tui::events::Action::ReadSubagentHistory {
        session_id,
        child_session_id,
        request_id,
    } = h.app.take_activity_request(Instant::now()).unwrap()
    else {
        unreachable!()
    };
    h.app
        .apply_daemon_notification(&mew_protocol::ServerMessage::SubagentHistory {
            session_id,
            child_session_id,
            request_id,
            messages: trace_messages(),
            error: None,
        });
    gallery.capture(&mut h.app, "desktop-subagent-120", 128, 40);
    gallery.capture(&mut h.app, "desktop-subagent-40", 40, 24);
    h.app.activity_view.as_mut().unwrap().scroll = 0;
    gallery.capture(&mut h.app, "desktop-subagent-30", 30, 14);
    h.app.activity_view.as_mut().unwrap().scroll = usize::MAX;
    gallery.capture(&mut h.app, "desktop-subagent-30-end", 30, 14);
    h.app.close_activity();
    h.app.handle_agent_event(mew_agent::AgentEvent::JobUpdate {
        job_id: "tests".into(),
        command:
            "cargo test --workspace --all-features -- --nocapture activity_flow long-command-tail"
                .into(),
        state: "running".into(),
    });
    h.app.background_jobs[0].started_at = Instant::now() - Duration::from_secs(42);
    h.app.open_job("tests".into());
    gallery.capture(&mut h.app, "desktop-job-120", 128, 40);
}
