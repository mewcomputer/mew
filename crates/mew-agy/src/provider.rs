//! Provider integration for an explicitly isolated AGY process.
//! No concrete isolation boundary is supplied by this module.

#[cfg(unix)]
mod unix {
    use crate::{
        launch::{self, Capabilities},
        mcp::{McpServer, Tool},
        process::{IsolationBoundary, LaunchSpec, ManagedProcess},
        session::{Session, SessionEvent, Turn},
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
        collections::HashMap,
        ffi::OsString,
        path::PathBuf,
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
        let text = results[0]["content"]
            .as_str()
            .ok_or_else(|| invalid("invalid tool result text"))?;
        let is_error = results[0]["is_error"]
            .as_bool()
            .ok_or_else(|| invalid("invalid tool result state"))?;
        Ok(json!({"content":[{"type":"text","text":text}],"isError":is_error}))
    }
    #[async_trait]
    impl<B: IsolationBoundary + Send + Sync + 'static> Provider for AgyProvider<B> {
        fn name(&self) -> &str {
            "antigravity"
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
                        || slot
                            .try_lock()
                            .map_or(true, |entry| entry.session.is_some())
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
                        if let Some(session) = guard.session.take() {
                            let _ = session.shutdown().await;
                        }
                        return Err(error.into());
                    }
                }
            } else {
                if guard.session.is_some() {
                    return Err(invalid("AGY session is already active"));
                }
                let mut defs = Vec::new();
                for t in &request.tools {
                    defs.push(
                        Tool::new(&t.name, &t.description, t.schema.clone()).map_err(invalid)?,
                    )
                }
                let (server, calls) = McpServer::bind(defs, 16).await.map_err(invalid)?;
                let names: Vec<_> = request.tools.iter().map(|t| t.name.as_str()).collect();
                let launch = launch::prepare(
                    &self.config.workspace_parent,
                    &request.model,
                    &names,
                    server.url(),
                    Capabilities::default(),
                    &system,
                )?;
                let cwd = launch.workspace().to_path_buf();
                let (workspace, args) = launch.into_parts();
                let spec = LaunchSpec {
                    program: self.config.executable.clone(),
                    args: args.into_iter().map(OsString::from).collect(),
                    env: self.config.environment.clone(),
                    cwd,
                };
                let process =
                    ManagedProcess::spawn(spec, &self.boundary, workspace, self.config.lifetime)?;
                let session = Session::new(process, server, calls, self.config.lifetime)?;
                let replay = transcript::project(&request).map_err(invalid)?;
                let prompt=format!("Historical conversation follows as JSON. Historical tool calls are complete; do not repeat them. Respond to the latest user message.\n{replay}");
                let mut input =
                    serde_json::to_vec(&json!({"event":"user","message":{"content":prompt}}))?;
                input.push(b'\n');
                let turn = session.start(&input, request.cancel_token.clone()).await?;
                guard.session = Some(session);
                turn
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
            ));
            Ok(Box::pin(Output { rx }))
        }
    }
    async fn deliver(
        mut turn: Turn,
        mut slot: OwnedMutexGuard<Slot>,
        request: Request,
        contract: Value,
        history: Vec<Value>,
        tx: mpsc::Sender<Delivery>,
        shutdown: CancellationToken,
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
                    let cleanup = if let Some(session) = slot.session.take() {
                        session.shutdown().await
                    } else {
                        Err(std::io::Error::other("session unavailable"))
                    };
                    if cleanup.is_err() {
                        break "AGY process cleanup failed";
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
                        "AGY authentication failed" => "AGY authentication failed",
                        "AGY service is overloaded" => "AGY service is overloaded",
                        _ => "AGY session failed",
                    };
                }
            }
        };
        drop(turn);
        slot.pending = None;
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
