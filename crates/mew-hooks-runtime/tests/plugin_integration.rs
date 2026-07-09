//! Integration test for the subprocess plugin runtime.
//!
//! Uses the sample-plugin example binary (cargo build --example sample-plugin).

use mew_hooks::{Dispatcher, PermissionDecision, ToolCall, ToolOutput};
use mew_hooks_runtime::SubprocessDispatcher;

mod common;
use common::{make_plugin_dir_with_binary, test_host};

#[tokio::test]
async fn test_plugin_init_and_shutdown() {
    let dir = make_plugin_dir_with_binary();
    let dispatcher = SubprocessDispatcher::from_dirs(vec![dir.path().to_path_buf()], test_host())
        .await
        .expect("dispatcher creation");

    dispatcher.init(&test_host()).await;
    dispatcher.shutdown().await;
}

#[tokio::test]
async fn test_plugin_adds_header() {
    let dir = make_plugin_dir_with_binary();
    let dispatcher = SubprocessDispatcher::from_dirs(vec![dir.path().to_path_buf()], test_host())
        .await
        .expect("dispatcher creation");

    dispatcher.init(&test_host()).await;

    let mut headers = http::HeaderMap::new();
    headers.insert(
        http::HeaderName::from_static("content-type"),
        http::HeaderValue::from_static("application/json"),
    );

    let result = dispatcher.on_chat_headers(headers).await;
    let plugin_header = result.get("x-plugin");
    assert!(plugin_header.is_some(), "plugin should add x-plugin header");
    assert_eq!(plugin_header.unwrap().to_str().unwrap(), "sample-plugin");

    dispatcher.shutdown().await;
}

#[tokio::test]
async fn test_plugin_denies_permission() {
    let dir = make_plugin_dir_with_binary();
    let dispatcher = SubprocessDispatcher::from_dirs(vec![dir.path().to_path_buf()], test_host())
        .await
        .expect("dispatcher creation");

    dispatcher.init(&test_host()).await;

    let call = ToolCall {
        tool_name: "bash".to_string(),
        call_id: "call-1".to_string(),
        input: serde_json::json!({"command": "rm -rf /"}),
    };

    let decision = dispatcher
        .on_permission_ask(&call, PermissionDecision::AllowOnce)
        .await;
    assert_eq!(
        decision,
        mew_hooks::HookOutcome::Proceed(PermissionDecision::Deny),
        "plugin should deny AllowOnce"
    );

    let decision2 = dispatcher
        .on_permission_ask(&call, PermissionDecision::AllowSession)
        .await;
    assert_eq!(
        decision2,
        mew_hooks::HookOutcome::Proceed(PermissionDecision::AllowSession),
        "AllowSession should pass through"
    );

    dispatcher.shutdown().await;
}

#[tokio::test]
async fn test_plugin_redacts_secrets() {
    let dir = make_plugin_dir_with_binary();
    let dispatcher = SubprocessDispatcher::from_dirs(vec![dir.path().to_path_buf()], test_host())
        .await
        .expect("dispatcher creation");

    dispatcher.init(&test_host()).await;

    let call = ToolCall {
        tool_name: "read".to_string(),
        call_id: "call-2".to_string(),
        input: serde_json::json!({"path": "/tmp/config"}),
    };

    let output = ToolOutput {
        output: "api_key=SECRET, host=localhost".to_string(),
        error: String::new(),
        diff: None,
        metadata: None,
        file_delta: None,
    };

    let result = dispatcher.on_tool_execute_after(&call, output).await;
    assert!(
        !result.output.contains("SECRET"),
        "plugin should redact SECRET, got: {}",
        result.output
    );
    assert!(
        result.output.contains("***REDACTED***"),
        "plugin should replace SECRET with ***REDACTED***"
    );

    dispatcher.shutdown().await;
}

#[tokio::test]
async fn test_plugin_register_tools() {
    let dir = make_plugin_dir_with_binary();
    let dispatcher = SubprocessDispatcher::from_dirs(vec![dir.path().to_path_buf()], test_host())
        .await
        .expect("dispatcher creation");

    let tools = dispatcher.on_register_tools().await;
    assert_eq!(tools.len(), 1, "sample-plugin registers one tool");
    assert_eq!(tools[0].name, "sample-echo");
    assert!(!tools[0].description.is_empty());
}

#[tokio::test]
async fn test_no_plugins_graceful() {
    let empty_dir = tempfile::tempdir().expect("tempdir");
    let dispatcher =
        SubprocessDispatcher::from_dirs(vec![empty_dir.path().to_path_buf()], test_host())
            .await
            .expect("dispatcher creation with empty dir");

    // Should not crash
    dispatcher.init(&test_host()).await;
    dispatcher.shutdown().await;
}

#[tokio::test]
async fn test_plugin_transforms_system_prompt() {
    let dir = make_plugin_dir_with_binary();
    let dispatcher = SubprocessDispatcher::from_dirs(vec![dir.path().to_path_buf()], test_host())
        .await
        .expect("dispatcher creation");
    dispatcher.init(&test_host()).await;

    let result = dispatcher.on_system_prompt("hello world".into()).await;
    assert!(
        result.contains("[sample-plugin]"),
        "plugin should prepend its tag to system prompt, got: {result}"
    );

    dispatcher.shutdown().await;
}

#[tokio::test]
async fn test_plugin_on_turn_end_notification() {
    let dir = make_plugin_dir_with_binary();
    let dispatcher = SubprocessDispatcher::from_dirs(vec![dir.path().to_path_buf()], test_host())
        .await
        .expect("dispatcher creation");
    dispatcher.init(&test_host()).await;

    // Should not panic; notification is fire-and-forget.
    dispatcher.on_turn_end(&[]).await;

    dispatcher.shutdown().await;
}

#[tokio::test]
async fn test_plugin_registers_slash_commands() {
    let dir = make_plugin_dir_with_binary();
    let dispatcher = SubprocessDispatcher::from_dirs(vec![dir.path().to_path_buf()], test_host())
        .await
        .expect("dispatcher creation");
    dispatcher.init(&test_host()).await;

    let cmds = dispatcher.on_register_slash_commands().await;
    assert!(
        !cmds.is_empty(),
        "sample-plugin should register slash commands"
    );
    assert!(
        cmds.iter().any(|c| c.name == "/sample-plugin"),
        "should register /sample-plugin command"
    );

    dispatcher.shutdown().await;
}

#[tokio::test]
async fn test_plugin_executes_slash_command() {
    let dir = make_plugin_dir_with_binary();
    let dispatcher = SubprocessDispatcher::from_dirs(vec![dir.path().to_path_buf()], test_host())
        .await
        .expect("dispatcher creation");
    dispatcher.init(&test_host()).await;

    let result = dispatcher
        .execute_slash_command("/sample-plugin", "greet")
        .await;
    assert!(result.is_some(), "slash command should return a result");
    assert!(
        result.unwrap().contains("sample-plugin"),
        "slash command result should include plugin name"
    );

    dispatcher.shutdown().await;
}

#[tokio::test]
async fn test_plugin_slash_command_returns_result_for_any_input() {
    let dir = make_plugin_dir_with_binary();
    let dispatcher = SubprocessDispatcher::from_dirs(vec![dir.path().to_path_buf()], test_host())
        .await
        .expect("dispatcher creation");
    dispatcher.init(&test_host()).await;

    // The sample plugin handles execute-slash-command for any command.
    let result = dispatcher
        .execute_slash_command("/anything", "arg1 arg2")
        .await;
    assert!(
        result.is_some(),
        "plugin should return a result for any slash command"
    );
    assert!(
        result.unwrap().contains("sample-plugin"),
        "result should identify the plugin"
    );

    dispatcher.shutdown().await;
}
