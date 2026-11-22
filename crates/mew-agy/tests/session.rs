#![cfg(unix)]

use mew_agy::{
    mcp::{McpServer, Tool},
    process::{IsolationBoundary, LaunchSpec, ManagedProcess},
    session::{Session, SessionEvent},
};
use serde_json::{json, Value};
use std::{io, path::Path, time::Duration};
use tokio_util::sync::CancellationToken;

struct Boundary;
impl IsolationBoundary for Boundary {
    fn wrap(&self, spec: LaunchSpec, _: &Path) -> io::Result<LaunchSpec> {
        Ok(spec)
    }
}

async fn setup(script: &str) -> (Session, reqwest::Client, String, String, std::path::PathBuf) {
    let tool = Tool::new("echo", "Echo", json!({"type":"object","properties":{"text":{"type":"string"}},"required":["text"],"additionalProperties":false})).unwrap();
    let (server, calls) = McpServer::bind(vec![tool], 4).await.unwrap();
    let url = server.url().to_owned();
    let client = reqwest::Client::new();
    let response = client.post(&url).json(&json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"1"}}})).send().await.unwrap();
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
    let spec = LaunchSpec {
        program: "/bin/sh".into(),
        args: vec!["-c".into(), script.into()],
        env: vec![("PATH".into(), "/usr/bin:/bin".into())],
        cwd: workspace.path().to_owned(),
    };
    let process =
        ManagedProcess::spawn(spec, &Boundary, workspace, Duration::from_secs(15)).unwrap();
    (
        Session::new(process, server, calls, Duration::from_secs(3)).unwrap(),
        client,
        url,
        sid,
        path,
    )
}

fn call(
    client: &reqwest::Client,
    url: &str,
    sid: &str,
    id: u64,
    text: &str,
) -> tokio::task::JoinHandle<Result<Value, reqwest::Error>> {
    let (client, url, sid, text) = (
        client.clone(),
        url.to_owned(),
        sid.to_owned(),
        text.to_owned(),
    );
    tokio::spawn(async move {
        client.post(url).header("mcp-session-id", sid).json(&json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":"echo","arguments":{"text":text}}})).send().await?.json().await
    })
}

#[tokio::test]
async fn queued_calls_require_exact_handoff_and_single_correlated_result() {
    let (session, client, url, sid, _) = setup("sleep 10").await;
    let cancel = CancellationToken::new();
    let mut turn = session.start(b"", cancel.clone()).await.unwrap();
    let first = call(&client, &url, &sid, 2, "first");
    let SessionEvent::ToolCall(first_call) = turn.next().await.unwrap().unwrap() else {
        panic!("expected first tool")
    };
    let second = call(&client, &url, &sid, 3, "second");
    turn.ack_handoff(&first_call.id).await.unwrap();
    drop(turn);
    assert!(session
        .resume("wrong", json!({"content":[]}))
        .await
        .is_err());
    assert!(!first.is_finished());
    assert!(!second.is_finished());
    let result = json!({"content":[{"type":"text","text":"first result"}]});
    let mut next = session
        .resume(&first_call.id, result.clone())
        .await
        .unwrap();
    assert_eq!(first.await.unwrap().unwrap()["result"], result);
    assert!(session.resume(&first_call.id, result).await.is_err());
    let SessionEvent::ToolCall(second_call) = next.next().await.unwrap().unwrap() else {
        panic!("expected queued second tool")
    };
    assert_ne!(first_call.id, second_call.id);
    next.ack_handoff(&second_call.id).await.unwrap();
    drop(next);
    let result2 = json!({"content":[{"type":"text","text":"second result"}]});
    let followup = session
        .resume(&second_call.id, result2.clone())
        .await
        .unwrap();
    assert_eq!(second.await.unwrap().unwrap()["result"], result2);
    drop(followup);
}

#[tokio::test]
async fn dropping_unacknowledged_tool_turn_cancels_process_and_pending_mcp() {
    let (session, client, url, sid, workspace) = setup("sleep 10").await;
    let cancel = CancellationToken::new();
    let mut turn = session.start(b"", cancel.clone()).await.unwrap();
    let pending = call(&client, &url, &sid, 2, "hello");
    let SessionEvent::ToolCall(_) = turn.next().await.unwrap().unwrap() else {
        panic!("expected tool")
    };
    drop(turn);
    tokio::time::timeout(Duration::from_secs(4), cancel.cancelled())
        .await
        .unwrap();
    let response = tokio::time::timeout(Duration::from_secs(5), pending)
        .await
        .unwrap()
        .unwrap();
    assert!(response.map_or(true, |value| value.get("error").is_some()));
    tokio::time::timeout(Duration::from_secs(5), async {
        while workspace.exists() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn mcp_cancellation_after_handoff_cancels_the_owning_turn() {
    let (session, client, url, sid, workspace) = setup("sleep 10").await;
    let cancel = CancellationToken::new();
    let mut turn = session.start(b"", cancel.clone()).await.unwrap();
    let pending = call(&client, &url, &sid, 2, "hello");
    let SessionEvent::ToolCall(tool) = turn.next().await.unwrap().unwrap() else {
        panic!("expected tool")
    };
    turn.ack_handoff(&tool.id).await.unwrap();
    drop(turn);
    assert_eq!(client.post(&url).header("mcp-session-id", &sid).json(&json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":2}})).send().await.unwrap().status(), 202);
    tokio::time::timeout(Duration::from_secs(4), cancel.cancelled())
        .await
        .unwrap();
    let response = tokio::time::timeout(Duration::from_secs(5), pending)
        .await
        .unwrap()
        .unwrap();
    assert!(response.map_or(true, |value| value.get("error").is_some()));
    tokio::time::timeout(Duration::from_secs(5), async {
        while workspace.exists() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn early_cli_eof_is_an_error_and_does_not_report_completion() {
    let (session, _, _, _, _) = setup("exit 0").await;
    let mut turn = session.start(b"", CancellationToken::new()).await.unwrap();
    assert!(tokio::time::timeout(Duration::from_secs(5), turn.next())
        .await
        .unwrap()
        .is_err());
}

#[tokio::test]
async fn text_usage_and_terminal_result_are_delivered() {
    let script = r#"printf '%s\n' '{"event":"step_update","step_update":{"step_type":"agent_response","text_delta":"hello"}}' '{"event":"step_update","step_update":{"state":"DONE","usage":{"input_tokens":12,"output_tokens":4,"cache_read_tokens":3}}}' '{"event":"result","result":{"status":"SUCCESS"}}'; sleep 10"#;
    let (session, _, _, _, workspace) = setup(script).await;
    let mut turn = session.start(b"", CancellationToken::new()).await.unwrap();
    assert!(
        matches!(turn.next().await.unwrap(), Some(SessionEvent::Text(text)) if text == "hello")
    );
    assert!(matches!(
        turn.next().await.unwrap(),
        Some(SessionEvent::Usage {
            input: 12,
            output: 4,
            cache_read: 3
        })
    ));
    assert!(matches!(
        turn.next().await.unwrap(),
        Some(SessionEvent::Finished)
    ));
    tokio::time::timeout(Duration::from_secs(5), async {
        while workspace.exists() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn resolved_tool_can_complete_the_session() {
    let script = r#"while [ ! -f trigger ]; do sleep 0.02; done; printf '%s\n' '{"event":"result","result":{"status":"SUCCESS"}}'; sleep 10"#;
    let (session, client, url, sid, workspace) = setup(script).await;
    let mut turn = session.start(b"", CancellationToken::new()).await.unwrap();
    let pending = call(&client, &url, &sid, 2, "hello");
    let SessionEvent::ToolCall(tool) = turn.next().await.unwrap().unwrap() else {
        panic!("expected tool")
    };
    turn.ack_handoff(&tool.id).await.unwrap();
    drop(turn);
    let result = json!({"content":[{"type":"text","text":"done"}]});
    let mut resumed = session.resume(&tool.id, result.clone()).await.unwrap();
    tokio::fs::write(workspace.join("trigger"), b"")
        .await
        .unwrap();
    assert_eq!(pending.await.unwrap().unwrap()["result"], result);
    assert!(matches!(
        resumed.next().await.unwrap(),
        Some(SessionEvent::Finished)
    ));
    assert!(!workspace.exists());
    session.shutdown().await.unwrap();
}

#[tokio::test]
async fn terminal_cli_result_with_pending_mcp_call_fails_closed() {
    let script =
        r#"sleep 0.3; printf '%s\n' '{"event":"result","result":{"status":"SUCCESS"}}'; sleep 10"#;
    let (session, client, url, sid, workspace) = setup(script).await;
    let cancel = CancellationToken::new();
    let mut turn = session.start(b"", cancel.clone()).await.unwrap();
    let pending = call(&client, &url, &sid, 2, "hello");
    assert!(matches!(
        turn.next().await.unwrap(),
        Some(SessionEvent::ToolCall(_))
    ));
    assert!(tokio::time::timeout(Duration::from_secs(2), turn.next())
        .await
        .unwrap()
        .is_err());
    assert!(cancel.is_cancelled());
    let response = tokio::time::timeout(Duration::from_secs(5), pending)
        .await
        .unwrap()
        .unwrap();
    assert!(response.map_or(true, |value| value.get("error").is_some()));
    tokio::time::timeout(Duration::from_secs(5), async {
        while workspace.exists() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn unresolved_handoff_expires_and_cleans_up() {
    let (session, client, url, sid, workspace) = setup("sleep 10").await;
    let cancel = CancellationToken::new();
    let mut turn = session.start(b"", cancel.clone()).await.unwrap();
    let pending = call(&client, &url, &sid, 2, "hello");
    let SessionEvent::ToolCall(tool) = turn.next().await.unwrap().unwrap() else {
        panic!("expected tool")
    };
    turn.ack_handoff(&tool.id).await.unwrap();
    drop(turn);
    tokio::time::timeout(Duration::from_secs(5), cancel.cancelled())
        .await
        .unwrap();
    let response = tokio::time::timeout(Duration::from_secs(5), pending)
        .await
        .unwrap()
        .unwrap();
    assert!(response.map_or(true, |value| value.get("error").is_some()));
    tokio::time::timeout(Duration::from_secs(5), async {
        while workspace.exists() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn quota_failure_reports_category_without_exposing_cli_text() {
    let script = r#"printf '%s\n' '{"event":"result","result":{"status":"ERROR","error":"quota exceeded private-secret-value"}}'"#;
    let (session, _, _, _, _) = setup(script).await;
    let mut turn = session.start(b"", CancellationToken::new()).await.unwrap();
    let error = tokio::time::timeout(Duration::from_secs(3), turn.next())
        .await
        .unwrap()
        .unwrap_err()
        .to_string();
    assert!(error.to_lowercase().contains("quota"));
    assert!(!error.contains("private-secret-value"));
}

#[tokio::test]
async fn original_turn_cancellation_remains_effective_after_resume() {
    let (session, client, url, sid, workspace) = setup("sleep 10").await;
    let original = CancellationToken::new();
    let mut turn = session.start(b"", original.clone()).await.unwrap();
    let pending = call(&client, &url, &sid, 2, "hello");
    let SessionEvent::ToolCall(tool) = turn.next().await.unwrap().unwrap() else {
        panic!("expected tool")
    };
    turn.ack_handoff(&tool.id).await.unwrap();
    drop(turn);
    let mut resumed = session
        .resume(&tool.id, json!({"content":[]}))
        .await
        .unwrap();
    assert!(pending.await.unwrap().is_ok());
    original.cancel();
    assert!(tokio::time::timeout(Duration::from_secs(2), resumed.next())
        .await
        .unwrap()
        .is_err());
    tokio::time::timeout(Duration::from_secs(5), async {
        while workspace.exists() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn dropping_ack_future_before_receipt_still_cancels_handoff() {
    use std::{
        future::{poll_fn, Future},
        task::Poll,
    };
    let (session, client, url, sid, workspace) = setup("sleep 10").await;
    let cancel = CancellationToken::new();
    let mut turn = session.start(b"", cancel.clone()).await.unwrap();
    let pending = call(&client, &url, &sid, 2, "hello");
    let SessionEvent::ToolCall(tool) = turn.next().await.unwrap().unwrap() else {
        panic!("expected tool")
    };
    let mut ack = Box::pin(turn.ack_handoff(&tool.id));
    let polled = poll_fn(|cx| Poll::Ready(ack.as_mut().poll(cx))).await;
    assert!(polled.is_pending());
    tokio::time::sleep(Duration::from_millis(50)).await;
    drop(ack);
    drop(turn);
    tokio::time::timeout(Duration::from_secs(2), cancel.cancelled())
        .await
        .unwrap();
    let response = tokio::time::timeout(Duration::from_secs(5), pending)
        .await
        .unwrap()
        .unwrap();
    assert!(response.map_or(true, |value| value.get("error").is_some()));
    tokio::time::timeout(Duration::from_secs(5), async {
        while workspace.exists() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn oversized_result_does_not_consume_pending_tool() {
    let (session, client, url, sid, _) = setup("sleep 10").await;
    let mut turn = session.start(b"", CancellationToken::new()).await.unwrap();
    let pending = call(&client, &url, &sid, 2, "hello");
    let SessionEvent::ToolCall(tool) = turn.next().await.unwrap().unwrap() else {
        panic!("expected tool")
    };
    turn.ack_handoff(&tool.id).await.unwrap();
    drop(turn);
    assert!(session
        .resume(&tool.id, json!({"text":"x".repeat(1_048_577)}))
        .await
        .is_err());
    assert!(!pending.is_finished());
    let result = json!({"content":[]});
    let next = session.resume(&tool.id, result.clone()).await.unwrap();
    assert_eq!(pending.await.unwrap().unwrap()["result"], result);
    drop(next);
}

#[tokio::test]
async fn explicit_shutdown_waits_for_process_and_workspace_cleanup() {
    let (session, client, url, sid, workspace) = setup("sleep 10").await;
    let cancel = CancellationToken::new();
    let mut turn = session.start(b"", cancel.clone()).await.unwrap();
    let pending = call(&client, &url, &sid, 2, "hello");
    assert!(matches!(
        turn.next().await.unwrap(),
        Some(SessionEvent::ToolCall(_))
    ));
    session.shutdown().await.unwrap();
    assert!(cancel.is_cancelled());
    assert!(!workspace.exists());
    let response = pending.await.unwrap();
    assert!(response.map_or(true, |value| value.get("error").is_some()));
}

#[tokio::test]
async fn dropped_shutdown_future_keeps_join_handle_for_retry() {
    use std::{
        future::{poll_fn, Future},
        task::Poll,
    };
    let (session, client, url, sid, workspace) = setup("sleep 10").await;
    let mut turn = session.start(b"", CancellationToken::new()).await.unwrap();
    let pending = call(&client, &url, &sid, 2, "hello");
    assert!(matches!(
        turn.next().await.unwrap(),
        Some(SessionEvent::ToolCall(_))
    ));
    let mut shutdown = Box::pin(session.shutdown());
    let first = poll_fn(|cx| Poll::Ready(shutdown.as_mut().poll(cx))).await;
    assert!(first.is_pending());
    drop(shutdown);
    session.shutdown().await.unwrap();
    assert!(!workspace.exists());
    let response = pending.await.unwrap();
    assert!(response.map_or(true, |value| value.get("error").is_some()));
}
