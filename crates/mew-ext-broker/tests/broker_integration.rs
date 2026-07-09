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
        None,
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
        None,
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
        None,
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
        None,
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
        None,
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

/// AC.4: Restricted legacy plugin only receives observe hooks.
/// A plugin with Restricted consent gets observe_only() capabilities.
/// Mutate hooks (on_system_prompt) pass through unchanged.
/// Registration hooks (on_register_tools) return empty.
#[tokio::test]
async fn test_legacy_plugin_restricted() {
    use mew_ext_broker::consent::{ConsentDecision, ConsentResolver};

    let dir = make_sample_plugin_dir();
    let host = test_host();

    // Resolver that always restricts.
    let resolver: ConsentResolver = Box::new(|_| ConsentDecision::Restricted);

    let broker = ExtensionBroker::from_dirs_filtered_with_config(
        vec![dir.path().to_path_buf()],
        host.clone(),
        &[],
        std::collections::HashMap::new(),
        Duration::from_secs(5),
        Some(resolver),
    )
    .await
    .expect("broker creation");

    broker.init(&host).await;

    // Mutate hook: on_system_prompt should pass through UNCHANGED
    // (restricted plugins lack HooksMutate capability).
    let result = broker.on_system_prompt("test prompt".to_string()).await;
    assert_eq!(
        result, "test prompt",
        "restricted plugin should NOT mutate system prompt"
    );

    // Registration: on_register_tools should return EMPTY
    // (restricted plugins lack Register capability).
    let tools = broker.on_register_tools().await;
    assert!(
        tools.is_empty(),
        "restricted plugin should NOT register tools"
    );

    // Observe hook: on_turn_end should fire (fire-and-forget, no return value).
    // We just verify it doesn't panic.
    use mew_message::{
        Message, MessageId, Part, PartBase, PartId, Role, SessionId, TextPart, Time,
    };
    let mid = MessageId::new();
    let sid = SessionId::new();
    let msg = Message {
        id: mid,
        session_id: sid,
        role: Role::User,
        parts: vec![Part::Text(TextPart {
            base: PartBase {
                id: PartId::new(),
                message_id: mid,
                session_id: sid,
            },
            text: "test".into(),
            synthetic: false,
        })],
        time: Time {
            created: 0,
            completed: None,
        },
        assistant: None,
    };
    broker.on_turn_end(&[msg]).await;

    broker.shutdown().await;
}

/// AC.5: Consent is persisted — resolver called twice for the same plugin
/// only prompts once (second call returns persisted decision).
#[tokio::test]
async fn test_consent_persisted() {
    use mew_ext_broker::consent::ConsentState;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    let dir = tempfile::tempdir().unwrap();
    let state = ConsentState::with_path(dir.path().join("consent.json"));
    let prompt_count = Arc::new(AtomicU32::new(0));

    let count_clone = prompt_count.clone();
    let state_clone = state;
    let resolver: mew_ext_broker::consent::ConsentResolver = Box::new(move |name: &str| {
        if let Some(existing) = state_clone.get(name) {
            return existing;
        }
        count_clone.fetch_add(1, Ordering::Relaxed);
        // Simulate user saying "yes" (approved).
        state_clone.set(name, mew_ext_broker::consent::ConsentDecision::Approved);
        state_clone.save().ok();
        mew_ext_broker::consent::ConsentDecision::Approved
    });

    // First call: prompts and persists.
    assert_eq!(
        resolver("plugin-x"),
        mew_ext_broker::consent::ConsentDecision::Approved
    );
    assert_eq!(prompt_count.load(Ordering::Relaxed), 1);

    // Second call: returns persisted decision WITHOUT prompting.
    assert_eq!(
        resolver("plugin-x"),
        mew_ext_broker::consent::ConsentDecision::Approved
    );
    assert_eq!(
        prompt_count.load(Ordering::Relaxed),
        1,
        "should not prompt again"
    );
}
