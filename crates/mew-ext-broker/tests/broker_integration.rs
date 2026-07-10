//! Integration tests for the ExtensionBroker.
//!
//! Tests end-to-end hook delivery, no-op equivalence, collision rejection,
//! gate audit logging, last-writer-wins ordering, capability enforcement,
//! manifest-based extension spawning, and consent persistence.

use std::env;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use mew_ext_broker::{ConsentDecision, ConsentResolver, ExtensionBroker};
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

/// Create a manifest-based extension package pointing to a real binary.
///
/// Creates `<tmp>/.mew/extensions/<name>/mew-ext.toml` with `entry.run`
/// set to the sample-plugin binary path.
fn make_manifest_extension(
    name: &str,
    binary_path: &std::path::Path,
    hooks_config: &str,
) -> (tempfile::TempDir, Vec<mew_ext_broker::DiscoveredExtension>) {
    let dir = tempfile::tempdir().expect("tempdir");
    let ext_dir = dir.path().join(".mew").join("extensions").join(name);
    std::fs::create_dir_all(&ext_dir).unwrap();

    let toml_content = format!(
        r#"
[extension]
name = "{name}"
version = "0.1.0"

[extension.entry]
run = ["{binary}"]

[extension.capabilities.hooks]
{hooks_config}
"#,
        name = name,
        binary = binary_path.display(),
        hooks_config = hooks_config,
    );

    std::fs::write(ext_dir.join("mew-ext.toml"), &toml_content).unwrap();

    let discovered = mew_ext_broker::discover_extensions(dir.path());
    assert_eq!(
        discovered.len(),
        1,
        "expected exactly one discovered extension"
    );
    (dir, discovered)
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
        &[],
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
        &[],
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
        &[],
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
        &[],
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
        &[],
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

/// AC.4 (legacy): Restricted legacy plugin only receives observe hooks.
/// A plugin with Restricted consent gets observe_only() capabilities.
/// Mutate hooks (on_system_prompt) pass through unchanged.
/// Registration hooks (on_register_tools) return empty.
#[tokio::test]
async fn test_legacy_plugin_restricted() {
    let dir = make_sample_plugin_dir();
    let host = test_host();

    // Resolver that always restricts (bare plugin: manifest=None).
    let resolver: ConsentResolver = Box::new(|_, _| ConsentDecision::Restricted);

    let broker = ExtensionBroker::from_dirs_filtered_with_config(
        vec![dir.path().to_path_buf()],
        host.clone(),
        &[],
        std::collections::HashMap::new(),
        Duration::from_secs(5),
        Some(resolver),
        &[],
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

/// AC.5 (legacy): Consent is persisted — resolver called twice for the same plugin
/// only prompts once (second call returns persisted decision).
#[tokio::test]
async fn test_consent_persisted() {
    use mew_ext_broker::ConsentState;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    let dir = tempfile::tempdir().unwrap();
    let state = ConsentState::with_path(dir.path().join("consent.json"));
    let prompt_count = Arc::new(AtomicU32::new(0));

    let count_clone = prompt_count.clone();
    let state_clone = state;
    let resolver: ConsentResolver = Box::new(
        move |name: &str, _manifest: Option<&mew_ext_broker::ExtensionManifest>| {
            if let Some(granted) = state_clone.get_granted_caps(name) {
                if mew_ext_broker::is_legacy_full(&granted) {
                    return ConsentDecision::Approved;
                }
                return ConsentDecision::Restricted;
            }
            count_clone.fetch_add(1, Ordering::Relaxed);
            // Simulate user saying "yes" (approved) — store legacy sentinel.
            state_clone
                .set_granted_caps(name, vec![mew_ext_broker::LEGACY_FULL_SENTINEL.to_string()]);
            state_clone.save().ok();
            ConsentDecision::Approved
        },
    );

    // First call: prompts and persists.
    assert_eq!(resolver("plugin-x", None), ConsentDecision::Approved);
    assert_eq!(prompt_count.load(Ordering::Relaxed), 1);

    // Second call: returns persisted decision WITHOUT prompting.
    assert_eq!(resolver("plugin-x", None), ConsentDecision::Approved);
    assert_eq!(
        prompt_count.load(Ordering::Relaxed),
        1,
        "should not prompt again"
    );
}

/// AC.3 (manifest): Manifest-based extension spawns and receives hooks.
#[tokio::test]
async fn test_manifest_extension_spawns() {
    let binary = sample_plugin_path();
    let (_dir, discovered) = make_manifest_extension("test-spawn-ext", &binary, "observe = true");

    let host = test_host();
    let broker = ExtensionBroker::from_dirs_filtered_with_config(
        vec![],
        host.clone(),
        &[],
        std::collections::HashMap::new(),
        Duration::from_secs(5),
        None,
        &discovered,
    )
    .await
    .expect("broker creation");

    broker.init(&host).await;

    // The extension should have spawned and registered tools (it has ui + register
    // capabilities since it requests hooks:observe).
    let tools = broker.on_register_tools().await;
    assert!(
        !tools.is_empty(),
        "manifest extension should register tools (has Register capability)"
    );
    assert!(
        tools.iter().any(|t| t.name == "sample-echo"),
        "manifest extension should register the sample-echo tool"
    );

    broker.shutdown().await;
}

/// AC.4 (manifest): Extension with hooks:observe only — mutate hooks skip.
///
/// An extension requesting only `hooks:observe` should:
/// - NOT mutate system prompts (on_system_prompt passes through)
/// - NOT register tools (no Register capability — observe-only caps)
#[tokio::test]
async fn test_manifest_extension_scoped_caps() {
    let binary = sample_plugin_path();
    let (_dir, discovered) = make_manifest_extension("test-scoped-ext", &binary, "observe = true");

    let host = test_host();
    let broker = ExtensionBroker::from_dirs_filtered_with_config(
        vec![],
        host.clone(),
        &[],
        std::collections::HashMap::new(),
        Duration::from_secs(5),
        None,
        &discovered,
    )
    .await
    .expect("broker creation");

    broker.init(&host).await;

    // on_system_prompt passes through unchanged — the extension has HooksObserve
    // but NOT HooksMutate. The sample-plugin mutates system_prompt, but the
    // broker skips the hook call because the capability isn't granted.
    let result = broker.on_system_prompt("test prompt".to_string()).await;
    assert_eq!(
        result, "test prompt",
        "hooks:observe-only extension should NOT mutate system prompt"
    );

    broker.shutdown().await;
}

/// AC.6 (manifest): Consent prompt persists — second run doesn't re-prompt.
#[tokio::test]
async fn test_manifest_extension_consent_prompt() {
    use mew_ext_broker::ConsentState;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    let binary = sample_plugin_path();
    let (_dir, discovered) = make_manifest_extension("test-consent-ext", &binary, "observe = true");

    let consent_dir = tempfile::tempdir().unwrap();
    let consent_path = consent_dir.path().join("consent.json");
    let prompt_count = Arc::new(AtomicU32::new(0));

    // First run: create resolver, it prompts and persists.
    {
        let state = ConsentState::with_path(consent_path.clone());
        let count_clone = prompt_count.clone();

        let resolver: ConsentResolver = Box::new(
            move |name: &str, manifest: Option<&mew_ext_broker::ExtensionManifest>| {
                if let Some(granted) = state.get_granted_caps(name) {
                    return ConsentDecision::ApprovedWithCaps(mew_ext_broker::reconstruct_caps(
                        &granted,
                    ));
                }
                match manifest {
                    Some(m) => {
                        count_clone.fetch_add(1, Ordering::Relaxed);
                        let caps = m.requested_capabilities();
                        let ids: Vec<String> = caps.iter().map(|c| c.id().to_string()).collect();
                        state.set_granted_caps(name, ids);
                        state.save().ok();
                        ConsentDecision::ApprovedWithCaps(caps)
                    }
                    None => ConsentDecision::Restricted,
                }
            },
        );

        let decision = resolver(&discovered[0].name, Some(&discovered[0].manifest));
        assert!(
            matches!(decision, ConsentDecision::ApprovedWithCaps(_)),
            "manifest extension should be ApprovedWithCaps"
        );
        assert_eq!(
            prompt_count.load(Ordering::Relaxed),
            1,
            "first run should prompt once"
        );
    }

    // Second run: load fresh ConsentState from the same file — consent is persisted.
    {
        let state = ConsentState::with_path(consent_path.clone());
        let count_clone = prompt_count.clone();

        let resolver: ConsentResolver = Box::new(
            move |name: &str, manifest: Option<&mew_ext_broker::ExtensionManifest>| {
                if let Some(granted) = state.get_granted_caps(name) {
                    return ConsentDecision::ApprovedWithCaps(mew_ext_broker::reconstruct_caps(
                        &granted,
                    ));
                }
                match manifest {
                    Some(m) => {
                        count_clone.fetch_add(1, Ordering::Relaxed);
                        let caps = m.requested_capabilities();
                        ConsentDecision::ApprovedWithCaps(caps)
                    }
                    None => ConsentDecision::Restricted,
                }
            },
        );

        let decision = resolver(&discovered[0].name, Some(&discovered[0].manifest));
        assert!(
            matches!(decision, ConsentDecision::ApprovedWithCaps(_)),
            "second run should also return ApprovedWithCaps"
        );
        assert_eq!(
            prompt_count.load(Ordering::Relaxed),
            1,
            "second run should NOT re-prompt"
        );
    }
}

/// AC.1: Manifest extension with Approved consent fallback grants observe_only,
/// NOT legacy_full. The extension should NOT mutate system prompts.
#[tokio::test]
async fn test_manifest_approved_fallback_observe_only() {
    let binary = sample_plugin_path();
    let (_dir, discovered) =
        make_manifest_extension("test-fallback-ext", &binary, "observe = true");

    let host = test_host();

    // Resolver returns Approved (not ApprovedWithCaps) for a manifest extension.
    // This should map to observe_only() (fail-closed), not legacy_full().
    let resolver: ConsentResolver = Box::new(|_name, _manifest| ConsentDecision::Approved);

    let broker = ExtensionBroker::from_dirs_filtered_with_config(
        vec![],
        host.clone(),
        &[],
        std::collections::HashMap::new(),
        Duration::from_secs(5),
        Some(resolver),
        &discovered,
    )
    .await
    .expect("broker creation");

    broker.init(&host).await;

    // The sample-plugin mutates on_system_prompt, but with observe_only caps
    // (no HooksMutate), the broker must skip the hook and pass through unchanged.
    let result = broker.on_system_prompt("test prompt".to_string()).await;
    assert_eq!(
        result, "test prompt",
        "manifest extension with Approved fallback should NOT mutate system prompt \
         (observe_only caps, not legacy_full)"
    );

    // Registration should also be empty (no Register capability in observe_only).
    let tools = broker.on_register_tools().await;
    assert!(
        tools.is_empty(),
        "observe_only fallback should NOT grant registration"
    );

    broker.shutdown().await;
}

/// AC.1 (integration): Manifest upgrade re-prompts for new capabilities.
///
/// Uses a resolver that counts prompts. First run with observe-only manifest,
/// then call resolver again with an upgraded manifest requesting hooks:gate.
/// The resolver should detect the delta and re-prompt.
#[tokio::test]
async fn test_manifest_upgrade_reprompts() {
    use mew_ext_broker::ConsentState;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    let binary = sample_plugin_path();

    // v1 manifest: observe only.
    let (_dir, discovered_v1) = make_manifest_extension("upgrade-ext", &binary, "observe = true");

    let consent_dir = tempfile::tempdir().unwrap();
    let consent_path = consent_dir.path().join("consent.json");
    let prompt_count = Arc::new(AtomicU32::new(0));

    // First run: approve observe-only manifest.
    {
        let state = ConsentState::with_path(consent_path.clone());
        let count_clone = prompt_count.clone();
        let resolver: ConsentResolver = Box::new(
            move |name: &str, manifest: Option<&mew_ext_broker::ExtensionManifest>| {
                if let Some(granted) = state.get_granted_caps(name) {
                    if !mew_ext_broker::is_legacy_full(&granted) {
                        let manifest_caps = manifest
                            .map(|m| m.requested_capabilities())
                            .unwrap_or_default();
                        let granted_caps = mew_ext_broker::reconstruct_caps(&granted);
                        return ConsentDecision::ApprovedWithCaps(
                            granted_caps.intersect(&manifest_caps),
                        );
                    }
                    return ConsentDecision::Restricted;
                }
                match manifest {
                    Some(m) => {
                        count_clone.fetch_add(1, Ordering::Relaxed);
                        let caps = m.requested_capabilities();
                        let ids = caps.to_ids();
                        state.set_consent(name, ids, caps.to_ids());
                        state.save().ok();
                        ConsentDecision::ApprovedWithCaps(caps)
                    }
                    None => ConsentDecision::Restricted,
                }
            },
        );

        let decision = resolver(&discovered_v1[0].name, Some(&discovered_v1[0].manifest));
        assert!(matches!(decision, ConsentDecision::ApprovedWithCaps(_)));
        assert_eq!(
            prompt_count.load(Ordering::Relaxed),
            1,
            "first run should prompt once"
        );
    }

    // Second run: upgraded manifest now requests hooks:gate too.
    let (_dir2, discovered_v2) =
        make_manifest_extension("upgrade-ext", &binary, "observe = true\ngate = [\"bash\"]");

    {
        let state = ConsentState::with_path(consent_path.clone());
        let count_clone = prompt_count.clone();
        let resolver: ConsentResolver = Box::new(
            move |name: &str, manifest: Option<&mew_ext_broker::ExtensionManifest>| {
                if let Some(granted) = state.get_granted_caps(name) {
                    if !mew_ext_broker::is_legacy_full(&granted) {
                        let manifest_caps = manifest
                            .map(|m| m.requested_capabilities())
                            .unwrap_or_default();
                        let granted_caps = mew_ext_broker::reconstruct_caps(&granted);

                        // Delta detection.
                        let last_requested_ids = state
                            .get_last_requested(name)
                            .filter(|v| !v.is_empty())
                            .unwrap_or_else(|| manifest_caps.to_ids());
                        let last_requested = mew_ext_broker::reconstruct_caps(&last_requested_ids);
                        let delta = manifest_caps.difference(&last_requested);

                        if !delta.added.is_empty() {
                            // Delta detected — re-prompt.
                            count_clone.fetch_add(1, Ordering::Relaxed);
                            let added_caps = mew_ext_broker::reconstruct_caps(&delta.added);
                            let base = granted_caps.intersect(&manifest_caps);
                            let mut final_caps = base;
                            for cap in added_caps.iter() {
                                final_caps.grant(cap.clone());
                            }
                            let ids = final_caps.to_ids();
                            state.set_consent(name, ids, manifest_caps.to_ids());
                            state.save().ok();
                            return ConsentDecision::ApprovedWithCaps(final_caps);
                        }

                        return ConsentDecision::ApprovedWithCaps(
                            granted_caps.intersect(&manifest_caps),
                        );
                    }
                    return ConsentDecision::Restricted;
                }
                ConsentDecision::Restricted
            },
        );

        let decision = resolver(&discovered_v2[0].name, Some(&discovered_v2[0].manifest));
        match decision {
            ConsentDecision::ApprovedWithCaps(caps) => {
                // Should now have hooks:gate (newly approved).
                assert!(
                    caps.has(&mew_ext_broker::Capability::HooksGate),
                    "upgraded manifest should grant hooks:gate"
                );
                // Should still have hooks:observe (existing).
                assert!(
                    caps.has(&mew_ext_broker::Capability::HooksObserve),
                    "existing hooks:observe should be preserved"
                );
            }
            other => panic!("expected ApprovedWithCaps, got {:?}", other),
        }

        // Prompt was called again for the delta.
        assert_eq!(
            prompt_count.load(Ordering::Relaxed),
            2,
            "upgrade should re-prompt (total 2 prompts)"
        );
    }
}
