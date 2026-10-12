use mew_client_core::ConnectionStatus;
use mew_tui::harness::LocalBackend;

pub fn run(gallery: &mut super::Gallery) {
    let mut backend = LocalBackend::new(128, 40);
    backend.app.daemon_mode = true;
    backend.app.status.session_id = "current".into();
    backend.app.open_sessions = vec!["current".into()];
    backend.app.short_cwd = "~/code/mew".into();
    backend.say("Inspect the complete output or review a change without losing your conversation.");
    backend.app.connection = Some(ConnectionStatus::Connected);
    backend.app.open_job("check".into());
    backend
        .app
        .handle_agent_event(mew_agent::AgentEvent::JobUpdate {
            job_id: "check".into(),
            command: "cargo test -p mew-daemon".into(),
            state: "completed".into(),
        });
    let mew_tui::events::Action::ReadJobOutput { request_id, .. } = backend
        .app
        .take_activity_request(std::time::Instant::now())
        .unwrap()
    else {
        panic!()
    };
    let output = "running 74 tests\ntest attach_restores_activity ... ok\ntest cancel_after_acceptance ... ok\ntest reconnect_keeps_pending_question ... ok\n\ntest result: ok. 74 passed; 0 failed\n";
    backend.app.apply_job_output(
        "current",
        "check",
        &request_id,
        0,
        output.len() as u64,
        output,
        true,
        false,
    );
    gallery.capture(&mut backend.app, "desktop-job-full-output", 128, 40);
    let message: mew_message::Message = serde_json::from_value(serde_json::json!({
        "id":"01ARZ3NDEKTSV4RRFFQ69G5FA1","session_id":"01ARZ3NDEKTSV4RRFFQ69G5FAV","role":"assistant","time":{"created":0},
        "parts":[{"type":"tool_call","id":"01ARZ3NDEKTSV4RRFFQ69G5FA2","message_id":"01ARZ3NDEKTSV4RRFFQ69G5FA1","session_id":"01ARZ3NDEKTSV4RRFFQ69G5FAV","tool_name":"bash","call_id":"verify-session","state":{"status":"completed","input":{"command":"cargo test -p mew-daemon","cwd":"/Users/natalie/code/mew"},"output":output,"time":{"start":0,"end":1}}}]
    })).unwrap();
    let id = message.id;
    let part = message.parts[0].id();
    backend.app.push_message(message);
    backend.app.open_tool(id, part);
    gallery.capture(&mut backend.app, "desktop-tool-full-output", 128, 40);
    backend.app.open_review();
    let mew_tui::events::Action::ReadSessionDiff { request_id, .. } = backend
        .app
        .take_activity_request(std::time::Instant::now())
        .unwrap()
    else {
        panic!()
    };
    let files: Vec<mew_diff::FileDiff> = serde_json::from_value(serde_json::json!([{ "path":"src/session.rs","old_path":null,"status":"Modified","added":2,"removed":1,"hunks":[{"old_start":24,"old_count":2,"new_start":24,"new_count":3,"lines":[{"kind":"Context","old_line":24,"new_line":24,"text":"let session = manager.get(session_id)?;"},{"kind":"Deletion","old_line":25,"new_line":null,"text":"session.start_turn(prompt).await;"},{"kind":"Addition","old_line":null,"new_line":25,"text":"let cancellation = session.register_turn()?;"},{"kind":"Addition","old_line":null,"new_line":26,"text":"session.start_turn(prompt, cancellation).await;"}]}]}])).unwrap();
    backend
        .app
        .apply_session_diff("current", &request_id, &files, None);
    gallery.capture(&mut backend.app, "desktop-session-review", 128, 40);
    backend.app.activity_view = None;
    let (tx, _rx) = tokio::sync::oneshot::channel();
    backend.app.handle_daemon_required_action(
        "child-question".into(),
        mew_agent::AgentEvent::SubagentAskUser {
            source: mew_message::SubagentQuestionSource {
                parent_call_id: "task".into(),
                child_session_id: "child".into(),
                name: "reviewer".into(),
                display_name: Some("Curie".into()),
            },
            call_id: "ask".into(),
            questions: vec![mew_message::AskUserQuestion {
                prompt: "which browser lifecycle should i verify first?".into(),
                options: vec![
                    mew_message::QuestionOption {
                        label: "reconnect".into(),
                        description: "keep drafts and pending questions while the daemon restarts"
                            .into(),
                    },
                    mew_message::QuestionOption {
                        label: "session switching".into(),
                        description: "verify browser pages stay owned by their conversation".into(),
                    },
                ],
            }],
            tx,
        },
    );
    gallery.capture(&mut backend.app, "desktop-child-question", 128, 40);
    backend.app.user_question = None;
    backend.app.mode = mew_tui::app::Mode::Help;
    backend.app.help_scroll = 0;
    gallery.capture(&mut backend.app, "desktop-readline-help", 128, 40);
    backend.app.help_scroll = 10;
    gallery.capture(&mut backend.app, "desktop-readline-editing", 128, 40);
    backend.app.help_scroll = 22;
    gallery.capture(&mut backend.app, "short-readline-editing", 40, 24);
}
