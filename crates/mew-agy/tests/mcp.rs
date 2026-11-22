use mew_agy::mcp::{McpServer, Tool};
use serde_json::{json, Value};
use std::time::Duration;

async fn setup() -> (
    McpServer,
    tokio::sync::mpsc::Receiver<mew_agy::mcp::ToolCall>,
    reqwest::Client,
) {
    let tool = Tool::new("echo", "Echo", json!({"type":"object","properties":{"text":{"type":"string"}},"required":["text"],"additionalProperties":false})).unwrap();
    let (server, calls) = McpServer::bind(vec![tool], 2).await.unwrap();
    (server, calls, reqwest::Client::new())
}

#[tokio::test]
async fn shutdown_waits_for_pending_connections_to_close() {
    let (mut server, mut calls, client) = setup().await;
    let session = initialize(&server, &client).await;
    let url = server.url().to_owned();
    let request = tokio::spawn(async move {
        client
            .post(url)
            .header("mcp-session-id", session)
            .json(&json!({
                "jsonrpc":"2.0", "id":2, "method":"tools/call",
                "params":{"name":"echo","arguments":{"text":"wait"}}
            }))
            .send()
            .await
    });
    let call = calls.recv().await.unwrap();
    server.shutdown().await.unwrap();
    assert!(call.cancellation.is_cancelled());
    assert!(request.await.unwrap().is_err());
}
async fn initialize(server: &McpServer, client: &reqwest::Client) -> String {
    let response = client.post(server.url()).json(&json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"1"}}})).send().await.unwrap();
    assert_eq!(response.status(), 200);
    let session = response
        .headers()
        .get("mcp-session-id")
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    let value: Value = response.json().await.unwrap();
    assert_eq!(value["result"]["protocolVersion"], "2025-06-18");
    let response = client
        .post(server.url())
        .header("mcp-session-id", &session)
        .json(&json!({"jsonrpc":"2.0","method":"notifications/initialized"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 202);
    session
}
#[tokio::test]
async fn real_client_roundtrip_waits_for_explicit_correlated_result_and_rejects_duplicate() {
    let (server, mut calls, client) = setup().await;
    let session = initialize(&server, &client).await;
    let list: Value = client
        .post(server.url())
        .header("mcp-session-id", &session)
        .json(&json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list["result"]["tools"][0]["name"], "echo");
    let req = json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"echo","arguments":{"text":"hello"}}});
    let url = server.url().to_owned();
    let client2 = client.clone();
    let session2 = session.clone();
    let req2 = req.clone();
    let pending = tokio::spawn(async move {
        client2
            .post(url)
            .header("mcp-session-id", session2)
            .json(&req2)
            .send()
            .await
            .unwrap()
            .json::<Value>()
            .await
            .unwrap()
    });
    let call = tokio::time::timeout(Duration::from_secs(3), calls.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(call.name, "echo");
    assert_eq!(call.arguments, json!({"text":"hello"}));
    let duplicate: Value = client
        .post(server.url())
        .header("mcp-session-id", &session)
        .json(&req)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(duplicate.get("error").is_some());
    assert!(
        tokio::time::timeout(Duration::from_millis(100), calls.recv())
            .await
            .is_err()
    );
    server
        .resolve(&call.id, json!({"content":[{"type":"text","text":"done"}]}))
        .unwrap();
    let result = pending.await.unwrap();
    assert!(!call.cancellation.is_cancelled());
    assert_eq!(result["result"]["content"][0]["text"], "done");
    assert!(server.resolve(&call.id, json!({})).is_err());
}
#[tokio::test]
async fn rejects_untrusted_origins_wrong_path_invalid_arguments_and_oversized_body() {
    let (server, mut calls, client) = setup().await;
    let session = initialize(&server, &client).await;
    assert_eq!(
        client
            .post(server.url())
            .header("Origin", "http://attacker.invalid")
            .body("{}")
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    let wrong = server
        .url()
        .replace(server.url().rsplit('/').next().unwrap(), "wrong");
    assert_eq!(
        client.post(wrong).body("{}").send().await.unwrap().status(),
        404
    );
    for (id, params) in [
        (10, json!({"name":"echo","arguments":{"text":9}})),
        (11, json!({"name":"unknown","arguments":{}})),
    ] {
        let v: Value = client
            .post(server.url())
            .header("mcp-session-id", &session)
            .json(&json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":params}))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert!(v.get("error").is_some());
    }
    assert_eq!(
        client
            .post(server.url())
            .body("x".repeat(1_048_577))
            .send()
            .await
            .unwrap()
            .status(),
        413
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(100), calls.recv())
            .await
            .is_err()
    );
}
#[tokio::test]
async fn dropping_connection_cancels_outstanding_call_and_keeps_id_consumed() {
    let (server, mut calls, client) = setup().await;
    let session = initialize(&server, &client).await;
    let req = json!({"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"echo","arguments":{"text":"test"}}});
    let url = server.url().to_owned();
    let session2 = session.clone();
    let client2 = client.clone();
    let req2 = req.clone();
    let task = tokio::spawn(async move {
        client2
            .post(url)
            .header("mcp-session-id", session2)
            .json(&req2)
            .send()
            .await
    });
    let call = tokio::time::timeout(Duration::from_secs(3), calls.recv())
        .await
        .unwrap()
        .unwrap();
    task.abort();
    tokio::time::timeout(Duration::from_secs(3), call.cancellation.cancelled())
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if server.is_cancelled(&call.id) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await
        }
    })
    .await
    .unwrap();
    assert!(server.resolve(&call.id, json!({})).is_err());
    let result: Value = client
        .post(server.url())
        .header("mcp-session-id", session)
        .json(&req)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(result.get("error").is_some());
}

#[test]
fn schemas_may_use_local_refs_but_cannot_read_external_files() {
    assert!(Tool::new(
        "local",
        "",
        json!({"$defs":{"t":{"type":"string"}},"$ref":"#/$defs/t"})
    )
    .is_ok());
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(file.path(), "{\"type\":\"object\"}").unwrap();
    let url = url::Url::from_file_path(file.path()).unwrap();
    assert!(Tool::new("external", "", json!({"$ref":url.as_str()})).is_err());
}

#[tokio::test]
async fn supports_other_known_mcp_versions_and_rejects_unknown_versions() {
    for (version, accepted) in [
        ("2025-03-26", true),
        ("2025-11-25", true),
        ("1900-01-01", false),
    ] {
        let (server, _calls, client) = setup().await;
        let result:Value=client.post(server.url()).json(&json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":version,"capabilities":{},"clientInfo":{"name":"test","version":"1"}}})).send().await.unwrap().json().await.unwrap();
        if accepted {
            assert_eq!(result["result"]["protocolVersion"], version)
        } else {
            assert!(result.get("error").is_some())
        }
    }
}

#[tokio::test]
async fn explicit_cancellation_drops_only_the_matching_pending_call() {
    let (server, mut calls, client) = setup().await;
    let session = initialize(&server, &client).await;
    let url = server.url().to_owned();
    let sid = session.clone();
    let c = client.clone();
    let pending = tokio::spawn(async move {
        c.post(url).header("mcp-session-id",sid).json(&json!({"jsonrpc":"2.0","id":13,"method":"tools/call","params":{"name":"echo","arguments":{"text":"hi"}}})).send().await.unwrap().json::<Value>().await.unwrap()
    });
    let call = tokio::time::timeout(Duration::from_secs(3), calls.recv())
        .await
        .unwrap()
        .unwrap();
    let response = client
        .post(server.url())
        .header("mcp-session-id", session)
        .json(
            &json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":13}}),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 202);
    tokio::time::timeout(Duration::from_secs(3), call.cancellation.cancelled())
        .await
        .unwrap();
    assert!(tokio::time::timeout(Duration::from_secs(3), pending)
        .await
        .unwrap()
        .unwrap()
        .get("error")
        .is_some());
    assert!(server.resolve(&call.id, json!({})).is_err());
}

#[tokio::test]
async fn pending_call_capacity_rejects_excess_without_delivery() {
    let (server, mut calls, client) = setup().await;
    let session = initialize(&server, &client).await;
    let mut tasks = Vec::new();
    for id in 20..22 {
        let url = server.url().to_owned();
        let sid = session.clone();
        let client = client.clone();
        tasks.push(tokio::spawn(async move {client.post(url).header("mcp-session-id",sid).json(&json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":"echo","arguments":{"text":"hi"}}})).send().await.unwrap().json::<Value>().await.unwrap()}));
    }
    let first = tokio::time::timeout(Duration::from_secs(3), calls.recv())
        .await
        .unwrap()
        .unwrap();
    let second = tokio::time::timeout(Duration::from_secs(3), calls.recv())
        .await
        .unwrap()
        .unwrap();
    let denied:Value=client.post(server.url()).header("mcp-session-id",session).json(&json!({"jsonrpc":"2.0","id":22,"method":"tools/call","params":{"name":"echo","arguments":{"text":"extra"}}})).send().await.unwrap().json().await.unwrap();
    assert!(denied.get("error").is_some());
    assert!(
        tokio::time::timeout(Duration::from_millis(100), calls.recv())
            .await
            .is_err()
    );
    server.resolve(&first.id, json!({"content":[]})).unwrap();
    server.resolve(&second.id, json!({"content":[]})).unwrap();
    for task in tasks {
        assert!(task.await.unwrap().get("result").is_some());
    }
}

#[tokio::test]
async fn tool_calls_accept_object_metadata_and_reject_invalid_or_unknown_fields() {
    let (server, mut calls, client) = setup().await;
    let session = initialize(&server, &client).await;
    for (id, params) in [
        (
            31,
            json!({"name":"echo","arguments":{"text":"hi"},"_meta":[]}),
        ),
        (
            32,
            json!({"name":"echo","arguments":{"text":"hi"},"_meta":null}),
        ),
        (
            33,
            json!({"name":"echo","arguments":{"text":"hi"},"unexpected":true}),
        ),
    ] {
        let response: Value = client
            .post(server.url())
            .header("mcp-session-id", &session)
            .json(&json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":params}))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert!(response.get("error").is_some());
    }
    let url = server.url().to_owned();
    let sid = session.clone();
    let client2 = client.clone();
    let response = tokio::spawn(async move {
        client2.post(url).header("mcp-session-id",sid).json(&json!({"jsonrpc":"2.0","id":34,"method":"tools/call","params":{"name":"echo","arguments":{"text":"hi"},"_meta":{"antigravity.google/root_conversation_id":"example", "progressToken": 7}}})).send().await.unwrap().json::<Value>().await.unwrap()
    });
    let call = tokio::time::timeout(Duration::from_secs(3), calls.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(call.arguments, json!({"text":"hi"}));
    server.resolve(&call.id, json!({"content":[]})).unwrap();
    assert!(response.await.unwrap().get("result").is_some());
}

#[tokio::test]
async fn oversized_tool_result_fails_closed_and_releases_pending_capacity() {
    let (server, mut calls, client) = setup().await;
    let session = initialize(&server, &client).await;
    let url = server.url().to_owned();
    let sid = session.clone();
    let client2 = client.clone();
    let response = tokio::spawn(async move {
        client2.post(url).header("mcp-session-id",sid).json(&json!({"jsonrpc":"2.0","id":40,"method":"tools/call","params":{"name":"echo","arguments":{"text":"hi"}}})).send().await.unwrap().json::<Value>().await.unwrap()
    });
    let call = tokio::time::timeout(Duration::from_secs(3), calls.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(server
        .resolve(
            &call.id,
            json!({"content":[{"type":"text","text":"a".repeat(1_048_577)}]})
        )
        .is_err());
    assert!(tokio::time::timeout(Duration::from_secs(3), response)
        .await
        .unwrap()
        .unwrap()
        .get("error")
        .is_some());
    assert!(server.is_cancelled(&call.id));
}

#[test]
fn oversized_tool_description_and_schema_are_rejected() {
    assert!(Tool::new("echo", &"a".repeat(8193), json!({"type":"object"})).is_err());
    assert!(Tool::new("echo", "", json!({"description":"a".repeat(65_537)})).is_err());
}

#[tokio::test]
async fn second_initialization_and_concurrent_initializations_fail_closed() {
    let (server, _calls, client) = setup().await;
    let req = json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"1"}}});
    let a = client.post(server.url()).json(&req).send();
    let b = client.post(server.url()).json(&req).send();
    let (a, b) = tokio::join!(a, b);
    let a: Value = a.unwrap().json().await.unwrap();
    let b: Value = b.unwrap().json().await.unwrap();
    assert_eq!(
        [&a, &b]
            .iter()
            .filter(|v| v.get("result").is_some())
            .count(),
        1
    );
    assert_eq!(
        [&a, &b].iter().filter(|v| v.get("error").is_some()).count(),
        1
    );
    let retry: Value = client
        .post(server.url())
        .json(&req)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(retry.get("error").is_some());
}

#[tokio::test]
async fn quiesce_atomically_rejects_new_calls_and_reports_existing_calls() {
    let (server, mut calls, client) = setup().await;
    let session = initialize(&server, &client).await;
    let request = json!({"jsonrpc":"2.0","id":91,"method":"tools/call","params":{"name":"echo","arguments":{"text":"one"}}});
    let client2 = client.clone();
    let url = server.url().to_owned();
    let session2 = session.clone();
    let waiting = tokio::spawn(async move {
        client2
            .post(url)
            .header("mcp-session-id", session2)
            .json(&request)
            .send()
            .await
            .unwrap()
            .json::<Value>()
            .await
            .unwrap()
    });
    let call = tokio::time::timeout(Duration::from_secs(3), calls.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(!server.quiesce());
    let rejected: Value = client.post(server.url()).header("mcp-session-id", &session).json(&json!({"jsonrpc":"2.0","id":92,"method":"tools/call","params":{"name":"echo","arguments":{"text":"two"}}})).send().await.unwrap().json().await.unwrap();
    assert!(rejected.get("error").is_some());
    assert!(
        tokio::time::timeout(Duration::from_millis(100), calls.recv())
            .await
            .is_err()
    );
    server.resolve(&call.id, json!({"content":[]})).unwrap();
    assert!(!server.quiesce(), "the HTTP request is still completing");
    assert!(waiting.await.unwrap().get("result").is_some());
    assert!(server.quiesce());
}
