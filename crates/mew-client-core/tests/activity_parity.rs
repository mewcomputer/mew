use mew_client_core::{ClientEvent, ClientSession, ClientState, ProviderRetry, SessionEvent};
use mew_message::{ErrorKind, MessageError, ProviderEventWire};
use mew_protocol::{ServerMessage, SessionState};

fn attach(state: &mut ClientState, id: &str) {
    state.apply_server_message(ServerMessage::SessionReady {
        session_id: id.into(),
        cwd: None,
        model: None,
        provider: None,
        permission_mode: None,
    });
}

fn job(state: &mut ClientState, command: &str, status: &str) -> Vec<ClientEvent> {
    state.apply_server_message(ServerMessage::JobUpdate {
        job_id: "job-1".into(),
        command: command.into(),
        state: status.into(),
    })
}

fn retry_event() -> ProviderEventWire {
    ProviderEventWire::RetryWait {
        attempt: 2,
        max_attempts: 5,
        delay_secs: 17,
        reason: "overloaded".into(),
    }
}

fn expected_retry() -> ProviderRetry {
    ProviderRetry {
        attempt: 2,
        max_attempts: 5,
        delay_secs: 17,
        reason: "overloaded".into(),
    }
}

#[test]
fn job_updates_without_an_attached_session_do_not_invent_ownership() {
    let mut state = ClientState::default();
    assert!(job(&mut state, "build", "running").is_empty());
    assert!(state.sessions.is_empty());
}

#[test]
fn job_updates_preserve_payload_and_upsert_within_their_session() {
    let mut state = ClientState::default();
    attach(&mut state, "a");
    let events = job(&mut state, "build", "running");
    assert!(
        matches!(events.as_slice(), [ClientEvent::JobUpdated { session_id, job }]
        if session_id == "a" && job.job_id == "job-1" && job.command == "build" && job.state == "running")
    );
    job(&mut state, "new build", "failed");
    job(&mut state, "restart", "running");
    let jobs = &state.session("a").unwrap().jobs;
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].command, "restart");
    assert_eq!(jobs[0].state, "running");
    attach(&mut state, "b");
    job(&mut state, "test", "completed");
    assert_eq!(state.session("b").unwrap().jobs[0].state, "completed");
    attach(&mut state, "a");
    assert_eq!(state.session("a").unwrap().jobs[0].command, "restart");
}

#[test]
fn retry_wait_is_a_session_owned_status_event_instead_of_an_error() {
    let mut state = ClientState::default();
    attach(&mut state, "a");
    let events = state.apply_server_message(ServerMessage::Provider {
        event: retry_event(),
    });
    assert!(
        matches!(events.as_slice(), [ClientEvent::RetryWait { session_id, retry }]
        if session_id == "a" && retry == &expected_retry())
    );
    assert_eq!(state.session("a").unwrap().retry, Some(expected_retry()));
    assert!(state.session_turn_outcomes.is_empty());
    attach(&mut state, "b");
    assert!(state.session("b").unwrap().retry.is_none());
    assert_eq!(state.session("a").unwrap().retry, Some(expected_retry()));
}

#[test]
fn provider_resumption_and_errors_clear_only_their_session_retry() {
    let mut state = ClientState::default();
    attach(&mut state, "a");
    state.apply_server_message(ServerMessage::Provider {
        event: retry_event(),
    });
    attach(&mut state, "b");
    state.apply_server_message(ServerMessage::Provider {
        event: retry_event(),
    });
    state.apply_server_message(ServerMessage::Provider {
        event: ProviderEventWire::PartEnd {
            part_id: ulid::Ulid::new(),
        },
    });
    assert!(state.session("b").unwrap().retry.is_none());
    assert_eq!(state.session("a").unwrap().retry, Some(expected_retry()));
    attach(&mut state, "a");
    let events = state.apply_server_message(ServerMessage::Provider {
        event: ProviderEventWire::Error(MessageError {
            kind: ErrorKind::ProviderApi,
            message: "exhausted".into(),
        }),
    });
    assert!(state.session("a").unwrap().retry.is_none());
    assert!(
        matches!(events.as_slice(), [ClientEvent::TurnError { session_id, message }] if session_id == "a" && message == "exhausted")
    );
}

#[test]
fn idle_and_terminal_daemon_errors_clear_retry_without_clearing_jobs() {
    let mut state = ClientState::default();
    attach(&mut state, "a");
    job(&mut state, "build", "running");
    state.apply_server_message(ServerMessage::Provider {
        event: retry_event(),
    });
    state.apply_server_message(ServerMessage::SessionActivityChanged {
        session_id: "a".into(),
        activity: SessionState::Active,
    });
    assert!(state.session("a").unwrap().retry.is_none());
    state.apply_server_message(ServerMessage::Provider {
        event: retry_event(),
    });
    state.apply_server_message(ServerMessage::ErrorEvent {
        message: "failed".into(),
    });
    assert!(state.session("a").unwrap().retry.is_none());
    assert_eq!(state.session("a").unwrap().jobs[0].state, "running");
}

#[test]
fn session_provider_reducer_preserves_retry_status_payload() {
    let mut session = ClientSession::new("a".into());
    let events = session.apply_provider_event(retry_event());
    assert!(
        matches!(events.as_slice(), [SessionEvent::RetryWait(retry)] if retry == &expected_retry())
    );
    assert_eq!(session.retry, Some(expected_retry()));
}

#[test]
fn metadata_snapshots_do_not_clone_session_activity() {
    let mut state = ClientState::default();
    attach(&mut state, "a");
    job(&mut state, "build", "running");
    state.apply_server_message(ServerMessage::Provider {
        event: retry_event(),
    });
    assert!(state.ui_metadata_snapshot().sessions.is_empty());
    let snapshot = state.ui_snapshot();
    assert_eq!(snapshot.session("a").unwrap().jobs[0].command, "build");
    assert_eq!(snapshot.session("a").unwrap().retry, Some(expected_retry()));
}

#[test]
fn attached_metadata_snapshot_preserves_activity_and_omits_history() {
    let mut state = ClientState::default();
    attach(&mut state, "a");
    state.record_prompt("a", "hello".into());
    job(&mut state, "build", "running");
    let snapshot = state.session("a").unwrap().ui_metadata_snapshot();
    assert!(snapshot.messages.is_empty());
    assert_eq!(snapshot.jobs[0].command, "build");
    assert_eq!(snapshot.session_id, "a");
}

#[test]
fn todo_save_replies_keep_request_and_session_identity() {
    let mut state = ClientState::default();
    attach(&mut state, "a");
    let events = state.apply_server_message(ServerMessage::TodoUpdated {
        session_id: "b".into(),
        request_id: "edit-1".into(),
        todos: vec![],
    });
    assert!(
        matches!(events.as_slice(), [ClientEvent::TodoUpdated {session_id,request_id,todos}]
        if session_id=="b" && request_id=="edit-1" && todos.is_empty())
    );
    let events = state.apply_server_message(ServerMessage::TodoUpdateError {
        session_id: "b".into(),
        request_id: "edit-1".into(),
        message: "conflict".into(),
    });
    assert!(
        matches!(events.as_slice(), [ClientEvent::TodoUpdateError {session_id,request_id,message}]
        if session_id=="b" && request_id=="edit-1" && message=="conflict")
    );
}

#[test]
fn subagent_history_responses_keep_owner_request_and_child_identity() {
    let mut state = ClientState::default();
    let events = state.apply_server_message(ServerMessage::SubagentHistory {
        session_id: "parent".into(),
        child_session_id: "child".into(),
        request_id: "trace-1".into(),
        messages: vec![],
        error: Some("gone".into()),
    });
    assert!(
        matches!(events.as_slice(), [ClientEvent::SubagentHistory {session_id,child_session_id,request_id,messages,error}]
        if session_id=="parent" && child_session_id=="child" && request_id=="trace-1" && messages.is_empty() && error.as_deref()==Some("gone"))
    );
    assert!(state.sessions.is_empty());
}

#[test]
fn prompt_receipts_keep_their_exact_request_owner_and_status() {
    let mut state = ClientState::default();
    let events = state.apply_server_message(ServerMessage::PromptReceipt {
        session_id: "a".into(),
        request_id: "prompt-1".into(),
        status: mew_protocol::PromptStatus::Unknown,
        message: Some("uncertain".into()),
    });
    assert!(
        matches!(events.as_slice(), [ClientEvent::PromptReceipt {session_id,request_id,status:mew_protocol::PromptStatus::Unknown,message}]
        if session_id=="a" && request_id=="prompt-1" && message.as_deref()==Some("uncertain"))
    );
    assert!(state.sessions.is_empty());
}

#[test]
fn subagent_events_preserve_child_and_terminal_payloads() {
    let mut state = ClientState::default();
    attach(&mut state, "a");
    let events = state.apply_server_message(ServerMessage::SubagentStart {
        parent_call_id: "task".into(),
        child_session_id: "child".into(),
        name: "researcher".into(),
        display_name: Some("Curie".into()),
    });
    assert!(
        matches!(events.last(),Some(ClientEvent::SubagentStarted{session_id,parent_call_id,child_session_id,name,display_name})
        if session_id=="a" && parent_call_id=="task" && child_session_id=="child" && name=="researcher" && display_name.as_deref()==Some("Curie"))
    );
    let events = state.apply_server_message(ServerMessage::SubagentEnd {
        parent_call_id: "task".into(),
        child_session_id: "child".into(),
        outcome: mew_protocol::SubagentOutcome::Failed {
            reason: "timeout".into(),
        },
        manifests: vec![],
    });
    assert!(
        matches!(events.last(),Some(ClientEvent::SubagentEnded{session_id,parent_call_id,child_session_id,outcome:mew_protocol::SubagentOutcome::Failed{reason},manifests})
        if session_id=="a" && parent_call_id=="task" && child_session_id=="child" && reason=="timeout" && manifests.is_empty())
    );
}

#[test]
fn old_child_completion_does_not_remove_a_restarted_subagent() {
    let mut state = ClientState::default();
    attach(&mut state, "a");
    for child in ["old", "new"] {
        state.apply_server_message(ServerMessage::SubagentStart {
            parent_call_id: "task".into(),
            child_session_id: child.into(),
            name: "researcher".into(),
            display_name: None,
        });
    }
    state.apply_server_message(ServerMessage::SubagentEnd {
        parent_call_id: "task".into(),
        child_session_id: "old".into(),
        outcome: mew_protocol::SubagentOutcome::Completed,
        manifests: vec![],
    });
    assert_eq!(state.session("a").unwrap().subagents.len(), 1);
}

fn session_info(id: &str, created: i64, last: Option<i64>) -> mew_protocol::SessionInfo {
    serde_json::from_value(serde_json::json!({
        "session_id":id,"state":"idle","created_at":created,"last_message_at":last,
        "client_count":0,"model":null,"provider":null,"summary":null
    }))
    .unwrap()
}

#[test]
fn session_lists_are_latest_active_first_with_stable_ties() {
    let mut state = ClientState::default();
    state.apply_server_message(ServerMessage::SessionList {
        sessions: vec![
            session_info("old", 100, Some(10)),
            session_info("z", 0, Some(30)),
            session_info("new", 40, None),
            session_info("a", 0, Some(30)),
        ],
    });
    assert_eq!(
        state
            .session_list
            .iter()
            .map(|session| session.session_id.as_str())
            .collect::<Vec<_>>(),
        vec!["new", "a", "z", "old"]
    );
}

#[test]
fn attention_arriving_before_a_list_and_its_resolution_survive_list_replay() {
    let mut state = ClientState::default();
    state.apply_server_message(ServerMessage::SessionAttentionChanged {
        session_id: "a".into(),
        pending_permissions: 2,
        pending_questions: 1,
    });
    state.apply_server_message(ServerMessage::SessionList {
        sessions: vec![session_info("a", 0, None)],
    });
    assert_eq!(
        (
            state.session_list[0].pending_permissions,
            state.session_list[0].pending_questions
        ),
        (2, 1)
    );
    state.apply_server_message(ServerMessage::SessionAttentionChanged {
        session_id: "a".into(),
        pending_permissions: 0,
        pending_questions: 0,
    });
    let mut stale = session_info("a", 0, None);
    stale.pending_permissions = 2;
    state.apply_server_message(ServerMessage::SessionList {
        sessions: vec![stale],
    });
    assert_eq!(state.session_list[0].pending_permissions, 0);
}

#[test]
fn background_failure_is_unread_until_its_owner_is_attached() {
    let mut state = ClientState::default();
    attach(&mut state, "a");
    state.apply_server_message(ServerMessage::SessionAlert {
        session_id: "b".into(),
        kind: mew_protocol::AlertKind::TurnFailed,
        detail: Some("timeout".into()),
        title: "failed".into(),
    });
    assert!(state.unread_completions.contains("b"));
    attach(&mut state, "b");
    assert!(!state.unread_completions.contains("b"));
    assert!(
        matches!(state.session_turn_outcomes.get("b"),Some(mew_client_core::TurnOutcome::Failed{detail}) if detail.as_deref()==Some("timeout"))
    );
}

#[test]
fn identical_authoritative_user_messages_are_not_mistaken_for_optimistic_echoes() {
    let mut state = ClientState::default();
    attach(&mut state, "a");
    for _ in 0..2 {
        state.apply_server_message(ServerMessage::UserMessage {
            text: "same".into(),
            request_id: None,
        });
    }
    assert_eq!(state.session("a").unwrap().messages.len(), 2);
}

#[tokio::test]
async fn failed_prompt_send_does_not_add_an_optimistic_message() {
    let transport = mew_client_core::InMemoryTransport::default();
    transport.push_server_message(ServerMessage::Pong {
        version: "test".into(),
        protocol_version: mew_protocol::PROTOCOL_VERSION,
    });
    let mut engine = mew_client_core::ClientEngine::connect(&transport)
        .await
        .unwrap();
    attach(engine.state_mut(), "a");
    engine.close().await.unwrap();
    assert!(engine.send_prompt("hello".into()).await.is_err());
    assert!(engine.state().session("a").unwrap().messages.is_empty());
}

#[test]
fn part_end_preserves_the_rendering_boundary() {
    let mut state = ClientState::default();
    attach(&mut state, "a");
    let part_id = ulid::Ulid::new();
    let events = state.apply_server_message(ServerMessage::Provider {
        event: ProviderEventWire::PartEnd { part_id },
    });
    assert!(
        matches!(events.as_slice(),[ClientEvent::ProviderPartEnded {session_id,part_id:ended}] if session_id=="a" && ended==&part_id)
    );
}

#[test]
fn part_updates_preserve_the_exact_older_part_payload() {
    let mut state = ClientState::default();
    attach(&mut state, "a");
    state.record_prompt("a", "older".into());
    state.record_prompt("a", "latest".into());
    let mut part = state.session("a").unwrap().messages[0].parts[0].clone();
    if let mew_message::Part::Text(text) = &mut part {
        text.text = "changed older".into();
    }
    let part_id = part.id();
    let events = state.apply_server_message(ServerMessage::PartUpdated {
        part_id,
        part: part.clone(),
    });
    assert!(
        matches!(events.last(),Some(ClientEvent::PartUpdated{session_id,part_id:changed,part:updated}) if session_id=="a" && changed==&part_id && updated.as_ref()==&part)
    );
    assert_eq!(state.session("a").unwrap().messages[0].parts[0], part);
}

#[test]
fn canonical_clear_has_an_explicit_session_owned_display_event() {
    let mut state = ClientState::default();
    attach(&mut state, "a");
    state.record_prompt("a", "hello".into());
    let events = state.apply_server_message(ServerMessage::SessionCleared);
    assert!(
        matches!(events.last(),Some(ClientEvent::SessionCleared {session_id}) if session_id=="a")
    );
    assert!(state.session("a").unwrap().messages.is_empty());
}

#[test]
fn identical_prompt_echoes_preserve_distinct_submission_provenance() {
    let mut state = ClientState::default();
    attach(&mut state, "a");
    let mut ids = Vec::new();
    for request in ["first", "second"] {
        let events = state.apply_server_message(ServerMessage::UserMessage {
            text: "same prompt".into(),
            request_id: Some(request.into()),
        });
        let message = state.session("a").unwrap().messages.last().unwrap();
        assert!(events.iter().any(|event| matches!(event,
            ClientEvent::UserMessage {session_id,request_id,message_id}
                if session_id=="a" && request_id.as_deref()==Some(request) && *message_id==message.id)));
        ids.push(message.id);
    }
    assert_ne!(ids[0], ids[1]);
    assert_eq!(state.session("a").unwrap().messages.len(), 2);
}

#[test]
fn activity_snapshot_replaces_only_its_session_and_clears_stale_activity() {
    let mut state = ClientState::default();
    attach(&mut state, "a");
    job(&mut state, "old command", "running");
    attach(&mut state, "b");
    job(&mut state, "other command", "running");
    let events = state.apply_server_message(ServerMessage::SessionActivitySnapshot {
        session_id: "a".into(),
        subagents: vec![mew_protocol::SubagentActivity {
            task_id: "task".into(),
            child_session_id: Some("child".into()),
            name: "worker".into(),
            display_name: Some("worker one".into()),
            tool_name: Some("shell".into()),
            status: "checking".into(),
        }],
        jobs: vec![mew_protocol::JobActivity {
            job_id: "job-1".into(),
            command: "old command".into(),
            state: "completed".into(),
        }],
    });
    assert!(
        matches!(events.as_slice(),[ClientEvent::ActivitySnapshot {session_id}] if session_id=="a")
    );
    assert_eq!(state.session("a").unwrap().subagents[0].status, "checking");
    assert_eq!(state.session("a").unwrap().jobs[0].state, "completed");
    assert_eq!(state.session("b").unwrap().jobs[0].state, "running");
    state.apply_server_message(ServerMessage::SessionActivitySnapshot {
        session_id: "a".into(),
        subagents: Vec::new(),
        jobs: Vec::new(),
    });
    assert!(state.session("a").unwrap().jobs.is_empty());
    assert!(state.session("a").unwrap().subagents.is_empty());
}

#[tokio::test]
async fn session_refreshes_coalesce_until_exact_list_reply_and_follow_up_once() {
    use mew_client_core::{ClientEngine, InMemoryTransport};
    use mew_protocol::ClientMessage;
    let transport = InMemoryTransport::default();
    transport.push_server_message(ServerMessage::Pong {
        version: "test".into(),
        protocol_version: mew_protocol::PROTOCOL_VERSION,
    });
    let mut engine = ClientEngine::connect(&transport).await.unwrap();
    transport.push_server_message(ServerMessage::SessionReady {
        session_id: "a".into(),
        cwd: None,
        model: None,
        provider: None,
        permission_mode: None,
    });
    engine.receive().await.unwrap();
    let list_count = || {
        transport
            .sent_messages()
            .iter()
            .filter(|message| matches!(message, ClientMessage::ListSessions))
            .count()
    };
    assert_eq!(list_count(), 1);
    assert!(transport
        .sent_messages()
        .iter()
        .any(|message| matches!(message, ClientMessage::ListPersonas)));
    for message in [
        ServerMessage::SessionTitleChanged {
            session_id: "a".into(),
            title: "renamed".into(),
        },
        ServerMessage::SessionAttentionChanged {
            session_id: "a".into(),
            pending_permissions: 1,
            pending_questions: 0,
        },
        ServerMessage::SessionActivityChanged {
            session_id: "a".into(),
            activity: SessionState::Running,
        },
        ServerMessage::SessionActivityChanged {
            session_id: "a".into(),
            activity: SessionState::Idle,
        },
        ServerMessage::SessionMetaChanged {
            session_id: "a".into(),
            archived: true,
            pinned: false,
            group_id: None,
        },
    ] {
        transport.push_server_message(message);
        engine.receive().await.unwrap();
        assert_eq!(list_count(), 1);
    }
    for expected in [2, 2] {
        transport.push_server_message(ServerMessage::SessionList {
            sessions: vec![session_info("a", 1, Some(2))],
        });
        engine.receive().await.unwrap();
        assert_eq!(list_count(), expected);
    }
    transport.push_server_message(ServerMessage::SessionActivityChanged {
        session_id: "a".into(),
        activity: SessionState::Idle,
    });
    engine.receive().await.unwrap();
    assert_eq!(list_count(), 2, "duplicate idle must not refresh again");
    engine.send(ClientMessage::ListSessions).await.unwrap();
    engine.send(ClientMessage::ListSessions).await.unwrap();
    assert_eq!(list_count(), 3, "explicit repeated refreshes coalesce too");
    transport.push_server_message(ServerMessage::SessionList {
        sessions: vec![session_info("a", 1, Some(2))],
    });
    engine.receive().await.unwrap();
    assert_eq!(list_count(), 4);
}
