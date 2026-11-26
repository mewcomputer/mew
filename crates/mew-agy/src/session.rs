//! Coordination of one isolated ACP CLI process and its client-owned MCP calls.

#[cfg(unix)]
mod unix {
    use std::{
        collections::VecDeque,
        future::pending,
        io,
        os::fd::AsRawFd,
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc, Mutex,
        },
        time::Duration,
    };

    use serde_json::{json, Value};
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        sync::{mpsc, oneshot},
    };
    use tokio_util::sync::CancellationToken;

    use crate::{
        mcp::{McpServer, ToolCall},
        process::ManagedProcess,
        stream::{AcpFrame, AcpStream, RpcError},
    };

    const MAX_FRAME: usize = 1_048_576;
    pub(crate) const MAX_INPUT: usize = 16 * 1_048_576;
    const EVENTS: usize = 16;
    const COMMANDS: usize = 8;
    /// Bounded wait for in-flight MCP tool calls after the CLI ends its turn.
    const SETTLE_GRACE_MS: u64 = 200;

    #[derive(Debug, Clone, Default)]
    pub struct SessionConfig {
        pub model: String,
        pub cwd: String,
    }

    #[derive(Debug)]
    pub enum SessionEvent {
        Text(String),
        Usage {
            input: u64,
            output: u64,
            cache_read: u64,
        },
        ToolCall(ToolCall),
        Finished,
    }

    type Reply<T> = oneshot::Sender<io::Result<T>>;
    struct Active {
        events: Option<mpsc::Sender<io::Result<SessionEvent>>>,
        turn_cancel: CancellationToken,
        consumer_gone: CancellationToken,
    }
    struct PendingCall {
        id: String,
        cancelled: CancellationToken,
        acknowledged: bool,
    }
    enum Phase {
        Idle,
        WaitInit {
            prompt: Vec<Value>,
        },
        WaitAuth {
            prompt: Vec<Value>,
        },
        WaitNew {
            rpc_id: u64,
            prompt: Vec<Value>,
        },
        WaitResume {
            rpc_id: u64,
            sid: String,
            prompt: Vec<Value>,
            fallback_prompt: Option<Vec<Value>>,
        },
        WaitSetModel {
            rpc_id: u64,
            sid: String,
            target_model: String,
            prompt: Vec<Value>,
            prompt_id: u64,
        },
        RunningPrompt {
            rpc_id: u64,
            sid: String,
            current_model: String,
        },
        Ready {
            sid: String,
            current_model: String,
        },
    }
    impl Phase {
        fn label(&self) -> &'static str {
            match self {
                Self::Idle => "idle",
                Self::WaitInit { .. } => "initialize",
                Self::WaitAuth { .. } => "authenticate",
                Self::WaitNew { .. } => "session/new",
                Self::WaitResume { .. } => "session/resume",
                Self::WaitSetModel { .. } => "session/set_model",
                Self::RunningPrompt { .. } => "session/prompt",
                Self::Ready { .. } => "ready",
            }
        }
        fn session_id(&self) -> Option<&str> {
            match self {
                Self::WaitResume { sid, .. }
                | Self::WaitSetModel { sid, .. }
                | Self::RunningPrompt { sid, .. }
                | Self::Ready { sid, .. } => Some(sid),
                _ => None,
            }
        }
    }

    enum Command {
        Start {
            prompt: Vec<Value>,
            cancel: CancellationToken,
            consumer: CancellationToken,
            events: mpsc::Sender<io::Result<SessionEvent>>,
            reply: Reply<()>,
        },
        Prompt {
            prompt: Vec<Value>,
            model: String,
            cancel: CancellationToken,
            consumer: CancellationToken,
            events: mpsc::Sender<io::Result<SessionEvent>>,
            reply: Reply<()>,
        },
        Resume {
            id: String,
            result: Value,
            consumer: CancellationToken,
            events: mpsc::Sender<io::Result<SessionEvent>>,
            reply: Reply<()>,
        },
        Ack {
            id: String,
            reply: Reply<()>,
        },
    }

    pub(crate) struct PersistentOptions {
        pub resume_session_id: Option<String>,
        pub fallback_prompt: Option<Vec<Value>>,
        pub parent_shutdown: CancellationToken,
        pub idle_timeout: Duration,
    }

    pub struct Session {
        commands: mpsc::Sender<Command>,
        shutdown: CancellationToken,
        suppress_turn_cancel: Arc<AtomicBool>,
        acp_session_id: Arc<Mutex<Option<String>>>,
        root_conversation_id: Arc<Mutex<Option<String>>>,
        actor: tokio::sync::Mutex<Option<tokio::task::JoinHandle<io::Result<()>>>>,
    }
    pub struct Turn {
        events: mpsc::Receiver<io::Result<SessionEvent>>,
        commands: mpsc::Sender<Command>,
        consumer_gone: CancellationToken,
        handoff_id: Option<String>,
        done: bool,
    }
    impl Turn {
        pub async fn next(&mut self) -> io::Result<Option<SessionEvent>> {
            match self.events.recv().await {
                Some(Ok(event)) => {
                    if let SessionEvent::ToolCall(call) = &event {
                        self.handoff_id = Some(call.id.clone());
                    }
                    if matches!(event, SessionEvent::Finished) {
                        self.done = true;
                    }
                    Ok(Some(event))
                }
                Some(Err(error)) => {
                    self.done = true;
                    Err(error)
                }
                None if self.done => Ok(None),
                None => {
                    self.done = true;
                    Err(invalid("session ended without a terminal event"))
                }
            }
        }
        /// Call only after the consumer receives its terminal ToolUse event.
        pub async fn ack_handoff(&mut self, id: &str) -> io::Result<()> {
            if self.handoff_id.as_deref() != Some(id) || self.done {
                return Err(invalid("handoff does not match delivered call"));
            }
            let (tx, rx) = oneshot::channel();
            self.commands
                .send(Command::Ack {
                    id: id.to_owned(),
                    reply: tx,
                })
                .await
                .map_err(|_| invalid("session closed"))?;
            rx.await.map_err(|_| invalid("session closed"))??;
            self.done = true;
            Ok(())
        }
    }
    impl Drop for Turn {
        fn drop(&mut self) {
            if !self.done {
                self.consumer_gone.cancel();
            }
        }
    }
    impl Session {
        pub fn new(
            process: ManagedProcess,
            server: McpServer,
            calls: mpsc::Receiver<ToolCall>,
            lifetime: Duration,
            config: SessionConfig,
        ) -> io::Result<Self> {
            Self::spawn_inner(
                process,
                server,
                calls,
                lifetime,
                config,
                false,
                PersistentOptions {
                    resume_session_id: None,
                    fallback_prompt: None,
                    parent_shutdown: CancellationToken::new(),
                    idle_timeout: lifetime,
                },
            )
        }

        pub(crate) fn new_persistent(
            process: ManagedProcess,
            server: McpServer,
            calls: mpsc::Receiver<ToolCall>,
            lifetime: Duration,
            config: SessionConfig,
            options: PersistentOptions,
        ) -> io::Result<Self> {
            Self::spawn_inner(process, server, calls, lifetime, config, true, options)
        }

        fn spawn_inner(
            mut process: ManagedProcess,
            server: McpServer,
            calls: mpsc::Receiver<ToolCall>,
            lifetime: Duration,
            config: SessionConfig,
            persistent: bool,
            options: PersistentOptions,
        ) -> io::Result<Self> {
            if lifetime.is_zero() || tokio::time::Instant::now().checked_add(lifetime).is_none() {
                return Err(invalid("invalid session lifetime"));
            }
            if options.idle_timeout.is_zero()
                || tokio::time::Instant::now()
                    .checked_add(options.idle_timeout)
                    .is_none()
            {
                return Err(invalid("invalid session idle timeout"));
            }
            let stdin = process
                .take_stdin()
                .ok_or_else(|| invalid("process stdin unavailable"))?;
            let stdout = process
                .take_stdout()
                .ok_or_else(|| invalid("process stdout unavailable"))?;
            let (commands, receiver) = mpsc::channel(COMMANDS);
            let shutdown = options.parent_shutdown.child_token();
            let suppress_turn_cancel = Arc::new(AtomicBool::new(false));
            let acp_session_id = Arc::new(Mutex::new(None));
            let root_conversation_id = server.root_conversation_id_handle();
            let actor = tokio::spawn(run(
                process,
                stdin,
                stdout,
                server,
                calls,
                receiver,
                shutdown.clone(),
                suppress_turn_cancel.clone(),
                acp_session_id.clone(),
                lifetime,
                config,
                persistent,
                options.idle_timeout,
                options.resume_session_id,
                options.fallback_prompt,
            ));
            Ok(Self {
                commands,
                shutdown,
                suppress_turn_cancel,
                acp_session_id,
                root_conversation_id,
                actor: tokio::sync::Mutex::new(Some(actor)),
            })
        }

        pub(crate) fn acp_session_id(&self) -> Option<String> {
            self.acp_session_id
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone()
        }

        pub(crate) fn conversation_ids(&self) -> Vec<String> {
            let mut ids = Vec::with_capacity(2);
            if let Some(acp_sid) = self
                .acp_session_id
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_ref()
                .filter(|id| !id.is_empty())
            {
                ids.push(acp_sid.clone());
            }
            if let Some(root_cid) = self
                .root_conversation_id
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_ref()
                .filter(|id| !id.is_empty() && !ids.contains(id))
            {
                ids.push(root_cid.clone());
            }
            ids
        }
        /// Close an idle/pending session when the owning agent ends its turn
        /// without resuming, cleaning up the subprocess without cancelling the
        /// caller's turn token.
        pub async fn abandon(&self) -> io::Result<()> {
            self.suppress_turn_cancel.store(true, Ordering::Release);
            self.shutdown().await
        }
        /// Request shutdown and wait for bounded subprocess cleanup.
        pub async fn shutdown(&self) -> io::Result<()> {
            self.shutdown.cancel();
            let mut guard = self.actor.lock().await;
            let actor = guard
                .as_mut()
                .ok_or_else(|| invalid("session already joined"))?;
            let result = actor.await.map_err(|_| invalid("session actor stopped"))?;
            *guard = None;
            result
        }
        pub async fn start(
            &self,
            prompt: Vec<Value>,
            cancel: CancellationToken,
        ) -> io::Result<Turn> {
            let wire_size = crate::transcript::prompt_frame_size(&prompt)
                .map_err(|_| invalid("invalid session prompt"))?;
            if wire_size > MAX_INPUT {
                return Err(invalid("session input too large"));
            }
            let (events, rx) = mpsc::channel(EVENTS);
            let consumer = CancellationToken::new();
            let (reply, response) = oneshot::channel();
            self.commands
                .send(Command::Start {
                    prompt,
                    cancel,
                    consumer: consumer.clone(),
                    events,
                    reply,
                })
                .await
                .map_err(|_| invalid("session closed"))?;
            response.await.map_err(|_| invalid("session closed"))??;
            Ok(Turn {
                events: rx,
                commands: self.commands.clone(),
                consumer_gone: consumer,
                handoff_id: None,
                done: false,
            })
        }
        pub(crate) async fn prompt(
            &self,
            prompt: Vec<Value>,
            model: String,
            cancel: CancellationToken,
        ) -> io::Result<Turn> {
            let wire_size = crate::transcript::prompt_frame_size(&prompt)
                .map_err(|_| invalid("invalid session prompt"))?;
            if wire_size > MAX_INPUT {
                return Err(invalid("session input too large"));
            }
            let (events, rx) = mpsc::channel(EVENTS);
            let consumer = CancellationToken::new();
            let (reply, response) = oneshot::channel();
            self.commands
                .send(Command::Prompt {
                    prompt,
                    model,
                    cancel,
                    consumer: consumer.clone(),
                    events,
                    reply,
                })
                .await
                .map_err(|_| invalid("session closed"))?;
            response.await.map_err(|_| invalid("session closed"))??;
            Ok(Turn {
                events: rx,
                commands: self.commands.clone(),
                consumer_gone: consumer,
                handoff_id: None,
                done: false,
            })
        }
        pub async fn resume(&self, id: &str, result: Value) -> io::Result<Turn> {
            if id.len() > 128 {
                return Err(invalid("tool call id too large"));
            }
            let (events, rx) = mpsc::channel(EVENTS);
            let consumer = CancellationToken::new();
            let (reply, response) = oneshot::channel();
            self.commands
                .send(Command::Resume {
                    id: id.to_owned(),
                    result,
                    consumer: consumer.clone(),
                    events,
                    reply,
                })
                .await
                .map_err(|_| invalid("session closed"))?;
            response.await.map_err(|_| invalid("session closed"))??;
            Ok(Turn {
                events: rx,
                commands: self.commands.clone(),
                consumer_gone: consumer,
                handoff_id: None,
                done: false,
            })
        }
    }
    impl Drop for Session {
        fn drop(&mut self) {
            self.shutdown.cancel();
        }
    }

    fn invalid(message: &'static str) -> io::Error {
        io::Error::new(io::ErrorKind::InvalidData, message)
    }
    fn rpc_error_message(error: &RpcError) -> &'static str {
        let message = error.message.to_ascii_lowercase();
        if error.code == -32000
            || message.contains("auth")
            || message.contains("sign in")
            || message.contains("sign-in")
            || message.contains("unauthorized")
        {
            "AGY authentication failed; run 'mew auth login antigravity'"
        } else if message.contains("quota") || message.contains("rate limit") {
            "AGY quota or rate limit reached"
        } else if message.contains("overload") || message.contains("capacity") {
            "AGY service is overloaded"
        } else {
            "AGY stream failed"
        }
    }
    fn event_size(event: &SessionEvent) -> usize {
        match event {
            SessionEvent::Text(text) => text.len(),
            _ => 1,
        }
    }
    async fn cancelled(token: Option<CancellationToken>) {
        if let Some(token) = token {
            token.cancelled().await
        } else {
            pending::<()>().await
        }
    }
    async fn emit(
        active: &Active,
        event: io::Result<SessionEvent>,
        deadline: tokio::time::Instant,
        shutdown: &CancellationToken,
    ) -> io::Result<()> {
        let Some(sender) = active.events.as_ref() else {
            return Err(invalid("no active turn"));
        };
        tokio::select! {
            _ = shutdown.cancelled() => Err(invalid("session closed")),
            _ = tokio::time::sleep_until(deadline) => Err(invalid("session timed out")),
            _ = active.turn_cancel.cancelled() => Err(invalid("turn cancelled")),
            _ = active.consumer_gone.cancelled() => Err(invalid("consumer disconnected")),
            result = sender.send(event) => result.map_err(|_| invalid("consumer disconnected")),
        }
    }
    async fn write_frame(
        stdin: &mut tokio::net::UnixStream,
        frame: &Value,
        deadline: tokio::time::Instant,
        shutdown: &CancellationToken,
        turn_cancel: Option<&CancellationToken>,
    ) -> io::Result<()> {
        let mut line = serde_json::to_vec(frame).map_err(|_| invalid("invalid ACP request"))?;
        line.push(b'\n');
        tokio::select! {
            _ = shutdown.cancelled() => Err(invalid("session closed")),
            _ = tokio::time::sleep_until(deadline) => Err(invalid("session timed out")),
            _ = cancelled(turn_cancel.cloned()) => Err(invalid("turn cancelled")),
            result = stdin.write_all(&line) => result,
        }
    }
    fn peer_closed(stream: &tokio::net::UnixStream) -> bool {
        let mut byte = 0u8;
        let rc = unsafe {
            libc::recv(
                stream.as_raw_fd(),
                (&mut byte as *mut u8).cast::<libc::c_void>(),
                1,
                libc::MSG_PEEK | libc::MSG_DONTWAIT,
            )
        };
        rc == 0
    }
    async fn send_prompt(
        stdin: &mut tokio::net::UnixStream,
        rpc_id: u64,
        sid: &str,
        prompt: Vec<Value>,
        deadline: tokio::time::Instant,
        shutdown: &CancellationToken,
        turn_cancel: Option<&CancellationToken>,
    ) -> io::Result<()> {
        let prompt_req = json!({
            "jsonrpc": "2.0",
            "id": rpc_id,
            "method": "session/prompt",
            "params": {
                "sessionId": sid,
                "prompt": prompt
            }
        });
        write_frame(stdin, &prompt_req, deadline, shutdown, turn_cancel).await
    }

    fn tool_call_allowed(tool_call: &Value) -> bool {
        let Some(name) = tool_call.get("name").and_then(Value::as_str) else {
            return false;
        };
        if name == "call_mcp_tool" {
            let Some(args) = tool_call
                .get("args")
                .or_else(|| tool_call.get("rawInput"))
                .and_then(Value::as_object)
            else {
                return false;
            };
            return args.get("ServerName").and_then(Value::as_str) == Some("mew");
        }
        name.starts_with("mcp_mew_") || name.starts_with("mcp__mew__")
    }

    fn permission_reply(id: Value, params: &Value) -> Value {
        let allowed = params.get("toolCall").is_some_and(tool_call_allowed);
        let options = params.get("options").and_then(Value::as_array);
        let preferred = if allowed {
            ["allow_once", "allow_always"]
        } else {
            ["reject_once", "reject_always"]
        };
        let selected = options.and_then(|opts| {
            preferred.iter().find_map(|target| {
                opts.iter().find_map(|opt| {
                    (opt.get("kind").and_then(Value::as_str) == Some(*target))
                        .then(|| opt.get("optionId").and_then(Value::as_str))
                        .flatten()
                })
            })
        });
        let outcome = match selected {
            Some(option_id) => json!({"outcome": "selected", "optionId": option_id}),
            None => json!({"outcome": "cancelled"}),
        };
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": { "outcome": outcome }
        })
    }

    #[allow(clippy::too_many_arguments)]
    async fn run(
        mut process: ManagedProcess,
        mut stdin: tokio::net::UnixStream,
        mut stdout: tokio::net::UnixStream,
        mut server: McpServer,
        mut calls: mpsc::Receiver<ToolCall>,
        mut commands: mpsc::Receiver<Command>,
        shutdown: CancellationToken,
        suppress_turn_cancel: Arc<AtomicBool>,
        acp_session_id: Arc<Mutex<Option<String>>>,
        lifetime: Duration,
        config: SessionConfig,
        persistent: bool,
        idle_timeout: Duration,
        mut resume_session_id: Option<String>,
        mut fallback_prompt: Option<Vec<Value>>,
    ) -> io::Result<()> {
        let mut stream = AcpStream::new(MAX_FRAME);
        let mut active: Option<Active> = None;
        let mut pending_call: Option<PendingCall> = None;
        let mut phase = Phase::Idle;
        let mut next_rpc_id: u64 = 6;
        let mut buffer = [0u8; 8192];
        let mut deferred = VecDeque::new();
        let mut deferred_bytes = 0usize;
        let mut deadline_at = tokio::time::Instant::now() + lifetime;
        let deadline = tokio::time::sleep_until(deadline_at);
        tokio::pin!(deadline);
        let failure = 'actor: loop {
            let ready =
                active.as_ref().is_some_and(|a| a.events.is_some()) && pending_call.is_none();
            if ready {
                if let Some(event) = deferred.pop_front() {
                    deferred_bytes = deferred_bytes.saturating_sub(event_size(&event));
                    if emit(active.as_ref().unwrap(), Ok(event), deadline_at, &shutdown)
                        .await
                        .is_err()
                    {
                        break Some("consumer disconnected");
                    }
                    continue;
                }
            }
            let turn_cancel = active.as_ref().map(|a| a.turn_cancel.clone());
            let consumer = active.as_ref().map(|a| a.consumer_gone.clone());
            let mcp_cancel = pending_call.as_ref().map(|p| p.cancelled.clone());
            let started = !matches!(phase, Phase::Idle);
            tokio::select! {
                _ = shutdown.cancelled() => break Some("session closed"),
                _ = &mut deadline => {
                    if matches!(phase, Phase::Ready { .. }) && active.is_none() {
                        break Some("session closed");
                    }
                    break Some("session timed out");
                }
                _ = cancelled(turn_cancel.clone()) => break Some("turn cancelled"),
                _ = cancelled(consumer) => break Some("consumer disconnected"),
                _ = cancelled(mcp_cancel) => {
                    if let Some(active) = &active { active.turn_cancel.cancel(); }
                    break Some("MCP call cancelled");
                }
                command = commands.recv() => match command {
                    None => break Some("session closed"),
                    Some(Command::Start { prompt, cancel, consumer, events, mut reply }) => {
                        if !matches!(phase, Phase::Idle) { let _ = reply.send(Err(invalid("session already started"))); continue; }
                        if cancel.is_cancelled() { let _ = reply.send(Err(invalid("turn cancelled"))); break Some("turn cancelled"); }
                        let init = json!({
                            "jsonrpc": "2.0",
                            "id": 1,
                            "method": "initialize",
                            "params": {
                                "protocolVersion": 1,
                                "clientCapabilities": {}
                            }
                        });
                        let write = tokio::select! {
                            _ = reply.closed() => Err(invalid("consumer disconnected")),
                            result = write_frame(&mut stdin, &init, deadline_at, &shutdown, Some(&cancel)) => result,
                        };
                        if let Err(error) = write {
                            let _ = reply.send(Err(error));
                            break Some("failed to write CLI input");
                        }
                        phase = Phase::WaitInit { prompt };
                        active = Some(Active { events: Some(events), turn_cancel: cancel, consumer_gone: consumer });
                        if reply.send(Ok(())).is_err() { break Some("consumer disconnected"); }
                    }
                    Some(Command::Prompt { prompt, model, cancel, consumer, events, mut reply }) => {
                        if !matches!(phase, Phase::Ready { .. }) || active.is_some() || pending_call.is_some() {
                            let _ = reply.send(Err(invalid("session is not ready")));
                            continue;
                        }
                        if cancel.is_cancelled() {
                            let _ = reply.send(Err(invalid("turn cancelled")));
                            continue;
                        }
                        if peer_closed(&stdout) {
                            let _ = reply.send(Err(invalid("session closed")));
                            break Some("session closed");
                        }
                        let Phase::Ready { sid, current_model } = std::mem::replace(&mut phase, Phase::Idle) else {
                            unreachable!();
                        };
                        server.unquiesce();
                        deadline_at = tokio::time::Instant::now() + lifetime;
                        deadline.as_mut().reset(deadline_at);
                        if !model.is_empty() && model != current_model {
                            let set_model_id = next_rpc_id;
                            next_rpc_id += 1;
                            let prompt_id = next_rpc_id;
                            next_rpc_id += 1;
                            let set_model = json!({
                                "jsonrpc": "2.0",
                                "id": set_model_id,
                                "method": "session/set_model",
                                "params": {
                                    "sessionId": sid,
                                    "modelId": model
                                }
                            });
                            let write = tokio::select! {
                                _ = reply.closed() => Err(invalid("consumer disconnected")),
                                result = write_frame(&mut stdin, &set_model, deadline_at, &shutdown, Some(&cancel)) => result,
                            };
                            if let Err(error) = write {
                                let _ = reply.send(Err(error));
                                phase = Phase::Ready { sid, current_model };
                                break Some("failed to write CLI input");
                            }
                            phase = Phase::WaitSetModel {
                                rpc_id: set_model_id,
                                sid,
                                target_model: model,
                                prompt,
                                prompt_id,
                            };
                        } else {
                            let prompt_id = next_rpc_id;
                            next_rpc_id += 1;
                            let write = tokio::select! {
                                _ = reply.closed() => Err(invalid("consumer disconnected")),
                                result = send_prompt(&mut stdin, prompt_id, &sid, prompt, deadline_at, &shutdown, Some(&cancel)) => result,
                            };
                            if let Err(error) = write {
                                let _ = reply.send(Err(error));
                                phase = Phase::Ready { sid, current_model };
                                break Some("failed to write CLI input");
                            }
                            phase = Phase::RunningPrompt {
                                rpc_id: prompt_id,
                                sid,
                                current_model,
                            };
                        }
                        active = Some(Active { events: Some(events), turn_cancel: cancel, consumer_gone: consumer });
                        if reply.send(Ok(())).is_err() { break Some("consumer disconnected"); }
                    }
                    Some(Command::Ack { id, reply }) => {
                        let Some(pending) = pending_call.as_mut() else { let _ = reply.send(Err(invalid("no pending tool call"))); continue; };
                        if pending.id != id || pending.acknowledged { let _ = reply.send(Err(invalid("invalid handoff acknowledgment"))); continue; }
                        pending.acknowledged = true;
                        if let Some(active) = &mut active { active.events.take(); }
                        if reply.send(Ok(())).is_err() { break Some("consumer disconnected"); }
                    }
                    Some(Command::Resume { id, result, consumer, events, reply }) => {
                        if reply.is_closed() { break Some("consumer disconnected"); }
                        let Some(pending) = pending_call.as_ref() else { let _ = reply.send(Err(invalid("no pending tool call"))); continue; };
                        if pending.id != id || !pending.acknowledged { let _ = reply.send(Err(invalid("tool result does not match pending call"))); continue; }
                        let cancel = active.as_ref().expect("pending call requires owning turn").turn_cancel.clone();
                        if pending.cancelled.is_cancelled() || cancel.is_cancelled() { let _ = reply.send(Err(invalid("tool call cancelled"))); break Some("tool call cancelled"); }
                        if server.resolve(&id, result).is_err() { let _ = reply.send(Err(invalid("tool result could not be delivered"))); break Some("tool result could not be delivered"); }
                        pending_call = None;
                        active = Some(Active { events: Some(events), turn_cancel: cancel, consumer_gone: consumer });
                        if reply.send(Ok(())).is_err() { break Some("consumer disconnected"); }
                    }
                },
                call = calls.recv(), if ready => match call {
                    Some(call) if call.cancellation.is_cancelled() => {},
                    Some(call) => {
                        pending_call = Some(PendingCall { id: call.id.clone(), cancelled: call.cancellation.clone(), acknowledged: false });
                        if emit(active.as_ref().unwrap(), Ok(SessionEvent::ToolCall(call)), deadline_at, &shutdown).await.is_err() { break Some("consumer disconnected"); }
                    }
                    None => break Some("MCP server closed"),
                },
                read = stdout.read(&mut buffer), if started => match read {
                    Ok(0) => {
                        if matches!(phase, Phase::Ready { .. }) && active.is_none() {
                            break Some("session closed");
                        }
                        break Some("AGY stream ended before its result");
                    }
                    Err(_) => break Some("AGY stream read failed"),
                    Ok(count) => match stream.push(&buffer[..count]) {
                        Err(_) => break Some("AGY stream failed"),
                        Ok(frames) => {
                            for frame in frames {
                                match frame {
                                    AcpFrame::Request { id, method, params } => {
                                        let response = if method == "session/request_permission" {
                                            permission_reply(id, &params)
                                        } else {
                                            json!({
                                                "jsonrpc": "2.0",
                                                "id": id,
                                                "error": { "code": -32601, "message": "method not found" }
                                            })
                                        };
                                        if write_frame(&mut stdin, &response, deadline_at, &shutdown, turn_cancel.as_ref()).await.is_err() {
                                            break 'actor Some("failed to write CLI input");
                                        }
                                    }
                                    AcpFrame::Notification { method, params } => {
                                        if !matches!(phase, Phase::RunningPrompt { .. }) || method != "session/update" {
                                            continue;
                                        }
                                        let Some(update) = params.get("update") else {
                                            continue;
                                        };
                                        if update.get("sessionUpdate").and_then(Value::as_str) != Some("agent_message_chunk") {
                                            continue;
                                        }
                                        let Some(content) = update.get("content") else {
                                            continue;
                                        };
                                        if content.get("type").and_then(Value::as_str) != Some("text") {
                                            continue;
                                        }
                                        let Some(text) = content.get("text").and_then(Value::as_str).filter(|s| !s.is_empty()) else {
                                            continue;
                                        };
                                        let mapped = SessionEvent::Text(text.to_owned());
                                        if pending_call.is_some() {
                                            deferred_bytes = deferred_bytes.saturating_add(event_size(&mapped));
                                            if deferred.len() >= EVENTS || deferred_bytes > MAX_FRAME {
                                                break 'actor Some("deferred output exceeded limit");
                                            }
                                            deferred.push_back(mapped);
                                        } else if emit(active.as_ref().unwrap(), Ok(mapped), deadline_at, &shutdown).await.is_err() {
                                            break 'actor Some("consumer disconnected");
                                        }
                                    }
                                    AcpFrame::Response { id, result, error } => {
                                        if let Some(error) = error {
                                            let can_fallback = matches!(
                                                &phase,
                                                Phase::WaitResume { rpc_id, fallback_prompt: Some(_), .. }
                                                    if *rpc_id == id
                                            ) && rpc_error_message(&error) == "AGY stream failed";
                                            if can_fallback {
                                                let Phase::WaitResume {
                                                    fallback_prompt: Some(prompt),
                                                    ..
                                                } = std::mem::replace(&mut phase, Phase::Idle)
                                                else {
                                                    unreachable!();
                                                };
                                                *acp_session_id
                                                    .lock()
                                                    .unwrap_or_else(|e| e.into_inner()) = None;
                                                tracing::warn!(
                                                    code = error.code,
                                                    message = %error.message,
                                                    "AGY session/resume failed; falling back to session/new"
                                                );
                                                let new_id = next_rpc_id;
                                                next_rpc_id += 1;
                                                let new_session = json!({
                                                    "jsonrpc": "2.0",
                                                    "id": new_id,
                                                    "method": "session/new",
                                                    "params": {
                                                        "cwd": config.cwd,
                                                        "mcpServers": []
                                                    }
                                                });
                                                if write_frame(&mut stdin, &new_session, deadline_at, &shutdown, turn_cancel.as_ref()).await.is_err() {
                                                    break 'actor Some("failed to write CLI input");
                                                }
                                                phase = Phase::WaitNew {
                                                    rpc_id: new_id,
                                                    prompt,
                                                };
                                                continue;
                                            }
                                            tracing::warn!(
                                                phase = phase.label(),
                                                rpc_id = id,
                                                code = error.code,
                                                message = %error.message,
                                                "AGY JSON-RPC request failed"
                                            );
                                            break 'actor Some(rpc_error_message(&error));
                                        }
                                        let result = result.unwrap_or(Value::Null);
                                        match std::mem::replace(&mut phase, Phase::Idle) {
                                            Phase::WaitInit { prompt } if id == 1 => {
                                                let auth = json!({
                                                    "jsonrpc": "2.0",
                                                    "id": 2,
                                                    "method": "authenticate",
                                                    "params": { "methodId": "oauth-personal" }
                                                });
                                                if write_frame(&mut stdin, &auth, deadline_at, &shutdown, turn_cancel.as_ref()).await.is_err() {
                                                    break 'actor Some("failed to write CLI input");
                                                }
                                                phase = Phase::WaitAuth { prompt };
                                            }
                                            Phase::WaitAuth { prompt } if id == 2 => {
                                                if let Some(sid) = resume_session_id.take() {
                                                    let resume_session = json!({
                                                        "jsonrpc": "2.0",
                                                        "id": 3,
                                                        "method": "session/resume",
                                                        "params": {
                                                            "sessionId": sid,
                                                            "cwd": config.cwd,
                                                            "mcpServers": []
                                                        }
                                                    });
                                                    if write_frame(&mut stdin, &resume_session, deadline_at, &shutdown, turn_cancel.as_ref()).await.is_err() {
                                                        break 'actor Some("failed to write CLI input");
                                                    }
                                                    phase = Phase::WaitResume {
                                                        rpc_id: 3,
                                                        sid,
                                                        prompt,
                                                        fallback_prompt: fallback_prompt.take(),
                                                    };
                                                } else {
                                                    let new_session = json!({
                                                        "jsonrpc": "2.0",
                                                        "id": 3,
                                                        "method": "session/new",
                                                        "params": {
                                                            "cwd": config.cwd,
                                                            "mcpServers": []
                                                        }
                                                    });
                                                    if write_frame(&mut stdin, &new_session, deadline_at, &shutdown, turn_cancel.as_ref()).await.is_err() {
                                                        break 'actor Some("failed to write CLI input");
                                                    }
                                                    phase = Phase::WaitNew { rpc_id: 3, prompt };
                                                }
                                            }
                                            Phase::WaitNew { rpc_id, prompt } if id == rpc_id => {
                                                let Some(sid) = result.get("sessionId").and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_owned) else {
                                                    break 'actor Some("AGY stream failed");
                                                };
                                                *acp_session_id
                                                    .lock()
                                                    .unwrap_or_else(|e| e.into_inner()) = Some(sid.clone());
                                                if config.model.is_empty() {
                                                    let prompt_id = if rpc_id == 3 {
                                                        5
                                                    } else {
                                                        let id = next_rpc_id;
                                                        next_rpc_id += 1;
                                                        id
                                                    };
                                                    if send_prompt(&mut stdin, prompt_id, &sid, prompt, deadline_at, &shutdown, turn_cancel.as_ref()).await.is_err() {
                                                        break 'actor Some("failed to write CLI input");
                                                    }
                                                    phase = Phase::RunningPrompt {
                                                        rpc_id: prompt_id,
                                                        sid,
                                                        current_model: String::new(),
                                                    };
                                                } else {
                                                    let (set_model_id, prompt_id) = if rpc_id == 3 {
                                                        (4, 5)
                                                    } else {
                                                        let m = next_rpc_id;
                                                        next_rpc_id += 1;
                                                        let p = next_rpc_id;
                                                        next_rpc_id += 1;
                                                        (m, p)
                                                    };
                                                    let set_model = json!({
                                                        "jsonrpc": "2.0",
                                                        "id": set_model_id,
                                                        "method": "session/set_model",
                                                        "params": {
                                                            "sessionId": sid,
                                                            "modelId": config.model
                                                        }
                                                    });
                                                    if write_frame(&mut stdin, &set_model, deadline_at, &shutdown, turn_cancel.as_ref()).await.is_err() {
                                                        break 'actor Some("failed to write CLI input");
                                                    }
                                                    phase = Phase::WaitSetModel {
                                                        rpc_id: set_model_id,
                                                        sid,
                                                        target_model: config.model.clone(),
                                                        prompt,
                                                        prompt_id,
                                                    };
                                                }
                                            }
                                            Phase::WaitResume { rpc_id, sid, prompt, .. } if id == rpc_id => {
                                                let sid = result
                                                    .get("sessionId")
                                                    .and_then(Value::as_str)
                                                    .filter(|s| !s.is_empty())
                                                    .map(str::to_owned)
                                                    .unwrap_or(sid);
                                                *acp_session_id
                                                    .lock()
                                                    .unwrap_or_else(|e| e.into_inner()) = Some(sid.clone());
                                                if config.model.is_empty() {
                                                    let prompt_id = if rpc_id == 3 {
                                                        5
                                                    } else {
                                                        let id = next_rpc_id;
                                                        next_rpc_id += 1;
                                                        id
                                                    };
                                                    if send_prompt(&mut stdin, prompt_id, &sid, prompt, deadline_at, &shutdown, turn_cancel.as_ref()).await.is_err() {
                                                        break 'actor Some("failed to write CLI input");
                                                    }
                                                    phase = Phase::RunningPrompt {
                                                        rpc_id: prompt_id,
                                                        sid,
                                                        current_model: String::new(),
                                                    };
                                                } else {
                                                    let (set_model_id, prompt_id) = if rpc_id == 3 {
                                                        (4, 5)
                                                    } else {
                                                        let m = next_rpc_id;
                                                        next_rpc_id += 1;
                                                        let p = next_rpc_id;
                                                        next_rpc_id += 1;
                                                        (m, p)
                                                    };
                                                    let set_model = json!({
                                                        "jsonrpc": "2.0",
                                                        "id": set_model_id,
                                                        "method": "session/set_model",
                                                        "params": {
                                                            "sessionId": sid,
                                                            "modelId": config.model
                                                        }
                                                    });
                                                    if write_frame(&mut stdin, &set_model, deadline_at, &shutdown, turn_cancel.as_ref()).await.is_err() {
                                                        break 'actor Some("failed to write CLI input");
                                                    }
                                                    phase = Phase::WaitSetModel {
                                                        rpc_id: set_model_id,
                                                        sid,
                                                        target_model: config.model.clone(),
                                                        prompt,
                                                        prompt_id,
                                                    };
                                                }
                                            }
                                            Phase::WaitSetModel { rpc_id, sid, target_model, prompt, prompt_id } if id == rpc_id => {
                                                if send_prompt(&mut stdin, prompt_id, &sid, prompt, deadline_at, &shutdown, turn_cancel.as_ref()).await.is_err() {
                                                    break 'actor Some("failed to write CLI input");
                                                }
                                                phase = Phase::RunningPrompt {
                                                    rpc_id: prompt_id,
                                                    sid,
                                                    current_model: target_model,
                                                };
                                            }
                                            Phase::RunningPrompt { rpc_id, sid, current_model } if id == rpc_id => {
                                                phase = Phase::RunningPrompt {
                                                    rpc_id,
                                                    sid: sid.clone(),
                                                    current_model: current_model.clone(),
                                                };
                                                match result.get("stopReason").and_then(Value::as_str) {
                                                    Some("cancelled") => {
                                                        break 'actor Some("AGY prompt was cancelled");
                                                    }
                                                    Some("refusal") => {
                                                        break 'actor Some("AGY refused the prompt");
                                                    }
                                                    _ => {}
                                                }
                                                // The CLI ended its turn, so any tool call still
                                                // outstanding can never be delivered. Refuse new
                                                // calls and give the last ones a bounded chance to
                                                // settle before failing the turn.
                                                let already_settled = server.quiesce();
                                                if pending_call.is_some() {
                                                    break 'actor Some("AGY finished with pending MCP calls");
                                                }
                                                if !already_settled {
                                                    let grace = (tokio::time::Instant::now() + Duration::from_millis(SETTLE_GRACE_MS)).min(deadline_at);
                                                    let settled = tokio::select! {
                                                        _ = shutdown.cancelled() => break 'actor Some("session closed"),
                                                        _ = active.as_ref().unwrap().turn_cancel.cancelled() => break 'actor Some("turn cancelled"),
                                                        _ = active.as_ref().unwrap().consumer_gone.cancelled() => break 'actor Some("consumer disconnected"),
                                                        _ = tokio::time::timeout_at(grace, server.settled()) => server.is_settled(),
                                                    };
                                                    if !settled {
                                                        break 'actor Some("AGY finished with pending MCP calls");
                                                    }
                                                }
                                                if !deferred.is_empty() {
                                                    break 'actor Some("AGY finished before pending events were delivered");
                                                }
                                                if let Some(usage) = result.get("usage").and_then(Value::as_object) {
                                                    let count = |key| usage.get(key).and_then(Value::as_u64).unwrap_or(0);
                                                    let usage_event = SessionEvent::Usage {
                                                        input: count("inputTokens"),
                                                        output: count("outputTokens"),
                                                        cache_read: count("cachedReadTokens"),
                                                    };
                                                    if emit(active.as_ref().unwrap(), Ok(usage_event), deadline_at, &shutdown).await.is_err() {
                                                        break 'actor Some("consumer disconnected");
                                                    }
                                                }
                                                if !persistent {
                                                    break 'actor None;
                                                }
                                                phase = Phase::Ready { sid, current_model };
                                                let finished_active = active.take().expect("active turn");
                                                if emit(&finished_active, Ok(SessionEvent::Finished), deadline_at, &shutdown).await.is_err() {
                                                    active = Some(finished_active);
                                                    break 'actor Some("consumer disconnected");
                                                }
                                                deadline_at = tokio::time::Instant::now() + idle_timeout;
                                                deadline.as_mut().reset(deadline_at);
                                            }
                                            _ => break 'actor Some("AGY stream failed"),
                                        }
                                    }
                                }
                            }
                        }
                    }
                },
            }
        };
        let idle_close =
            failure == Some("session closed") && active.is_none() && pending_call.is_none();
        let send_cancel_rpc = !idle_close
            && (pending_call.is_some()
                || matches!(
                    failure,
                    Some(
                        "consumer disconnected"
                            | "session closed"
                            | "MCP call cancelled"
                            | "turn cancelled"
                            | "AGY finished with pending MCP calls"
                    )
                ));
        let cancel_turn = send_cancel_rpc
            && !(failure == Some("session closed") && suppress_turn_cancel.load(Ordering::Acquire));
        if idle_close {
            if let Some(sid) = phase.session_id() {
                if let Ok(mut close_line) = serde_json::to_vec(&json!({
                    "jsonrpc": "2.0",
                    "id": next_rpc_id,
                    "method": "session/close",
                    "params": { "sessionId": sid }
                })) {
                    close_line.push(b'\n');
                    let _ = tokio::time::timeout(
                        Duration::from_millis(100),
                        stdin.write_all(&close_line),
                    )
                    .await;
                }
            }
        } else if send_cancel_rpc {
            if let Some(sid) = phase.session_id() {
                if let Ok(mut cancel_line) = serde_json::to_vec(&json!({
                    "jsonrpc": "2.0",
                    "method": "session/cancel",
                    "params": { "sessionId": sid }
                })) {
                    cancel_line.push(b'\n');
                    let _ = tokio::time::timeout(
                        Duration::from_millis(100),
                        stdin.write_all(&cancel_line),
                    )
                    .await;
                }
            }
        }
        if let Some(message) = failure {
            if let Some(active) = &active {
                if cancel_turn {
                    active.turn_cancel.cancel();
                }
                if let Some(events) = &active.events {
                    let _ = events.try_send(Err(invalid(message)));
                }
            }
        }
        let server_cleanup = server.shutdown().await;
        let cleanup = process.cancel().await.and_then(|outcome| match outcome {
            crate::process::ProcessOutcome::TimedOut => Err(invalid("AGY process timed out")),
            _ => Ok(()),
        });
        if let Err(error) = cleanup.and(server_cleanup) {
            if let Some(active) = &active {
                if cancel_turn {
                    active.turn_cancel.cancel();
                }
                if let Some(events) = &active.events {
                    let _ = events.try_send(Err(invalid("AGY process cleanup failed")));
                }
            }
            return Err(error);
        }
        if let Some(message) = failure {
            if message == "session closed" {
                return Ok(());
            }
            return Err(invalid(message));
        }
        if let Some(active) = &active {
            emit(active, Ok(SessionEvent::Finished), deadline_at, &shutdown).await?;
        }
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::{
            mcp::{McpServer, Tool},
            process::{IsolationBoundary, LaunchSpec, ManagedProcess},
        };

        struct PassThroughBoundary;
        impl IsolationBoundary for PassThroughBoundary {
            fn wrap(&self, spec: LaunchSpec, _: &std::path::Path) -> io::Result<LaunchSpec> {
                Ok(spec)
            }
        }

        async fn spawn_persistent_py(
            py_script: &str,
            options: PersistentOptions,
        ) -> (Session, reqwest::Client, String, String, std::path::PathBuf) {
            let tool = Tool::new(
                "echo",
                "Echo",
                json!({"type":"object","properties":{"text":{"type":"string"}},"required":["text"],"additionalProperties":false}),
            )
            .unwrap();
            let (server, calls) = McpServer::bind(vec![tool], 4).await.unwrap();
            let url = server.url().to_owned();
            let client = reqwest::Client::new();
            let response = client
                .post(&url)
                .json(&json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"1"}}}))
                .send()
                .await
                .unwrap();
            let sid = response
                .headers()
                .get("mcp-session-id")
                .unwrap()
                .to_str()
                .unwrap()
                .to_owned();
            assert_eq!(
                client
                    .post(&url)
                    .header("mcp-session-id", &sid)
                    .json(&json!({"jsonrpc":"2.0","method":"notifications/initialized"}))
                    .send()
                    .await
                    .unwrap()
                    .status(),
                202
            );
            let workspace = tempfile::tempdir().unwrap();
            let path = workspace.path().to_owned();
            let script = workspace.path().join("mock_acp.py");
            std::fs::write(&script, py_script).unwrap();
            let spec = LaunchSpec {
                program: "/usr/bin/python3".into(),
                args: vec!["-u".into(), script.as_os_str().into()],
                env: vec![("PATH".into(), "/usr/bin:/bin".into())],
                cwd: path.clone(),
            };
            let process = ManagedProcess::spawn(
                spec,
                &PassThroughBoundary,
                workspace,
                Duration::from_secs(15),
            )
            .unwrap();
            let config = SessionConfig {
                model: "model-a".into(),
                cwd: path.to_string_lossy().into_owned(),
            };
            (
                Session::new_persistent(
                    process,
                    server,
                    calls,
                    Duration::from_secs(10),
                    config,
                    options,
                )
                .unwrap(),
                client,
                url,
                sid,
                path,
            )
        }

        fn text_prompt(text: &str) -> Vec<Value> {
            vec![json!({"type": "text", "text": text})]
        }

        #[tokio::test]
        async fn persistent_session_handles_multiple_prompt_turns_and_model_switch() {
            let py = r#"import sys,json,urllib.request
def read_req():
 line=sys.stdin.readline()
 if not line: sys.exit(0)
 return json.loads(line)
for i,r in [(1,{'protocolVersion':1}),(2,{}),(3,{'sessionId':'s-persist'}),(4,{})]:
 req=read_req()
 assert req['id']==i
 print(json.dumps({'jsonrpc':'2.0','id':i,'result':r}),flush=True)
with open('.agents_url','w') as _: pass
p1=read_req()
assert p1['method']=='session/prompt' and p1['params']['sessionId']=='s-persist'
print(json.dumps({'jsonrpc':'2.0','method':'session/update','params':{'sessionId':'s-persist','update':{'sessionUpdate':'agent_message_chunk','content':{'type':'text','text':'turn1'}}}}),flush=True)
print(json.dumps({'jsonrpc':'2.0','id':p1['id'],'result':{'stopReason':'end_turn'}}),flush=True)
p2=read_req()
assert p2['method']=='session/prompt' and p2['params']['sessionId']=='s-persist'
print(json.dumps({'jsonrpc':'2.0','method':'session/update','params':{'sessionId':'s-persist','update':{'sessionUpdate':'agent_message_chunk','content':{'type':'text','text':'turn2'}}}}),flush=True)
print(json.dumps({'jsonrpc':'2.0','id':p2['id'],'result':{'stopReason':'end_turn'}}),flush=True)
m3=read_req()
assert m3['method']=='session/set_model' and m3['params']['modelId']=='model-b'
print(json.dumps({'jsonrpc':'2.0','id':m3['id'],'result':{}}),flush=True)
p3=read_req()
assert p3['method']=='session/prompt'
print(json.dumps({'jsonrpc':'2.0','method':'session/update','params':{'sessionId':'s-persist','update':{'sessionUpdate':'agent_message_chunk','content':{'type':'text','text':'turn3-model-b'}}}}),flush=True)
print(json.dumps({'jsonrpc':'2.0','id':p3['id'],'result':{'stopReason':'end_turn'}}),flush=True)
c=read_req()
assert c['method']=='session/close' and isinstance(c.get('id'), int) and c['params']['sessionId']=='s-persist'
"#;
            let parent = CancellationToken::new();
            let (session, client, url, sid, workspace) = spawn_persistent_py(
                py,
                PersistentOptions {
                    resume_session_id: None,
                    fallback_prompt: None,
                    parent_shutdown: parent,
                    idle_timeout: Duration::from_secs(10),
                },
            )
            .await;
            let _ = (&client, &url, &sid);

            let mut turn1 = session
                .start(text_prompt("one"), CancellationToken::new())
                .await
                .unwrap();
            assert!(
                matches!(turn1.next().await.unwrap(), Some(SessionEvent::Text(t)) if t == "turn1")
            );
            assert!(matches!(
                turn1.next().await.unwrap(),
                Some(SessionEvent::Finished)
            ));
            drop(turn1);
            assert_eq!(session.acp_session_id().as_deref(), Some("s-persist"));

            let mut turn2 = session
                .prompt(
                    text_prompt("two"),
                    "model-a".into(),
                    CancellationToken::new(),
                )
                .await
                .unwrap();
            assert!(
                matches!(turn2.next().await.unwrap(), Some(SessionEvent::Text(t)) if t == "turn2")
            );
            assert!(matches!(
                turn2.next().await.unwrap(),
                Some(SessionEvent::Finished)
            ));
            drop(turn2);

            let mut turn3 = session
                .prompt(
                    text_prompt("three"),
                    "model-b".into(),
                    CancellationToken::new(),
                )
                .await
                .unwrap();
            assert!(
                matches!(turn3.next().await.unwrap(), Some(SessionEvent::Text(t)) if t == "turn3-model-b")
            );
            assert!(matches!(
                turn3.next().await.unwrap(),
                Some(SessionEvent::Finished)
            ));
            drop(turn3);

            session.shutdown().await.unwrap();
            assert!(!workspace.exists());
        }

        #[tokio::test]
        async fn resume_falls_back_to_session_new_with_replay_when_resume_is_rejected() {
            let py = r#"import sys,json,time
def read_req():
 line=sys.stdin.readline()
 if not line: sys.exit(0)
 return json.loads(line)
for i,r in [(1,{'protocolVersion':1}),(2,{})]:
 req=read_req()
 assert req['id']==i
 print(json.dumps({'jsonrpc':'2.0','id':i,'result':r}),flush=True)
res=read_req()
assert res['method']=='session/resume' and res['params']['sessionId']=='dead-sid'
print(json.dumps({'jsonrpc':'2.0','id':res['id'],'error':{'code':-32004,'message':'session not found'}}),flush=True)
new_req=read_req()
assert new_req['method']=='session/new'
print(json.dumps({'jsonrpc':'2.0','id':new_req['id'],'result':{'sessionId':'fresh-sid'}}),flush=True)
set_m=read_req()
assert set_m['method']=='session/set_model' and set_m['params']['sessionId']=='fresh-sid'
print(json.dumps({'jsonrpc':'2.0','id':set_m['id'],'result':{}}),flush=True)
prompt=read_req()
assert prompt['method']=='session/prompt' and prompt['params']['sessionId']=='fresh-sid'
assert prompt['params']['prompt'][0]['text']=='full-replay'
print(json.dumps({'jsonrpc':'2.0','method':'session/update','params':{'sessionId':'fresh-sid','update':{'sessionUpdate':'agent_message_chunk','content':{'type':'text','text':'recovered-via-new'}}}}),flush=True)
print(json.dumps({'jsonrpc':'2.0','id':prompt['id'],'result':{'stopReason':'end_turn'}}),flush=True)
time.sleep(5)
"#;
            let (session, _client, _url, _sid, _workspace) = spawn_persistent_py(
                py,
                PersistentOptions {
                    resume_session_id: Some("dead-sid".into()),
                    fallback_prompt: Some(text_prompt("full-replay")),
                    parent_shutdown: CancellationToken::new(),
                    idle_timeout: Duration::from_secs(10),
                },
            )
            .await;

            let mut turn = session
                .start(text_prompt("delta-only"), CancellationToken::new())
                .await
                .unwrap();
            assert!(
                matches!(turn.next().await.unwrap(), Some(SessionEvent::Text(t)) if t == "recovered-via-new")
            );
            assert!(matches!(
                turn.next().await.unwrap(),
                Some(SessionEvent::Finished)
            ));
            assert_eq!(session.acp_session_id().as_deref(), Some("fresh-sid"));
            drop(turn);
            session.shutdown().await.unwrap();
        }

        #[test]
        fn tool_call_allowed_accepts_args_and_raw_input_for_mew_only() {
            assert!(tool_call_allowed(&json!({
                "name": "call_mcp_tool",
                "args": { "ServerName": "mew", "ToolName": "read" }
            })));
            assert!(tool_call_allowed(&json!({
                "name": "call_mcp_tool",
                "rawInput": { "ServerName": "mew", "ToolName": "read" }
            })));
            assert!(tool_call_allowed(&json!({ "name": "mcp_mew_read" })));
            assert!(!tool_call_allowed(&json!({
                "name": "call_mcp_tool",
                "rawInput": { "ServerName": "other", "ToolName": "read" }
            })));
            assert!(!tool_call_allowed(&json!({ "name": "run_command" })));
        }
    }
}

#[cfg(unix)]
pub(crate) use unix::{PersistentOptions, MAX_INPUT};
#[cfg(unix)]
pub use unix::{Session, SessionConfig, SessionEvent, Turn};
