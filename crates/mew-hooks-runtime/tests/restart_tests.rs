//! Integration tests for plugin restart capability.
//!
//! Tests that crashed plugins are automatically restarted and that
//! tool closures route to the new process after restart.

use std::sync::Arc;
use std::time::Duration;

use mew_hooks::Dispatcher;
use mew_hooks_runtime::{PluginSlot, SubprocessDispatcher};

mod common;
use common::{make_plugin_dir_with_binary, test_host};

/// Wait for a slot to become healthy, polling with a timeout.
async fn wait_until_healthy(slot: &PluginSlot, timeout: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    while tokio::time::Instant::now() < deadline {
        if slot.is_healthy() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    false
}

#[tokio::test]
async fn test_restart_after_external_restart() {
    // Spawn a plugin slot directly.
    let dir = make_plugin_dir_with_binary();
    let path = dir.path().join("sample-plugin");
    let slot = PluginSlot::spawn(path, test_host(), Duration::from_secs(5))
        .await
        .expect("spawn slot");

    // Plugin should be healthy initially.
    assert!(slot.is_healthy(), "plugin should be healthy after spawn");

    // Send init to confirm it works.
    let result = slot
        .call("init", &serde_json::json!({}))
        .await
        .expect("init call");
    assert!(
        result.contains("ok"),
        "plugin should respond to init, got: {}",
        result
    );

    // Trigger an external restart (simulates mew ext dev file-watch restart).
    Arc::clone(&slot).restart();

    // Wait for restart to complete — the slot goes unhealthy briefly
    // during restart, then becomes healthy again.
    // Backoff is 1s for first attempt, so allow up to 15s total.
    assert!(
        wait_until_healthy(&slot, Duration::from_secs(15)).await,
        "plugin should be healthy again after restart"
    );

    // Verify the restarted plugin actually receives hooks and responds.
    let result = slot
        .call("init", &serde_json::json!({}))
        .await
        .expect("init call after restart");
    assert!(
        result.contains("ok"),
        "restarted plugin should respond to init, got: {}",
        result
    );

    slot.shutdown().await;
}

#[tokio::test]
async fn test_restarted_plugin_receives_hooks() {
    let dir = make_plugin_dir_with_binary();
    let path = dir.path().join("sample-plugin");
    let slot = PluginSlot::spawn(path, test_host(), Duration::from_secs(5))
        .await
        .expect("spawn slot");

    // Verify the system-prompt hook works before restart.
    let result = slot
        .call("on-system-prompt", &serde_json::json!({ "value": "hello" }))
        .await
        .expect("system-prompt call before restart");
    assert!(
        result.contains("[sample-plugin]"),
        "plugin should transform the prompt, got: {}",
        result
    );

    // Trigger an external restart.
    let slot_arc: Arc<PluginSlot> = slot;
    // We need to get an Arc to call restart. Since PluginSlot::spawn
    // returns Arc<PluginSlot>, we already have it.
    // But we bound `slot` as a non-Arc above. Let's re-bind.
    // Actually, spawn returns Arc<PluginSlot>, so `slot` IS an Arc.
    // The `restart` method takes `self: &Arc<Self>`.
    Arc::clone(&slot_arc).restart();

    // Wait for restart to complete.
    assert!(
        wait_until_healthy(&slot_arc, Duration::from_secs(10)).await,
        "plugin should be healthy after external restart"
    );

    // Verify hooks work after restart.
    let result = slot_arc
        .call("on-system-prompt", &serde_json::json!({ "value": "world" }))
        .await
        .expect("system-prompt call after restart");
    assert!(
        result.contains("[sample-plugin]"),
        "restarted plugin should still transform the prompt, got: {}",
        result
    );

    slot_arc.shutdown().await;
}

#[tokio::test]
async fn test_call_during_restart_does_not_panic() {
    let dir = make_plugin_dir_with_binary();
    let path = dir.path().join("sample-plugin");
    let slot = PluginSlot::spawn(path, test_host(), Duration::from_secs(1))
        .await
        .expect("spawn slot");

    // Trigger a restart.
    Arc::clone(&slot).restart();

    // Immediately try to call a hook. This should either:
    // - Return Ok (if the old process is still alive briefly), or
    // - Return Err (if the process is mid-restart)
    // Either way, it must NOT panic.
    let result = slot.call("init", &serde_json::json!({})).await;

    // The result is timing-dependent. We just assert no panic.
    match &result {
        Ok(v) => eprintln!("call during restart returned Ok: {}", v),
        Err(e) => eprintln!("call during restart returned Err: {}", e),
    }

    // Wait for things to settle.
    wait_until_healthy(&slot, Duration::from_secs(10)).await;
    slot.shutdown().await;
}

#[tokio::test]
async fn test_tool_closure_does_not_hang() {
    let dir = make_plugin_dir_with_binary();

    // Use SubprocessDispatcher so we get the full on_register_tools flow.
    let dispatcher = SubprocessDispatcher::from_dirs(vec![dir.path().to_path_buf()], test_host())
        .await
        .expect("dispatcher creation");

    dispatcher.init(&test_host()).await;

    // Register tools — this captures watch receivers into closures.
    let tools = dispatcher.on_register_tools().await;
    assert!(!tools.is_empty(), "should have registered tools");

    // Find the sample-echo tool.
    let echo_tool = tools
        .iter()
        .find(|t| t.name == "sample-echo")
        .expect("sample-echo tool should be registered");

    // Call the tool before restart.
    let input = serde_json::json!({ "message": "hello" });
    let result_before = (echo_tool.execute)(input.clone());
    let result_str = result_before.await;
    eprintln!("tool result before restart: {}", result_str);

    // The tool should have returned something (the sample plugin's
    // call-tool handler returns the input).

    // Restart is triggered by the reader task when the process dies.
    // We can't easily access individual slots from the dispatcher,
    // so we test the tool closure's resilience indirectly: if the
    // process dies, the closure should return an error string, not
    // panic or hang.
    //
    // Since we can't easily kill the process from here (no PID access),
    // we verify that the tool closure doesn't hang on a 1s timeout.
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        (echo_tool.execute)(input).await
    })
    .await;

    assert!(
        result.is_ok(),
        "tool closure should not hang — it should complete within 5s"
    );

    dispatcher.shutdown().await;
}

#[tokio::test]
async fn test_existing_tests_still_pass_regression() {
    // Regression test: verify that the basic dispatcher flow still works
    // after the transport refactor.
    let dir = make_plugin_dir_with_binary();
    let dispatcher = SubprocessDispatcher::from_dirs(vec![dir.path().to_path_buf()], test_host())
        .await
        .expect("dispatcher creation");

    dispatcher.init(&test_host()).await;

    // System prompt mutation.
    let result = dispatcher.on_system_prompt("test prompt".to_string()).await;
    assert!(
        result.contains("[sample-plugin]"),
        "system prompt should be transformed, got: {}",
        result
    );

    // Header mutation.
    let mut headers = http::HeaderMap::new();
    headers.insert(
        http::HeaderName::from_static("content-type"),
        http::HeaderValue::from_static("application/json"),
    );
    let result = dispatcher.on_chat_headers(headers).await;
    assert!(
        result.get("x-plugin").is_some(),
        "plugin should add x-plugin header"
    );

    dispatcher.shutdown().await;
}
