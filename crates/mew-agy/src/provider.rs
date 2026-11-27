//! Provider integration for an AGY process with an explicit execution boundary.

#[cfg(unix)]
mod unix {
    use crate::{
        launch::{self, Capabilities},
        mcp::{McpServer, Tool},
        process::{IsolationBoundary, LaunchSpec, ManagedProcess},
        session::{PersistentOptions, Session, SessionConfig, SessionEvent, Turn},
        transcript,
    };
    use async_trait::async_trait;
    use futures::Stream;
    use mew_message::{
        ErrorKind, Finish, MessageError, Part, PartBase, SessionId, TextPart, Tokens, ToolCallPart,
        ToolState, ToolStatePending, ToolTime,
    };
    use mew_provider::{EventStream, Provider, ProviderError, ProviderEvent, Request};
    use serde_json::{json, Value};
    use std::{
        collections::{HashMap, HashSet},
        ffi::OsString,
        path::{Component, Path, PathBuf},
        pin::Pin,
        sync::{Arc, Mutex},
        task::{Context, Poll},
        time::Duration,
    };
    use tokio::sync::{mpsc, oneshot, Mutex as AsyncMutex, OwnedMutexGuard};
    use tokio_util::sync::CancellationToken;
    use ulid::Ulid;

    pub struct Config {
        pub executable: PathBuf,
        pub workspace_parent: PathBuf,
        pub environment: Vec<(OsString, OsString)>,
        pub lifetime: Duration,
    }
    pub struct AgyProvider<B> {
        config: Config,
        boundary: B,
        sessions: Mutex<HashMap<SessionId, Arc<AsyncMutex<Slot>>>>,
        shutdown: CancellationToken,
    }
    #[derive(Default)]
    struct Slot {
        session: Option<Session>,
        pending: Option<Pending>,
        continuity: Option<Continuity>,
    }
    struct Continuity {
        acp_session_id: String,
        session_contract: Value,
        /// The projected history the CLI has already been given. Shared behind
        /// an `Arc` because it is the full transcript and every turn checks it.
        history: Arc<Vec<Value>>,
        after_abandon: bool,
    }
    #[derive(serde::Serialize, serde::Deserialize)]
    struct PersistedContinuity {
        acp_session_id: String,
        session_contract: Value,
        history: Vec<Value>,
        after_abandon: bool,
    }
    fn continuity_path(workspace_parent: &Path, session_id: SessionId) -> PathBuf {
        let base = workspace_parent
            .parent()
            .map(|p| p.join("continuity"))
            .unwrap_or_else(|| workspace_parent.join("continuity"));
        base.join(format!("{session_id}.json"))
    }
    fn load_continuity(workspace_parent: &Path, session_id: SessionId) -> Option<Continuity> {
        let path = continuity_path(workspace_parent, session_id);
        let bytes = std::fs::read(path).ok()?;
        let parsed: PersistedContinuity = serde_json::from_slice(&bytes).ok()?;
        Some(Continuity {
            acp_session_id: parsed.acp_session_id,
            session_contract: parsed.session_contract,
            history: Arc::new(parsed.history),
            after_abandon: parsed.after_abandon,
        })
    }
    fn save_continuity(workspace_parent: &Path, session_id: SessionId, continuity: &Continuity) {
        let path = continuity_path(workspace_parent, session_id);
        if let Some(parent) = path.parent() {
            if let Err(error) = std::fs::create_dir_all(parent) {
                tracing::warn!(%error, "failed to create AGY continuity directory");
                return;
            }
        }
        let payload = PersistedContinuity {
            acp_session_id: continuity.acp_session_id.clone(),
            session_contract: continuity.session_contract.clone(),
            history: (*continuity.history).clone(),
            after_abandon: continuity.after_abandon,
        };
        let Ok(bytes) = serde_json::to_vec(&payload) else {
            return;
        };
        let tmp = path.with_extension("json.tmp");
        if let Err(error) = std::fs::write(&tmp, bytes) {
            tracing::warn!(%error, "failed to write AGY continuity file");
            return;
        }
        if let Err(error) = std::fs::rename(&tmp, &path) {
            let _ = std::fs::remove_file(&tmp);
            tracing::warn!(%error, "failed to commit AGY continuity file");
        }
    }
    fn remove_continuity(workspace_parent: &Path, session_id: SessionId) {
        let _ = std::fs::remove_file(continuity_path(workspace_parent, session_id));
    }
    struct Pending {
        id: String,
        name: String,
        input: Value,
        text: String,
        history: Vec<Value>,
        contract: Value,
        watch_stop: CancellationToken,
    }
    impl<B> Drop for AgyProvider<B> {
        fn drop(&mut self) {
            self.shutdown.cancel();
        }
    }
    impl<B> AgyProvider<B> {
        pub fn new(config: Config, boundary: B) -> Self {
            Self {
                config,
                boundary,
                sessions: Mutex::new(HashMap::new()),
                shutdown: CancellationToken::new(),
            }
        }
    }
    enum Delivery {
        Event(ProviderEvent),
        Handoff(ProviderEvent, oneshot::Sender<()>),
    }
    struct Output {
        rx: mpsc::Receiver<Delivery>,
    }
    impl Stream for Output {
        type Item = ProviderEvent;
        fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
            match self.rx.poll_recv(cx) {
                Poll::Ready(Some(Delivery::Event(event))) => Poll::Ready(Some(event)),
                Poll::Ready(Some(Delivery::Handoff(event, ack))) => {
                    let _ = ack.send(());
                    Poll::Ready(Some(event))
                }
                Poll::Ready(None) => Poll::Ready(None),
                Poll::Pending => Poll::Pending,
            }
        }
    }
    fn invalid(message: impl Into<String>) -> ProviderError {
        ProviderError::Message(message.into())
    }
    fn checked(request: &Request) -> Result<(Value, Vec<Value>, String), ProviderError> {
        if request.session_id.is_none() && !request.tools.is_empty() {
            return Err(invalid("tool calls require a session identity"));
        }
        if request.reasoning.is_some() {
            return Err(invalid("AGY reasoning controls are not supported"));
        }
        if request.params.as_ref().is_some_and(|p| {
            p.temperature.is_some()
                || p.top_p.is_some()
                || p.max_tokens.is_some()
                || p.tool_choice.is_some()
        }) {
            return Err(invalid(
                "AGY sampling and tool-choice controls are not supported",
            ));
        }
        let system = transcript::system_instructions(request).map_err(invalid)?;
        let history: Value = serde_json::from_str(&transcript::project(request).map_err(invalid)?)
            .map_err(ProviderError::Json)?;
        let messages = history["messages"]
            .as_array()
            .ok_or_else(|| invalid("invalid history"))?
            .clone();
        let tools: Vec<_> = request
            .tools
            .iter()
            .map(|t| json!({"name":t.name,"description":t.description,"schema":t.schema}))
            .collect();
        let contract = json!({"model":request.model,"system":system,"tools":tools,"supports_vision":request.supports_vision});
        Ok((contract, messages, system))
    }
    fn continuation(
        p: &Pending,
        contract: &Value,
        messages: &[Value],
    ) -> Result<Value, ProviderError> {
        if &p.contract != contract
            || messages.len() != p.history.len() + 2
            || messages[..p.history.len()] != p.history
        {
            return Err(invalid("AGY continuation contract or history changed"));
        }
        let a = &messages[p.history.len()];
        let u = &messages[p.history.len() + 1];
        let parts = a["content"]
            .as_array()
            .ok_or_else(|| invalid("invalid assistant continuation"))?;
        let uses: Vec<_> = parts.iter().filter(|v| v["type"] == "tool_use").collect();
        let mut text = String::new();
        for part in parts {
            if part["type"] == "text" {
                text.push_str(
                    part["text"]
                        .as_str()
                        .ok_or_else(|| invalid("invalid assistant text"))?,
                );
            } else if part["type"] != "tool_use" {
                return Err(invalid("unexpected assistant continuation"));
            }
        }
        if text != p.text
            || a["role"] != "assistant"
            || uses.len() != 1
            || uses[0]["id"] != p.id
            || uses[0]["name"] != p.name
            || uses[0]["input"] != p.input
        {
            return Err(invalid("AGY tool call continuation mismatch"));
        }
        let content = u["content"]
            .as_array()
            .ok_or_else(|| invalid("invalid tool result"))?;
        let results: Vec<_> = content
            .iter()
            .filter(|v| v["type"] == "tool_result")
            .collect();
        if u["role"] != "user"
            || content.len() != 1
            || results.len() != 1
            || results[0]["tool_use_id"] != p.id
        {
            return Err(invalid("AGY tool result continuation mismatch"));
        }
        let mut text_value = results[0]["content"]
            .as_str()
            .ok_or_else(|| invalid("invalid tool result text"))?
            .to_owned();
        let is_error = results[0]["is_error"]
            .as_bool()
            .ok_or_else(|| invalid("invalid tool result state"))?;
        if !is_error
            && matches!(p.name.as_str(), "read" | "grep")
            && p.input
                .get("path")
                .and_then(Value::as_str)
                .is_some_and(|path| is_spill_output_path(Path::new(path)))
        {
            if let Some(bounded) =
                truncate_spill_tool_result(&p.name, &p.input, &text_value, MAX_SPILL_READ_BYTES)
            {
                text_value = bounded;
            }
        }
        let mut content_blocks = vec![json!({"type": "text", "text": text_value})];
        if let Some(images) = results[0].get("images").and_then(Value::as_array) {
            for image in images {
                let mime = image
                    .get("mime")
                    .and_then(Value::as_str)
                    .ok_or_else(|| invalid("invalid tool result image mime"))?;
                let data = image
                    .get("data")
                    .and_then(Value::as_str)
                    .ok_or_else(|| invalid("invalid tool result image data"))?;
                content_blocks.push(json!({"type": "image", "data": data, "mimeType": mime}));
            }
        }
        Ok(json!({"content": content_blocks, "isError": is_error}))
    }
    fn session_contract_of(contract: &Value) -> Value {
        json!({
            "system": contract["system"],
            "tools": contract["tools"],
            "supports_vision": contract["supports_vision"],
        })
    }
    /// Why a live session cannot be continued with a delta.
    #[derive(Debug, PartialEq, Eq)]
    enum Mismatch {
        ContractChanged,
        HistoryDidNotGrow,
        HistoryPrefixChanged,
        SuffixNotPureUserAppend,
        SuffixHasCompaction,
        SuffixDoesNotEndWithUser,
    }
    fn match_continuity<'a>(
        continuity: &Continuity,
        session_contract: &Value,
        history: &'a [Value],
    ) -> Result<&'a [Value], Mismatch> {
        let seen = continuity.history.as_slice();
        if continuity.session_contract != *session_contract {
            return Err(Mismatch::ContractChanged);
        }
        if history.len() <= seen.len() {
            return Err(Mismatch::HistoryDidNotGrow);
        }
        if history[..seen.len()] != *seen {
            return Err(Mismatch::HistoryPrefixChanged);
        }
        let suffix = &history[seen.len()..];
        if !continuity.after_abandon {
            let pure_user_append = suffix.iter().all(|message| {
                message["role"] == "user"
                    && message["content"].as_array().is_some_and(|parts| {
                        parts.iter().all(|part| {
                            matches!(part["type"].as_str(), Some("text" | "attachment"))
                        })
                    })
            });
            if !pure_user_append {
                return Err(Mismatch::SuffixNotPureUserAppend);
            }
        } else {
            let has_compaction = suffix.iter().any(|message| {
                message["content"].as_array().is_some_and(|parts| {
                    parts.iter().any(|part| {
                        matches!(
                            part["type"].as_str(),
                            Some("compaction" | "preserved_context")
                        )
                    })
                })
            });
            if has_compaction {
                return Err(Mismatch::SuffixHasCompaction);
            }
            let ends_with_user = suffix.last().is_some_and(|message| {
                message["role"] == "user"
                    && message["content"].as_array().is_some_and(|parts| {
                        parts.iter().any(|part| {
                            matches!(part["type"].as_str(), Some("text" | "attachment"))
                        })
                    })
            });
            if !ends_with_user {
                return Err(Mismatch::SuffixDoesNotEndWithUser);
            }
        }
        Ok(suffix)
    }
    const MAX_SPILL_READ_BYTES: usize = 8_000;

    fn env_value(environment: &[(OsString, OsString)], key: &str) -> Option<OsString> {
        environment
            .iter()
            .rev()
            .find(|(k, v)| k == key && !v.is_empty())
            .map(|(_, v)| v.clone())
            .or_else(|| std::env::var_os(key).filter(|v| !v.is_empty()))
    }

    fn app_data_dir(config: &Config) -> Option<PathBuf> {
        if let Some(dir) = env_value(&config.environment, "ANTIGRAVITY_APP_DATA_DIR") {
            return Some(PathBuf::from(dir));
        }
        let home = env_value(&config.environment, "HOME")?;
        Some(PathBuf::from(home).join(".gemini").join("antigravity-cli"))
    }

    fn is_step_dir(component: &std::ffi::OsStr) -> bool {
        component
            .to_str()
            .is_some_and(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
    }

    fn is_spill_output_path(path: &Path) -> bool {
        let normals: Vec<&std::ffi::OsStr> = path
            .components()
            .filter_map(|c| match c {
                Component::Normal(s) => Some(s),
                _ => None,
            })
            .collect();
        let len = normals.len();
        len >= 4
            && normals[len - 4] == ".system_generated"
            && normals[len - 3] == "steps"
            && is_step_dir(normals[len - 2])
            && normals[len - 1] == "output.txt"
    }

    fn matches_session_spill_file(app_data_dir: &Path, conv_ids: &[String], path: &Path) -> bool {
        if conv_ids.is_empty() {
            return false;
        }
        if !std::fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_file()) {
            return false;
        }
        let Ok(canonical_path) = std::fs::canonicalize(path) else {
            return false;
        };
        for conv_id in conv_ids {
            if !crate::mcp::valid_conversation_id(conv_id) {
                continue;
            }
            let steps_dir = app_data_dir
                .join("brain")
                .join(conv_id)
                .join(".system_generated")
                .join("steps");
            let Ok(canonical_steps) = std::fs::canonicalize(&steps_dir) else {
                continue;
            };
            let Ok(rel) = canonical_path.strip_prefix(&canonical_steps) else {
                continue;
            };
            let parts: Vec<Component<'_>> = rel.components().collect();
            if parts.len() == 2
                && matches!(parts[0], Component::Normal(step) if is_step_dir(step))
                && matches!(parts[1], Component::Normal(file) if file == "output.txt")
            {
                return true;
            }
        }
        false
    }

    fn floor_char_boundary(s: &str, max_bytes: usize) -> usize {
        let mut idx = max_bytes.min(s.len());
        while idx > 0 && !s.is_char_boundary(idx) {
            idx -= 1;
        }
        idx
    }

    fn parse_numbered_line_prefix(line: &str) -> Option<usize> {
        let (prefix, _) = line.split_once(':')?;
        if prefix.is_empty() || !prefix.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        prefix.parse().ok()
    }

    fn format_spill_footer(
        tool_name: &str,
        input: &Value,
        kept: &str,
        total_bytes: usize,
    ) -> String {
        let shown = kept.len();
        if tool_name == "read" {
            if let Some(last_line_no) = kept.lines().rev().find_map(parse_numbered_line_prefix) {
                return format!(
                    "\n\n[mew-agy: truncated spill-file read ({shown} of {total_bytes} bytes) to stay below AGY's inline limit. Call `read` with offset={last_line_no} and a smaller limit (e.g. limit=120), or use `grep` on this path, to inspect more.]"
                );
            }
            let next_offset = input
                .get("offset")
                .and_then(Value::as_u64)
                .unwrap_or(0)
                .saturating_add(1);
            return format!(
                "\n\n[mew-agy: truncated long line in spill-file read ({shown} of {total_bytes} bytes) to stay below AGY's inline limit. Use `grep` on this path, or call `read` with offset={next_offset} and a smaller limit to continue.]"
            );
        }
        format!(
            "\n\n[mew-agy: truncated spill-file grep ({shown} of {total_bytes} bytes) to stay below AGY's inline limit. Retry `grep` with a narrower pattern or lower `max_results`.]"
        )
    }

    fn truncate_spill_tool_result(
        tool_name: &str,
        input: &Value,
        text: &str,
        max_bytes: usize,
    ) -> Option<String> {
        if text.len() <= max_bytes {
            return None;
        }
        const FOOTER_RESERVE: usize = 320;
        let body_budget = max_bytes.saturating_sub(FOOTER_RESERVE);
        let window =
            if text.starts_with("[Output offloaded:") || text.starts_with("[Output truncated:") {
                if let Some(omitted_pos) = text.find("\n\n[... middle omitted") {
                    &text[..omitted_pos]
                } else {
                    text
                }
            } else {
                text
            };
        let char_cut = floor_char_boundary(window, body_budget);
        let prefix = &window[..char_cut];
        let mut kept = if char_cut == window.len() {
            prefix
        } else if let Some(nl_pos) = prefix.rfind('\n') {
            let line_prefix = &prefix[..nl_pos];
            let has_usable_line = if tool_name == "read" {
                line_prefix
                    .lines()
                    .any(|line| parse_numbered_line_prefix(line).is_some())
            } else {
                line_prefix.lines().any(|line| !line.is_empty())
            };
            if has_usable_line {
                line_prefix
            } else {
                prefix
            }
        } else {
            prefix
        };

        let mut footer = format_spill_footer(tool_name, input, kept, text.len());
        if kept.len() + footer.len() > max_bytes {
            let trim_to = floor_char_boundary(kept, max_bytes.saturating_sub(footer.len()));
            kept = &kept[..trim_to];
            footer = format_spill_footer(tool_name, input, kept, text.len());
        }
        let mut out = String::with_capacity(kept.len() + footer.len());
        out.push_str(kept);
        out.push_str(&footer);
        Some(out)
    }

    fn idle_timeout(lifetime: Duration) -> Duration {
        lifetime.min(Duration::from_secs(600))
    }
    impl<B: IsolationBoundary + Send + Sync + 'static> AgyProvider<B> {
        async fn spawn_session(
            &self,
            request: &Request,
            system: &str,
            persistent: Option<PersistentOptions>,
        ) -> Result<Session, ProviderError> {
            let mut seen = HashSet::new();
            for tool in &request.tools {
                if !seen.insert(tool.name.as_str()) {
                    return Err(invalid("duplicate tool name"));
                }
            }
            let mut used: HashSet<String> = request
                .tools
                .iter()
                .filter(|tool| crate::mcp::valid_name(&tool.name))
                .map(|tool| tool.name.clone())
                .collect();
            let mut defs = Vec::with_capacity(request.tools.len());
            for (index, tool) in request.tools.iter().enumerate() {
                let alias = if crate::mcp::valid_name(&tool.name) {
                    tool.name.clone()
                } else {
                    let mut candidate = format!("mew_tool_{index}");
                    let mut suffix = 0usize;
                    while used.contains(&candidate) {
                        suffix += 1;
                        candidate = format!("mew_tool_{index}_{suffix}");
                    }
                    used.insert(candidate.clone());
                    candidate
                };
                defs.push(
                    Tool::aliased(&alias, &tool.name, &tool.description, tool.schema.clone())
                        .map_err(invalid)?,
                );
            }
            let tool_pairs: Vec<(String, String)> = defs
                .iter()
                .map(|tool| (tool.name().to_owned(), tool.original_name().to_owned()))
                .collect();
            let tool_refs: Vec<(&str, &str)> = tool_pairs
                .iter()
                .map(|(name, original)| (name.as_str(), original.as_str()))
                .collect();
            let (server, calls) = McpServer::bind(defs, 16).await.map_err(invalid)?;
            let launch = launch::prepare_with_tool_aliases(
                &self.config.workspace_parent,
                &request.model,
                server.url(),
                Capabilities {
                    attachments: true,
                    ..Default::default()
                },
                system,
                &tool_refs,
            )?;
            let cwd = launch.workspace().to_path_buf();
            let cwd_str = cwd
                .to_str()
                .ok_or_else(|| invalid("workspace path is not UTF-8"))?
                .to_owned();
            let (workspace, args) = launch.into_parts();
            let spec = LaunchSpec {
                program: self.config.executable.clone(),
                args: args.into_iter().map(OsString::from).collect(),
                env: self.config.environment.clone(),
                cwd,
            };
            let process =
                ManagedProcess::spawn(spec, &self.boundary, workspace, self.config.lifetime)?;
            let session_config = SessionConfig {
                model: request.model.clone(),
                cwd: cwd_str,
            };
            let session = match persistent {
                Some(options) => Session::new_persistent(
                    process,
                    server,
                    calls,
                    self.config.lifetime,
                    session_config,
                    options,
                )?,
                None => Session::new(process, server, calls, self.config.lifetime, session_config)?,
            };
            Ok(session)
        }
    }
    #[async_trait]
    impl<B: IsolationBoundary + Send + Sync + 'static> Provider for AgyProvider<B> {
        fn name(&self) -> &str {
            "antigravity"
        }
        async fn can_accept_guidance(&self, request: &Request) -> Result<bool, ProviderError> {
            let slot = request.session_id.and_then(|id| {
                self.sessions
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .get(&id)
                    .cloned()
            });
            Ok(match slot {
                Some(slot) => slot.lock().await.pending.is_none(),
                None => true,
            })
        }
        async fn abandon_pending(&self, session_id: SessionId) {
            let slot = self
                .sessions
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(&session_id)
                .cloned();
            let Some(slot) = slot else {
                return;
            };
            let mut guard = slot.lock().await;
            let Some(pending) = guard.pending.take() else {
                return;
            };
            pending.watch_stop.cancel();
            if let Some(session) = guard.session.take() {
                let acp_sid = session.acp_session_id();
                let _ = session.abandon().await;
                if let Some(acp_session_id) = acp_sid {
                    let next = Continuity {
                        acp_session_id,
                        session_contract: session_contract_of(&pending.contract),
                        history: Arc::new(pending.history),
                        after_abandon: true,
                    };
                    save_continuity(&self.config.workspace_parent, session_id, &next);
                    guard.continuity = Some(next);
                } else {
                    guard.continuity = None;
                    remove_continuity(&self.config.workspace_parent, session_id);
                }
            } else {
                guard.continuity = None;
                remove_continuity(&self.config.workspace_parent, session_id);
            }
        }
        async fn allows_readonly_spill_path(&self, session_id: SessionId, path: &Path) -> bool {
            let slot = self
                .sessions
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(&session_id)
                .cloned();
            let Some(slot) = slot else {
                return false;
            };
            let guard = slot.lock().await;
            let mut conv_ids = guard
                .session
                .as_ref()
                .map(Session::conversation_ids)
                .unwrap_or_default();
            if let Some(continuity) = guard.continuity.as_ref() {
                if !continuity.acp_session_id.is_empty()
                    && !conv_ids.contains(&continuity.acp_session_id)
                {
                    conv_ids.push(continuity.acp_session_id.clone());
                }
            }
            drop(guard);
            let Some(app_data_dir) = app_data_dir(&self.config) else {
                return false;
            };
            matches_session_spill_file(&app_data_dir, &conv_ids, path)
        }
        async fn replay_budget(
            &self,
            request: &Request,
        ) -> Result<Option<(usize, usize)>, ProviderError> {
            let slot = request.session_id.and_then(|id| {
                self.sessions
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .get(&id)
                    .cloned()
            });
            let continuity = if let Some(slot) = slot {
                let guard = slot.lock().await;
                if guard.pending.is_some() {
                    return Ok(None);
                }
                guard.continuity.as_ref().map(|c| {
                    (
                        c.session_contract.clone(),
                        Arc::clone(&c.history),
                        c.after_abandon,
                    )
                })
            } else if let Some(sid) = request.session_id {
                load_continuity(&self.config.workspace_parent, sid)
                    .map(|c| (c.session_contract, c.history, c.after_abandon))
            } else {
                None
            };
            if let Some((session_contract, seen_history, after_abandon)) = continuity {
                if let Ok((contract, history, _)) = checked(request) {
                    let temp = Continuity {
                        acp_session_id: String::new(),
                        session_contract,
                        history: seen_history,
                        after_abandon,
                    };
                    if let Ok(suffix) =
                        match_continuity(&temp, &session_contract_of(&contract), &history)
                    {
                        let req_for_size = request.clone();
                        let seen_len = temp.history.len();
                        let suffix_owned = suffix.to_vec();
                        let budget = tokio::task::spawn_blocking(move || {
                            transcript::delta_replay_budget(
                                &req_for_size,
                                seen_len,
                                &suffix_owned,
                                after_abandon,
                                crate::session::MAX_INPUT,
                            )
                        })
                        .await
                        .map_err(|error| invalid(format!("replay size task failed: {error}")))?
                        .map_err(invalid)?;
                        return Ok(Some(budget));
                    }
                }
            }
            let req_for_size = request.clone();
            let budget = tokio::task::spawn_blocking(move || {
                transcript::replay_budget(&req_for_size, crate::session::MAX_INPUT)
            })
            .await
            .map_err(|error| invalid(format!("replay size task failed: {error}")))?
            .map_err(invalid)?;
            Ok(Some(budget))
        }
        async fn stream_auxiliary(
            &self,
            mut request: Request,
        ) -> Result<EventStream, ProviderError> {
            if request.session_id.is_some() || !request.tools.is_empty() {
                return Err(invalid(
                    "AGY auxiliary requests must be sessionless and tool-free",
                ));
            }
            if request.reasoning.as_ref().is_some_and(|reasoning| {
                reasoning.params.len() != 1
                    || reasoning.params.get("type") != Some(&json!("disabled"))
            }) {
                return Err(invalid("AGY reasoning controls are not supported"));
            }
            if request
                .params
                .as_ref()
                .is_some_and(|params| params.top_p.is_some() || params.tool_choice.is_some())
            {
                return Err(invalid(
                    "AGY sampling and tool-choice controls are not supported",
                ));
            }
            request.reasoning = None;
            request.params = None;
            self.stream(request).await
        }
        async fn stream(&self, request: Request) -> Result<EventStream, ProviderError> {
            let (contract, history, system) = match checked(&request) {
                Ok(checked) => checked,
                Err(error) => {
                    let existing = request.session_id.and_then(|id| {
                        self.sessions
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .get(&id)
                            .cloned()
                    });
                    if let Some(slot) = existing {
                        let mut slot = slot.lock().await;
                        slot.continuity = None;
                        if let Some(pending) = slot.pending.take() {
                            pending.watch_stop.cancel();
                        }
                        if let Some(session) = slot.session.take() {
                            let _ = session.shutdown().await;
                        }
                    }
                    return Err(error);
                }
            };
            if request.cancel_token.is_cancelled() {
                return Err(invalid("turn cancelled"));
            }
            let slot = if let Some(id) = request.session_id {
                let mut sessions = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
                sessions.retain(|_, slot| {
                    Arc::strong_count(slot) > 1
                        || slot.try_lock().map_or(true, |entry| {
                            entry.session.is_some() || entry.continuity.is_some()
                        })
                });
                if !sessions.contains_key(&id) && sessions.len() >= 128 {
                    return Err(invalid("AGY session capacity exceeded"));
                }
                sessions
                    .entry(id)
                    .or_insert_with(|| Arc::new(AsyncMutex::new(Slot::default())))
                    .clone()
            } else {
                Arc::new(AsyncMutex::new(Slot::default()))
            };
            let mut guard = slot.lock_owned().await;
            let turn = if let Some(pending) = guard.pending.as_ref() {
                let result = match continuation(pending, &contract, &history) {
                    Ok(v) => v,
                    Err(e) => {
                        guard.continuity = None;
                        if let Some(session) = guard.session.take() {
                            let _ = session.shutdown().await;
                        }
                        if let Some(pending) = guard.pending.take() {
                            pending.watch_stop.cancel();
                        }
                        return Err(e);
                    }
                };
                let id = pending.id.clone();
                let resumed = guard
                    .session
                    .as_ref()
                    .ok_or_else(|| invalid("AGY session unavailable"))?
                    .resume(&id, result)
                    .await;
                if let Some(pending) = guard.pending.take() {
                    pending.watch_stop.cancel();
                }
                match resumed {
                    Ok(turn) => turn,
                    Err(error) => {
                        guard.continuity = None;
                        if let Some(session) = guard.session.take() {
                            let _ = session.shutdown().await;
                        }
                        return Err(error.into());
                    }
                }
            } else {
                let session_contract = session_contract_of(&contract);
                if guard.continuity.is_none() {
                    if let Some(sid) = request.session_id {
                        guard.continuity = load_continuity(&self.config.workspace_parent, sid);
                    }
                }
                let matched = if request.session_id.is_some() {
                    guard.continuity.as_ref().and_then(|continuity| {
                        match match_continuity(continuity, &session_contract, &history) {
                            Ok(suffix) => Some((
                                continuity.acp_session_id.clone(),
                                continuity.history.len(),
                                suffix.to_vec(),
                                continuity.after_abandon,
                            )),
                            Err(reason) => {
                                // A silent fall back to a full replay costs a
                                // whole replayed transcript per turn, so record
                                // why the live session could not be reused.
                                tracing::debug!(
                                    ?reason,
                                    seen = continuity.history.len(),
                                    current = history.len(),
                                    "AGY continuity not reused"
                                );
                                None
                            }
                        }
                    })
                } else {
                    None
                };

                if let Some((acp_session_id, seen_len, suffix_owned, after_abandon)) = matched {
                    let req_for_delta = request.clone();
                    let (delta_blocks, delta_text_size) = tokio::task::spawn_blocking(move || {
                        transcript::build_delta_prompt_blocks(
                            &req_for_delta,
                            seen_len,
                            &suffix_owned,
                            after_abandon,
                        )
                    })
                    .await
                    .map_err(|error| invalid(format!("delta build task failed: {error}")))?
                    .map_err(invalid)?;
                    if delta_text_size > transcript::MAX_REPLAY_TEXT {
                        return Err(invalid("AGY transcript replay exceeds text limit"));
                    }

                    let mut live_turn = None;
                    if !after_abandon {
                        if let Some(session) = guard.session.as_ref() {
                            match session
                                .prompt(
                                    delta_blocks.clone(),
                                    request.model.clone(),
                                    request.cancel_token.clone(),
                                )
                                .await
                            {
                                Ok(turn) => live_turn = Some(turn),
                                Err(_) => {
                                    if let Some(dead) = guard.session.take() {
                                        let _ = dead.abandon().await;
                                    }
                                }
                            }
                        }
                    }

                    if let Some(turn) = live_turn {
                        tracing::debug!(
                            text_size = delta_text_size,
                            block_count = delta_blocks.len(),
                            "reusing live AGY session"
                        );
                        turn
                    } else {
                        if let Some(old) = guard.session.take() {
                            let _ = old.abandon().await;
                        }
                        let req_for_fallback = request.clone();
                        let fallback_prompt = tokio::task::spawn_blocking(move || {
                            transcript::build_prompt_blocks(&req_for_fallback)
                        })
                        .await
                        .map_err(|error| invalid(format!("prompt build task failed: {error}")))?
                        .ok()
                        .and_then(|(blocks, text_size)| {
                            (text_size <= transcript::MAX_REPLAY_TEXT).then_some(blocks)
                        });
                        let session = self
                            .spawn_session(
                                &request,
                                &system,
                                Some(PersistentOptions {
                                    resume_session_id: Some(acp_session_id),
                                    fallback_prompt,
                                    parent_shutdown: self.shutdown.clone(),
                                    idle_timeout: idle_timeout(self.config.lifetime),
                                }),
                            )
                            .await?;
                        let turn = session
                            .start(delta_blocks, request.cancel_token.clone())
                            .await?;
                        guard.session = Some(session);
                        turn
                    }
                } else {
                    if let Some(old) = guard.session.take() {
                        let _ = old.abandon().await;
                    }
                    guard.continuity = None;
                    if let Some(sid) = request.session_id {
                        remove_continuity(&self.config.workspace_parent, sid);
                    }
                    let req_for_blocks = request.clone();
                    let (prompt, text_size) = tokio::task::spawn_blocking(move || {
                        transcript::build_prompt_blocks(&req_for_blocks)
                    })
                    .await
                    .map_err(|error| invalid(format!("prompt build task failed: {error}")))?
                    .map_err(invalid)?;
                    if text_size > transcript::MAX_REPLAY_TEXT {
                        return Err(invalid("AGY transcript replay exceeds text limit"));
                    }
                    if text_size > transcript::MAX_REPLAY_TEXT * 4 / 5 {
                        tracing::warn!(
                            text_size,
                            limit = transcript::MAX_REPLAY_TEXT,
                            system_bytes = system.len(),
                            block_count = prompt.len(),
                            "AGY transcript replay approaching silent truncation ceiling"
                        );
                    } else {
                        tracing::debug!(
                            text_size,
                            system_bytes = system.len(),
                            block_count = prompt.len(),
                            "prepared AGY session prompt"
                        );
                    }
                    let persistent = request.session_id.map(|_| PersistentOptions {
                        resume_session_id: None,
                        fallback_prompt: None,
                        parent_shutdown: self.shutdown.clone(),
                        idle_timeout: idle_timeout(self.config.lifetime),
                    });
                    let session = self.spawn_session(&request, &system, persistent).await?;
                    let turn = session.start(prompt, request.cancel_token.clone()).await?;
                    guard.session = Some(session);
                    turn
                }
            };
            let (tx, rx) = mpsc::channel(16);
            tokio::spawn(deliver(
                turn,
                guard,
                request,
                contract,
                history,
                tx,
                self.shutdown.clone(),
                self.config.workspace_parent.clone(),
            ));
            Ok(Box::pin(Output { rx }))
        }
    }
    #[allow(clippy::too_many_arguments)]
    async fn deliver(
        mut turn: Turn,
        mut slot: OwnedMutexGuard<Slot>,
        request: Request,
        contract: Value,
        history: Vec<Value>,
        tx: mpsc::Sender<Delivery>,
        shutdown: CancellationToken,
        workspace_parent: PathBuf,
    ) {
        macro_rules! send {
            ($event:expr) => {
                tokio::select! {
                    biased;
                    _ = request.cancel_token.cancelled() => false,
                    _ = shutdown.cancelled() => false,
                    result = tx.send($event) => result.is_ok(),
                }
            };
        }
        let session_id = match request.session_id {
            Some(id) => id,
            None => Ulid::new(),
        };
        let message_id = Ulid::new();
        let mut text_id = None;
        let mut emitted_text = String::new();
        let mut usage = Tokens::default();
        let failure = loop {
            let event = tokio::select! {biased; _=tx.closed()=>break "provider stream dropped", _=shutdown.cancelled()=>break "provider stopped", e=turn.next()=>e, _=request.cancel_token.cancelled()=>break "turn cancelled"};
            match event {
                Ok(Some(SessionEvent::Text(text))) => {
                    emitted_text.push_str(&text);
                    let id = if let Some(id) = text_id {
                        id
                    } else {
                        let base = PartBase {
                            id: Ulid::new(),
                            message_id,
                            session_id,
                        };
                        let id = base.id;
                        if !send!(Delivery::Event(ProviderEvent::PartStart {
                            part: Part::Text(TextPart {
                                base,
                                text: String::new(),
                                synthetic: false,
                            }),
                        })) {
                            break "provider stream dropped";
                        }
                        text_id = Some(id);
                        id
                    };
                    if !send!(Delivery::Event(ProviderEvent::PartDelta {
                        part_id: id,
                        field: "text",
                        delta: text,
                    })) {
                        break "provider stream dropped";
                    }
                }
                Ok(Some(SessionEvent::Usage {
                    input,
                    output,
                    cache_read,
                })) => {
                    usage.input = input.min(u32::MAX as u64) as u32;
                    usage.output = output.min(u32::MAX as u64) as u32;
                    usage.cache_read = cache_read.min(u32::MAX as u64) as u32;
                }
                Ok(Some(SessionEvent::ToolCall(call))) => {
                    if request.session_id.is_none() {
                        break "tool call has no session identity";
                    }
                    if let Some(id) = text_id.take() {
                        if !send!(Delivery::Event(ProviderEvent::PartEnd { part_id: id })) {
                            break "provider stream dropped";
                        }
                    }
                    let base = PartBase {
                        id: Ulid::new(),
                        message_id,
                        session_id,
                    };
                    let part_id = base.id;
                    let part = ToolCallPart {
                        base,
                        tool_name: call.name.clone(),
                        call_id: call.id.clone(),
                        state: ToolState::Pending(ToolStatePending {
                            input: call.arguments.clone(),
                            time: ToolTime {
                                start: 0,
                                end: None,
                            },
                        }),
                        raw_input: String::new(),
                    };
                    if !send!(Delivery::Event(ProviderEvent::PartStart {
                        part: Part::ToolCall(part),
                    })) || !send!(Delivery::Event(ProviderEvent::PartEnd { part_id }))
                    {
                        break "provider stream dropped";
                    }
                    let (ack, received) = oneshot::channel();
                    if !send!(Delivery::Handoff(
                        ProviderEvent::MessageEnd {
                            finish: Finish::ToolUse,
                            usage,
                            cost: 0.0,
                        },
                        ack,
                    )) {
                        break "provider stream dropped";
                    }
                    let accepted = tokio::select! {biased; result=received=>result.is_ok(), _=request.cancel_token.cancelled()=>false, _=shutdown.cancelled()=>false, _=tx.closed()=>false};
                    if !accepted || turn.ack_handoff(&call.id).await.is_err() {
                        break "tool handoff failed";
                    }
                    let watch_stop = CancellationToken::new();
                    let slot_ref = OwnedMutexGuard::mutex(&slot).clone();
                    let turn_cancel = request.cancel_token.clone();
                    let call_id = call.id.clone();
                    let stop = watch_stop.clone();
                    slot.pending = Some(Pending {
                        id: call.id,
                        name: call.name,
                        input: call.arguments,
                        text: emitted_text,
                        history,
                        contract,
                        watch_stop,
                    });
                    tokio::spawn(async move {
                        tokio::select! { biased; _ = stop.cancelled() => return, _ = turn_cancel.cancelled() => {}, _ = shutdown.cancelled() => {} }
                        let mut slot = slot_ref.lock().await;
                        if slot
                            .pending
                            .as_ref()
                            .is_some_and(|pending| pending.id == call_id)
                        {
                            slot.pending = None;
                            if let Some(session) = slot.session.take() {
                                let _ = session.shutdown().await;
                            }
                        }
                    });
                    return;
                }
                Ok(Some(SessionEvent::Finished)) => {
                    if let Some(id) = text_id.take() {
                        if !send!(Delivery::Event(ProviderEvent::PartEnd { part_id: id })) {
                            break "provider stream dropped";
                        }
                    } else {
                        let base = PartBase {
                            id: Ulid::new(),
                            message_id,
                            session_id,
                        };
                        let part_id = base.id;
                        if !send!(Delivery::Event(ProviderEvent::PartStart {
                            part: Part::Text(TextPart {
                                base,
                                text: String::new(),
                                synthetic: false,
                            }),
                        })) {
                            break "provider stream dropped";
                        }
                        if !send!(Delivery::Event(ProviderEvent::PartEnd { part_id })) {
                            break "provider stream dropped";
                        }
                    }
                    if request.session_id.is_none() {
                        let cleanup = if let Some(session) = slot.session.take() {
                            session.shutdown().await
                        } else {
                            Err(std::io::Error::other("session unavailable"))
                        };
                        if cleanup.is_err() {
                            break "AGY process cleanup failed";
                        }
                    } else if let Some(sid) = request.session_id {
                        let acp_sid = slot.session.as_ref().and_then(Session::acp_session_id);
                        if let Some(acp_session_id) = acp_sid {
                            let mut seen_history = history;
                            seen_history.push(json!({
                                "role": "assistant",
                                "content": [{"type": "text", "text": emitted_text}]
                            }));
                            let next = Continuity {
                                acp_session_id,
                                session_contract: session_contract_of(&contract),
                                history: Arc::new(seen_history),
                                after_abandon: false,
                            };
                            save_continuity(&workspace_parent, sid, &next);
                            slot.continuity = Some(next);
                        } else {
                            slot.continuity = None;
                            remove_continuity(&workspace_parent, sid);
                        }
                    }
                    let _ = send!(Delivery::Event(ProviderEvent::MessageEnd {
                        finish: Finish::Stop,
                        usage,
                        cost: 0.0,
                    }));
                    return;
                }
                Ok(None) => {
                    break "AGY session ended unexpectedly";
                }
                Err(error) => {
                    break match error.to_string().as_str() {
                        "AGY quota or rate limit reached" => "AGY quota or rate limit reached",
                        "AGY authentication failed; run 'mew auth login antigravity'" => {
                            "AGY authentication failed; run 'mew auth login antigravity'"
                        }
                        "AGY service is overloaded" => "AGY service is overloaded",
                        "AGY prompt was cancelled" => "AGY prompt was cancelled",
                        "AGY refused the prompt" => "AGY refused the prompt",
                        _ => "AGY session failed",
                    };
                }
            }
        };
        drop(turn);
        slot.pending = None;
        slot.continuity = None;
        if let Some(sid) = request.session_id {
            remove_continuity(&workspace_parent, sid);
        }
        if let Some(session) = slot.session.take() {
            let _ = session.shutdown().await;
        }
        drop(slot);
        let error = Delivery::Event(ProviderEvent::Error(MessageError {
            kind: ErrorKind::ProviderApi,
            message: failure.into(),
        }));
        if let Err(mpsc::error::TrySendError::Full(error)) = tx.try_send(error) {
            let _ = send!(error);
        }
    }
}
#[cfg(unix)]
pub use unix::{AgyProvider, Config};
