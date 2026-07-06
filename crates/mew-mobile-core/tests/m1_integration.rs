//! M1 integration test: MobileCore connects to daemon, events arrive via
//! CoreListener, full round-trip including streaming text.
//!
//! Unlike the M0 spike (which tests the raw transport path), this test
//! exercises MobileCore::connect() and verifies CoreEvents are emitted
//! through the listener to Swift would receive them.

#![cfg(feature = "test-harness")]

use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use mew_daemon::iroh_transport::{MewIrohHandler, NodeIdAllowlist, MEW_ALPN};
use mew_daemon::DaemonServer;
use mew_hooks::NopDispatcher;
use mew_mobile_core::{CoreEvent, CoreListener, MobileCore};
use mew_provider_fake::FakeProvider;
use std::sync::Mutex as StdMutex;

/// A test listener that collects all events into a shared Vec.
/// Uses Arc internally so the test can retain access after set_listener.
struct TestListener {
    events: Arc<StdMutex<Vec<CoreEvent>>>,
}

impl TestListener {
    fn new() -> (Self, Arc<StdMutex<Vec<CoreEvent>>>) {
        let events = Arc::new(StdMutex::new(Vec::new()));
        (
            Self {
                events: events.clone(),
            },
            events,
        )
    }
}

impl CoreListener for TestListener {
    fn on_event(&self, event: CoreEvent) {
        self.events.lock().unwrap().push(event);
    }
}

fn fake_builder() -> mew_daemon::AgentBuilder {
    Arc::new(|params: mew_daemon::AgentBuildParams| {
        use mew_message::SessionId;
        let provider = Arc::new(FakeProvider::new(FakeProvider::text_response(
            "hello from mobile core M1",
        )));
        let dispatcher = Arc::new(NopDispatcher);
        let session_id: Option<SessionId> = params
            .session_id
            .strip_prefix("sess_")
            .and_then(|s| ulid::Ulid::from_string(s).ok());
        Ok((
            {
                let mut a = mew_agent::Agent::new(
                    provider,
                    dispatcher,
                    Some(params.writer),
                    Vec::new(),
                    session_id,
                );
                a.set_model_info("fake", "fake");
                a
            },
            Some("fake".to_string()),
            Some("fake".to_string()),
        ))
    })
}

/// Wait until the events Vec contains at least one event matching the predicate.
/// Times out after `timeout_secs` with a panic showing all events received.
fn wait_for_event(
    events: &Arc<StdMutex<Vec<CoreEvent>>>,
    timeout_secs: u64,
    pred: impl Fn(&CoreEvent) -> bool,
) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout_secs);
    loop {
        {
            let evs = events.lock().unwrap();
            if evs.iter().any(&pred) {
                return;
            }
        }
        if tokio::time::Instant::now() >= deadline {
            let evs = events.lock().unwrap();
            panic!(
                "wait_for_event timed out after {}s; {} events received: {:?}",
                timeout_secs,
                evs.len(),
                evs
            );
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Count events matching a predicate.
fn count_events(
    events: &Arc<StdMutex<Vec<CoreEvent>>>,
    pred: impl Fn(&CoreEvent) -> bool,
) -> usize {
    events.lock().unwrap().iter().filter(|e| pred(e)).count()
}

#[tokio::test(flavor = "multi_thread")]
async fn mobile_core_emits_events_through_listener() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let session_dir = dir.path().join("sessions");
    let allowlist_path = dir.path().join("authorized_nodes.json");
    let core_data_dir = dir.path().join("mobile-core");

    let allowlist = Arc::new(NodeIdAllowlist::new(allowlist_path.clone()));

    // Start the daemon.
    let server = DaemonServer::with_session_dir(fake_builder(), session_dir.clone());
    let handler = MewIrohHandler {
        allowlist: allowlist.clone(),
        session_manager: server.session_manager.clone(),
        groups_store: server.groups_store.clone(),
        thinking_setter: None,
    };

    let daemon_endpoint = iroh::Endpoint::builder(iroh::endpoint::presets::N0)
        .alpns(vec![MEW_ALPN.to_vec()])
        .bind()
        .await?;
    let _ = tokio::time::timeout(Duration::from_secs(15), daemon_endpoint.online()).await;
    let daemon_node_id = daemon_endpoint.id().to_string();

    let router = iroh::protocol::Router::builder(daemon_endpoint.clone())
        .accept(MEW_ALPN, handler)
        .spawn();

    // Create the mobile core.
    let phone_key = iroh::SecretKey::generate();
    let core = MobileCore::new(
        phone_key.to_bytes().to_vec(),
        core_data_dir.to_str().unwrap().into(),
    )
    .await
    .map_err(|e| anyhow::anyhow!("{e}"))?;

    // Pre-authorize the phone in the allowlist.
    allowlist.add(&core.node_id())?;

    // Set up the listener — retain the shared events Arc.
    let (listener, events) = TestListener::new();
    core.set_listener(Arc::new(listener));

    // Add the daemon and connect.
    let daemon_id = core.add_daemon(daemon_node_id, "Test Daemon".into());
    core.connect(daemon_id.clone());

    // Wait for DaemonStatusChanged(Connected).
    wait_for_event(&events, 20, |e| {
        matches!(
            e,
            CoreEvent::DaemonStatusChanged {
                status: mew_mobile_core::DaemonStatus::Connected,
                ..
            }
        )
    });
    println!("✓ Received DaemonStatusChanged(Connected)");

    // Wait for DaemonVersion (from Ping/Pong).
    wait_for_event(&events, 10, |e| {
        matches!(e, CoreEvent::DaemonVersion { .. })
    });
    println!("✓ Received DaemonVersion (Pong)");

    // Create a new session.
    core.new_session(daemon_id.clone(), None);

    // Wait for SessionReloaded.
    wait_for_event(&events, 10, |e| {
        matches!(e, CoreEvent::SessionReloaded { .. })
    });
    println!("✓ Received SessionReloaded (SessionReady)");

    // Send a prompt.
    core.prompt(daemon_id.clone(), "hello from M1 test".into());

    // Wait for TurnEnded (the fake provider streams "hello from mobile core M1").
    wait_for_event(&events, 15, |e| matches!(e, CoreEvent::TurnEnded { .. }));
    println!("✓ Received TurnEnded (streaming complete)");

    // Verify we got TextDelta events (streaming text).
    let delta_count = count_events(&events, |e| matches!(e, CoreEvent::TextDelta { .. }));
    assert!(
        delta_count > 0,
        "should have received at least one TextDelta"
    );
    println!("✓ Received {delta_count} TextDelta event(s)");

    // Verify snapshot has the session.
    let snapshot = core
        .snapshot(daemon_id.clone())
        .expect("snapshot should exist");
    assert!(
        snapshot
            .sessions
            .first()
            .map(|s| !s.messages.is_empty())
            .unwrap_or(false),
        "snapshot should have messages"
    );
    println!(
        "✓ Snapshot has {} message(s)",
        snapshot
            .sessions
            .first()
            .map(|s| s.messages.len())
            .unwrap_or(0)
    );

    // Verify daemon version is in snapshot.
    assert!(
        snapshot.daemon_version.is_some(),
        "snapshot should have daemon version"
    );
    println!(
        "✓ Snapshot has daemon version: {:?}",
        snapshot.daemon_version
    );

    // Verify the events vec has the right shape.
    let (total, has_connected, has_version, has_reloaded, has_turn_ended) = {
        let evs = events.lock().unwrap();
        let total = evs.len();
        let has_connected = evs.iter().any(|e| {
            matches!(
                e,
                CoreEvent::DaemonStatusChanged {
                    status: mew_mobile_core::DaemonStatus::Connected,
                    ..
                }
            )
        });
        let has_version = evs
            .iter()
            .any(|e| matches!(e, CoreEvent::DaemonVersion { .. }));
        let has_reloaded = evs
            .iter()
            .any(|e| matches!(e, CoreEvent::SessionReloaded { .. }));
        let has_turn_ended = evs.iter().any(|e| matches!(e, CoreEvent::TurnEnded { .. }));
        (
            total,
            has_connected,
            has_version,
            has_reloaded,
            has_turn_ended,
        )
    };
    assert!(
        has_connected && has_version && has_reloaded && has_turn_ended,
        "expected all key events"
    );
    println!("✓ All key events present: Connected, Version, Reloaded, TurnEnded");
    println!("✓ Total events: {total}");

    println!("\n✓✓✓ M1 integration test complete: events verified through listener");

    let _ = router.shutdown().await;
    Ok(())
}
