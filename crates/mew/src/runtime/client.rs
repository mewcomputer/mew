//! One persistent core-backed connection for the terminal and capture clients.

use anyhow::{Context, Result};
use mew_agent::{AgentEvent, AskUserQuestion, GoalDecision, QuestionOption};
use mew_client_core::{
    ActionKind, ClientEngine, ClientEvent, ClientState, ClientTransport, ConnectionStatus,
    PendingAction,
};
use mew_message::Message;
use mew_protocol::{Attachment, ClientKind, ClientMessage, PromptStatus, Todo, TodoEdit};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot, watch};

const COMMAND_TIMEOUT: Duration = Duration::from_secs(3);

pub struct ClientUpdate {
    pub events: Vec<ClientEvent>,
    pub state: Box<ClientState>,
    pub changed_message: Option<Message>,
    pub required_action: Option<(String, AgentEvent)>,
}

pub struct TuiClient {
    commands: mpsc::Sender<Command>,
    session: watch::Receiver<Option<String>>,
    task: tokio::task::AbortHandle,
}

impl Drop for TuiClient {
    fn drop(&mut self) {
        self.task.abort();
    }
}

struct Command {
    session_owner: Option<String>,
    message: ClientMessage,
    reply: Reply,
}

enum Reply {
    Response,
    Sent(oneshot::Sender<Result<()>>),
    Prompt(oneshot::Sender<Result<()>>),
    Todo(oneshot::Sender<Result<Vec<Todo>>>),
}

impl Reply {
    fn is_closed(&self) -> bool {
        match self {
            Self::Response => false,
            Self::Sent(reply) | Self::Prompt(reply) => reply.is_closed(),
            Self::Todo(reply) => reply.is_closed(),
        }
    }

    fn fail(self, message: &str) {
        match self {
            Self::Response => {}
            Self::Sent(reply) | Self::Prompt(reply) => {
                let _ = reply.send(Err(anyhow::anyhow!("{message}")));
            }
            Self::Todo(reply) => {
                let _ = reply.send(Err(anyhow::anyhow!("{message}")));
            }
        }
    }
}

impl TuiClient {
    pub async fn connect(url: &str) -> Result<(Arc<Self>, mpsc::UnboundedReceiver<ClientUpdate>)> {
        Self::connect_with_transport(Box::new(mew_client_local::LocalWebSocketTransport::new(
            url,
        )))
        .await
    }

    async fn connect_with_transport(
        transport: Box<dyn ClientTransport>,
    ) -> Result<(Arc<Self>, mpsc::UnboundedReceiver<ClientUpdate>)> {
        let engine = ClientEngine::connect(transport.as_ref()).await?;
        let (commands, command_rx) = mpsc::channel(64);
        let (updates, update_rx) = mpsc::unbounded_channel();
        let (session_tx, session) = watch::channel(None);
        let task = tokio::spawn(run_client(
            transport,
            engine,
            command_rx,
            commands.clone(),
            updates,
            session_tx,
        ));
        let client = Arc::new(Self {
            commands,
            session,
            task: task.abort_handle(),
        });
        Ok((client, update_rx))
    }

    pub async fn session_id(&self) -> Option<String> {
        self.session.borrow().clone()
    }

    pub async fn send(&self, message: ClientMessage) -> Result<()> {
        self.send_command(None, message).await
    }

    pub async fn send_to_session(&self, session_id: &str, message: ClientMessage) -> Result<()> {
        self.send_command(Some(session_id), message).await
    }

    async fn send_command(&self, session_id: Option<&str>, message: ClientMessage) -> Result<()> {
        let (reply, receive) = oneshot::channel();
        tokio::time::timeout(COMMAND_TIMEOUT, async {
            self.commands
                .send(Command {
                    session_owner: session_id.map(str::to_owned),
                    message,
                    reply: Reply::Sent(reply),
                })
                .await
                .context("client driver stopped")?;
            receive
                .await
                .context("client driver stopped before sending command")?
        })
        .await
        .context("daemon command send timed out")?
    }

    pub async fn submit(
        &self,
        session_id: &str,
        request_id: &str,
        text: String,
        attachments: Vec<Attachment>,
    ) -> Result<()> {
        let (reply, receive) = oneshot::channel();
        let message = ClientMessage::SubmitPrompt {
            session_id: session_id.into(),
            request_id: request_id.into(),
            text,
            attachments,
        };
        tokio::time::timeout(COMMAND_TIMEOUT, async {
            self.commands
                .send(Command {
                    session_owner: None,
                    message,
                    reply: Reply::Prompt(reply),
                })
                .await
                .context("client driver stopped")?;
            receive
                .await
                .context("client driver stopped before confirming prompt")?
        })
        .await
        .context("prompt acceptance is unconfirmed; reconnect will check its receipt")?
    }

    pub async fn update_todo(
        &self,
        session_id: &str,
        edit: TodoEdit,
    ) -> Result<Vec<mew_agent::Todo>> {
        let (reply, receive) = oneshot::channel();
        let message = ClientMessage::UpdateTodo {
            session_id: session_id.into(),
            request_id: ulid::Ulid::new().to_string(),
            edit,
        };
        let todos = tokio::time::timeout(COMMAND_TIMEOUT, async {
            self.commands
                .send(Command {
                    session_owner: None,
                    message,
                    reply: Reply::Todo(reply),
                })
                .await
                .context("client driver stopped")?;
            receive
                .await
                .context("daemon disconnected before confirming todo save")?
        })
        .await
        .context("todo save was not confirmed; reconnect before retrying")??;
        todos
            .into_iter()
            .map(|todo| {
                Ok(mew_agent::Todo {
                    id: todo.id,
                    content: todo.content,
                    status: mew_agent::TodoStatus::parse(&todo.status)
                        .context("invalid todo status in reply")?,
                    depends_on: todo.depends_on,
                })
            })
            .collect()
    }
}

fn project_update(state: &ClientState, events: Vec<ClientEvent>) -> ClientUpdate {
    let replay = events.iter().any(|event| {
        matches!(
            event,
            ClientEvent::SessionReady { .. }
                | ClientEvent::SessionHistoryLoaded { .. }
                | ClientEvent::SessionHistoryPageLoaded { .. }
        )
    });
    let changed = !replay
        && !events
            .iter()
            .any(|event| matches!(event, ClientEvent::PartUpdated { .. }))
        && events.iter().any(|event| {
            matches!(
                event,
                ClientEvent::MessageChanged { .. } | ClientEvent::ProviderMessageEnded { .. }
            )
        });
    let changed_message = changed
        .then(|| {
            state
                .attached_session
                .as_deref()
                .and_then(|id| state.session(id))
                .and_then(|session| session.messages.last())
                .cloned()
        })
        .flatten();
    let mut snapshot = if replay {
        state.ui_snapshot()
    } else {
        state.ui_metadata_snapshot()
    };
    if !replay {
        if let Some(session) = state
            .attached_session
            .as_deref()
            .and_then(|id| state.session(id))
        {
            snapshot
                .sessions
                .insert(session.session_id.clone(), session.ui_metadata_snapshot());
        }
    }
    ClientUpdate {
        events,
        state: Box::new(snapshot),
        changed_message,
        required_action: None,
    }
}

fn reject_unsent_prompt(
    message: &ClientMessage,
    state: &ClientState,
    updates: &mpsc::UnboundedSender<ClientUpdate>,
    reason: &str,
) {
    if let ClientMessage::SubmitPrompt {
        session_id,
        request_id,
        ..
    } = message
    {
        let _ = updates.send(project_update(
            state,
            vec![ClientEvent::PromptReceipt {
                session_id: session_id.clone(),
                request_id: request_id.clone(),
                status: PromptStatus::Rejected,
                message: Some(reason.into()),
            }],
        ));
    }
}

async fn run_client(
    transport: Box<dyn ClientTransport>,
    mut engine: ClientEngine,
    mut commands: mpsc::Receiver<Command>,
    command_tx: mpsc::Sender<Command>,
    updates: mpsc::UnboundedSender<ClientUpdate>,
    session_tx: watch::Sender<Option<String>>,
) {
    let mut prompts: BTreeMap<(String, String), Reply> = BTreeMap::new();
    let mut todos: BTreeMap<(String, String), Reply> = BTreeMap::new();
    let mut required = BTreeSet::new();
    let mut owner = None;
    let mut reconnect_attempt = 0u32;
    let _ = updates.send(project_update(
        engine.state(),
        vec![ClientEvent::ConnectionChanged(ConnectionStatus::Connected)],
    ));
    loop {
        let failure = loop {
            tokio::select! {
                command = commands.recv() => {
                    let Some(command) = command else { return; };
                    if command.reply.is_closed() {
                        reject_unsent_prompt(&command.message,engine.state(),&updates,"prompt expired before it was sent");
                        continue;
                    }
                    if command.session_owner.as_deref().is_some_and(|owner|engine.state().attached_session.as_deref()!=Some(owner)) {
                        command.reply.fail("command belongs to a different session");
                        continue;
                    }
                    if let ClientMessage::SubmitPrompt {session_id,request_id,..} = &command.message {
                        if engine.state().attached_session.as_deref() != Some(session_id) {
                            reject_unsent_prompt(&command.message,engine.state(),&updates,"prompt owner is no longer the attached session");
                            command.reply.fail("prompt owner is no longer the attached session");
                            continue;
                        }
                        if prompts.contains_key(&(session_id.clone(),request_id.clone())) {
                            command.reply.fail("prompt is already awaiting its receipt");
                            continue;
                        }
                    }
                    let key = match &command.message {
                        ClientMessage::SubmitPrompt {session_id,request_id,..} | ClientMessage::UpdateTodo {session_id,request_id,..} => Some((session_id.clone(),request_id.clone())),
                        _ => None,
                    };
                    let result = engine.send(command.message).await;
                    match command.reply {
                        Reply::Response => {},
                        reply @ Reply::Prompt(_) => { prompts.insert(key.expect("prompt key"),reply); }
                        reply @ Reply::Todo(_) => {
                            if result.is_ok() { todos.insert(key.expect("todo key"),reply); }
                            else { reply.fail("daemon disconnected during todo save; result is unconfirmed"); }
                        }
                        Reply::Sent(reply) => { let _ = reply.send(result.as_ref().map(|_|()).map_err(|error|anyhow::anyhow!("{error}"))); }
                    }
                    if let Err(error) = result { break error.to_string(); }
                }
                result = engine.receive() => match result {
                    Ok(events) => {
                        let mut send_failure=None;
                        for event in &events {
                            match event {
                                ClientEvent::SessionReady {session_id} => {
                                    owner = Some(session_id.clone());
                                    session_tx.send_replace(owner.clone());
                                    required.clear();
                                    if reconnect_attempt > 0 {
                                        for (session_id,request_id) in prompts.keys() {
                                            if let Err(error)=engine.send(ClientMessage::ReadPromptReceipt {session_id:session_id.clone(),request_id:request_id.clone()}).await {
                                                send_failure=Some(error.to_string());
                                                break;
                                            }
                                        }
                                        if send_failure.is_none() {
                                            for message in [ClientMessage::ListSessions {},ClientMessage::ListProjects {},ClientMessage::ListModels {}] {
                                                if let Err(error)=engine.send(message).await {send_failure=Some(error.to_string());break;}
                                            }
                                        }
                                        reconnect_attempt = 0;
                                    }
                                }
                                ClientEvent::PromptReceipt {session_id,request_id,status,message} => {
                                    if let Some(reply) = prompts.remove(&(session_id.clone(),request_id.clone())) {
                                        match status {
                                            PromptStatus::Accepted => if let Reply::Prompt(reply) = reply { let _ = reply.send(Ok(())); },
                                            PromptStatus::Rejected => reply.fail(message.as_deref().unwrap_or("daemon rejected the prompt")),
                                            PromptStatus::Unknown => reply.fail(message.as_deref().unwrap_or("prompt acceptance remains unknown; it was not resubmitted")),
                                        }
                                    }
                                }
                                ClientEvent::TodoUpdated {session_id,request_id,todos:result} => {
                                    if let Some(Reply::Todo(reply)) = todos.remove(&(session_id.clone(),request_id.clone())) { let _ = reply.send(Ok(result.clone())); }
                                }
                                ClientEvent::TodoUpdateError {session_id,request_id,message} => {
                                    if let Some(reply) = todos.remove(&(session_id.clone(),request_id.clone())) { reply.fail(message); }
                                }
                                ClientEvent::RequestResolved {request_id} => { required.remove(request_id); }
                                _ => {}
                            }
                        }
                        let mut update = project_update(engine.state(), events);
                        for event in &update.events {
                            if let ClientEvent::RequiredActionChanged {session_id,request_id} = event {
                                if required.insert(request_id.clone()) {
                                    if let Some(action) = engine.state().session(session_id).and_then(|session|session.pending_actions.iter().find(|action|action.request_id==*request_id)) {
                                        update.required_action = Some((request_id.clone(),bridge_action(action,session_id,&command_tx)));
                                    }
                                }
                            }
                        }
                        if updates.send(update).is_err() { return; }
                        if let Some(error)=send_failure {break error;}
                    }
                    Err(error) => break error.to_string(),
                }
            }
        };
        for (_, reply) in std::mem::take(&mut todos) {
            reply.fail("daemon disconnected before confirming todo save");
        }
        required.clear();
        loop {
            reconnect_attempt = reconnect_attempt.saturating_add(1);
            let status = ConnectionStatus::Backoff {
                attempt: reconnect_attempt,
                error: failure.clone(),
            };
            engine.state_mut().set_connection_status(status.clone());
            if updates
                .send(project_update(
                    engine.state(),
                    vec![ClientEvent::ConnectionChanged(status)],
                ))
                .is_err()
            {
                return;
            }
            let delay = Duration::from_millis(
                (250u64.saturating_mul(1 << reconnect_attempt.min(5).saturating_sub(1))).min(5000),
            );
            let wait = tokio::time::sleep(delay);
            tokio::pin!(wait);
            loop {
                tokio::select! {
                    _ = &mut wait => break,
                    command = commands.recv() => match command {
                        Some(command) => {
                            reject_unsent_prompt(&command.message,engine.state(),&updates,"daemon is reconnecting; prompt was not sent");
                            command.reply.fail("daemon is reconnecting; command was not sent");
                        },
                        None => return,
                    }
                }
            }
            match ClientEngine::connect(transport.as_ref()).await {
                Ok(mut connected) => {
                    if let Some(session_id) = &owner {
                        if connected
                            .send(ClientMessage::AttachSession {
                                session_id: session_id.clone(),
                                client_kind: ClientKind::Tui,
                            })
                            .await
                            .is_err()
                        {
                            continue;
                        }
                    }
                    let handshake = std::mem::take(connected.state_mut());
                    let mut previous = std::mem::take(engine.state_mut());
                    previous.attached_session = None;
                    previous.connection = handshake.connection;
                    previous.daemon_version = handshake.daemon_version;
                    previous.remote_scope = handshake.remote_scope;
                    previous.session_attention.clear();
                    previous.presence.clear();
                    previous.control_yielded_by = None;
                    *connected.state_mut() = previous;
                    engine = connected;
                    if updates
                        .send(project_update(
                            engine.state(),
                            vec![ClientEvent::ConnectionChanged(ConnectionStatus::Connected)],
                        ))
                        .is_err()
                    {
                        return;
                    }
                    break;
                }
                Err(_) => continue,
            }
        }
    }
}

fn bridge_action(
    action: &PendingAction,
    session_id: &str,
    commands: &mpsc::Sender<Command>,
) -> AgentEvent {
    let request_id = action.request_id.clone();
    let session_owner = Some(session_id.to_owned());
    let commands = commands.clone();
    match &action.kind {
        ActionKind::Permission { tool_name, input }
        | ActionKind::SubagentPermission {
            tool_name, input, ..
        } => {
            let (tx, rx) = oneshot::channel();
            tokio::spawn(async move {
                if let Ok(decision) = rx.await {
                    let _ = commands
                        .send(Command {
                            session_owner,
                            message: ClientMessage::PermissionResponse {
                                request_id,
                                decision: mew_protocol::PermissionDecision::from(decision),
                            },
                            reply: Reply::Response,
                        })
                        .await;
                }
            });
            let call = mew_hooks::ToolCall {
                tool_name: tool_name.clone(),
                call_id: String::new(),
                input: input.clone(),
            };
            if let ActionKind::SubagentPermission { parent_call_id, .. } = &action.kind {
                AgentEvent::SubagentPermissionRequest {
                    parent_call_id: parent_call_id.clone(),
                    call,
                    tx,
                }
            } else {
                AgentEvent::PermissionRequest { call, tx }
            }
        }
        ActionKind::WorkspacePermission { path } => {
            let (tx, rx) = oneshot::channel();
            tokio::spawn(async move {
                if let Ok(decision) = rx.await {
                    let _ = commands
                        .send(Command {
                            session_owner,
                            message: ClientMessage::PermissionResponse {
                                request_id,
                                decision: mew_protocol::PermissionDecision::from(decision),
                            },
                            reply: Reply::Response,
                        })
                        .await;
                }
            });
            AgentEvent::WorkspacePermissionRequest {
                path: path.into(),
                tx,
            }
        }
        ActionKind::AskUser {
            call_id,
            questions,
            subagent,
        } => {
            let (tx, rx) = oneshot::channel();
            tokio::spawn(async move {
                if let Ok(answers) = rx.await {
                    let _ = commands
                        .send(Command {
                            session_owner,
                            message: ClientMessage::AskUserResponse {
                                request_id,
                                cancelled: false,
                                answers,
                            },
                            reply: Reply::Response,
                        })
                        .await;
                }
            });
            let questions = questions
                .iter()
                .map(|q| AskUserQuestion {
                    prompt: q.prompt.clone(),
                    options: q
                        .options
                        .iter()
                        .map(|o| QuestionOption {
                            label: o.label.clone(),
                            description: o.description.clone(),
                        })
                        .collect(),
                })
                .collect();
            if let Some(source) = subagent {
                AgentEvent::SubagentAskUser {
                    source: source.clone(),
                    call_id: call_id.clone(),
                    questions,
                    tx,
                }
            } else {
                AgentEvent::AskUser {
                    call_id: call_id.clone(),
                    questions,
                    tx,
                }
            }
        }
        ActionKind::GoalApproval { call_id, objective } => {
            let (tx, rx) = oneshot::channel();
            tokio::spawn(async move {
                if let Ok(decision) = rx.await {
                    let _ = commands
                        .send(Command {
                            session_owner,
                            message: ClientMessage::GoalResponse {
                                request_id,
                                accepted: matches!(decision, GoalDecision::Accepted),
                            },
                            reply: Reply::Response,
                        })
                        .await;
                }
            });
            AgentEvent::GoalProposed {
                call_id: call_id.clone(),
                objective: objective.clone(),
                tx,
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use mew_client_core::{ClientConnection, ClientTransport, TransportError};
    use mew_protocol::{ClientKind, PromptStatus, ServerMessage};
    use std::collections::VecDeque;
    use std::sync::Mutex;

    struct ChannelConnection {
        inbound: mpsc::UnboundedReceiver<ServerMessage>,
        outbound: mpsc::UnboundedSender<ClientMessage>,
    }

    #[async_trait::async_trait]
    impl ClientConnection for ChannelConnection {
        async fn send(&mut self, message: ClientMessage) -> Result<(), TransportError> {
            self.outbound
                .send(message)
                .map_err(|_| TransportError::Closed)
        }
        async fn receive(&mut self) -> Result<Option<ServerMessage>, TransportError> {
            Ok(self.inbound.recv().await)
        }
        async fn close(&mut self) -> Result<(), TransportError> {
            Ok(())
        }
    }

    struct ChannelTransport(Mutex<VecDeque<ChannelConnection>>);
    #[async_trait::async_trait]
    impl ClientTransport for ChannelTransport {
        async fn connect(&self) -> Result<Box<dyn ClientConnection>, TransportError> {
            self.0
                .lock()
                .unwrap()
                .pop_front()
                .map(|c| Box::new(c) as Box<dyn ClientConnection>)
                .ok_or(TransportError::Closed)
        }
    }

    fn connection() -> (
        ChannelConnection,
        mpsc::UnboundedSender<ServerMessage>,
        mpsc::UnboundedReceiver<ClientMessage>,
    ) {
        let (inbound, rx) = mpsc::unbounded_channel();
        let (tx, outbound) = mpsc::unbounded_channel();
        inbound
            .send(ServerMessage::Pong {
                version: "test".into(),
                protocol_version: mew_protocol::PROTOCOL_VERSION,
            })
            .unwrap();
        (
            ChannelConnection {
                inbound: rx,
                outbound: tx,
            },
            inbound,
            outbound,
        )
    }

    async fn ready_catalogs(outbound: &mut mpsc::UnboundedReceiver<ClientMessage>, list: bool) {
        if list {
            assert!(matches!(
                outbound.recv().await.unwrap(),
                ClientMessage::ListSessions
            ));
        }
        assert!(matches!(
            outbound.recv().await.unwrap(),
            ClientMessage::ListPersonas
        ));
    }

    fn ready(id: &str) -> ServerMessage {
        ServerMessage::SessionReady {
            session_id: id.into(),
            cwd: Some("/tmp/project".into()),
            model: None,
            provider: None,
            permission_mode: None,
        }
    }

    #[test]
    fn incremental_updates_copy_only_the_changed_message() {
        let mut state = ClientState::default();
        state.apply_server_message(ready("a"));
        state.record_prompt("a", "older".into());
        state.record_prompt("a", "latest".into());
        let update = project_update(
            &state,
            vec![ClientEvent::MessageChanged {
                session_id: "a".into(),
            }],
        );
        assert!(update.state.session("a").unwrap().messages.is_empty());
        assert_eq!(update.changed_message.unwrap().parts.len(), 1);
        let replay = project_update(
            &state,
            vec![ClientEvent::SessionHistoryLoaded {
                session_id: "a".into(),
            }],
        );
        assert_eq!(replay.state.session("a").unwrap().messages.len(), 2);
        assert!(replay.changed_message.is_none());
    }

    #[test]
    fn activity_snapshot_projects_attached_activity_without_transcript_copy() {
        let mut state = ClientState::default();
        state.apply_server_message(ready("a"));
        state.record_prompt("a", "history".into());
        let events = state.apply_server_message(ServerMessage::SessionActivitySnapshot {
            session_id: "a".into(),
            subagents: Vec::new(),
            jobs: vec![mew_protocol::JobActivity {
                job_id: "job".into(),
                command: "sleep 1".into(),
                state: "completed".into(),
            }],
        });
        let update = project_update(&state, events);
        assert!(
            matches!(update.events.as_slice(),[ClientEvent::ActivitySnapshot {session_id}] if session_id=="a")
        );
        let session = update.state.session("a").unwrap();
        assert!(session.messages.is_empty());
        assert_eq!(session.jobs[0].state, "completed");
        assert!(update.changed_message.is_none());
    }

    #[tokio::test]
    async fn accepted_prompt_requires_the_matching_session_and_request_receipt() {
        let (connection, inbound, mut outbound) = connection();
        let (client, mut updates) = TuiClient::connect_with_transport(Box::new(ChannelTransport(
            Mutex::new(VecDeque::from([connection])),
        )))
        .await
        .unwrap();
        assert!(matches!(
            outbound.recv().await.unwrap(),
            ClientMessage::Ping
        ));
        inbound.send(ready("a")).unwrap();
        while !matches!(
            updates.recv().await.unwrap().events.as_slice(),
            [ClientEvent::SessionReady { .. }]
        ) {}
        ready_catalogs(&mut outbound, true).await;
        let submit_client = client.clone();
        let submit = tokio::spawn(async move {
            submit_client
                .submit("a", "request", "hello".into(), vec![])
                .await
        });
        assert!(
            matches!(outbound.recv().await.unwrap(),ClientMessage::SubmitPrompt{session_id,request_id,..} if session_id=="a" && request_id=="request")
        );
        inbound
            .send(ServerMessage::PromptReceipt {
                session_id: "b".into(),
                request_id: "request".into(),
                status: PromptStatus::Accepted,
                message: None,
            })
            .unwrap();
        let _ = updates.recv().await.unwrap();
        assert!(!submit.is_finished());
        inbound
            .send(ServerMessage::PromptReceipt {
                session_id: "a".into(),
                request_id: "request".into(),
                status: PromptStatus::Accepted,
                message: None,
            })
            .unwrap();
        submit.await.unwrap().unwrap();
        assert_eq!(client.session_id().await.as_deref(), Some("a"));
    }

    #[tokio::test]
    async fn reconnect_queries_uncertain_prompt_and_never_resubmits_it() {
        let (first, inbound, mut first_outbound) = connection();
        let (second, reconnect_inbound, mut second_outbound) = connection();
        let (client, mut updates) = TuiClient::connect_with_transport(Box::new(ChannelTransport(
            Mutex::new(VecDeque::from([first, second])),
        )))
        .await
        .unwrap();
        assert!(matches!(
            first_outbound.recv().await.unwrap(),
            ClientMessage::Ping
        ));
        inbound.send(ready("a")).unwrap();
        while !matches!(
            updates.recv().await.unwrap().events.as_slice(),
            [ClientEvent::SessionReady { .. }]
        ) {}
        ready_catalogs(&mut first_outbound, true).await;
        let submit_client = client.clone();
        let submit = tokio::spawn(async move {
            submit_client
                .submit("a", "request", "hello".into(), vec![])
                .await
        });
        assert!(matches!(
            first_outbound.recv().await.unwrap(),
            ClientMessage::SubmitPrompt { .. }
        ));
        drop(inbound);
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(2), second_outbound.recv())
                .await
                .unwrap()
                .unwrap(),
            ClientMessage::Ping
        ));
        assert!(
            matches!(second_outbound.recv().await.unwrap(),ClientMessage::AttachSession{session_id,client_kind:ClientKind::Tui} if session_id=="a")
        );
        reconnect_inbound.send(ready("a")).unwrap();
        ready_catalogs(&mut second_outbound, true).await;
        assert!(
            matches!(second_outbound.recv().await.unwrap(),ClientMessage::ReadPromptReceipt{session_id,request_id} if session_id=="a" && request_id=="request")
        );
        reconnect_inbound
            .send(ServerMessage::PromptReceipt {
                session_id: "a".into(),
                request_id: "request".into(),
                status: PromptStatus::Accepted,
                message: None,
            })
            .unwrap();
        submit.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn required_reply_is_dropped_after_its_session_is_replaced() {
        let (connection, inbound, mut outbound) = connection();
        let (client, mut updates) = TuiClient::connect_with_transport(Box::new(ChannelTransport(
            Mutex::new(VecDeque::from([connection])),
        )))
        .await
        .unwrap();
        let _ = outbound.recv().await.unwrap();
        inbound.send(ready("a")).unwrap();
        inbound
            .send(ServerMessage::PermissionRequest {
                request_id: "permission".into(),
                tool_name: "shell".into(),
                input: serde_json::json!({}),
            })
            .unwrap();
        let reply = loop {
            let update = updates.recv().await.unwrap();
            if let Some((_, AgentEvent::PermissionRequest { tx, .. })) = update.required_action {
                break tx;
            }
        };
        inbound.send(ready("b")).unwrap();
        while !matches!(updates.recv().await.unwrap().events.as_slice(),[ClientEvent::SessionReady{session_id}] if session_id=="b")
        {
        }
        ready_catalogs(&mut outbound, true).await;
        ready_catalogs(&mut outbound, false).await;
        reply
            .send(mew_hooks::PermissionDecision::AllowOnce)
            .unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(100), outbound.recv())
                .await
                .is_err()
        );
        drop(client);
    }

    #[tokio::test]
    async fn expired_queued_commands_are_not_written() {
        let (connection, _inbound, mut outbound) = connection();
        let (client, _updates) = TuiClient::connect_with_transport(Box::new(ChannelTransport(
            Mutex::new(VecDeque::from([connection])),
        )))
        .await
        .unwrap();
        let _ = outbound.recv().await.unwrap();
        let (reply, receive) = oneshot::channel();
        drop(receive);
        client
            .commands
            .send(Command {
                session_owner: None,
                message: ClientMessage::Cancel,
                reply: Reply::Sent(reply),
            })
            .await
            .unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(100), outbound.recv())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn required_reply_is_forwarded_to_its_current_session() {
        let (connection, inbound, mut outbound) = connection();
        let (_client, mut updates) = TuiClient::connect_with_transport(Box::new(ChannelTransport(
            Mutex::new(VecDeque::from([connection])),
        )))
        .await
        .unwrap();
        let _ = outbound.recv().await.unwrap();
        inbound.send(ready("a")).unwrap();
        inbound
            .send(ServerMessage::PermissionRequest {
                request_id: "permission".into(),
                tool_name: "shell".into(),
                input: serde_json::json!({}),
            })
            .unwrap();
        let reply = loop {
            if let Some((_, AgentEvent::PermissionRequest { tx, .. })) =
                updates.recv().await.unwrap().required_action
            {
                break tx;
            }
        };
        ready_catalogs(&mut outbound, true).await;
        reply
            .send(mew_hooks::PermissionDecision::AllowOnce)
            .unwrap();
        assert!(
            matches!(tokio::time::timeout(Duration::from_millis(100),outbound.recv()).await.unwrap().unwrap(),ClientMessage::PermissionResponse {request_id,decision:mew_protocol::PermissionDecision::AllowOnce} if request_id=="permission")
        );
    }

    #[tokio::test]
    async fn unsent_prompt_has_a_definitive_rejection_receipt() {
        let (connection, _inbound, mut outbound) = connection();
        let (client, mut updates) = TuiClient::connect_with_transport(Box::new(ChannelTransport(
            Mutex::new(VecDeque::from([connection])),
        )))
        .await
        .unwrap();
        let _ = outbound.recv().await.unwrap();
        assert!(client
            .submit("a", "request", "hello".into(), vec![])
            .await
            .is_err());
        let update = tokio::time::timeout(Duration::from_millis(100), async {
            loop {
                let update = updates.recv().await.unwrap();
                if update
                    .events
                    .iter()
                    .any(|event| matches!(event, ClientEvent::PromptReceipt { .. }))
                {
                    break update;
                }
            }
        })
        .await
        .unwrap();
        assert!(
            matches!(update.events.as_slice(),[ClientEvent::PromptReceipt{session_id,request_id,status:PromptStatus::Rejected,..}] if session_id=="a" && request_id=="request")
        );
    }

    #[tokio::test]
    async fn scoped_commands_do_not_cross_the_confirmed_session() {
        let (connection, inbound, mut outbound) = connection();
        let (client, mut updates) = TuiClient::connect_with_transport(Box::new(ChannelTransport(
            Mutex::new(VecDeque::from([connection])),
        )))
        .await
        .unwrap();
        let _ = outbound.recv().await.unwrap();
        inbound.send(ready("b")).unwrap();
        while !matches!(
            updates.recv().await.unwrap().events.as_slice(),
            [ClientEvent::SessionReady { .. }]
        ) {}
        ready_catalogs(&mut outbound, true).await;
        for command in [
            ClientMessage::Cancel,
            ClientMessage::Guide {
                text: "steer".into(),
            },
        ] {
            assert!(client.send_to_session("a", command).await.is_err());
        }
        assert!(
            tokio::time::timeout(Duration::from_millis(100), outbound.recv())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn reconnect_keeps_unread_outcomes_and_refreshes_catalogs() {
        let (first, inbound, mut first_outbound) = connection();
        let (second, reconnect_inbound, mut second_outbound) = connection();
        let (_client, mut updates) = TuiClient::connect_with_transport(Box::new(ChannelTransport(
            Mutex::new(VecDeque::from([first, second])),
        )))
        .await
        .unwrap();
        let _ = first_outbound.recv().await.unwrap();
        inbound.send(ready("a")).unwrap();
        inbound
            .send(ServerMessage::SessionAlert {
                session_id: "b".into(),
                title: "failed".into(),
                kind: mew_protocol::AlertKind::TurnFailed,
                detail: Some("timeout".into()),
            })
            .unwrap();
        while !updates
            .recv()
            .await
            .unwrap()
            .state
            .unread_completions
            .contains("b")
        {}
        drop(inbound);
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(2), second_outbound.recv())
                .await
                .unwrap()
                .unwrap(),
            ClientMessage::Ping
        ));
        let _ = second_outbound.recv().await.unwrap();
        reconnect_inbound.send(ready("a")).unwrap();
        loop {
            let update = updates.recv().await.unwrap();
            if matches!(update.events.as_slice(), [ClientEvent::SessionReady { .. }]) {
                assert!(update.state.unread_completions.contains("b"));
                break;
            }
        }
        assert!(matches!(
            tokio::time::timeout(Duration::from_millis(100), second_outbound.recv())
                .await
                .unwrap()
                .unwrap(),
            ClientMessage::ListSessions
        ));
        assert!(matches!(
            second_outbound.recv().await.unwrap(),
            ClientMessage::ListPersonas
        ));
        assert!(matches!(
            second_outbound.recv().await.unwrap(),
            ClientMessage::ListProjects
        ));
        assert!(matches!(
            second_outbound.recv().await.unwrap(),
            ClientMessage::ListModels
        ));
    }
    struct DisconnectTransport {
        inner: mew_client_local::LocalWebSocketTransport,
        disconnect: Arc<tokio::sync::Notify>,
    }

    struct DisconnectConnection {
        inner: Box<dyn ClientConnection>,
        disconnect: Arc<tokio::sync::Notify>,
    }

    #[async_trait::async_trait]
    impl ClientTransport for DisconnectTransport {
        async fn connect(&self) -> Result<Box<dyn ClientConnection>, TransportError> {
            Ok(Box::new(DisconnectConnection {
                inner: self.inner.connect().await?,
                disconnect: self.disconnect.clone(),
            }))
        }
    }

    #[async_trait::async_trait]
    impl ClientConnection for DisconnectConnection {
        async fn send(&mut self, message: ClientMessage) -> Result<(), TransportError> {
            self.inner.send(message).await
        }
        async fn receive(&mut self) -> Result<Option<ServerMessage>, TransportError> {
            tokio::select! {
                result = self.inner.receive() => result,
                _ = self.disconnect.notified() => Err(TransportError::Closed),
            }
        }
        async fn close(&mut self) -> Result<(), TransportError> {
            self.inner.close().await
        }
    }

    #[tokio::test]
    async fn child_question_survives_same_session_reconnect_until_explicit_close() {
        use crate::runtime::CommandTarget;
        let dir = tempfile::tempdir().unwrap();
        let server = mew_daemon::DaemonServer::with_session_dir(
            Arc::new(|params| {
                Ok((
                    mew_agent::Agent::new(
                        Arc::new(mew_provider_fake::FakeProvider::new(Vec::new())),
                        Arc::new(mew_hooks::NopDispatcher),
                        Some(params.writer),
                        Vec::new(),
                        None,
                    ),
                    None,
                    None,
                ))
            }),
            dir.path().to_path_buf(),
        );
        let session = server.session_manager.create(None).await.unwrap();
        let (answer, mut pending) = oneshot::channel();
        let request = ServerMessage::AskUserRequest {
            request_id: "child-question".into(),
            call_id: "ask".into(),
            questions: Vec::new(),
            subagent: Some(mew_protocol::SubagentQuestionSource {
                parent_call_id: "parent-call".into(),
                child_session_id: "child".into(),
                name: "worker".into(),
                display_name: Some("worker one".into()),
            }),
        };
        session.pending_ask_user.lock().await.insert(
            "child-question".into(),
            mew_daemon::session::PendingRequest {
                sender: answer,
                message: request,
            },
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let daemon = tokio::spawn(server.run_tcp(address));
        let url = format!("ws://{address}");
        let disconnect = Arc::new(tokio::sync::Notify::new());
        let (client, mut updates) = tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if let Ok(client) =
                    TuiClient::connect_with_transport(Box::new(DisconnectTransport {
                        inner: mew_client_local::LocalWebSocketTransport::new(&url),
                        disconnect: disconnect.clone(),
                    }))
                    .await
                {
                    break client;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        client
            .send(ClientMessage::AttachSession {
                session_id: session.id.clone(),
                client_kind: ClientKind::Tui,
            })
            .await
            .unwrap();
        let original = tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if let Some((_, AgentEvent::SubagentAskUser { tx, .. })) =
                    updates.recv().await.unwrap().required_action
                {
                    break tx;
                }
            }
        })
        .await
        .unwrap();
        disconnect.notify_one();
        let replay = tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if let Some((_, AgentEvent::SubagentAskUser { tx, .. })) =
                    updates.recv().await.unwrap().required_action
                {
                    break tx;
                }
            }
        })
        .await
        .unwrap();
        drop(original);
        assert!(
            tokio::time::timeout(Duration::from_millis(100), &mut pending)
                .await
                .is_err(),
            "dropping a superseded UI sender must leave the daemon question pending"
        );
        assert!(session
            .pending_ask_user
            .lock()
            .await
            .contains_key("child-question"));
        let mut target = crate::runtime::daemon::DaemonTarget::new(client);
        target
            .close_question(&session.id, "child-question")
            .await
            .unwrap();
        assert!(tokio::time::timeout(Duration::from_secs(3), pending)
            .await
            .unwrap()
            .is_err());
        assert!(!session
            .pending_ask_user
            .lock()
            .await
            .contains_key("child-question"));
        drop(replay);
        daemon.abort();
    }
    #[tokio::test]
    async fn refresh_send_failure_enters_visible_backoff() {
        let (connection, inbound, mut outbound) = connection();
        let (_client, mut updates) = TuiClient::connect_with_transport(Box::new(ChannelTransport(
            Mutex::new(VecDeque::from([connection])),
        )))
        .await
        .unwrap();
        assert!(matches!(
            outbound.recv().await.unwrap(),
            ClientMessage::Ping
        ));
        drop(outbound);
        inbound.send(ready("a")).unwrap();
        let update = tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let update = updates.recv().await.unwrap();
                if matches!(
                    update.state.connection,
                    Some(ConnectionStatus::Backoff { .. })
                ) {
                    break update;
                }
            }
        })
        .await
        .unwrap();
        assert!(
            matches!(&update.state.connection,Some(ConnectionStatus::Backoff {error,..}) if error.contains("closed"))
        );
    }

    #[tokio::test]
    async fn real_daemon_offscreen_turns_refresh_latest_sessions_with_persisted_timestamps() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().to_path_buf();
        let server = mew_daemon::DaemonServer::with_session_dir(
            Arc::new(|params| {
                Ok((
                    mew_agent::Agent::new(
                        Arc::new(mew_provider_fake::FakeProvider::new(
                            mew_provider_fake::FakeProvider::text_response("completed response"),
                        )),
                        Arc::new(mew_hooks::NopDispatcher),
                        Some(params.writer),
                        Vec::new(),
                        None,
                    ),
                    None,
                    None,
                ))
            }),
            root.clone(),
        );
        let a = server.session_manager.create(None).await.unwrap();
        let b = server.session_manager.create(None).await.unwrap();
        for (session, timestamp) in [(&a, 100), (&b, 200)] {
            let mut events = session.agent.lock().await.run("initial prompt".into());
            while events.recv().await.is_some() {}
            let mut meta = mew_session::Meta::read(&root, &session.id)
                .await
                .unwrap()
                .unwrap();
            meta.last_message_at = Some(timestamp);
            meta.created_at = 1;
            meta.write(&root).await.unwrap();
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let daemon = tokio::spawn(server.run_tcp(address));
        let url = format!("ws://{address}");
        let (observer, mut updates) = tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if let Ok(client) = TuiClient::connect(&url).await {
                    break client;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        observer
            .send(ClientMessage::AttachSession {
                session_id: a.id.clone(),
                client_kind: ClientKind::Tui,
            })
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let update = updates.recv().await.unwrap();
                if update
                    .state
                    .session_list
                    .first()
                    .is_some_and(|session| session.session_id == b.id)
                {
                    assert_eq!(update.state.session_list[0].last_message_at, Some(200));
                    break;
                }
            }
        })
        .await
        .unwrap();
        let (other, mut other_updates) = TuiClient::connect(&url).await.unwrap();
        other
            .send(ClientMessage::NewSession {
                cwd: Some(root.display().to_string()),
                client_kind: ClientKind::Tui,
            })
            .await
            .unwrap();
        let new_session = tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let update = other_updates.recv().await.unwrap();
                if let Some(id) = update.events.iter().find_map(|event| match event {
                    ClientEvent::SessionReady { session_id } => Some(session_id.clone()),
                    _ => None,
                }) {
                    break id;
                }
            }
        })
        .await
        .unwrap();
        other
            .submit(
                &new_session,
                "offscreen",
                "offscreen prompt".into(),
                Vec::new(),
            )
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(3),async {
            loop {
                let update=updates.recv().await.unwrap();
                if update.events.iter().any(|event|matches!(event,ClientEvent::SessionActivityChanged {session_id,running:false} if session_id==&new_session)) {break;}
            }
        }).await.unwrap();
        let persisted = mew_session::Meta::read(&root, &new_session)
            .await
            .unwrap()
            .unwrap()
            .last_message_at;
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let update = updates.recv().await.unwrap();
                if update.state.session_list.first().is_some_and(|session| {
                    session.session_id == new_session
                        && session.last_message_at == persisted
                        && session.state != mew_protocol::SessionState::Running
                }) {
                    assert_eq!(
                        update.state.attached_session.as_deref(),
                        Some(a.id.as_str())
                    );
                    break;
                }
            }
        })
        .await
        .unwrap();
        observer
            .submit(&a.id, "attached", "latest prompt".into(), Vec::new())
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(3),async {
            loop {
                let update=updates.recv().await.unwrap();
                if update.events.iter().any(|event|matches!(event,ClientEvent::SessionActivityChanged {session_id,running:false} if session_id==&a.id)) {break;}
            }
        }).await.unwrap();
        let persisted = mew_session::Meta::read(&root, &a.id)
            .await
            .unwrap()
            .unwrap()
            .last_message_at;
        assert_ne!(persisted, Some(100));
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let update = updates.recv().await.unwrap();
                if update.state.session_list.first().is_some_and(|session| {
                    session.session_id == a.id
                        && session.last_message_at == persisted
                        && session.state != mew_protocol::SessionState::Running
                }) {
                    break;
                }
            }
        })
        .await
        .unwrap();
        observer.send(ClientMessage::ListProjects).await.unwrap();
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let update = updates.recv().await.unwrap();
                if update
                    .events
                    .iter()
                    .any(|event| matches!(event, ClientEvent::ProjectListChanged))
                {
                    let project = update
                        .state
                        .projects
                        .iter()
                        .find(|project| project.path == root.display().to_string())
                        .unwrap();
                    let meta = mew_session::Meta::read(&root, &new_session)
                        .await
                        .unwrap()
                        .unwrap();
                    assert_eq!(project.last_used_at, meta.last_message_at);
                    break;
                }
            }
        })
        .await
        .unwrap();
        daemon.abort();
    }
}
