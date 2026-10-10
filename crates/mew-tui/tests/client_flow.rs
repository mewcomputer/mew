use mew_client_core::{ClientEvent, ClientState, ConnectionStatus};
use mew_message::{Part, PartBase, ProviderEventWire, TextPart};
use mew_protocol::{ServerMessage, SessionState};
use mew_tui::App;

fn apply(app: &mut App, state: &mut ClientState, message: ServerMessage) {
    let events = state.apply_server_message(message);
    let changed = state
        .attached_session
        .as_deref()
        .and_then(|id| state.session(id))
        .and_then(|session| session.messages.last());
    app.apply_client_update(&events, state, changed);
}

fn ready(id: &str, cwd: &str) -> ServerMessage {
    ServerMessage::SessionReady {
        session_id: id.into(),
        cwd: Some(cwd.into()),
        model: None,
        provider: None,
        permission_mode: None,
    }
}

#[test]
fn passive_turn_streams_and_finishes_without_a_local_prompt_receiver() {
    let mut app = App::new();
    app.daemon_mode = true;
    let mut state = ClientState::default();
    apply(&mut app, &mut state, ready("a", "/tmp/a"));
    apply(
        &mut app,
        &mut state,
        ServerMessage::UserMessage {
            text: "from desktop".into(),
            request_id: None,
        },
    );
    apply(
        &mut app,
        &mut state,
        ServerMessage::SessionActivityChanged {
            session_id: "a".into(),
            activity: SessionState::Running,
        },
    );
    let part_id = ulid::Ulid::new();
    apply(
        &mut app,
        &mut state,
        ServerMessage::Provider {
            event: ProviderEventWire::PartStart {
                part: Part::Text(TextPart {
                    base: PartBase {
                        id: part_id,
                        message_id: ulid::Ulid::new(),
                        session_id: ulid::Ulid::new(),
                    },
                    text: String::new(),
                    synthetic: false,
                }),
            },
        },
    );
    apply(
        &mut app,
        &mut state,
        ServerMessage::Provider {
            event: ProviderEventWire::PartDelta {
                part_id,
                field: "text".into(),
                delta: "visible reply".into(),
            },
        },
    );
    assert!(app.streaming);
    assert_eq!(app.messages().len(), 2);
    assert!(
        matches!(&app.messages()[1].parts[0], Part::Text(text) if text.text == "visible reply")
    );
    app.queued_messages.push("follow-up".into());
    apply(
        &mut app,
        &mut state,
        ServerMessage::SessionActivityChanged {
            session_id: "a".into(),
            activity: SessionState::Idle,
        },
    );
    assert!(!app.streaming);
    assert!(app.pending_queued_send);
}

#[test]
fn confirmed_project_changes_replace_local_discovery_and_file_catalog() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    std::fs::write(a.path().join("only-a.txt"), "a").unwrap();
    std::fs::write(b.path().join("only-b.txt"), "b").unwrap();
    let mut app = App::new();
    app.daemon_mode = true;
    let mut state = ClientState::default();
    apply(&mut app, &mut state, ready("a", a.path().to_str().unwrap()));
    app.open_at_picker();
    assert!(app.at_file_items.iter().any(|item| item.id == "only-a.txt"));
    apply(&mut app, &mut state, ready("b", b.path().to_str().unwrap()));
    app.open_at_picker();
    assert_eq!(app.project_cwd(), b.path());
    assert!(app.at_file_items.iter().any(|item| item.id == "only-b.txt"));
    assert!(!app.at_file_items.iter().any(|item| item.id == "only-a.txt"));
}

#[test]
fn confirmed_worktree_displays_its_own_branch() {
    let project = tempfile::tempdir().unwrap();
    let metadata = tempfile::tempdir().unwrap();
    std::fs::write(
        metadata.path().join("HEAD"),
        "ref: refs/heads/worktree-branch\n",
    )
    .unwrap();
    std::fs::write(
        project.path().join(".git"),
        format!("gitdir: {}\n", metadata.path().display()),
    )
    .unwrap();
    let mut app = App::new();
    let mut state = ClientState::default();
    apply(
        &mut app,
        &mut state,
        ready("worktree", project.path().to_str().unwrap()),
    );
    assert_eq!(app.git_branch.as_deref(), Some("worktree-branch"));
}

#[test]
fn disconnect_preserves_the_draft_and_does_not_release_the_queue() {
    let mut app = App::new();
    app.daemon_mode = true;
    let mut state = ClientState::default();
    apply(&mut app, &mut state, ready("a", "/tmp/a"));
    app.input = "draft".into();
    app.streaming = true;
    app.queued_messages.push("next".into());
    app.apply_client_update(
        &[ClientEvent::ConnectionChanged(ConnectionStatus::Backoff {
            attempt: 1,
            error: "closed".into(),
        })],
        &state,
        None,
    );
    assert!(!app.client_connected());
    assert_eq!(app.input, "draft");
    assert!(app.streaming);
    assert!(!app.pending_queued_send);
}

#[test]
fn returning_to_an_idle_conversation_releases_its_queue_despite_a_stale_list() {
    let mut app = App::new();
    app.daemon_mode = true;
    let mut state = ClientState::default();
    apply(&mut app, &mut state, ready("a", "/tmp/a"));
    app.queued_messages.push("a follow-up".into());
    apply(&mut app, &mut state, ready("b", "/tmp/b"));
    apply(
        &mut app,
        &mut state,
        ServerMessage::SessionActivityChanged {
            session_id: "a".into(),
            activity: SessionState::Idle,
        },
    );
    apply(&mut app, &mut state, ready("a", "/tmp/a"));
    assert!(!app.streaming);
    assert!(app.pending_queued_send);
}

#[test]
fn prepending_history_preserves_manual_scroll_and_auto_scroll_choice() {
    let mut app = App::new();
    app.daemon_mode = true;
    let mut state = ClientState::default();
    apply(&mut app, &mut state, ready("a", "/tmp/a"));
    state.record_prompt("a", "older".into());
    state.record_prompt("a", "newer".into());
    let messages = state.session("a").unwrap().messages.clone();
    apply(
        &mut app,
        &mut state,
        ServerMessage::SessionHistoryPage {
            session_id: "a".into(),
            messages: vec![messages[1].clone()],
            next_cursor: Some(1),
            replace: true,
        },
    );
    app.auto_scroll = false;
    apply(
        &mut app,
        &mut state,
        ServerMessage::SessionHistoryPage {
            session_id: "a".into(),
            messages: vec![messages[0].clone()],
            next_cursor: None,
            replace: false,
        },
    );
    assert!(!app.auto_scroll);
    assert_eq!(app.messages().len(), 2);
    assert_eq!(app.messages()[0].id, messages[0].id);
}

#[test]
fn policy_updates_without_display_events_refresh_the_active_controls() {
    let mut app = App::new();
    let mut state = ClientState::default();
    apply(&mut app, &mut state, ready("a", "/tmp/a"));
    apply(
        &mut app,
        &mut state,
        ServerMessage::ModelSwitched {
            provider: "provider".into(),
            model: "new-model".into(),
        },
    );
    assert_eq!(app.status.provider, "provider");
    assert_eq!(app.status.model, "new-model");
    apply(
        &mut app,
        &mut state,
        ServerMessage::ThinkingVariantChanged {
            variant: Some("high".into()),
        },
    );
    assert_eq!(app.active_thinking_variant.as_deref(), Some("high"));
    apply(
        &mut app,
        &mut state,
        ServerMessage::PersonaList {
            personas: vec![mew_protocol::PersonaInfo {
                name: "builder".into(),
                description: "build things".into(),
                color: Some("blue".into()),
                active: true,
            }],
        },
    );
    assert_eq!(app.active_persona.as_deref(), Some("builder"));
    assert_eq!(app.active_persona_color.as_deref(), Some("blue"));
    assert_eq!(app.personas, [("builder".into(), "build things".into())]);
    apply(
        &mut app,
        &mut state,
        ServerMessage::ThinkingVariantChanged { variant: None },
    );
    apply(
        &mut app,
        &mut state,
        ServerMessage::PersonaList { personas: vec![] },
    );
    assert!(app.active_thinking_variant.is_none());
    assert!(app.active_persona.is_none());
    assert!(app.active_persona_color.is_none());
}

fn project(path: &str) -> mew_protocol::ProjectInfo {
    mew_protocol::ProjectInfo {
        path: path.into(),
        display_name: path.into(),
        session_count: 1,
        last_used_at: None,
        pinned: false,
    }
}

#[test]
fn background_project_refresh_preserves_navigation_and_editor() {
    let mut app = App::new();
    let mut state = ClientState::default();
    apply(&mut app, &mut state, ready("a", "/tmp/a"));
    app.open_attention_picker();
    app.input = "draft".into();
    apply(
        &mut app,
        &mut state,
        ServerMessage::ProjectList {
            projects: vec![project("/tmp/a")],
        },
    );
    assert_eq!(app.picker.as_ref().unwrap().kind, "session_attention");
    assert_eq!(app.input, "draft");
    assert_eq!(app.projects.len(), 1);
}

#[test]
fn project_picker_refresh_preserves_query_and_selected_project() {
    let mut app = App::new();
    let mut state = ClientState::default();
    apply(&mut app, &mut state, ready("a", "/tmp/a"));
    apply(
        &mut app,
        &mut state,
        ServerMessage::ProjectList {
            projects: vec![project("/tmp/a"), project("/tmp/b")],
        },
    );
    app.open_project_picker();
    app.picker.as_mut().unwrap().filter = "tmp".into();
    app.picker.as_mut().unwrap().selected = 1;
    apply(
        &mut app,
        &mut state,
        ServerMessage::ProjectList {
            projects: vec![project("/tmp/b"), project("/tmp/a")],
        },
    );
    let picker = app.picker.as_ref().unwrap();
    assert_eq!(picker.filter, "tmp");
    assert_eq!(picker.filtered()[picker.selected].id, "/tmp/b");
}

#[test]
fn foreign_equal_text_echo_cannot_accept_or_reformat_our_draft() {
    let mut app = App::new();
    let mut state = ClientState::default();
    apply(&mut app, &mut state, ready("a", "/tmp/a"));
    app.input = "  same text  ".into();
    app.begin_submission("ours".into(), "our display".into(), vec![], false);
    apply(
        &mut app,
        &mut state,
        ServerMessage::UserMessage {
            text: "same text".into(),
            request_id: Some("theirs".into()),
        },
    );
    assert_eq!(app.input, "  same text  ");
    assert!(app.submission_pending());
    assert!(matches!(&app.messages()[0].parts[0], Part::Text(text) if text.text == "same text"));
    apply(
        &mut app,
        &mut state,
        ServerMessage::PromptReceipt {
            session_id: "a".into(),
            request_id: "ours".into(),
            status: mew_protocol::PromptStatus::Rejected,
            message: Some("busy".into()),
        },
    );
    assert_eq!(app.input, "  same text  ");
    assert!(!app.submission_pending());
}

#[test]
fn matching_echo_waits_for_receipt_and_then_applies_only_its_own_display() {
    let mut app = App::new();
    let mut state = ClientState::default();
    apply(&mut app, &mut state, ready("a", "/tmp/a"));
    app.input = "our draft".into();
    app.begin_submission("ours".into(), "our display".into(), vec![], false);
    apply(
        &mut app,
        &mut state,
        ServerMessage::UserMessage {
            text: "enriched".into(),
            request_id: Some("ours".into()),
        },
    );
    assert_eq!(app.input, "our draft");
    assert!(app.submission_pending());
    apply(
        &mut app,
        &mut state,
        ServerMessage::PromptReceipt {
            session_id: "a".into(),
            request_id: "ours".into(),
            status: mew_protocol::PromptStatus::Accepted,
            message: None,
        },
    );
    assert!(app.input.is_empty());
    assert!(!app.submission_pending());
    assert!(matches!(&app.messages()[0].parts[0], Part::Text(text) if text.text == "our display"));
    apply(
        &mut app,
        &mut state,
        ServerMessage::UserMessage {
            text: "enriched".into(),
            request_id: Some("theirs".into()),
        },
    );
    assert!(matches!(&app.messages()[1].parts[0], Part::Text(text) if text.text == "enriched"));
}

#[test]
fn accepted_receipt_before_echo_preserves_request_ownership() {
    let mut app = App::new();
    let mut state = ClientState::default();
    apply(&mut app, &mut state, ready("a", "/tmp/a"));
    app.input = "draft".into();
    app.begin_submission("ours".into(), "our display".into(), vec![], false);
    apply(
        &mut app,
        &mut state,
        ServerMessage::PromptReceipt {
            session_id: "a".into(),
            request_id: "ours".into(),
            status: mew_protocol::PromptStatus::Accepted,
            message: None,
        },
    );
    apply(
        &mut app,
        &mut state,
        ServerMessage::UserMessage {
            text: "enriched".into(),
            request_id: Some("theirs".into()),
        },
    );
    assert!(matches!(&app.messages()[0].parts[0], Part::Text(text) if text.text == "enriched"));
    apply(
        &mut app,
        &mut state,
        ServerMessage::UserMessage {
            text: "enriched".into(),
            request_id: Some("ours".into()),
        },
    );
    assert!(matches!(&app.messages()[1].parts[0], Part::Text(text) if text.text == "our display"));
}

#[test]
fn returning_to_a_conversation_restores_its_silent_running_activity() {
    let mut app = App::new();
    let mut state = ClientState::default();
    apply(&mut app, &mut state, ready("a", "/tmp/a"));
    apply(
        &mut app,
        &mut state,
        ServerMessage::SubagentStart {
            parent_call_id: "task".into(),
            child_session_id: "child".into(),
            name: "researcher".into(),
            display_name: Some("river".into()),
        },
    );
    apply(
        &mut app,
        &mut state,
        ServerMessage::SubagentStatus {
            parent_call_id: "task".into(),
            tool_name: "ask_user".into(),
            message: "waiting for your answer".into(),
        },
    );
    apply(
        &mut app,
        &mut state,
        ServerMessage::JobUpdate {
            job_id: "job".into(),
            command: "build".into(),
            state: "running".into(),
        },
    );
    apply(&mut app, &mut state, ready("b", "/tmp/b"));
    assert!(app.subagents.is_empty());
    assert!(app.background_jobs.is_empty());
    apply(&mut app, &mut state, ready("a", "/tmp/a"));
    assert_eq!(app.subagents.len(), 1);
    assert_eq!(app.subagents[0].child_session_id, "child");
    assert_eq!(
        app.subagents[0].last_progress.as_deref(),
        Some("waiting for your answer")
    );
    assert_eq!(app.background_jobs.len(), 1);
    assert_eq!(app.background_jobs[0].job_id, "job");
    assert!(matches!(
        app.background_jobs[0].status,
        mew_tui::app::BackgroundJobStatus::Running
    ));
}

#[test]
fn authoritative_activity_snapshot_removes_stale_tasks_without_touching_another_session() {
    let mut app = App::new();
    let mut state = ClientState::default();
    apply(&mut app, &mut state, ready("a", "/tmp/a"));
    apply(
        &mut app,
        &mut state,
        ServerMessage::SubagentStart {
            parent_call_id: "stale".into(),
            child_session_id: "child".into(),
            name: "researcher".into(),
            display_name: None,
        },
    );
    apply(
        &mut app,
        &mut state,
        ServerMessage::JobUpdate {
            job_id: "stale-job".into(),
            command: "old build".into(),
            state: "running".into(),
        },
    );
    apply(
        &mut app,
        &mut state,
        ServerMessage::SessionActivitySnapshot {
            session_id: "b".into(),
            subagents: vec![],
            jobs: vec![],
        },
    );
    assert_eq!(app.subagents.len(), 1);
    assert_eq!(app.background_jobs.len(), 1);
    apply(
        &mut app,
        &mut state,
        ServerMessage::SessionActivitySnapshot {
            session_id: "a".into(),
            subagents: vec![],
            jobs: vec![],
        },
    );
    assert!(app.subagents.is_empty());
    assert!(app.background_jobs.is_empty());
}

#[test]
fn a_previous_turns_error_cannot_confirm_or_cancel_a_pending_attachment() {
    let mut app = App::new();
    let mut state = ClientState::default();
    apply(&mut app, &mut state, ready("a", "/tmp/a"));
    app.attaching_session = true;
    apply(
        &mut app,
        &mut state,
        ServerMessage::ErrorEvent {
            message: "previous turn aborted".into(),
        },
    );
    assert!(app.attaching_session);
    assert_eq!(app.status.session_id, "a");
    assert!(app
        .messages()
        .iter()
        .flat_map(|message| &message.parts)
        .any(
            |part| matches!(part, Part::Text(text) if text.text.contains("previous turn aborted"))
        ));
}

#[test]
fn review_projection_accepts_only_matching_responses_and_refreshes_the_current_workspace() {
    let mut app = App::new();
    let mut state = ClientState::default();
    apply(&mut app, &mut state, ready("a", "/tmp/a"));
    app.open_review();
    let Some(mew_tui::events::Action::ReadSessionDiff { request_id, .. }) =
        app.take_activity_request(std::time::Instant::now())
    else {
        panic!("review must request its confirmed session diff");
    };
    apply(
        &mut app,
        &mut state,
        ServerMessage::SessionDiff {
            session_id: "other".into(),
            request_id: request_id.clone(),
            files: vec![],
        },
    );
    assert!(
        !app.activity_view
            .as_ref()
            .unwrap()
            .review
            .as_ref()
            .unwrap()
            .loaded
    );
    apply(
        &mut app,
        &mut state,
        ServerMessage::SessionDiffError {
            session_id: "a".into(),
            request_id: "old-request".into(),
            message: "wrong response".into(),
        },
    );
    assert!(app
        .activity_view
        .as_ref()
        .unwrap()
        .review
        .as_ref()
        .unwrap()
        .error
        .is_none());
    apply(
        &mut app,
        &mut state,
        ServerMessage::SessionDiff {
            session_id: "a".into(),
            request_id: request_id.clone(),
            files: vec![],
        },
    );
    assert!(
        app.activity_view
            .as_ref()
            .unwrap()
            .review
            .as_ref()
            .unwrap()
            .loaded
    );
    app.apply_client_update(
        &[ClientEvent::WorkspaceChanged {
            session_id: "other".into(),
        }],
        &state,
        None,
    );
    assert!(app
        .take_activity_request(std::time::Instant::now())
        .is_none());
    app.apply_client_update(
        &[ClientEvent::WorkspaceChanged {
            session_id: "a".into(),
        }],
        &state,
        None,
    );
    assert!(
        matches!(app.take_activity_request(std::time::Instant::now()), Some(mew_tui::events::Action::ReadSessionDiff { session_id, request_id: fresh }) if session_id == "a" && fresh != request_id)
    );
}

#[test]
fn review_projection_shows_read_errors_and_preserves_the_chat_draft() {
    let mut app = App::new();
    let mut state = ClientState::default();
    apply(&mut app, &mut state, ready("a", "/tmp/a"));
    app.input = "saved draft".into();
    app.open_review();
    let Some(mew_tui::events::Action::ReadSessionDiff { request_id, .. }) =
        app.take_activity_request(std::time::Instant::now())
    else {
        panic!("review request");
    };
    apply(
        &mut app,
        &mut state,
        ServerMessage::SessionDiffError {
            session_id: "a".into(),
            request_id,
            message: "workspace is unavailable".into(),
        },
    );
    assert_eq!(
        app.activity_view
            .as_ref()
            .unwrap()
            .review
            .as_ref()
            .unwrap()
            .error
            .as_deref(),
        Some("workspace is unavailable")
    );
    assert_eq!(app.input, "saved draft");
}

#[test]
fn job_output_projection_preserves_owner_request_offsets_and_final_status() {
    let mut app = App::new();
    let mut state = ClientState::default();
    apply(&mut app, &mut state, ready("a", "/tmp/a"));
    app.open_job("job".into());
    let Some(mew_tui::events::Action::ReadJobOutput { request_id, .. }) =
        app.take_activity_request(std::time::Instant::now())
    else {
        panic!("job output request");
    };
    for (session_id, reply_id) in [("other", request_id.as_str()), ("a", "old")] {
        apply(
            &mut app,
            &mut state,
            ServerMessage::JobOutput {
                session_id: session_id.into(),
                job_id: "job".into(),
                request_id: reply_id.into(),
                offset: 0,
                next_offset: 7,
                output: "foreign".into(),
                done: true,
                incomplete: false,
            },
        );
    }
    assert!(app
        .activity_view
        .as_ref()
        .unwrap()
        .job_output
        .as_ref()
        .unwrap()
        .output
        .is_empty());
    apply(
        &mut app,
        &mut state,
        ServerMessage::JobOutput {
            session_id: "a".into(),
            job_id: "job".into(),
            request_id,
            offset: 0,
            next_offset: 7,
            output: "日本\n".into(),
            done: true,
            incomplete: true,
        },
    );
    let output = app
        .activity_view
        .as_ref()
        .unwrap()
        .job_output
        .as_ref()
        .unwrap();
    assert_eq!(output.output, "日本\n");
    assert_eq!(output.offset, 7);
    assert!(output.done);
    assert!(output.incomplete);
}

#[test]
fn job_output_projection_shows_matching_read_errors() {
    let mut app = App::new();
    let mut state = ClientState::default();
    apply(&mut app, &mut state, ready("a", "/tmp/a"));
    app.open_job("job".into());
    let Some(mew_tui::events::Action::ReadJobOutput { request_id, .. }) =
        app.take_activity_request(std::time::Instant::now())
    else {
        panic!("job output request");
    };
    apply(
        &mut app,
        &mut state,
        ServerMessage::JobOutputError {
            session_id: "a".into(),
            job_id: "other-job".into(),
            request_id: request_id.clone(),
            message: "foreign".into(),
        },
    );
    assert!(app
        .activity_view
        .as_ref()
        .unwrap()
        .job_output
        .as_ref()
        .unwrap()
        .error
        .is_none());
    apply(
        &mut app,
        &mut state,
        ServerMessage::JobOutputError {
            session_id: "a".into(),
            job_id: "job".into(),
            request_id,
            message: "job is unavailable".into(),
        },
    );
    assert_eq!(
        app.activity_view
            .as_ref()
            .unwrap()
            .job_output
            .as_ref()
            .unwrap()
            .error
            .as_deref(),
        Some("job is unavailable")
    );
}
