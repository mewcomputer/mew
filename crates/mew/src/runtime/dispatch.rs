//! The single dispatch path — all `Action` and `SlashResult` matching lives here.
//!
//! `handle_action` is the only function that matches on `Action` or
//! `SlashResult`. The `deny(clippy::wildcard_enum_match_arm)` lint ensures
//! every variant is explicitly handled — adding a variant breaks the build.

#![deny(clippy::wildcard_enum_match_arm)]

use std::sync::Arc;

use mew_catalog::Catalog;
use mew_hooks::PermissionMode;
use mew_tui::app::Mode as TuiMode;
use mew_tui::events::Action;
use mew_tui::SlashResult;

use crate::commands::tui::{copy_to_clipboard, read_clipboard_image};
use crate::runtime::mentions::process_mentions;
use crate::runtime::target::{CommandTarget, Unsupported};
use crate::setup::personas::persona_summary;

/// Whether to continue or quit the event loop after handling an action.
#[derive(Debug)]
pub enum Flow {
    Continue,
    Quit,
}

/// Context bundle for `handle_action`. Holds all the mutable state the
/// dispatch needs: the App, the CommandTarget (backend), the event-loop
/// sender, settings editor, and config/catalog/personas for model/persona
/// switching.
pub struct Ctx<'a, T: CommandTarget> {
    pub app: &'a mut mew_tui::App,
    pub target: &'a mut T,
    pub event_loop: &'a mew_tui::EventLoop,
    pub should_break: &'a mut bool,
    pub cat: Option<&'a Catalog>,
    pub loaded_personas: &'a [mew_personas::Persona],
    pub plugin_info: &'a Arc<std::sync::Mutex<crate::PluginInfo>>,
}

/// Handle a single `Action`, producing a `Flow` indicating whether to
/// continue or quit. This is the single dispatch path — all `Action` and
/// `SlashResult` matching lives here.
///
/// Every message push goes through `app.push_message()` /
/// `app.push_synthetic_message()` / `app.push_user()` so dirty state is
/// always maintained.
pub async fn handle_action<T: CommandTarget>(cx: &mut Ctx<'_, T>, action: Action) -> Flow {
    match action {
        Action::Submit(text) => {
            handle_submit(cx, text).await;
            Flow::Continue
        }
        Action::SlashCommand(text) => handle_slash_command(cx, text).await,
        Action::Cancel => {
            cx.target.cancel().await;
            // Keep `streaming` true until the turn actually ends (the turn is
            // still winding down). This is the turn-alive guard: submits in
            // this window are queued instead of racing the daemon's
            // "turn in progress" window. The ending MessageEnd/Error event
            // clears `cancelling` + `streaming`.
            cx.app.cancelling = true;
            Flow::Continue
        }
        Action::Quit => {
            *cx.should_break = true;
            Flow::Quit
        }
        Action::Clear => {
            match cx.target.clear().await {
                Ok(()) => {
                    cx.app.clear_messages();
                    cx.app.push_synthetic_message("context cleared".into());
                }
                Err(Unsupported(reason)) => cx.app.set_alert(reason),
            }
            Flow::Continue
        }
        Action::SwitchModel(spec) => {
            handle_switch_model(cx, &spec).await;
            Flow::Continue
        }
        Action::InsertAtMention(mention) => {
            cx.app.insert_mention(&mention);
            Flow::Continue
        }
        Action::InsertNamespaceMention(kind, value) => {
            cx.app.insert_namespace_mention(&kind, &value);
            Flow::Continue
        }
        Action::CopySelection(text) => {
            copy_to_clipboard(&text);
            cx.app.set_alert(format!("copied {} chars", text.len()));
            cx.app.clear_selection();
            Flow::Continue
        }
        Action::ToggleSidebarEnvironment => {
            cx.app.toggle_sidebar_section("environment");
            Flow::Continue
        }
        Action::OpenSettings => {
            open_settings(cx);
            Flow::Continue
        }
        Action::CancelMostRecentSubagent(task_id) => {
            match cx.target.cancel_subagent(&task_id).await {
                Ok(true) => cx.app.set_alert("subagent cancellation requested"),
                Ok(false) => cx.app.set_alert("subagent already finished"),
                Err(Unsupported(reason)) => cx.app.set_alert(reason),
            }
            Flow::Continue
        }
        Action::PersonaSwitchConfirmed(name) => {
            handle_switch_persona(cx, &name).await;
            Flow::Continue
        }
        Action::CyclePersona(delta) => {
            handle_cycle_persona(cx, delta).await;
            Flow::Continue
        }
        Action::SetPermissionMode(mode) => {
            handle_set_permission_mode(cx, mode).await;
            Flow::Continue
        }
        Action::SetThinkingVariant(variant) => {
            handle_set_thinking_variant(cx, &variant).await;
            Flow::Continue
        }
        Action::AttachSession(id) => {
            handle_attach_session(cx, &id).await;
            Flow::Continue
        }
        Action::SendQueuedNow(text) => {
            // Cancel the current turn, then submit the queued message.
            cx.target.cancel().await;
            cx.app.streaming = false;
            cx.app.cancelling = false;
            handle_submit(cx, text).await;
            Flow::Continue
        }
        Action::GuideQueued(text) => {
            // Steer the running turn: inject the queued message as guidance
            // into its next provider request without cancelling. Show it in
            // the transcript so the user sees what was injected.
            if text.trim().is_empty() {
                return Flow::Continue;
            }
            cx.app.push_guidance(text.clone());
            cx.target.guide(text).await;
            Flow::Continue
        }
        Action::PasteClipboardImage => {
            handle_paste_clipboard_image(cx).await;
            Flow::Continue
        }
        Action::NewSessionInProject(path) => {
            match cx.target.new_session_in(&path).await {
                Ok(()) => {
                    // The daemon attaches the client to the new session and
                    // pushes its history; clear the stale display.
                    cx.app.clear_messages();
                    cx.app
                        .push_synthetic_message(format!("new session in {}", path));
                    cx.app.auto_scroll = true;
                    cx.app.scroll = cx.app.max_scroll;
                }
                Err(Unsupported(reason)) => cx.app.set_alert(reason),
            }
            Flow::Continue
        }
        Action::ToggleSessionArchived(id) => {
            let current = cx
                .app
                .daemon_sessions
                .iter()
                .find(|s| s.session_id == id)
                .map(|s| s.archived)
                .unwrap_or(false);
            match cx.target.archive_session(&id, !current).await {
                Ok(()) => {
                    if let Some(s) = cx
                        .app
                        .daemon_sessions
                        .iter_mut()
                        .find(|s| s.session_id == id)
                    {
                        s.archived = !current;
                    }
                    cx.app.push_synthetic_message(format!(
                        "{} session {}",
                        if current { "unarchived" } else { "archived" },
                        id
                    ));
                }
                Err(Unsupported(reason)) => cx.app.set_alert(reason),
            }
            Flow::Continue
        }
        Action::ToggleSessionPinned(id) => {
            let current = cx
                .app
                .daemon_sessions
                .iter()
                .find(|s| s.session_id == id)
                .map(|s| s.pinned)
                .unwrap_or(false);
            match cx.target.pin_session(&id, !current).await {
                Ok(()) => {
                    if let Some(s) = cx
                        .app
                        .daemon_sessions
                        .iter_mut()
                        .find(|s| s.session_id == id)
                    {
                        s.pinned = !current;
                    }
                    cx.app.push_synthetic_message(format!(
                        "{} session {}",
                        if current { "unpinned" } else { "pinned" },
                        id
                    ));
                }
                Err(Unsupported(reason)) => cx.app.set_alert(reason),
            }
            Flow::Continue
        }
    }
}

/// Handle `Action::Submit` — process mentions, push user message, start the turn.
async fn handle_submit<T: CommandTarget>(cx: &mut Ctx<'_, T>, text: String) {
    if cx.app.streaming {
        // Queue the message instead of rejecting it. It will be sent
        // automatically when the current turn finishes, or immediately
        // if the user presses Up-Up.
        if !text.trim().is_empty() {
            cx.app.queued_messages.push(text);
            cx.app.mark_chat_dirty();
        }
        return;
    }
    let text = cx.target.intercept_user_input(text).await;
    let cwd = std::env::current_dir().unwrap_or_default();
    let (enriched, display, attachments) = process_mentions(
        &text,
        &cwd,
        &mut cx.app.context_files,
        &cx.app.skill_catalog,
        &cx.app.subagent_catalog,
    )
    .await;
    cx.app.push_user(display, attachments.clone());
    cx.app.streaming = true;
    let rx = cx.target.prompt(enriched, attachments);
    cx.event_loop.forward_agent_events(rx);
}

/// Handle `Action::PasteClipboardImage` — read image data from the system
/// clipboard, save it to a temp file, and insert it as an @-mention so the
/// existing image attachment pipeline picks it up on submit.
///
/// On SSH connections the system clipboard is the remote machine's
/// clipboard, not the user's local one — detect this and warn early.
async fn handle_paste_clipboard_image<T: CommandTarget>(cx: &mut Ctx<'_, T>) {
    handle_paste_clipboard_image_with(cx, read_clipboard_image).await;
}

/// Shared implementation with an injectable reader so the error branch can
/// be tested without depending on the developer machine's clipboard.
pub(crate) async fn handle_paste_clipboard_image_with<T, F>(cx: &mut Ctx<'_, T>, read_image: F)
where
    T: CommandTarget,
    F: FnOnce() -> Result<std::path::PathBuf, String>,
{
    // SSH detection: if SSH_CONNECTION or SSH_TTY is set, the user is almost
    // certainly on a remote host whose clipboard is not the one they copied
    // the image to.
    if std::env::var("SSH_CONNECTION").is_ok() || std::env::var("SSH_TTY").is_ok() {
        cx.app
            .set_alert("clipboard image paste doesn't work over SSH — save the image to a file and @mention it instead");
        return;
    }

    match read_image() {
        Ok(path) => {
            let path_str = path.to_string_lossy().to_string();
            cx.app.insert_mention(&format!("@{}", path_str));
            cx.app.set_alert("image pasted from clipboard");
        }
        Err(e) => {
            cx.app.set_alert(e);
        }
    }
}

/// Handle `Action::SlashCommand` — parse and route the slash command.
async fn handle_slash_command<T: CommandTarget>(cx: &mut Ctx<'_, T>, text: String) -> Flow {
    let result = cx.app.handle_slash(&text);
    match result {
        SlashResult::Continue => {
            // Unknown slash command — fall through to the model as a normal prompt.
            // This matches the enum doc comment ("fall through to the model").
            if cx.app.streaming {
                cx.app.set_alert("wait for the current response to finish");
                return Flow::Continue;
            }
            let cwd = std::env::current_dir().unwrap_or_default();
            let (enriched, display, attachments) = process_mentions(
                &text,
                &cwd,
                &mut cx.app.context_files,
                &cx.app.skill_catalog,
                &cx.app.subagent_catalog,
            )
            .await;
            cx.app.push_user(display, attachments.clone());
            cx.app.streaming = true;
            let rx = cx.target.prompt(enriched, attachments);
            cx.event_loop.forward_agent_events(rx);
            Flow::Continue
        }
        SlashResult::Quit => {
            *cx.should_break = true;
            Flow::Quit
        }
        SlashResult::Clear => {
            match cx.target.clear().await {
                Ok(()) => {
                    cx.app.clear_messages();
                    cx.app.push_synthetic_message("context cleared".into());
                }
                Err(Unsupported(reason)) => cx.app.set_alert(reason),
            }
            Flow::Continue
        }
        SlashResult::Message(msg) => {
            cx.app.push_synthetic_message(msg);
            Flow::Continue
        }
        SlashResult::Compact => {
            match cx.target.compact().await {
                Ok(()) => {
                    cx.app
                        .push_synthetic_message("compaction will run on next turn".into());
                }
                Err(Unsupported(reason)) => cx.app.set_alert(reason),
            }
            Flow::Continue
        }
        SlashResult::Todo => {
            match cx.target.todos().await {
                Ok(list) => cx.app.push_synthetic_message(list),
                Err(Unsupported(reason)) => cx.app.set_alert(reason),
            }
            Flow::Continue
        }
        SlashResult::SwitchModel(spec) => {
            handle_switch_model(cx, &spec).await;
            Flow::Continue
        }
        SlashResult::SwitchPersona(name) => {
            handle_switch_persona(cx, &name).await;
            Flow::Continue
        }
        SlashResult::PersonaSwitchConfirm(name) => {
            if let Some(persona) = cx.loaded_personas.iter().find(|p| p.name == name) {
                let target = persona_summary(persona);
                let current = cx
                    .app
                    .active_persona
                    .as_ref()
                    .and_then(|cur_name| cx.loaded_personas.iter().find(|p| &p.name == cur_name))
                    .map(persona_summary);
                cx.app.request_persona_switch_confirm(target, current);
            } else {
                cx.app.push_synthetic_message(format!(
                    "unknown persona: {}. use /persona to list available.",
                    name
                ));
            }
            Flow::Continue
        }
        SlashResult::ResumeSession(id) => {
            handle_resume_session(cx, &id).await;
            Flow::Continue
        }
        SlashResult::Rewind(n) => {
            handle_rewind(cx, n).await;
            Flow::Continue
        }
        SlashResult::OpenModelPicker => {
            cx.app.open_command_palette();
            Flow::Continue
        }
        SlashResult::PermissionModeMenu => {
            cx.app.open_permission_mode_picker();
            Flow::Continue
        }
        SlashResult::SetPermissionMode(mode) => {
            handle_set_permission_mode(cx, mode).await;
            Flow::Continue
        }
        SlashResult::SetThinkingVariant(variant) => {
            handle_set_thinking_variant(cx, &variant).await;
            Flow::Continue
        }
        SlashResult::SetTheme(name) => {
            cx.app.theme = mew_tui::theme::Theme::load(&name);
            {
                let mut save = mew_config::load_state().unwrap_or_default();
                save.theme = cx.app.theme.name.clone();
                let _ = mew_config::save_state(&save);
            }
            // cx.app
            //     .push_synthetic_message(format!("theme: {}", cx.app.theme.name));
            Flow::Continue
        }
        SlashResult::ToggleMouseCapture => {
            // Signal the event loop to perform the terminal toggle.
            // The actual toggle needs a Terminal reference, which dispatch
            // doesn't have. The loop checks pending_mouse_toggle after
            // handle_action returns.
            cx.app.pending_mouse_toggle = true;
            Flow::Continue
        }
        SlashResult::PluginCommand { name, args } => {
            match cx.target.plugin_command(&name, &args).await {
                Ok(result) => cx.app.push_synthetic_message(result),
                Err(Unsupported(reason)) => cx.app.set_alert(reason),
            }
            Flow::Continue
        }
        SlashResult::OpenThinkingVariantPicker => {
            cx.app.open_thinking_variant_picker();
            Flow::Continue
        }
        SlashResult::OpenCommandPalette => {
            cx.app.open_command_palette();
            Flow::Continue
        }
        SlashResult::OpenThemePicker => {
            cx.app.open_theme_picker();
            Flow::Continue
        }
        SlashResult::OpenPersonaPicker => {
            cx.app.open_persona_picker();
            Flow::Continue
        }
        SlashResult::OpenRewindPicker => {
            cx.app.open_rewind_picker();
            Flow::Continue
        }
        SlashResult::OpenSessionPickerFromDisk => {
            cx.app.open_session_picker_from_disk();
            Flow::Continue
        }
        SlashResult::OpenSessionPicker => {
            cx.app.open_session_picker();
            Flow::Continue
        }
        SlashResult::OpenHelp => {
            cx.app.mode = mew_tui::app::Mode::Help;
            Flow::Continue
        }
        SlashResult::GoalCommand(cmd) => {
            let action = match cmd {
                mew_tui::app::GGoalCommand::Set(text) => {
                    crate::runtime::target::GoalAction::Set(text)
                }
                mew_tui::app::GGoalCommand::Status => crate::runtime::target::GoalAction::Status,
                mew_tui::app::GGoalCommand::Pause => crate::runtime::target::GoalAction::Pause,
                mew_tui::app::GGoalCommand::Resume => crate::runtime::target::GoalAction::Resume,
                mew_tui::app::GGoalCommand::Clear => crate::runtime::target::GoalAction::Clear,
                mew_tui::app::GGoalCommand::Complete => {
                    crate::runtime::target::GoalAction::Complete
                }
            };
            match cx.target.manage_goal(action).await {
                Ok(msg) => {
                    cx.app.push_synthetic_message(msg);
                }
                Err(Unsupported(reason)) => {
                    cx.app.set_alert(reason);
                }
            }
            Flow::Continue
        }
        SlashResult::SetAutoTitle(enabled) => {
            match cx.target.set_auto_title(enabled).await {
                Ok(()) => {
                    cx.app.push_synthetic_message(format!(
                        "auto-title {}",
                        if enabled { "enabled" } else { "disabled" }
                    ));
                }
                Err(Unsupported(reason)) => {
                    cx.app.set_alert(reason);
                }
            }
            Flow::Continue
        }
        SlashResult::SetAutoSummary(enabled) => {
            match cx.target.set_auto_summary(enabled).await {
                Ok(()) => {
                    cx.app.push_synthetic_message(format!(
                        "auto-summary {}",
                        if enabled { "enabled" } else { "disabled" }
                    ));
                }
                Err(Unsupported(reason)) => {
                    cx.app.set_alert(reason);
                }
            }
            Flow::Continue
        }
        SlashResult::YieldControl => {
            match cx.target.yield_control().await {
                Ok(()) => {
                    cx.app
                        .push_synthetic_message("yielded control to other clients".into());
                }
                Err(Unsupported(reason)) => {
                    cx.app.set_alert(reason);
                }
            }
            Flow::Continue
        }
        SlashResult::UnflagFile(path) => {
            match cx.target.unflag_file(&path).await {
                Ok(()) => {
                    cx.app.flagged_files.retain(|f| f.path != path);
                    cx.app.push_synthetic_message(format!("unflagged {}", path));
                }
                Err(Unsupported(reason)) => {
                    cx.app.set_alert(reason);
                }
            }
            Flow::Continue
        }
        SlashResult::OpenProjectPicker => {
            // The picker opens when the daemon's `ProjectList` response
            // arrives on the notify channel.
            match cx.target.list_projects().await {
                Ok(()) => {}
                Err(Unsupported(reason)) => {
                    cx.app.set_alert(reason);
                }
            }
            Flow::Continue
        }
        SlashResult::RenameSession(title) => {
            let id = cx.app.status.session_id.clone();
            if id.is_empty() {
                cx.app.set_alert("no active session to rename");
                return Flow::Continue;
            }
            match cx.target.rename_session(&id, &title).await {
                Ok(()) => {
                    cx.app.session_titles.insert(id, title.clone());
                    cx.app
                        .push_synthetic_message(format!("session renamed to \"{}\"", title));
                }
                Err(Unsupported(reason)) => {
                    cx.app.set_alert(reason);
                }
            }
            Flow::Continue
        }
    }
}

/// Handle model switching — shared by `Action::SwitchModel` and
/// `SlashResult::SwitchModel`.
async fn handle_switch_model<T: CommandTarget>(cx: &mut Ctx<'_, T>, spec: &str) {
    match cx.target.switch_model(spec).await {
        Ok(switched) => {
            cx.app.status.model = switched.model_id.clone();
            cx.app.status.provider = switched.provider_id.clone();
            // Try to carry over the thinking variant to the new model.
            // Maps by effort level (e.g. "max" on k3 → "high" on a model
            // that only has low/medium/high).
            let prev_variant = cx.app.active_thinking_variant.clone();
            cx.app.active_thinking_variant = None;
            if let Some(ref prev) = prev_variant {
                if let Some(c) = cx.cat {
                    if let Some(mapped) = c.map_variant(prev, &switched.model_id) {
                        // set_thinking resolves the variant via the catalog
                        // and applies the reasoning config to the agent.
                        if cx.target.set_thinking(&mapped.name).await.is_ok() {
                            cx.app.active_thinking_variant = Some(mapped.name);
                        }
                    }
                }
            }
            // Update context window from catalog
            if let Some(c) = cx.cat {
                cx.app.status.context_window = c.context_window(&switched.model_id) as u32;
            }
            // Persist to state
            let mut state = mew_config::load_state().unwrap_or_default();
            state.last_model = switched.model_id.clone();
            state.last_provider = switched.provider_id.clone();
            state.last_thinking_variant = cx.app.active_thinking_variant.clone();
            state.sidebar_collapsed = cx.app.sidebar_collapsed.clone();
            // Record in recent models: move to front, dedupe, cap at 6.
            let full_id = format!("{}/{}", switched.provider_id, switched.model_id);
            cx.app.recent_models.retain(|m| m != &full_id);
            cx.app.recent_models.insert(0, full_id);
            if cx.app.recent_models.len() > 6 {
                cx.app.recent_models.truncate(6);
            }
            state.recent_models = cx.app.recent_models.clone();
            if let Err(e) = mew_config::save_state(&state) {
                tracing::warn!("failed to save state: {}", e);
            }
            cx.app
                .push_synthetic_message(format!("switched to {}", switched.display));
        }
        Err(Unsupported(reason)) => {
            // Include available model IDs in the error so the user knows
            // what they can switch to.
            let available: Vec<&str> = cx.app.models.iter().map(|(id, _)| id.as_str()).collect();
            let hint = if available.is_empty() {
                "no models available — check your config and provider credentials".to_string()
            } else {
                format!(
                    "available models: {}. Use /model (no arg) to open the picker",
                    available.join(", ")
                )
            };
            cx.app
                .push_synthetic_message(format!("failed to switch model: {} — {hint}", reason));
        }
    }
}

/// Handle `SetPermissionMode` — shared by `Action::SetPermissionMode` and
/// `SlashResult::SetPermissionMode`.
async fn handle_set_permission_mode<T: CommandTarget>(cx: &mut Ctx<'_, T>, mode: PermissionMode) {
    match cx.target.set_permission_mode(mode).await {
        Ok(()) => {
            cx.app.permission_mode = mode;
            let alert = match mode {
                PermissionMode::Standard => {
                    "Standard permission mode — prompts restored for Mutating/Dangerous tools."
                        .to_string()
                }
                PermissionMode::Permissive => {
                    "Permissive mode — Mutating tools auto-allow; bash still prompts and your rules still apply."
                        .to_string()
                }
                PermissionMode::Auto => {
                    "Auto mode — small LLM classifier decides each tool call. Falls back to user on escalate."
                        .to_string()
                }
                PermissionMode::AutoPlus => {
                    "Auto+ mode — classifier decides, but escalate or failure means Deny (fail closed). No human in the loop."
                        .to_string()
                }
                PermissionMode::Dangerous => {
                    "⚠ Dangerous! mode — every tool auto-runs; overrides deny rules, ask rules, and the secret-file guard."
                        .to_string()
                }
            };
            cx.app.set_alert(alert);
        }
        Err(Unsupported(reason)) => cx.app.set_alert(reason),
    }
}

/// Handle `SetThinkingVariant` — shared by `Action::SetThinkingVariant` and
/// `SlashResult::SetThinkingVariant`.
async fn handle_set_thinking_variant<T: CommandTarget>(cx: &mut Ctx<'_, T>, variant: &str) {
    match cx.target.set_thinking(variant).await {
        Ok(()) => {
            if variant.is_empty() || variant == "off" || variant == "none" {
                cx.app.active_thinking_variant = None;
                cx.app.set_alert("thinking disabled");
                // Persist: clear the saved variant.
                let mut state = mew_config::load_state().unwrap_or_default();
                state.last_thinking_variant = None;
                let _ = mew_config::save_state(&state);
            } else {
                cx.app.active_thinking_variant = Some(variant.to_string());
                cx.app.set_alert(format!("thinking: {}", variant));
                // Persist the variant.
                let mut state = mew_config::load_state().unwrap_or_default();
                state.last_thinking_variant = Some(variant.to_string());
                let _ = mew_config::save_state(&state);
            }
        }
        Err(Unsupported(reason)) => cx.app.set_alert(reason),
    }
}

/// Handle `SwitchPersona` slash result.
async fn handle_switch_persona<T: CommandTarget>(cx: &mut Ctx<'_, T>, name: &str) {
    let old = cx.app.active_persona.clone();
    match cx.target.switch_persona(name, cx.loaded_personas).await {
        Ok(applied) => {
            if name == "default" || name == "none" {
                cx.app.active_persona = None;
                cx.app.active_persona_color = None;
                cx.plugin_info.lock().unwrap().active_persona = None;
            } else {
                // Update persona display state
                if let Some(persona) = cx.loaded_personas.iter().find(|p| p.name == name) {
                    cx.app.active_persona = Some(persona.name.clone());
                    cx.app.active_persona_color = persona.config.color.clone();
                    cx.plugin_info.lock().unwrap().active_persona = Some(name.to_string());
                }
                // If persona pinned a model, update app status
                if let Some(ref model_str) = applied.pinned_model {
                    let (_, new_model_id) =
                        crate::setup::providers::split_provider_model(model_str, "");
                    cx.app.status.model = new_model_id.clone();
                    if let Some(c) = cx.cat {
                        cx.app.status.context_window = c.context_window(&new_model_id) as u32;
                    }
                }
            }
            cx.app.push_synthetic_message(applied.display);
        }
        Err(Unsupported(_reason)) => {
            cx.app.push_synthetic_message(format!(
                "unknown persona: {}. use /persona to list available.",
                name
            ));
        }
    }
    cx.target.on_persona_change(old.as_deref(), name).await;
}

/// Cycle the active persona by `delta` (+1 forward, -1 backward) through
/// the loaded persona list. The cycle includes "default" (no persona) as the
/// last position, so Shift+Tab from the last persona returns to default.
/// Reuses `handle_switch_persona` so model pinning, accent color, and the
/// synthetic display message all fire identically to `/persona <name>`.
async fn handle_cycle_persona<T: CommandTarget>(cx: &mut Ctx<'_, T>, delta: i32) {
    if cx.app.personas.is_empty() {
        cx.app.set_alert("no personas loaded");
        return;
    }
    // Build the cycle list: loaded persona names, then "default".
    let mut names: Vec<String> = cx.app.personas.iter().map(|(n, _)| n.clone()).collect();
    names.push("default".into());
    let len = names.len();

    // Current position in the cycle. Active persona maps to its index;
    // None (no active persona) maps to the "default" slot at the end.
    let current_idx = match &cx.app.active_persona {
        Some(active) => names.iter().position(|n| n == active).unwrap_or(len - 1),
        None => len - 1, // "default"
    };

    // Wrap around in both directions. delta is +1 or -1.
    let next_idx = (current_idx as i32 + delta).rem_euclid(len as i32) as usize;
    let next_name = names[next_idx].clone();
    handle_switch_persona(cx, &next_name).await;
}

/// Handle `ResumeSession` — load a previous session.
async fn handle_resume_session<T: CommandTarget>(cx: &mut Ctx<'_, T>, id: &str) {
    // In daemon mode the daemon owns session state; attach instead of
    // loading from local disk (the JSONL may not exist on this machine).
    if cx.app.daemon_mode {
        handle_attach_session(cx, id).await;
        return;
    }
    // For local mode, resume loads from disk. The target handles agent state;
    // we handle app display state.
    // We need the loaded messages to repopulate the app display.
    // The LocalTarget loads them into the agent, but doesn't return them.
    // For now, reload from disk for the display.
    match mew_session::Reader::load(id).await {
        Ok(msgs) => {
            // Load into agent (target handles this for non-display state)
            let _ = cx.target.resume(id).await;
            // Update todos on app
            let resumed_todos_path = mew_session::session_dir().join(id).join("todos.json");
            if let Ok(list) = mew_agent::TodoList::load(&resumed_todos_path).await {
                cx.app.todos = list.items.clone();
            }
            // Repopulate display
            cx.app.clear_messages();
            for msg in &msgs {
                cx.app.push_message(msg.clone());
            }
            cx.app.status.session_id = id.to_string();
            cx.app.auto_scroll = true;
            cx.app.scroll = cx.app.max_scroll;
            cx.app
                .push_synthetic_message(format!("resumed session {}", id));
        }
        Err(e) => {
            cx.app
                .push_synthetic_message(format!("failed to load session {}: {}", id, e));
        }
    }
}

/// Handle `Rewind` — truncate to first N messages.
async fn handle_rewind<T: CommandTarget>(cx: &mut Ctx<'_, T>, n: usize) {
    if cx.app.streaming {
        cx.app
            .push_synthetic_message("cannot rewind while streaming".into());
        return;
    }
    if n >= cx.app.messages().len() {
        cx.app
            .push_synthetic_message(format!("only {} messages exist", cx.app.messages().len()));
        return;
    }
    let removed = cx.app.messages().len() - n;
    match cx.target.rewind(n).await {
        Ok(()) => {
            cx.app.rewind_to(n);
            cx.app
                .push_synthetic_message(format!("rewound to message {} (removed {})", n, removed));
        }
        Err(Unsupported(reason)) => cx.app.set_alert(reason),
    }
}

/// Handle `AttachSession` — daemon-mode session switching.
async fn handle_attach_session<T: CommandTarget>(cx: &mut Ctx<'_, T>, id: &str) {
    match cx.target.attach_session(id).await {
        Ok(()) => {
            // Clear the display — the daemon will push new messages
            // for the attached session. Without this, the old session's
            // messages persist and look stale.
            cx.app.clear_messages();
            cx.app
                .push_synthetic_message(format!("attached to session {}", id));
            cx.app.auto_scroll = true;
            cx.app.scroll = cx.app.max_scroll;
        }
        Err(Unsupported(reason)) => cx.app.set_alert(reason),
    }
}

/// Open the settings page with discovered plugins.
fn open_settings<T: CommandTarget>(cx: &mut Ctx<'_, T>) {
    let loader =
        mew_hooks_runtime::PluginLoader::new(mew_hooks_runtime::PluginLoader::default_dirs());
    let state = mew_config::load_state().unwrap_or_default();
    let plugins: Vec<mew_tui::settings::PluginEntry> = loader
        .discover_executables()
        .into_iter()
        .map(|path| {
            let name = path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let enabled = !state.disabled_plugins.contains(&name);
            mew_tui::settings::PluginEntry {
                name,
                path: path.display().to_string(),
                enabled,
            }
        })
        .collect();
    let cfg = mew_config::load().unwrap_or_default();
    cx.app.settings = Some(mew_tui::settings::SettingsState::new(cfg, plugins));
    cx.app.mode = TuiMode::Settings;
}
