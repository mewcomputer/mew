//! Coordination of one isolated CLI process and its client-owned MCP calls.

#[cfg(unix)]
mod unix {
    use std::{collections::VecDeque, future::pending, io, time::Duration};

    use serde_json::Value;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        sync::{mpsc, oneshot},
    };
    use tokio_util::sync::CancellationToken;

    use crate::{
        mcp::{McpServer, ToolCall},
        process::ManagedProcess,
        stream::{CliEvent, CliStream},
    };

    const MAX_FRAME: usize = 1_048_576;
    pub(crate) const MAX_INPUT: usize = 16 * 1_048_576;
    const EVENTS: usize = 16;
    const COMMANDS: usize = 8;

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
    enum Command {
        Start {
            input: Vec<u8>,
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

    pub struct Session {
        commands: mpsc::Sender<Command>,
        shutdown: CancellationToken,
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
            mut process: ManagedProcess,
            server: McpServer,
            calls: mpsc::Receiver<ToolCall>,
            lifetime: Duration,
        ) -> io::Result<Self> {
            if lifetime.is_zero() || tokio::time::Instant::now().checked_add(lifetime).is_none() {
                return Err(invalid("invalid session lifetime"));
            }
            let stdin = process
                .take_stdin()
                .ok_or_else(|| invalid("process stdin unavailable"))?;
            let stdout = process
                .take_stdout()
                .ok_or_else(|| invalid("process stdout unavailable"))?;
            let (commands, receiver) = mpsc::channel(COMMANDS);
            let shutdown = CancellationToken::new();
            let actor = tokio::spawn(run(
                process,
                stdin,
                stdout,
                server,
                calls,
                receiver,
                shutdown.clone(),
                lifetime,
            ));
            Ok(Self {
                commands,
                shutdown,
                actor: tokio::sync::Mutex::new(Some(actor)),
            })
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
        pub async fn start(&self, input: &[u8], cancel: CancellationToken) -> io::Result<Turn> {
            if input.len() > MAX_INPUT {
                return Err(invalid("session input too large"));
            }
            let (events, rx) = mpsc::channel(EVENTS);
            let consumer = CancellationToken::new();
            let (reply, response) = oneshot::channel();
            self.commands
                .send(Command::Start {
                    input: input.to_vec(),
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
    fn stream_error(error: &io::Error) -> &'static str {
        let message = error.to_string().to_ascii_lowercase();
        if message.contains("quota") || message.contains("rate limit") {
            "AGY quota or rate limit reached"
        } else if message.contains("auth")
            || message.contains("sign in")
            || message.contains("unauthorized")
        {
            "AGY authentication failed"
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

    #[allow(clippy::too_many_arguments)]
    async fn run(
        mut process: ManagedProcess,
        mut stdin: tokio::net::UnixStream,
        mut stdout: tokio::net::UnixStream,
        mut server: McpServer,
        mut calls: mpsc::Receiver<ToolCall>,
        mut commands: mpsc::Receiver<Command>,
        shutdown: CancellationToken,
        lifetime: Duration,
    ) -> io::Result<()> {
        let mut stream = CliStream::new(MAX_FRAME);
        let mut active: Option<Active> = None;
        let mut pending_call: Option<PendingCall> = None;
        let mut started = false;
        let mut buffer = [0u8; 8192];
        let mut deferred = VecDeque::new();
        let mut deferred_bytes = 0usize;
        let deadline_at = tokio::time::Instant::now() + lifetime;
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
            tokio::select! {
                _ = shutdown.cancelled() => break Some("session closed"),
                _ = &mut deadline => break Some("session timed out"),
                _ = cancelled(turn_cancel) => break Some("turn cancelled"),
                _ = cancelled(consumer) => break Some("consumer disconnected"),
                _ = cancelled(mcp_cancel) => {
                    if let Some(active) = &active { active.turn_cancel.cancel(); }
                    break Some("MCP call cancelled");
                }
                command = commands.recv() => match command {
                    None => break Some("session closed"),
                    Some(Command::Start { input, cancel, consumer, events, mut reply }) => {
                        if started { let _ = reply.send(Err(invalid("session already started"))); continue; }
                        if cancel.is_cancelled() { let _ = reply.send(Err(invalid("turn cancelled"))); break Some("turn cancelled"); }
                        let write = tokio::select! {
                            _ = reply.closed() => Err(invalid("consumer disconnected")),
                            _ = shutdown.cancelled() => Err(invalid("session closed")),
                            _ = cancel.cancelled() => Err(invalid("turn cancelled")),
                            _ = &mut deadline => Err(invalid("session timed out")),
                            result = stdin.write_all(&input) => result,
                        };
                        if let Err(error) = write { let _ = reply.send(Err(error)); break Some("failed to write CLI input"); }
                        started = true;
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
                    Ok(0) => break Some("AGY stream ended before its result"),
                    Err(_) => break Some("AGY stream read failed"),
                    Ok(count) => match stream.push(&buffer[..count]) {
                        Err(error) => break Some(stream_error(&error)),
                        Ok(events) => {
                            for event in events {
                                let mapped = match event {
                                    CliEvent::Text(text) => SessionEvent::Text(text),
                                    CliEvent::Usage { input, output, cache_read } => SessionEvent::Usage { input, output, cache_read },
                                    CliEvent::Finished => {
                                        if pending_call.is_some() { server.quiesce(); break 'actor Some("AGY finished with pending MCP calls"); }
                                        let grace = (tokio::time::Instant::now() + Duration::from_millis(200)).min(deadline_at);
                                        while !server.quiesce() {
                                            if tokio::time::Instant::now() >= grace { break 'actor Some("AGY finished with pending MCP calls"); }
                                            tokio::select! {
                                                _ = shutdown.cancelled() => break 'actor Some("session closed"),
                                                _ = active.as_ref().unwrap().turn_cancel.cancelled() => break 'actor Some("turn cancelled"),
                                                _ = active.as_ref().unwrap().consumer_gone.cancelled() => break 'actor Some("consumer disconnected"),
                                                _ = tokio::time::sleep(Duration::from_millis(10)) => {},
                                            }
                                        }
                                        if !deferred.is_empty() { break 'actor Some("AGY finished before pending events were delivered"); }
                                        break 'actor None;
                                    }
                                };
                                if pending_call.is_some() {
                                    deferred_bytes = deferred_bytes.saturating_add(event_size(&mapped));
                                    if deferred.len() >= EVENTS || deferred_bytes > MAX_FRAME { break 'actor Some("deferred output exceeded limit"); }
                                    deferred.push_back(mapped);
                                } else if emit(active.as_ref().unwrap(), Ok(mapped), deadline_at, &shutdown).await.is_err() {
                                    break 'actor Some("consumer disconnected");
                                }
                            }
                        }
                    }
                },
            }
        };
        let cancel_turn = pending_call.is_some()
            || matches!(
                failure,
                Some(
                    "consumer disconnected"
                        | "session closed"
                        | "MCP call cancelled"
                        | "turn cancelled"
                        | "AGY finished with pending MCP calls"
                )
            );
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
}

#[cfg(unix)]
pub(crate) use unix::MAX_INPUT;
#[cfg(unix)]
pub use unix::{Session, SessionEvent, Turn};
