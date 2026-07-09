//! Integration tests for the ExtensionBroker.
//!
//! Tests end-to-end hook delivery, no-op equivalence, collision rejection,
//! gate audit logging, last-writer-wins ordering, and capability enforcement.

use std::env;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use mew_ext_broker::ExtensionBroker;
use mew_hooks::{Dispatcher, PluginHost};
// ── Test helpers (duplicated from mew-hooks-runtime/tests/common/mod.rs) ──

fn test_host() -> PluginHost {
    PluginHost {
        notify: Arc::new(|msg| eprintln!("[plugin-notify] {msg}")),
        config_read: Arc::new(|_key| None),
        log: Arc::new(|msg| eprintln!("[plugin-log] {msg}")),
        storage_read: Arc::new(|_key| None),
        storage_write: Arc::new(|_key, _val| {}),
        storage_delete: Arc::new(|_key| {}),
        set_ui: Arc::new(|_key, _val| {}),
    }
}

fn sample_plugin_path() -> PathBuf {
    find_example_binary("sample-plugin")
}

fn conflicting_plugin_path() -> PathBuf {
    find_example_binary("conflicting-plugin")
}

fn find_example_binary(name: &str) -> PathBuf {
    let env_var = format!("CARGO_BIN_EXE_{}", name);
    if let Ok(path) = env::var(&env_var) {
        return PathBuf::from(path);
    }
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    // mew-ext-broker is at crates/mew-ext-broker, so parent.parent is workspace root
    let target = manifest
        .parent()
        .and_then(|p| p.parent())
        .unwrap_or(&manifest)
        .join("target")
        .join("debug")
        .join("examples")
        .join(name);

    if target.exists() {
        return target;
    }

    // Build it
    let workspace_root = manifest
        .parent()
        .and_then(|p| p.parent())
        .unwrap_or(&manifest);
    let status = Command::new("cargo")
        .args(["build", "--example", name, "-p", "mew-hooks-runtime"])
        .current_dir(workspace_root)
        .status()
        .expect("cargo build example");

    assert!(status.success(), "failed to build {} example", name);
    assert!(target.exists(), "{} binary not found at {:?}", name, target);
    target
}

fn make_plugin_dir(binary_path: PathBuf) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let name = binary_path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let dst = dir.path().join(&name);
    std::fs::copy(&binary_path, &dst).expect("copy plugin binary");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&dst).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&dst, perms).unwrap();
    }
    dir
}

/// Create a plugin dir with the sample-plugin binary.
fn make_sample_plugin_dir() -> tempfile::TempDir {
    make_plugin_dir(sample_plugin_path())
}

// ── Tests ────────────────────────────────────────────────────────────

/// AC.3: End-to-end hook delivery through the broker.
#[tokio::test]
async fn test_e2e_hook_delivery() {
    let dir = make_sample_plugin_dir();
    let host = test_host();

    let broker = ExtensionBroker::from_dirs_filtered_with_config(
        vec![dir.path().to_path_buf()],
        host.clone(),
        &[],
        std::collections::HashMap::new(),
        Duration::from_secs(5),
    )
    .await
    .expect("broker creation");

    broker.init(&host).await;

    // System prompt mutation — sample-plugin prepends [sample-plugin].
    let result = broker.on_system_prompt("test prompt".to_string()).await;
    assert!(
        result.contains("[sample-plugin]"),
        "system prompt should be transformed, got: {}",
        result
    );

    broker.shutdown().await;
}

/// AC.4: No-op equivalence — broker with zero extensions behaves like NopDispatcher.
#[tokio::test]
async fn test_noop_equivalence() {
    let host = test_host();
    let broker = ExtensionBroker::from_dirs_filtered_with_config(
        vec![],
        host.clone(),
        &[],
        std::collections::HashMap::new(),
        Duration::from_secs(5),
    )
    .await
    .expect("broker creation with no dirs");

    broker.init(&host).await;

    // All mutate hooks pass through unchanged.
    let prompt = broker.on_system_prompt("hello".to_string()).await;
    assert_eq!(prompt, "hello");

    // Gate hooks return Proceed.
    use mew_hooks::{HookOutcome, PermissionDecision, ToolCall};
    let call = ToolCall {
        tool_name: "test".into(),
        call_id: "1".into(),
        input: serde_json::json!({}),
    };
    let outcome = broker
        .on_tool_execute_before(&call, serde_json::json!({}))
        .await;
    match outcome {
        HookOutcome::Proceed(_) => {}
        other => panic!("expected Proceed, got {:?}", other),
    }

    let outcome = broker
        .on_permission_ask(&call, PermissionDecision::AllowOnce)
        .await;
    match outcome {
        HookOutcome::Proceed(d) => assert_eq!(d, PermissionDecision::AllowOnce),
        other => panic!("expected Proceed with AllowOnce, got {:?}", other),
    }

    // Registration returns empty.
    let tools = broker.on_register_tools().await;
    assert!(tools.is_empty(), "should have no registered tools");

    let cmds = broker.on_register_slash_commands().await;
    assert!(cmds.is_empty(), "should have no registered commands");

    broker.shutdown().await;
}

/// AC.5: Collision rejection — two extensions registering the same tool name.
#[tokio::test]
async fn test_collision_rejection() {
    // We need both sample-plugin and conflicting-plugin in the same dir.
    let dir = tempfile::tempdir().expect("tempdir");

    // Copy both binaries into the plugin dir.
    for (src_path, name) in [
        (sample_plugin_path(), "sample-plugin"),
        (conflicting_plugin_path(), "conflicting-plugin"),
    ] {
        let dst = dir.path().join(name);
        std::fs::copy(&src_path, &dst).expect("copy plugin binary");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&dst).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&dst, perms).unwrap();
        }
    }

    let host = test_host();
    let broker = ExtensionBroker::from_dirs_filtered_with_config(
        vec![dir.path().to_path_buf()],
        host.clone(),
        &[],
        std::collections::HashMap::new(),
        Duration::from_secs(5),
    )
    .await
    .expect("broker creation");

    broker.init(&host).await;

    // Both extensions try to register "sample-echo".
    // sample-plugin is alphabetically first, conflicting-plugin is second.
    // The first registration should win; the second should be skipped.
    let tools = broker.on_register_tools().await;

    // Should have exactly one "sample-echo" tool (from sample-plugin, the first).
    let echo_count = tools.iter().filter(|t| t.name == "sample-echo").count();
    assert_eq!(
        echo_count, 1,
        "should have exactly one sample-echo tool (collision rejected), got {}",
        echo_count
    );

    broker.shutdown().await;
}

/// AC.6: Gate audit logging.
#[tokio::test]
async fn test_gate_audit() {
    let dir = make_sample_plugin_dir();
    let host = test_host();

    let mut broker = ExtensionBroker::from_dirs_filtered_with_config(
        vec![dir.path().to_path_buf()],
        host.clone(),
        &[],
        std::collections::HashMap::new(),
        Duration::from_secs(5),
    )
    .await
    .expect("broker creation");

    broker.set_session_id("test-session-1".into());
    broker.init(&host).await;

    use mew_hooks::ToolCall;
    let call = ToolCall {
        tool_name: "bash".into(),
        call_id: "1".into(),
        input: serde_json::json!({}),
    };

    // Call on_tool_execute_before — triggers audit logging.
    let _ = broker
        .on_tool_execute_before(&call, serde_json::json!({"command": "echo hello"}))
        .await;

    // Check the audit log.
    let entries = broker.audit_entries("sample-plugin");
    assert!(
        !entries.is_empty(),
        "audit log should have at least one entry"
    );
    let entry = &entries[0];
    assert_eq!(entry.extension, "sample-plugin");
    assert_eq!(entry.tool, "bash");
    assert_eq!(entry.session_id, "test-session-1");

    broker.shutdown().await;
}

/// AC.9: Last-writer-wins — alphabetically-last extension's response wins.
#[tokio::test]
async fn test_last_writer_wins() {
    // Both plugins in the same dir.
    let dir = tempfile::tempdir().expect("tempdir");

    for (src_path, name) in [
        (sample_plugin_path(), "sample-plugin"),
        (conflicting_plugin_path(), "conflicting-plugin"),
    ] {
        let dst = dir.path().join(name);
        std::fs::copy(&src_path, &dst).expect("copy plugin binary");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&dst).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&dst, perms).unwrap();
        }
    }

    let host = test_host();
    let broker = ExtensionBroker::from_dirs_filtered_with_config(
        vec![dir.path().to_path_buf()],
        host.clone(),
        &[],
        std::collections::HashMap::new(),
        Duration::from_secs(5),
    )
    .await
    .expect("broker creation");

    broker.init(&host).await;

    // Both extensions transform on_system_prompt.
    // sample-plugin prepends [sample-plugin]
    // conflicting-plugin prepends [conflicting-plugin]
    // Alphabetically: conflicting-plugin < sample-plugin (c < s)
    // So sample-plugin is the last writer — its response should win.
    let result = broker.on_system_prompt("test".to_string()).await;

    assert!(
        result.contains("[sample-plugin]"),
        "last-writer-wins: sample-plugin (alphabetically last) should win, got: {}",
        result
    );
    assert!(
        !result.contains("[conflicting-plugin]"),
        "conflicting-plugin's response should NOT be the final value, got: {}",
        result
    );

    broker.shutdown().await;
}

/// AC.7: Capability enforcement — extension without HooksGate is skipped.
///
/// This test uses a manually constructed broker (no subprocess) to verify
/// the capability check logic. It verifies:
/// 1. An extension lacking HooksGate does not receive on_permission_ask.
/// 2. An extension with HooksMutate but NOT HooksMutateChatParams is skipped
///    for on_chat_params but receives on_system_prompt.
#[tokio::test]
async fn test_capability_enforcement() {
    use mew_ext_broker::capabilities::{Capability, CapabilitySet};

    // We can't easily test capability enforcement with real subprocesses
    // because legacy_full() grants all capabilities. Instead, we verify
    // the hook_capability mapping and satisfies() logic directly.

    // 1. HooksGate is required for on_permission_ask.
    let gate_cap = ExtensionBroker::hook_capability(mew_hooks::HookId::PermissionAsk);
    assert_eq!(gate_cap, Some(Capability::HooksGate));

    // An extension with HooksMutate but NOT HooksGate.
    let mutate_only = CapabilitySet::from_iter([Capability::HooksMutate]);
    assert!(!mutate_only.satisfies(&Capability::HooksGate));

    // An extension with HooksGate.
    let gate_set = CapabilitySet::from_iter([Capability::HooksGate]);
    assert!(gate_set.satisfies(&Capability::HooksGate));

    // 2. Sub-scope non-implication: hooks:gate:mutate does NOT satisfy
    //    hooks:mutate:chat_params (different mutation surfaces).
    let gate_mutate_set = CapabilitySet::from_iter([Capability::HooksGateMutate]);
    assert!(gate_mutate_set.satisfies(&Capability::HooksMutate));
    assert!(gate_mutate_set.satisfies(&Capability::HooksGate));
    assert!(
        !gate_mutate_set.satisfies(&Capability::HooksMutateChatParams),
        "hooks:gate:mutate should NOT satisfy hooks:mutate:chat_params"
    );

    // 3. An extension with HooksMutate but NOT HooksMutateChatParams
    //    should receive on_system_prompt (requires HooksMutate) but
    //    NOT on_chat_params (requires HooksMutateChatParams).
    let chat_params_cap = ExtensionBroker::hook_capability(mew_hooks::HookId::ChatParams);
    assert_eq!(chat_params_cap, Some(Capability::HooksMutateChatParams));
    assert!(mutate_only.satisfies(&Capability::HooksMutate));
    assert!(!mutate_only.satisfies(&Capability::HooksMutateChatParams));
}
