use super::*;
use crate::request_tests::{connect, flush, send};

#[tokio::test]
async fn failed_browser_registration_releases_detached_session_runtime() {
    for endpoint in ["not-an-endpoint", "http://127.0.0.1:9223"] {
        let directory = tempfile::tempdir().unwrap();
        let executable = directory.path().join("missing-agent-browser");
        let server = DaemonServer::with_session_dir(
            Arc::new(move |params| {
                let mut agent = Agent::new(
                    Arc::new(mew_provider_fake::FakeProvider::new(Vec::new())),
                    Arc::new(mew_hooks::NopDispatcher),
                    Some(params.writer),
                    Vec::new(),
                    None,
                );
                agent.browser_access =
                    mew_tools::browser::BrowserAccess::with_executable(executable.clone());
                Ok((agent, None, None))
            }),
            directory.path().into(),
        );
        let a = server.session_manager.create(None).await.unwrap();
        let b = server.session_manager.create(None).await.unwrap();
        let weak = Arc::downgrade(&a);
        let mut socket = connect(&server).await;
        for session in [&a, &b] {
            send(
                &mut socket,
                ClientMessage::AttachSession {
                    session_id: session.id.clone(),
                    client_kind: ClientKind::Desktop,
                },
            )
            .await;
            flush(&mut socket).await;
        }
        send(
            &mut socket,
            ClientMessage::RegisterBrowserPage {
                session_id: a.id.clone(),
                page_instance: "failed-page".into(),
                cdp_endpoint: endpoint.into(),
                target_id: "target".into(),
            },
        )
        .await;
        assert!(matches!(
            next_browser_reply(&mut socket).await,
            ServerMessage::BrowserPageRegistration { error: Some(_), .. }
        ));
        assert!(!a.browser_access.is_available());
        assert!(
            server.session_manager.unload_detached(&a).await,
            "failed registration retained the detached runtime for {endpoint}"
        );
        drop(a);
        assert!(weak.upgrade().is_none());
        socket.close(None).await.unwrap();
    }
}

#[tokio::test]
async fn browser_response_captures_unavailable_page_before_task_scheduling() {
    let access = mew_tools::browser::BrowserAccess::default();
    let cancel = CancellationToken::new();
    let response = crate::browser::respond(
        &access,
        ClientMessage::BrowserSnapshot { tab_id: None },
        &cancel,
    );
    let preparation = access.register(
        "owner",
        mew_tools::browser::BrowserTarget {
            page_instance: "late-page".into(),
            cdp_endpoint: "http://127.0.0.1:9223".into(),
            target_id: "0123456789ABCDEF0123456789ABCDEF".into(),
        },
    );
    assert!(matches!(
        response.await,
        ServerMessage::BrowserError { tab_id: None, .. }
    ));
    access.clear();
    drop(preparation);
}

#[tokio::test]
async fn browser_registration_requires_confirmed_local_desktop_session() {
    let directory = tempfile::tempdir().unwrap();
    let server = DaemonServer::with_session_dir(
        Arc::new(|params| {
            Ok((
                Agent::new(
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
        directory.path().into(),
    );
    let session = server.session_manager.create(None).await.unwrap();
    for kind in [
        ClientKind::Tui,
        ClientKind::Web,
        ClientKind::Mobile,
        ClientKind::Cli,
        ClientKind::Unknown,
    ] {
        let mut socket = connect(&server).await;
        send(
            &mut socket,
            ClientMessage::AttachSession {
                session_id: session.id.clone(),
                client_kind: kind,
            },
        )
        .await;
        flush(&mut socket).await;
        send(
            &mut socket,
            ClientMessage::RegisterBrowserPage {
                session_id: session.id.clone(),
                page_instance: "page-a".into(),
                cdp_endpoint: "http://127.0.0.1:9223".into(),
                target_id: "0123456789ABCDEF0123456789ABCDEF".into(),
            },
        )
        .await;
        let replies = flush(&mut socket).await;
        assert!(replies.iter().any(|message| matches!(message, ServerMessage::BrowserPageRegistration { session_id, page_instance, error: Some(error) } if session_id == &session.id && page_instance == "page-a" && error.contains("desktop"))));
        assert!(!session.browser_access.is_available());
        socket.close(None).await.unwrap();
    }
}

#[tokio::test]
async fn remote_and_legacy_connections_cannot_register_local_pages() {
    for (scope, legacy) in [(Some(RemoteScope::Control), false), (None, true)] {
        let directory = tempfile::tempdir().unwrap();
        let server = DaemonServer::with_session_dir(
            Arc::new(|params| {
                Ok((
                    Agent::new(
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
            directory.path().into(),
        );
        let (client, stream) = tokio::io::duplex(256 * 1024);
        tokio::spawn(async move {
            handle_connection_with_authorizer(
                stream,
                server.session_manager.clone(),
                server.groups_store.clone(),
                None,
                server.auto_summary_enabled.clone(),
                scope,
                None,
                legacy,
            )
            .await
            .unwrap();
        });
        let mut socket = tokio_tungstenite::client_async("ws://localhost/", client)
            .await
            .unwrap()
            .0;
        send(
            &mut socket,
            ClientMessage::RemoteHello {
                token: None,
                device_name: "desktop".into(),
            },
        )
        .await;
        send(
            &mut socket,
            ClientMessage::RegisterBrowserPage {
                session_id: "session-a".into(),
                page_instance: "page-a".into(),
                cdp_endpoint: "http://127.0.0.1:9223".into(),
                target_id: "0123456789ABCDEF0123456789ABCDEF".into(),
            },
        )
        .await;
        let replies = flush(&mut socket).await;
        assert!(replies.iter().any(|reply| matches!(reply, ServerMessage::BrowserPageRegistration { session_id, page_instance, error: Some(error) } if session_id == "session-a" && page_instance == "page-a" && error.contains("remote"))));
        socket.close(None).await.unwrap();
    }
}

#[cfg(target_os = "macos")]
#[tokio::test]
#[ignore = "requires Chrome and MEW_BROWSER_TEST_CLI pointing to agent-browser >=0.34"]
async fn real_daemon_pages_survive_switching_and_revoke_on_disconnect_and_delete() {
    use mew_tools::browser::{BrowserAccess, BrowserAction};
    let profile = tempfile::tempdir().unwrap();
    let mut chrome = tokio::process::Command::new(
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    )
    .args([
        "--headless=new",
        "--remote-debugging-port=0",
        "--no-first-run",
        "--no-default-browser-check",
        "about:blank",
    ])
    .arg(format!("--user-data-dir={}", profile.path().display()))
    .stdout(std::process::Stdio::null())
    .stderr(std::process::Stdio::null())
    .kill_on_drop(true)
    .spawn()
    .unwrap();
    let port = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            if let Ok(text) =
                tokio::fs::read_to_string(profile.path().join("DevToolsActivePort")).await
            {
                break text.lines().next().unwrap().to_owned();
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    let endpoint = format!("http://127.0.0.1:{port}");
    let http = reqwest::Client::new();
    let target: serde_json::Value = http
        .put(format!("{endpoint}/json/new?about:blank"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let target_id = target["id"].as_str().unwrap().to_owned();
    let directory = tempfile::tempdir().unwrap();
    let executable =
        PathBuf::from(std::env::var("MEW_BROWSER_TEST_CLI").expect("set compatible CLI path"));
    let server = DaemonServer::with_session_dir(
        Arc::new(move |params| {
            let mut agent = Agent::new(
                Arc::new(mew_provider_fake::FakeProvider::new(Vec::new())),
                Arc::new(mew_hooks::NopDispatcher),
                Some(params.writer),
                Vec::new(),
                None,
            );
            agent.browser_access = BrowserAccess::with_executable(executable.clone());
            Ok((agent, None, None))
        }),
        directory.path().into(),
    );
    let a = server.session_manager.create(None).await.unwrap();
    let b = server.session_manager.create(None).await.unwrap();
    let registration = |instance: &str| ClientMessage::RegisterBrowserPage {
        session_id: a.id.clone(),
        page_instance: instance.into(),
        cdp_endpoint: endpoint.clone(),
        target_id: target_id.clone(),
    };
    let mut socket = connect(&server).await;
    send(
        &mut socket,
        ClientMessage::AttachSession {
            session_id: a.id.clone(),
            client_kind: ClientKind::Desktop,
        },
    )
    .await;
    flush(&mut socket).await;
    send(
        &mut socket,
        ClientMessage::AttachSession {
            session_id: b.id.clone(),
            client_kind: ClientKind::Desktop,
        },
    )
    .await;
    flush(&mut socket).await;
    // Late readiness belongs to previously confirmed A while B is selected.
    send(&mut socket, registration("page-a")).await;
    let ready = next_browser_reply(&mut socket).await;
    assert!(
        matches!(&ready, ServerMessage::BrowserPageRegistration { session_id, page_instance, error: None } if session_id == &a.id && page_instance == "page-a"),
        "page readiness: {ready:?}"
    );
    send(
        &mut socket,
        ClientMessage::RegisterBrowserPage {
            session_id: a.id.clone(),
            page_instance: "page-a".into(),
            cdp_endpoint: "http://127.0.0.1:1".into(),
            target_id: "different-target".into(),
        },
    )
    .await;
    assert!(matches!(
        next_browser_reply(&mut socket).await,
        ServerMessage::BrowserPageRegistration { error: Some(_), .. }
    ));
    assert!(
        !server.session_manager.unload_detached(&a).await,
        "failed duplicate registration released the live page's runtime"
    );
    send(&mut socket, registration("superseded-pending")).await;
    send(&mut socket, registration("page-a")).await;
    let mut registered = false;
    for _ in 0..2 {
        if let ServerMessage::BrowserPageRegistration {
            page_instance,
            error,
            ..
        } = next_browser_reply(&mut socket).await
        {
            if page_instance == "page-a" {
                assert!(
                    error.is_none(),
                    "replacement registration failed: {error:?}"
                );
                registered = true;
            }
        }
    }
    assert!(registered);
    assert!(
        !server.session_manager.unload_detached(&a).await,
        "late failed preparation removed the replacement's runtime"
    );
    let clone = a.agent.lock().await.clone();
    assert!(clone.browser_access.is_available());
    send(
        &mut socket,
        ClientMessage::AttachSession {
            session_id: a.id.clone(),
            client_kind: ClientKind::Desktop,
        },
    )
    .await;
    flush(&mut socket).await;
    send(&mut socket, ClientMessage::BrowserSnapshot { tab_id: None }).await;
    assert!(
        matches!(next_browser_reply(&mut socket).await, ServerMessage::BrowserSnapshot { tab_id: Some(id), .. } if id == "page-a")
    );
    let cancellation = CancellationToken::new();
    let old_snapshot = crate::browser::respond(
        &clone.browser_access,
        ClientMessage::BrowserSnapshot { tab_id: None },
        &cancellation,
    );
    send(&mut socket, registration("page-new")).await;
    assert!(matches!(
        next_browser_reply(&mut socket).await,
        ServerMessage::BrowserPageRegistration { error: None, .. }
    ));
    assert!(
        matches!(old_snapshot.await, ServerMessage::BrowserError { tab_id: Some(id), message } if id == "page-a" && message.contains("different"))
    );
    send(
        &mut socket,
        ClientMessage::UnregisterBrowserPage {
            session_id: a.id.clone(),
            page_instance: "page-a".into(),
        },
    )
    .await;
    assert!(matches!(
        next_browser_reply(&mut socket).await,
        ServerMessage::BrowserPageRegistration { error: Some(_), .. }
    ));
    assert!(clone.browser_access.is_available());
    send(
        &mut socket,
        ClientMessage::BrowserClick {
            selector: "body".into(),
            tab_id: Some("page-a".into()),
        },
    )
    .await;
    assert!(
        matches!(next_browser_reply(&mut socket).await, ServerMessage::BrowserError { tab_id: Some(id), message } if id == "page-a" && message.contains("different"))
    );
    socket.close(None).await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while clone.browser_access.is_available() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(clone
        .browser_access
        .execute(BrowserAction::Snapshot, None, &CancellationToken::new())
        .await
        .is_err());
    // Closing the CLI connection must preserve the actual user-owned page.
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let targets: Vec<serde_json::Value> = http
        .get(format!("{endpoint}/json/list"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(targets.iter().any(|target| target["id"] == target_id));
    let mut reconnect = connect(&server).await;
    send(&mut reconnect, registration("reconnected")).await;
    assert!(matches!(
        next_browser_reply(&mut reconnect).await,
        ServerMessage::BrowserPageRegistration { error: Some(_), .. }
    ));
    send(
        &mut reconnect,
        ClientMessage::AttachSession {
            session_id: a.id.clone(),
            client_kind: ClientKind::Desktop,
        },
    )
    .await;
    flush(&mut reconnect).await;
    send(&mut reconnect, registration("reconnected")).await;
    assert!(matches!(
        next_browser_reply(&mut reconnect).await,
        ServerMessage::BrowserPageRegistration { error: None, .. }
    ));
    let deleted = server.session_manager.create(None).await.unwrap();
    let deleted_id = deleted.id.clone();
    let deleted_runtime = Arc::downgrade(&deleted);
    send(
        &mut reconnect,
        ClientMessage::AttachSession {
            session_id: deleted_id.clone(),
            client_kind: ClientKind::Desktop,
        },
    )
    .await;
    flush(&mut reconnect).await;
    send(
        &mut reconnect,
        ClientMessage::RegisterBrowserPage {
            session_id: deleted_id.clone(),
            page_instance: "deleted-page".into(),
            cdp_endpoint: endpoint.clone(),
            target_id: target_id.clone(),
        },
    )
    .await;
    assert!(matches!(
        next_browser_reply(&mut reconnect).await,
        ServerMessage::BrowserPageRegistration { error: None, .. }
    ));
    send(
        &mut reconnect,
        ClientMessage::AttachSession {
            session_id: b.id.clone(),
            client_kind: ClientKind::Desktop,
        },
    )
    .await;
    flush(&mut reconnect).await;
    let mut deleting = connect(&server).await;
    send(
        &mut deleting,
        ClientMessage::DeleteSession {
            session_id: deleted_id.clone(),
        },
    )
    .await;
    flush(&mut deleting).await;
    drop(deleted);
    send(
        &mut reconnect,
        ClientMessage::RegisterBrowserPage {
            session_id: deleted_id,
            page_instance: "late-deleted-page".into(),
            cdp_endpoint: endpoint.clone(),
            target_id: target_id.clone(),
        },
    )
    .await;
    assert!(matches!(
        next_browser_reply(&mut reconnect).await,
        ServerMessage::BrowserPageRegistration { error: Some(_), .. }
    ));
    assert!(
        deleted_runtime.upgrade().is_none(),
        "failed registration retained a deleted runtime"
    );
    deleting.close(None).await.unwrap();
    // A disappeared lease still leaves a matching connection record to release.
    clone.browser_access.clear();
    send(
        &mut reconnect,
        ClientMessage::UnregisterBrowserPage {
            session_id: a.id.clone(),
            page_instance: "reconnected".into(),
        },
    )
    .await;
    assert!(matches!(
        next_browser_reply(&mut reconnect).await,
        ServerMessage::BrowserPageRegistration { error: None, .. }
    ));
    send(
        &mut reconnect,
        ClientMessage::AttachSession {
            session_id: b.id.clone(),
            client_kind: ClientKind::Desktop,
        },
    )
    .await;
    flush(&mut reconnect).await;
    assert!(
        server.session_manager.unload_detached(&a).await,
        "unregister retained a runtime after its lease disappeared"
    );
    send(
        &mut reconnect,
        ClientMessage::DeleteSession {
            session_id: a.id.clone(),
        },
    )
    .await;
    flush(&mut reconnect).await;
    assert!(!clone.browser_access.is_available());
    send(&mut reconnect, registration("late-deleted")).await;
    assert!(matches!(
        next_browser_reply(&mut reconnect).await,
        ServerMessage::BrowserPageRegistration { error: Some(_), .. }
    ));
    reconnect.close(None).await.unwrap();
    chrome.kill().await.unwrap();
}

async fn next_browser_reply(
    socket: &mut tokio_tungstenite::WebSocketStream<tokio::io::DuplexStream>,
) -> ServerMessage {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let frame = socket.next().await.unwrap().unwrap();
            if let Message::Text(text) = frame {
                let response: ServerMessage = mew_protocol::decode_json(&text).unwrap();
                if matches!(
                    response,
                    ServerMessage::BrowserPageRegistration { .. }
                        | ServerMessage::BrowserSnapshot { .. }
                        | ServerMessage::BrowserError { .. }
                ) {
                    return response;
                }
            }
        }
    })
    .await
    .unwrap()
}
