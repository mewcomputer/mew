use crate::runtime;
use crate::PluginInfo;
use mew_agent::AgentEvent;
use mew_message::Part;
use mew_tui::events::Action;
use strum::IntoEnumIterator;

/// Test double that logs every method call. When `failing` is true, all
/// Result-returning methods return `Err(Unsupported)` instead of `Ok` —
/// lets one struct cover both the "happy path" and "unsupported" tests.
struct RecordingTarget {
    calls: Vec<&'static str>,
    closed_questions: Vec<(String, String)>,
    failing: bool,
}

impl RecordingTarget {
    fn new() -> Self {
        Self {
            calls: Vec::new(),
            closed_questions: Vec::new(),
            failing: false,
        }
    }
    fn failing() -> Self {
        Self {
            calls: Vec::new(),
            closed_questions: Vec::new(),
            failing: true,
        }
    }
    fn record(&mut self, name: &'static str) {
        self.calls.push(name);
    }
    /// Returns `Err(Unsupported("test"))` if `failing` is true, else `Ok(())`.
    fn check(&self) -> Result<(), runtime::target::Unsupported> {
        if self.failing {
            Err(runtime::target::Unsupported("test"))
        } else {
            Ok(())
        }
    }
}

#[async_trait::async_trait]
impl runtime::target::CommandTarget for RecordingTarget {
    async fn close_question(&mut self, session_id: &str, request_id: &str) -> anyhow::Result<()> {
        self.record("close_question");
        self.closed_questions
            .push((session_id.into(), request_id.into()));
        if self.failing {
            anyhow::bail!("close rejected")
        }
        Ok(())
    }
    async fn prompt(
        &mut self,
        _session_id: &str,
        _request_id: &str,
        _enriched: String,
        _parts: Vec<Part>,
    ) -> anyhow::Result<()> {
        self.record("prompt");
        if self.failing {
            anyhow::bail!("submission rejected")
        }
        Ok(())
    }
    async fn cancel(&mut self, _session_id: &str) -> anyhow::Result<()> {
        self.record("cancel");
        if self.failing {
            anyhow::bail!("cancel rejected")
        }
        Ok(())
    }
    async fn guide(&mut self, _session_id: &str, _text: String) -> anyhow::Result<()> {
        self.record("guide");
        if self.failing {
            anyhow::bail!("guide rejected")
        }
        Ok(())
    }
    async fn clear(&mut self) -> Result<(), runtime::target::Unsupported> {
        self.record("clear");
        self.check()
    }
    async fn compact(&mut self) -> Result<(), runtime::target::Unsupported> {
        self.record("compact");
        self.check()
    }
    async fn read_subagent_history(
        &mut self,
        _session_id: &str,
        _child_session_id: &str,
        _request_id: &str,
    ) -> Result<(), runtime::target::Unsupported> {
        self.record("read_subagent_history");
        self.check()
    }
    async fn update_todo(
        &mut self,
        _session_id: &str,
        edit: mew_protocol::TodoEdit,
    ) -> anyhow::Result<Vec<mew_agent::Todo>> {
        self.record("update_todo");
        if self.failing {
            anyhow::bail!("save rejected")
        } else {
            Ok(vec![mew_agent::Todo {
                id: edit.todo_id,
                content: edit.content,
                status: mew_agent::TodoStatus::parse(&edit.status).unwrap(),
                depends_on: vec![],
            }])
        }
    }
    async fn switch_model(
        &mut self,
        _spec: &str,
    ) -> Result<runtime::target::SwitchedModel, runtime::target::Unsupported> {
        self.record("switch_model");
        if self.failing {
            Err(runtime::target::Unsupported("test"))
        } else {
            Ok(runtime::target::SwitchedModel {
                provider_id: "t".into(),
                model_id: "t".into(),
                display: "t".into(),
            })
        }
    }
    async fn set_permission_mode(
        &mut self,
        _mode: mew_hooks::PermissionMode,
    ) -> Result<(), runtime::target::Unsupported> {
        self.record("set_permission_mode");
        self.check()
    }
    async fn set_thinking(&mut self, _variant: &str) -> Result<(), runtime::target::Unsupported> {
        self.record("set_thinking");
        self.check()
    }
    async fn attach_session(&mut self, _id: &str) -> Result<(), runtime::target::Unsupported> {
        self.record("attach_session");
        self.check()
    }
    async fn resume(&mut self, _id: &str) -> Result<(), runtime::target::Unsupported> {
        self.record("resume");
        self.check()
    }
    async fn rewind(&mut self, _n: usize) -> Result<(), runtime::target::Unsupported> {
        self.record("rewind");
        self.check()
    }
    async fn switch_persona(
        &mut self,
        _name: &str,
        _personas: &[mew_personas::Persona],
    ) -> Result<runtime::target::PersonaApplied, runtime::target::Unsupported> {
        self.record("switch_persona");
        if self.failing {
            Err(runtime::target::Unsupported("test"))
        } else {
            Ok(runtime::target::PersonaApplied {
                pinned_model: None,
                display: "t".into(),
            })
        }
    }
    async fn plugin_command(
        &mut self,
        _name: &str,
        _args: &str,
    ) -> Result<String, runtime::target::Unsupported> {
        self.record("plugin_command");
        if self.failing {
            Err(runtime::target::Unsupported("test"))
        } else {
            Ok("r".into())
        }
    }
    async fn cancel_subagent(
        &mut self,
        _task_id: &str,
    ) -> Result<bool, runtime::target::Unsupported> {
        self.record("cancel_subagent");
        if self.failing {
            Err(runtime::target::Unsupported("test"))
        } else {
            Ok(true)
        }
    }
    async fn manage_goal(
        &mut self,
        _action: runtime::target::GoalAction,
    ) -> Result<String, runtime::target::Unsupported> {
        self.record("manage_goal");
        if self.failing {
            Err(runtime::target::Unsupported("test"))
        } else {
            Ok("goal managed".into())
        }
    }
    async fn set_auto_title(&mut self, _enabled: bool) -> Result<(), runtime::target::Unsupported> {
        self.record("set_auto_title");
        self.check()
    }
    async fn set_auto_summary(
        &mut self,
        _enabled: bool,
    ) -> Result<(), runtime::target::Unsupported> {
        self.record("set_auto_summary");
        self.check()
    }
    async fn yield_control(&mut self) -> Result<(), runtime::target::Unsupported> {
        self.record("yield_control");
        self.check()
    }
    async fn unflag_file(&mut self, _path: &str) -> Result<(), runtime::target::Unsupported> {
        self.record("unflag_file");
        self.check()
    }
    async fn list_projects(&mut self) -> Result<(), runtime::target::Unsupported> {
        self.record("list_projects");
        self.check()
    }
    async fn new_session_in(&mut self, _path: &str) -> Result<(), runtime::target::Unsupported> {
        self.record("new_session_in");
        self.check()
    }
    async fn archive_session(
        &mut self,
        _id: &str,
        _archived: bool,
    ) -> Result<(), runtime::target::Unsupported> {
        self.record("archive_session");
        self.check()
    }
    async fn pin_session(
        &mut self,
        _id: &str,
        _pinned: bool,
    ) -> Result<(), runtime::target::Unsupported> {
        self.record("pin_session");
        self.check()
    }
    async fn rename_session(
        &mut self,
        _id: &str,
        _title: &str,
    ) -> Result<(), runtime::target::Unsupported> {
        self.record("rename_session");
        self.check()
    }
}

/// Every `Action` variant must produce an observable effect through
/// `handle_action`. Catches silent drops.
#[tokio::test]
async fn test_action_variant_table() {
    let all_actions: Vec<Action> = Action::iter().collect();
    assert!(
        !all_actions.is_empty(),
        "Action::iter() should return variants"
    );

    for action in Action::iter() {
        // Skip actions that need external resources not available in tests.
        if matches!(action, Action::CopySelection(_)) {
            continue; // clipboard / settings editor handles these
        }
        // Provide meaningful test data for actions that carry payloads.
        let action = match action {
            Action::Submit(_) => Action::Submit("test prompt".into()),
            Action::SlashCommand(_) => Action::SlashCommand("/cost".into()),
            Action::SwitchModel(_) => Action::SwitchModel("test/model".into()),
            Action::InsertAtMention(_) => Action::InsertAtMention("@file".into()),
            Action::InsertNamespaceMention(_, _) => {
                Action::InsertNamespaceMention("skill".into(), "clarify".into())
            }
            Action::CopySelection(_) => unreachable!(),
            Action::CancelMostRecentSubagent(_) => {
                Action::CancelMostRecentSubagent("task_123".into())
            }
            Action::PersonaSwitchConfirmed(_) => Action::PersonaSwitchConfirmed("test".into()),
            Action::CyclePersona(_) => Action::CyclePersona(1),
            Action::SetPermissionMode(_) => {
                Action::SetPermissionMode(mew_hooks::PermissionMode::Standard)
            }
            Action::SetThinkingVariant(_) => Action::SetThinkingVariant("off".into()),
            Action::AttachSession(_) => Action::AttachSession("sess_123".into()),
            Action::SendQueuedNow { .. } => Action::SendQueuedNow {
                session_id: String::new(),
                text: "queued text".into(),
            },
            Action::GuideQueued { .. } => Action::GuideQueued {
                session_id: String::new(),
                text: "guided text".into(),
            },
            Action::UpdateTodo { .. } => Action::UpdateTodo {
                session_id: String::new(),
                edit: mew_protocol::TodoEdit {
                    todo_id: 7,
                    expected_content: "old".into(),
                    expected_status: "pending".into(),
                    content: "edited".into(),
                    status: "pending".into(),
                },
            },
            other => other,
        };
        let mut app = mew_tui::App::new();
        if matches!(
            action,
            Action::SubmitQueued { .. }
                | Action::SendQueuedNow { .. }
                | Action::GuideQueued { .. }
                | Action::CloseQuestion { .. }
        ) {
            app.status.session_id = String::new();
        }
        if matches!(action, Action::UpdateTodo { .. }) {
            app.open_todos(None);
        }
        let mut target = RecordingTarget::new();
        let mut should_break = false;
        let plugin_info = std::sync::Arc::new(std::sync::Mutex::new(PluginInfo {
            active_persona: None,
        }));

        let chat_dirty_before = app.chat_dirty;
        let messages_before = app.messages().len();
        let input_before = app.input.clone();
        let sidebar_before = app.sidebar_collapsed.clone();

        let mut cx = runtime::Ctx {
            app: &mut app,
            target: &mut target,
            should_break: &mut should_break,
            cat: None,
            loaded_personas: &[],
            plugin_info: &plugin_info,
        };

        let _ = runtime::handle_action(&mut cx, action.clone()).await;

        let target_called = !target.calls.is_empty();
        let chat_dirty_changed = app.chat_dirty != chat_dirty_before;
        let messages_changed = app.messages().len() != messages_before;
        let input_changed = app.input != input_before;
        let sidebar_changed = app.sidebar_collapsed != sidebar_before;
        let alert_set = app.alert.is_some();
        let quit_set = should_break;
        let mode_changed = app.mode != mew_tui::app::Mode::Normal;

        assert!(
            target_called
                || chat_dirty_changed
                || messages_changed
                || input_changed
                || sidebar_changed
                || alert_set
                || quit_set
                || mode_changed,
            "Action variant {:?} produced no observable effect — possible silent drop",
            action
        );
    }
}

// -----------------------------------------------------------------------
// AC.3: Synthetic message push renders immediately (chat_dirty bumped).
// -----------------------------------------------------------------------

#[tokio::test]
async fn test_synthetic_message_renders_immediately() {
    let mut app = mew_tui::App::new();
    let mut target = RecordingTarget::new();
    let mut should_break = false;
    let plugin_info = std::sync::Arc::new(std::sync::Mutex::new(PluginInfo {
        active_persona: None,
    }));

    // /cost returns SlashResult::Message which pushes a synthetic message.
    let dirty_before = app.chat_dirty;
    let mut cx = runtime::Ctx {
        app: &mut app,
        target: &mut target,
        should_break: &mut should_break,
        cat: None,
        loaded_personas: &[],
        plugin_info: &plugin_info,
    };
    let _ = runtime::handle_action(&mut cx, Action::SlashCommand("/cost".into())).await;

    // The synthetic message must have been pushed and dirty marked.
    assert!(
        app.chat_dirty != dirty_before,
        "chat_dirty should have been bumped by synthetic message push"
    );
    assert!(
        !app.messages().is_empty(),
        "a message should have been pushed"
    );
}

// -----------------------------------------------------------------------
// AC.5: Unknown slash command falls through to the model as a prompt.
// -----------------------------------------------------------------------

#[tokio::test]
async fn test_unknown_slash_falls_through() {
    let mut app = mew_tui::App::new();
    let mut target = RecordingTarget::new();
    let mut should_break = false;
    let plugin_info = std::sync::Arc::new(std::sync::Mutex::new(PluginInfo {
        active_persona: None,
    }));

    let mut cx = runtime::Ctx {
        app: &mut app,
        target: &mut target,
        should_break: &mut should_break,
        cat: None,
        loaded_personas: &[],
        plugin_info: &plugin_info,
    };
    let _ = runtime::handle_action(&mut cx, Action::SlashCommand("/xyz".into())).await;

    // The recording target should have received a prompt call —
    // the unknown slash fell through to the model.
    assert!(
        target.calls.contains(&"prompt"),
        "unknown slash command should fall through to prompt; got calls: {:?}",
        target.calls
    );
}

#[tokio::test]
async fn submission_waits_for_confirmation_and_failed_submission_preserves_editor() {
    let mut app = mew_tui::App::new();
    app.daemon_mode = true;
    app.status.session_id = "owner".into();
    app.input = "  unsent draft  ".into();
    app.cursor = 5;
    let mut target = RecordingTarget::failing();
    let mut should_break = false;
    let plugin_info = std::sync::Arc::new(std::sync::Mutex::new(PluginInfo {
        active_persona: None,
    }));
    let mut cx = runtime::Ctx {
        app: &mut app,
        target: &mut target,
        should_break: &mut should_break,
        cat: None,
        loaded_personas: &[],
        plugin_info: &plugin_info,
    };
    cx.app.attaching_session = true;
    runtime::handle_action(&mut cx, Action::Submit("unsent draft".into())).await;
    assert!(cx.target.calls.is_empty());
    assert_eq!(cx.app.input, "  unsent draft  ");
    cx.app.attaching_session = false;
    runtime::handle_action(&mut cx, Action::Submit("unsent draft".into())).await;
    assert_eq!(cx.target.calls, ["prompt"]);
    assert_eq!(cx.app.input, "  unsent draft  ");
    assert_eq!(cx.app.cursor, 5);
    assert!(cx.app.messages().is_empty());
    assert!(!cx.app.streaming);
    assert!(cx.app.submission_pending());
    runtime::handle_action(&mut cx, Action::Submit("unsent draft".into())).await;
    assert_eq!(cx.target.calls, ["prompt"]);
}

#[tokio::test]
async fn stale_queued_guidance_returns_to_its_owner_without_reaching_the_new_turn() {
    let mut app = mew_tui::App::new();
    app.daemon_mode = true;
    app.status.session_id = "b".into();
    app.streaming = true;
    let mut target = RecordingTarget::new();
    let mut should_break = false;
    let plugin_info = std::sync::Arc::new(std::sync::Mutex::new(PluginInfo {
        active_persona: None,
    }));
    let mut cx = runtime::Ctx {
        app: &mut app,
        target: &mut target,
        should_break: &mut should_break,
        cat: None,
        loaded_personas: &[],
        plugin_info: &plugin_info,
    };
    runtime::handle_action(
        &mut cx,
        Action::GuideQueued {
            session_id: "a".into(),
            text: "a guidance".into(),
        },
    )
    .await;
    assert!(cx.target.calls.is_empty());
    assert!(cx.app.messages().is_empty());
    cx.app
        .apply_daemon_notification(&mew_protocol::ServerMessage::SessionReady {
            session_id: "a".into(),
            cwd: None,
            model: None,
            provider: None,
            permission_mode: None,
        });
    assert_eq!(cx.app.queued_messages, ["a guidance"]);
}

#[tokio::test]
async fn new_conversation_keeps_the_confirmed_view_until_ready() {
    let mut app = mew_tui::App::new();
    app.daemon_mode = true;
    app.push_synthetic_message("current conversation".into());
    let mut target = RecordingTarget::new();
    let mut should_break = false;
    let plugin_info = std::sync::Arc::new(std::sync::Mutex::new(PluginInfo {
        active_persona: None,
    }));
    let mut cx = runtime::Ctx {
        app: &mut app,
        target: &mut target,
        should_break: &mut should_break,
        cat: None,
        loaded_personas: &[],
        plugin_info: &plugin_info,
    };
    runtime::handle_action(&mut cx, Action::NewSessionInProject("/tmp/new".into())).await;
    assert!(cx.app.attaching_session);
    assert_eq!(cx.app.messages().len(), 1);
    assert!(
        matches!(&cx.app.messages()[0].parts[0],Part::Text(text) if text.text == "current conversation")
    );
}

// -----------------------------------------------------------------------
// AC.6: /quit (Action::Quit) returns Flow::Quit regardless of target.
// -----------------------------------------------------------------------

#[tokio::test]
async fn test_daemon_quit() {
    let mut app = mew_tui::App::new();
    let mut target = RecordingTarget::new();
    let mut should_break = false;
    let plugin_info = std::sync::Arc::new(std::sync::Mutex::new(PluginInfo {
        active_persona: None,
    }));

    let mut cx = runtime::Ctx {
        app: &mut app,
        target: &mut target,
        should_break: &mut should_break,
        cat: None,
        loaded_personas: &[],
        plugin_info: &plugin_info,
    };
    let flow = runtime::handle_action(&mut cx, Action::Quit).await;
    assert!(
        matches!(flow, runtime::Flow::Quit),
        "Quit should return Flow::Quit"
    );
    assert!(should_break, "should_break flag should be set");
}

// -----------------------------------------------------------------------
// PasteClipboardImage: SSH detection and error paths.
// -----------------------------------------------------------------------

#[tokio::test]
async fn test_paste_clipboard_image_ssh_warning() {
    // Set SSH env vars to trigger the SSH guard path.
    std::env::set_var("SSH_CONNECTION", "fake");
    let mut app = mew_tui::App::new();
    let mut target = RecordingTarget::new();
    let mut should_break = false;
    let plugin_info = std::sync::Arc::new(std::sync::Mutex::new(PluginInfo {
        active_persona: None,
    }));

    let mut cx = runtime::Ctx {
        app: &mut app,
        target: &mut target,
        should_break: &mut should_break,
        cat: None,
        loaded_personas: &[],
        plugin_info: &plugin_info,
    };
    runtime::dispatch::handle_paste_clipboard_image_with(&mut cx, || {
        Err("clipboard image tool unavailable".into())
    })
    .await;
    std::env::remove_var("SSH_CONNECTION");

    assert!(
        app.alert.is_some(),
        "SSH session should set an alert warning"
    );
    let alert_text = app.alert.as_ref().unwrap().0.as_str();
    assert!(
        alert_text.contains("SSH"),
        "alert should mention SSH, got: {alert_text}"
    );
    // Nothing should have been inserted into the input.
    assert!(app.input.is_empty(), "input should remain empty on SSH");
}

#[tokio::test]
async fn test_paste_clipboard_image_no_tool_error() {
    // This test does not touch SSH env vars to avoid racing with
    // test_paste_clipboard_image_ssh_warning when tests run in parallel.
    // Both the SSH guard path and the missing-tool error path produce an
    // alert that is not "image pasted from clipboard", so the assertions
    // hold regardless of which path fires.

    let mut app = mew_tui::App::new();
    let mut target = RecordingTarget::new();
    let mut should_break = false;
    let plugin_info = std::sync::Arc::new(std::sync::Mutex::new(PluginInfo {
        active_persona: None,
    }));

    let mut cx = runtime::Ctx {
        app: &mut app,
        target: &mut target,
        should_break: &mut should_break,
        cat: None,
        loaded_personas: &[],
        plugin_info: &plugin_info,
    };
    runtime::dispatch::handle_paste_clipboard_image_with(&mut cx, || {
        Err("clipboard image tool unavailable".into())
    })
    .await;

    // Without pngpaste (or equivalent), we should get an error alert,
    // not a silent no-op.
    assert!(
        app.alert.is_some(),
        "missing clipboard tool should produce an alert"
    );
    let alert_text = app.alert.as_ref().unwrap().0.as_str();
    assert!(
        !alert_text.contains("image pasted from clipboard"),
        "should not report success when no clipboard tool is available"
    );
}

// -----------------------------------------------------------------------
// AC.14: Unknown daemon slash command returns an error, not silence.
// Tests the registry lookup + error message format used by the daemon.
// -----------------------------------------------------------------------

#[test]
fn test_daemon_unknown_command_reply() {
    // The daemon's handler checks is_known() and returns
    // Some(format!("unknown command {}", cmd)) for unknown commands.
    // Verify the registry correctly rejects unknown commands.
    assert!(
        !mew_protocol::is_known("/nonexistent"),
        "unknown command should not be in registry"
    );

    // Known commands should be recognized so they're NOT reported as errors.
    assert!(
        mew_protocol::is_known("/clear"),
        "/clear should be in registry"
    );
    assert!(
        mew_protocol::is_known("/compact"),
        "/compact should be in registry"
    );
    assert!(
        mew_protocol::is_known("/quit"),
        "/quit should be in registry"
    );
}

// -----------------------------------------------------------------------
// AC.9: DaemonTarget capabilities — verifies that DaemonTarget returns
// Err(Unsupported) for genuinely unsupported ops, and that the
// ClientMessage variants it constructs encode correctly.
// -----------------------------------------------------------------------

#[test]
fn test_daemon_capability_message_encoding() {
    // Verify the ClientMessage variants DaemonTarget uses encode correctly.
    // This is a smoke test for the wire format — a full test would need
    // a mock transport to intercept outgoing WebSocket frames.

    // SwitchModel
    let msg = mew_protocol::ClientMessage::SwitchModel {
        provider: "test-provider".into(),
        model: "test-model".into(),
    };
    let json = mew_protocol::encode_json(&msg).expect("encode SwitchModel");
    assert!(
        json.contains("switch_model"),
        "should have the right type tag: {json}"
    );
    assert!(
        json.contains("test-provider") && json.contains("test-model"),
        "SwitchModel JSON should contain provider and model: {json}"
    );

    // SetThinkingVariant
    let msg = mew_protocol::ClientMessage::SetThinkingVariant {
        variant: "high".into(),
    };
    let json = mew_protocol::encode_json(&msg).expect("encode SetThinkingVariant");
    assert!(
        json.contains("set_thinking_variant"),
        "should have the right type tag: {json}"
    );

    // SetPermissionMode
    let msg = mew_protocol::ClientMessage::SetPermissionMode {
        mode: "dangerous".into(),
    };
    let json = mew_protocol::encode_json(&msg).expect("encode SetPermissionMode");
    assert!(
        json.contains("set_permission_mode"),
        "should have the right type tag: {json}"
    );

    // SwitchPersona
    let msg = mew_protocol::ClientMessage::SwitchPersona {
        name: "test-persona".into(),
    };
    let json = mew_protocol::encode_json(&msg).expect("encode SwitchPersona");
    assert!(
        json.contains("switch_persona"),
        "should have the right type tag: {json}"
    );
}

/// AC.9 (supplemental): Verify that Unsupported errors from CommandTarget
/// are rendered as visible alerts (not swallowed) through handle_action.
/// Uses a target that returns Err(Unsupported) for every method.
#[tokio::test]
async fn test_unsupported_ops_render_alerts() {
    let mut app = mew_tui::App::new();
    let mut target = RecordingTarget::failing();
    let mut should_break = false;
    let plugin_info = std::sync::Arc::new(std::sync::Mutex::new(PluginInfo {
        active_persona: None,
    }));

    // Clear with a failing target should set an alert, not clear messages.
    let messages_before = app.messages().len();
    let mut cx = runtime::Ctx {
        app: &mut app,
        target: &mut target,
        should_break: &mut should_break,
        cat: None,
        loaded_personas: &[],
        plugin_info: &plugin_info,
    };
    let _ = runtime::handle_action(&mut cx, Action::Clear).await;
    assert!(
        app.alert.is_some(),
        "Clear with failing target should set an alert"
    );
    assert_eq!(
        app.messages().len(),
        messages_before,
        "Clear should not clear messages when target fails"
    );
}

// -----------------------------------------------------------------------
// AC.4: Action::SetPermissionMode is not dropped — it changes app state.
// -----------------------------------------------------------------------

#[tokio::test]
async fn test_set_permission_mode_not_dropped() {
    let mut app = mew_tui::App::new();
    let mut target = RecordingTarget::new();
    let mut should_break = false;
    let plugin_info = std::sync::Arc::new(std::sync::Mutex::new(PluginInfo {
        active_persona: None,
    }));

    let original_mode = app.permission_mode;
    let new_mode = mew_hooks::PermissionMode::Dangerous;
    assert_ne!(original_mode, new_mode, "precondition: modes differ");

    let mut cx = runtime::Ctx {
        app: &mut app,
        target: &mut target,
        should_break: &mut should_break,
        cat: None,
        loaded_personas: &[],
        plugin_info: &plugin_info,
    };
    let _ = runtime::handle_action(&mut cx, Action::SetPermissionMode(new_mode)).await;

    assert_eq!(
        app.permission_mode, new_mode,
        "SetPermissionMode must actually change the mode — not be dropped"
    );
    assert!(
        target.calls.contains(&"set_permission_mode"),
        "target.set_permission_mode should have been called"
    );
}

// -----------------------------------------------------------------------
// Persona cycling: Shift+Tab (CyclePersona(+1)) and Ctrl+Shift+Tab
// (CyclePersona(-1)) walk the persona list and wrap through "default".
// -----------------------------------------------------------------------

#[tokio::test]
async fn test_cycle_persona_forward_through_list() {
    let mut app = mew_tui::App::new();
    app.personas = vec![
        ("planner".into(), "plan".into()),
        ("builder".into(), "build".into()),
    ];
    let personas = mk_personas(&["planner", "builder"]);
    // No active persona → "default" slot. Forward should land on "planner".
    let mut target = RecordingTarget::new();
    let mut should_break = false;
    let plugin_info = std::sync::Arc::new(std::sync::Mutex::new(PluginInfo {
        active_persona: None,
    }));
    let mut cx = runtime::Ctx {
        app: &mut app,
        target: &mut target,
        should_break: &mut should_break,
        cat: None,
        loaded_personas: &personas,
        plugin_info: &plugin_info,
    };
    let _ = runtime::handle_action(&mut cx, Action::CyclePersona(1)).await;
    assert_eq!(app.active_persona.as_deref(), Some("planner"));
}

#[tokio::test]
async fn test_cycle_persona_forward_wraps_to_default() {
    let mut app = mew_tui::App::new();
    app.personas = vec![
        ("planner".into(), "plan".into()),
        ("builder".into(), "build".into()),
    ];
    let personas = mk_personas(&["planner", "builder"]);
    // Active is the last persona → forward wraps to "default" (None).
    app.active_persona = Some("builder".into());
    let mut target = RecordingTarget::new();
    let mut should_break = false;
    let plugin_info = std::sync::Arc::new(std::sync::Mutex::new(PluginInfo {
        active_persona: None,
    }));
    let mut cx = runtime::Ctx {
        app: &mut app,
        target: &mut target,
        should_break: &mut should_break,
        cat: None,
        loaded_personas: &personas,
        plugin_info: &plugin_info,
    };
    let _ = runtime::handle_action(&mut cx, Action::CyclePersona(1)).await;
    assert!(
        app.active_persona.is_none(),
        "forward from last persona should wrap to default"
    );
}

#[tokio::test]
async fn test_cycle_persona_backward_from_default_wraps_to_last() {
    let mut app = mew_tui::App::new();
    app.personas = vec![
        ("planner".into(), "plan".into()),
        ("builder".into(), "build".into()),
    ];
    let personas = mk_personas(&["planner", "builder"]);
    // No active persona (default) → backward should wrap to "builder".
    let mut target = RecordingTarget::new();
    let mut should_break = false;
    let plugin_info = std::sync::Arc::new(std::sync::Mutex::new(PluginInfo {
        active_persona: None,
    }));
    let mut cx = runtime::Ctx {
        app: &mut app,
        target: &mut target,
        should_break: &mut should_break,
        cat: None,
        loaded_personas: &personas,
        plugin_info: &plugin_info,
    };
    let _ = runtime::handle_action(&mut cx, Action::CyclePersona(-1)).await;
    assert_eq!(
        app.active_persona.as_deref(),
        Some("builder"),
        "backward from default should wrap to the last persona"
    );
}

#[tokio::test]
async fn test_cycle_persona_empty_list_sets_alert() {
    let mut app = mew_tui::App::new();
    // No personas loaded → should set an alert, not crash or dispatch.
    let mut target = RecordingTarget::new();
    let mut should_break = false;
    let plugin_info = std::sync::Arc::new(std::sync::Mutex::new(PluginInfo {
        active_persona: None,
    }));
    let mut cx = runtime::Ctx {
        app: &mut app,
        target: &mut target,
        should_break: &mut should_break,
        cat: None,
        loaded_personas: &[],
        plugin_info: &plugin_info,
    };
    let _ = runtime::handle_action(&mut cx, Action::CyclePersona(1)).await;
    assert!(
        app.alert.is_some(),
        "empty persona list should set an alert"
    );
    assert!(
        !target.calls.contains(&"switch_persona"),
        "should not call switch_persona when no personas loaded"
    );
}

/// Build a minimal `Persona` list for the given names, so
/// `handle_switch_persona` can find them and update display state.
fn mk_personas(names: &[&str]) -> Vec<mew_personas::Persona> {
    names
        .iter()
        .map(|n| mew_personas::Persona {
            name: (*n).into(),
            description: format!("{} persona", n),
            body: String::new(),
            path: std::path::PathBuf::new(),
            config: mew_personas::PersonaConfig::default(),
        })
        .collect()
}

// -----------------------------------------------------------------------
// /autotitle, /autosummary, /yield — daemon session toggles.
// -----------------------------------------------------------------------

#[tokio::test]
async fn test_autotitle_autosummary_yield_reach_target() {
    for (cmd, expected) in [
        ("/autotitle on", "set_auto_title"),
        ("/autotitle off", "set_auto_title"),
        ("/autosummary on", "set_auto_summary"),
        ("/yield", "yield_control"),
    ] {
        let mut app = mew_tui::App::new();
        app.daemon_mode = true;
        let mut target = RecordingTarget::new();
        let mut should_break = false;
        let plugin_info = std::sync::Arc::new(std::sync::Mutex::new(PluginInfo {
            active_persona: None,
        }));
        let mut cx = runtime::Ctx {
            app: &mut app,
            target: &mut target,
            should_break: &mut should_break,
            cat: None,
            loaded_personas: &[],
            plugin_info: &plugin_info,
        };
        let _ = runtime::handle_action(&mut cx, Action::SlashCommand(cmd.into())).await;
        assert!(
            target.calls.contains(&expected),
            "{cmd} should call {expected}; got calls: {:?}",
            target.calls
        );
    }
}

#[tokio::test]
async fn test_autotitle_unsupported_renders_alert() {
    let mut app = mew_tui::App::new();
    app.daemon_mode = true;
    let mut target = RecordingTarget::failing();
    let mut should_break = false;
    let plugin_info = std::sync::Arc::new(std::sync::Mutex::new(PluginInfo {
        active_persona: None,
    }));
    let mut cx = runtime::Ctx {
        app: &mut app,
        target: &mut target,
        should_break: &mut should_break,
        cat: None,
        loaded_personas: &[],
        plugin_info: &plugin_info,
    };
    let _ = runtime::handle_action(&mut cx, Action::SlashCommand("/autotitle on".into())).await;
    assert!(
        app.alert.is_some(),
        "unsupported set_auto_title should render a visible alert"
    );
}

#[tokio::test]
async fn test_unflag_reaches_target_and_updates_display() {
    let mut app = mew_tui::App::new();
    app.flagged_files = vec![mew_agent::FlaggedFileInfo {
        path: "src/main.rs".into(),
        reason: None,
    }];
    let mut target = RecordingTarget::new();
    let mut should_break = false;
    let plugin_info = std::sync::Arc::new(std::sync::Mutex::new(PluginInfo {
        active_persona: None,
    }));
    let mut cx = runtime::Ctx {
        app: &mut app,
        target: &mut target,
        should_break: &mut should_break,
        cat: None,
        loaded_personas: &[],
        plugin_info: &plugin_info,
    };
    let _ =
        runtime::handle_action(&mut cx, Action::SlashCommand("/unflag src/main.rs".into())).await;
    assert!(
        target.calls.contains(&"unflag_file"),
        "/unflag should call unflag_file; got calls: {:?}",
        target.calls
    );
    assert!(
        app.flagged_files.is_empty(),
        "display flagged set should drop the unflagged file"
    );
}

#[tokio::test]
async fn test_project_picker_flow_reaches_target() {
    // /project requests the project list; selecting a row creates the
    // session via NewSessionInProject.
    let mut app = mew_tui::App::new();
    app.daemon_mode = true;
    let mut target = RecordingTarget::new();
    let mut should_break = false;
    let plugin_info = std::sync::Arc::new(std::sync::Mutex::new(PluginInfo {
        active_persona: None,
    }));
    let mut cx = runtime::Ctx {
        app: &mut app,
        target: &mut target,
        should_break: &mut should_break,
        cat: None,
        loaded_personas: &[],
        plugin_info: &plugin_info,
    };
    let _ = runtime::handle_action(&mut cx, Action::SlashCommand("/project".into())).await;
    let _ = runtime::handle_action(&mut cx, Action::NewSessionInProject("/tmp/proj".into())).await;
    assert!(
        target.calls.contains(&"list_projects"),
        "/project should call list_projects; got calls: {:?}",
        target.calls
    );
    assert!(
        target.calls.contains(&"new_session_in"),
        "project selection should call new_session_in; got calls: {:?}",
        target.calls
    );
}

#[tokio::test]
async fn test_session_meta_actions_reach_target() {
    // Toggle archived/pinned + rename all reach the target and update
    // display state.
    let mut app = mew_tui::App::new();
    app.daemon_mode = true;
    app.status.session_id = "sess_1".into();
    app.daemon_sessions = vec![mew_protocol::SessionInfo {
        session_id: "sess_1".into(),
        state: mew_protocol::SessionState::Active,
        model: None,
        provider: None,
        created_at: 0,
        last_message_at: None,
        summary: None,
        client_count: 1,
        cwd: None,
        last_turn_failed: false,
        archived: false,
        pinned: false,
        group_id: None,
        workspace_path: None,
        change_stats: None,
        usage: None,
        context_tokens: None,
        pending_permissions: 0,
        pending_questions: 0,
        first_message: None,
    }];
    let mut target = RecordingTarget::new();
    let mut should_break = false;
    let plugin_info = std::sync::Arc::new(std::sync::Mutex::new(PluginInfo {
        active_persona: None,
    }));
    let mut cx = runtime::Ctx {
        app: &mut app,
        target: &mut target,
        should_break: &mut should_break,
        cat: None,
        loaded_personas: &[],
        plugin_info: &plugin_info,
    };
    let _ = runtime::handle_action(&mut cx, Action::ToggleSessionArchived("sess_1".into())).await;
    let _ = runtime::handle_action(&mut cx, Action::ToggleSessionPinned("sess_1".into())).await;
    let _ = runtime::handle_action(&mut cx, Action::SlashCommand("/rename fix-auth".into())).await;
    assert!(
        cx.app.daemon_sessions[0].archived,
        "archive toggle should flip local state"
    );
    assert!(
        cx.app.daemon_sessions[0].pinned,
        "pin toggle should flip local state"
    );
    assert_eq!(
        cx.app.session_titles.get("sess_1").map(String::as_str),
        Some("fix-auth"),
        "rename should update the display title"
    );
    assert!(
        target.calls.contains(&"archive_session"),
        "toggle should call archive_session; got: {:?}",
        target.calls
    );
    assert!(
        target.calls.contains(&"pin_session"),
        "toggle should call pin_session; got: {:?}",
        target.calls
    );
    assert!(
        target.calls.contains(&"rename_session"),
        "/rename should call rename_session; got: {:?}",
        target.calls
    );
}

// -----------------------------------------------------------------------
// Turn-alive guard: Cancel keeps streaming true until the turn actually ends.
// -----------------------------------------------------------------------

#[tokio::test]
async fn test_cancel_keeps_streaming_until_turn_ends() {
    let mut app = mew_tui::App::new();
    app.streaming = true;
    let mut target = RecordingTarget::new();
    let mut should_break = false;
    let plugin_info = std::sync::Arc::new(std::sync::Mutex::new(PluginInfo {
        active_persona: None,
    }));

    let mut cx = runtime::Ctx {
        app: &mut app,
        target: &mut target,
        should_break: &mut should_break,
        cat: None,
        loaded_personas: &[],
        plugin_info: &plugin_info,
    };

    // Cancel must NOT clear streaming immediately (turn-alive guard).
    let _ = runtime::handle_action(&mut cx, Action::Cancel).await;
    assert!(
        app.streaming,
        "cancel should keep streaming true until turn ends"
    );
    assert!(app.cancelling, "cancel should mark cancelling");

    // The turn-ending event clears both.
    app.handle_agent_event(mew_agent::AgentEvent::Error("aborted".into()));
    assert!(!app.streaming, "turn end should clear streaming");
    assert!(!app.cancelling, "turn end should clear cancelling");
}

#[tokio::test]
async fn test_submit_queues_until_full_turn_finishes() {
    let mut app = mew_tui::App::new();
    app.daemon_mode = true;
    app.streaming = true;
    let mut target = RecordingTarget::new();
    let mut should_break = false;
    let plugin_info = std::sync::Arc::new(std::sync::Mutex::new(PluginInfo {
        active_persona: None,
    }));
    let mut cx = runtime::Ctx {
        app: &mut app,
        target: &mut target,
        should_break: &mut should_break,
        cat: None,
        loaded_personas: &[],
        plugin_info: &plugin_info,
    };

    // Hidden reasoning can have no visible parts at all.
    runtime::handle_action(&mut cx, Action::Submit("first".into())).await;
    cx.app.handle_agent_event(AgentEvent::Provider(
        mew_provider::ProviderEvent::MessageEnd {
            finish: mew_message::Finish::Stop,
            usage: mew_message::Tokens::default(),
            cost: 0.0,
        },
    ));
    runtime::handle_action(&mut cx, Action::Submit("second".into())).await;
    runtime::handle_action(&mut cx, Action::SlashCommand("/unknown".into())).await;
    assert_eq!(cx.app.queued_messages, vec!["first", "second", "/unknown"]);
    assert!(cx.target.calls.is_empty());
    assert!(cx.app.messages().is_empty());
    assert!(!cx.app.pending_queued_send);

    cx.app.finish_turn();
    assert!(cx.app.pending_queued_send);
    let first = cx.app.pop_queued_message().unwrap();
    runtime::handle_action(&mut cx, Action::Submit(first)).await;
    assert_eq!(cx.target.calls, vec!["prompt"]);
    assert!(cx.app.streaming);
    assert_eq!(cx.app.queued_messages, vec!["second", "/unknown"]);
}

#[tokio::test]
async fn test_send_queued_now_waits_for_cancellation_shutdown() {
    let mut app = mew_tui::App::new();
    app.daemon_mode = true;
    app.streaming = true;
    app.queued_messages = vec!["later".into()];
    let mut target = RecordingTarget::new();
    let mut should_break = false;
    let plugin_info = std::sync::Arc::new(std::sync::Mutex::new(PluginInfo {
        active_persona: None,
    }));
    let mut cx = runtime::Ctx {
        app: &mut app,
        target: &mut target,
        should_break: &mut should_break,
        cat: None,
        loaded_personas: &[],
        plugin_info: &plugin_info,
    };
    let session_id = cx.app.status.session_id.clone();
    runtime::handle_action(
        &mut cx,
        Action::SendQueuedNow {
            session_id,
            text: "next".into(),
        },
    )
    .await;
    assert_eq!(cx.target.calls, vec!["cancel"]);
    assert!(cx.app.streaming);
    assert!(cx.app.cancelling);
    assert_eq!(cx.app.queued_messages, vec!["next", "later"]);
    cx.app
        .handle_agent_event(AgentEvent::Error("aborted".into()));
    runtime::handle_action(&mut cx, Action::Submit("also later".into())).await;
    assert_eq!(cx.target.calls, vec!["cancel"]);
    cx.app.finish_turn();
    let next = cx.app.pop_queued_message().unwrap();
    runtime::handle_action(&mut cx, Action::Submit(next)).await;
    assert_eq!(cx.target.calls, vec!["cancel", "prompt"]);
    assert_eq!(cx.app.queued_messages, vec!["later", "also later"]);
}

#[tokio::test]
async fn closing_a_child_question_is_scoped_and_preserves_the_parent_turn() {
    for (requested_session, failing, attaching) in [
        ("active", false, false),
        ("other", false, false),
        ("active", true, false),
        ("active", false, true),
    ] {
        let mut app = mew_tui::App::new();
        app.status.session_id = "active".into();
        app.streaming = true;
        app.attaching_session = attaching;
        let mut target = if failing {
            RecordingTarget::failing()
        } else {
            RecordingTarget::new()
        };
        let mut should_break = false;
        let plugin_info = std::sync::Arc::new(std::sync::Mutex::new(PluginInfo {
            active_persona: None,
        }));
        let mut cx = runtime::Ctx {
            app: &mut app,
            target: &mut target,
            should_break: &mut should_break,
            cat: None,
            loaded_personas: &[],
            plugin_info: &plugin_info,
        };
        runtime::handle_action(
            &mut cx,
            Action::CloseQuestion {
                session_id: requested_session.into(),
                request_id: "child-question".into(),
            },
        )
        .await;
        assert!(cx.app.streaming);
        assert!(!cx.app.cancelling);
        assert!(!*cx.should_break);
        if requested_session == "active" && !attaching {
            assert_eq!(cx.target.calls, vec!["close_question"]);
            assert_eq!(
                cx.target.closed_questions,
                vec![("active".into(), "child-question".into())]
            );
        } else {
            assert!(cx.target.calls.is_empty());
        }
        if failing || requested_session != "active" || attaching {
            assert!(
                cx.app.alert.is_some(),
                "close rejection must remain visible"
            );
        }
    }
}

#[tokio::test]
async fn explicit_project_command_opens_the_picker_without_waiting_for_catalog_refresh() {
    let mut app = mew_tui::App::new();
    app.daemon_mode = true;
    let mut target = RecordingTarget::new();
    let mut should_break = false;
    let plugin_info = std::sync::Arc::new(std::sync::Mutex::new(PluginInfo {
        active_persona: None,
    }));
    let mut cx = runtime::Ctx {
        app: &mut app,
        target: &mut target,
        should_break: &mut should_break,
        cat: None,
        loaded_personas: &[],
        plugin_info: &plugin_info,
    };
    runtime::handle_action(&mut cx, Action::SlashCommand("/project".into())).await;
    assert!(target.calls.contains(&"list_projects"));
    assert_eq!(
        app.picker.as_ref().map(|picker| picker.kind.as_str()),
        Some("project")
    );
}
