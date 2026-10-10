//! Ownership confirmation must precede the new session's implicit activity.

use std::sync::Arc;
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use mew_agent::Agent;
use mew_daemon::DaemonServer;
use mew_hooks::NopDispatcher;
use mew_protocol::{AlertKind, ClientKind, ClientMessage, ServerMessage};
use mew_provider_fake::FakeProvider;
use tokio::io::DuplexStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;

type Ws = WebSocketStream<DuplexStream>;

async fn connect(server: &DaemonServer) -> Ws {
    let (client, stream) = tokio::io::duplex(256 * 1024);
    let manager = server.session_manager.clone();
    let groups = server.groups_store.clone();
    tokio::spawn(async move {
        mew_daemon::handle_connection(
            stream,
            manager,
            groups,
            None,
            Arc::new(std::sync::atomic::AtomicBool::new(false)),
        )
        .await
        .unwrap();
    });
    let mut socket = tokio_tungstenite::client_async("ws://localhost/", client)
        .await
        .unwrap()
        .0;
    send(&mut socket, ClientMessage::SetAutoTitle { enabled: false }).await;
    socket
}

async fn send(socket: &mut Ws, command: ClientMessage) {
    socket
        .send(Message::Text(mew_protocol::encode_json(&command).unwrap()))
        .await
        .unwrap();
}

async fn receive(socket: &mut Ws) -> ServerMessage {
    let frame = tokio::time::timeout(Duration::from_secs(5), socket.next())
        .await
        .expect("daemon reply timed out")
        .unwrap()
        .unwrap();
    let message = mew_protocol::decode_json(&frame.into_text().unwrap()).unwrap();
    assert!(
        !matches!(&message, ServerMessage::Error { .. }),
        "unexpected daemon error: {message:?}"
    );
    message
}

async fn flush(socket: &mut Ws) -> Vec<ServerMessage> {
    send(socket, ClientMessage::Ping).await;
    let mut messages = Vec::new();
    loop {
        let message = receive(socket).await;
        if matches!(message, ServerMessage::Pong { .. }) {
            return messages;
        }
        messages.push(message);
    }
}

#[tokio::test]
async fn attaching_session_confirms_owner_before_an_idle_job_completion() {
    let directory = tempfile::tempdir().unwrap();
    let release = directory.path().join("release-job");
    let command = format!(
        "while [ ! -f '{}' ]; do sleep 0.01; done; printf 'done\\n'",
        release.display()
    );
    let mut script = FakeProvider::tool_call(
        "shell_background",
        "start-job",
        serde_json::json!({"command": command}),
    );
    script.pop();
    script.extend(FakeProvider::tool_call(
        "exit_tool",
        "exit",
        serde_json::json!({"final_answer": "started"}),
    ));
    let server = DaemonServer::with_session_dir(
        Arc::new(move |params| {
            let session_id = params
                .session_id
                .strip_prefix("sess_")
                .and_then(|id| ulid::Ulid::from_string(id).ok());
            let mut agent = Agent::new(
                Arc::new(FakeProvider::new(script.clone())),
                Arc::new(NopDispatcher),
                Some(params.writer),
                vec![Arc::new(mew_tools::tools::exit_tool::ExitTool)],
                session_id,
            );
            agent.set_permission_engine(Arc::new(
                mew_config::permissions::PermissionEngine::new(vec![])
                    .with_mode(mew_hooks::PermissionMode::Dangerous),
            ));
            Ok((agent, None, None))
        }),
        directory.path().join("sessions"),
    );
    let previous = server.session_manager.create(None).await.unwrap();
    let next = server.session_manager.create(None).await.unwrap();
    let mut observer = connect(&server).await;
    send(
        &mut observer,
        ClientMessage::AttachSession {
            session_id: next.id.clone(),
            client_kind: ClientKind::Tui,
        },
    )
    .await;
    flush(&mut observer).await;
    send(
        &mut observer,
        ClientMessage::Prompt {
            text: "start background work".into(),
            attachments: Vec::new(),
        },
    )
    .await;
    let mut job_id = None;
    loop {
        match receive(&mut observer).await {
            ServerMessage::JobUpdate {
                job_id: id, state, ..
            } if state == "running" => job_id = Some(id),
            ServerMessage::SessionAlert {
                kind: AlertKind::TurnComplete,
                ..
            } => break,
            _ => {}
        }
    }
    let job_id = job_id.expect("the parent must finish with a running job");
    let mut socket = connect(&server).await;
    send(
        &mut socket,
        ClientMessage::AttachSession {
            session_id: previous.id.clone(),
            client_kind: ClientKind::Tui,
        },
    )
    .await;
    flush(&mut socket).await;

    // Pause attach inside its metadata read while an existing job completes.
    let metadata = next.model.lock().await;
    send(
        &mut socket,
        ClientMessage::AttachSession {
            session_id: next.id.clone(),
            client_kind: ClientKind::Tui,
        },
    )
    .await;
    tokio::time::timeout(Duration::from_secs(3), async {
        while next.agent.try_lock().is_ok() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("attach must reach the paused metadata read");
    tokio::fs::write(&release, b"ready").await.unwrap();
    loop {
        if matches!(receive(&mut observer).await, ServerMessage::JobUpdate {job_id: id, state, ..} if id == job_id && state == "completed")
        {
            break;
        }
    }
    drop(metadata);
    let messages = flush(&mut socket).await;
    let ready = messages
        .iter()
        .position(|message| matches!(message, ServerMessage::SessionReady {session_id, ..} if session_id == &next.id))
        .expect("attach must confirm the new owner");
    assert!(
        messages[..ready]
            .iter()
            .all(|message| !matches!(message, ServerMessage::JobUpdate { .. })),
        "new session activity preceded ownership confirmation"
    );
    assert!(messages.iter().any(|message| matches!(message, ServerMessage::SessionActivitySnapshot {session_id, jobs, ..} if session_id == &next.id && jobs.iter().any(|job| job.job_id == job_id && job.state == "completed" && job.command == command))));
    socket.close(None).await.unwrap();
    observer.close(None).await.unwrap();
}
