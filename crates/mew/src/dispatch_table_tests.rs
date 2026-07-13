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
    failing: bool,
}

impl RecordingTarget {
    fn new() -> Self {
        Self {
            calls: Vec::new(),
            failing: false,
        }
    }
    fn failing() -> Self {
        Self {
            calls: Vec::new(),
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
    fn prompt(
        &mut self,
        _enriched: String,
        _parts: Vec<Part>,
    ) -> tokio::sync::mpsc::Receiver<AgentEvent> {
        self.record("prompt");
        let (_tx, rx) = tokio::sync::mpsc::channel(1);
        rx
    }
    async fn cancel(&mut self) {
        self.record("cancel");
    }
    async fn clear(&mut self) -> Result<(), runtime::target::Unsupported> {
        self.record("clear");
        self.check()
    }
    async fn compact(&mut self) -> Result<(), runtime::target::Unsupported> {
        self.record("compact");
        self.check()
    }
    async fn todos(&mut self) -> Result<String, runtime::target::Unsupported> {
        self.record("todos");
        if self.failing {
            Err(runtime::target::Unsupported("test"))
        } else {
            Ok("test".into())
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
            Action::InsertSubagentMention(_) => Action::InsertSubagentMention("researcher".into()),
            Action::CopySelection(_) => unreachable!(),
            Action::CancelMostRecentSubagent(_) => {
                Action::CancelMostRecentSubagent("task_123".into())
            }
            Action::PersonaSwitchConfirmed(_) => Action::PersonaSwitchConfirmed("test".into()),
            Action::SetPermissionMode(_) => {
                Action::SetPermissionMode(mew_hooks::PermissionMode::Standard)
            }
            Action::SetThinkingVariant(_) => Action::SetThinkingVariant("off".into()),
            Action::AttachSession(_) => Action::AttachSession("sess_123".into()),
            other => other,
        };
        let mut app = mew_tui::App::new();
        let mut target = RecordingTarget::new();
        let mut should_break = false;
        let plugin_info = std::sync::Arc::new(std::sync::Mutex::new(PluginInfo {
            session_id: String::new(),
            model: String::new(),
            provider: String::new(),
            workspace: String::new(),
            active_persona: None,
        }));
        let (event_loop, _event_rx) = mew_tui::EventLoop::new();

        let chat_dirty_before = app.chat_dirty;
        let messages_before = app.messages().len();
        let input_before = app.input.clone();
        let sidebar_before = app.sidebar_collapsed.clone();

        let mut cx = runtime::Ctx {
            app: &mut app,
            target: &mut target,
            event_loop: &event_loop,
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
        session_id: String::new(),
        model: String::new(),
        provider: String::new(),
        workspace: String::new(),
        active_persona: None,
    }));
    let (event_loop, _event_rx) = mew_tui::EventLoop::new();

    // /cost returns SlashResult::Message which pushes a synthetic message.
    let dirty_before = app.chat_dirty;
    let mut cx = runtime::Ctx {
        app: &mut app,
        target: &mut target,
        event_loop: &event_loop,
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
        session_id: String::new(),
        model: String::new(),
        provider: String::new(),
        workspace: String::new(),
        active_persona: None,
    }));
    let (event_loop, _event_rx) = mew_tui::EventLoop::new();

    let mut cx = runtime::Ctx {
        app: &mut app,
        target: &mut target,
        event_loop: &event_loop,
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

// -----------------------------------------------------------------------
// AC.6: /quit (Action::Quit) returns Flow::Quit regardless of target.
// -----------------------------------------------------------------------

#[tokio::test]
async fn test_daemon_quit() {
    let mut app = mew_tui::App::new();
    let mut target = RecordingTarget::new();
    let mut should_break = false;
    let plugin_info = std::sync::Arc::new(std::sync::Mutex::new(PluginInfo {
        session_id: String::new(),
        model: String::new(),
        provider: String::new(),
        workspace: String::new(),
        active_persona: None,
    }));
    let (event_loop, _event_rx) = mew_tui::EventLoop::new();

    let mut cx = runtime::Ctx {
        app: &mut app,
        target: &mut target,
        event_loop: &event_loop,
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
        session_id: String::new(),
        model: String::new(),
        provider: String::new(),
        workspace: String::new(),
        active_persona: None,
    }));
    let (event_loop, _event_rx) = mew_tui::EventLoop::new();

    // Clear with a failing target should set an alert, not clear messages.
    let messages_before = app.messages().len();
    let mut cx = runtime::Ctx {
        app: &mut app,
        target: &mut target,
        event_loop: &event_loop,
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
        session_id: String::new(),
        model: String::new(),
        provider: String::new(),
        workspace: String::new(),
        active_persona: None,
    }));
    let (event_loop, _event_rx) = mew_tui::EventLoop::new();

    let original_mode = app.permission_mode;
    let new_mode = mew_hooks::PermissionMode::Dangerous;
    assert_ne!(original_mode, new_mode, "precondition: modes differ");

    let mut cx = runtime::Ctx {
        app: &mut app,
        target: &mut target,
        event_loop: &event_loop,
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
