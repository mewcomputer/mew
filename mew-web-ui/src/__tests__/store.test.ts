import { describe, it, expect, beforeEach } from "vitest";
import { useSessionStore } from "../stores/session";

// Helper to get fresh store state.
function store() {
  return useSessionStore.getState();
}

describe("alert lifecycle", () => {
  beforeEach(() => {
    // Reset alerts between tests.
    store().clearAlertsForSession("test-a");
    store().clearAlertsForSession("test-b");
  });

  it("onSessionAlert pushes to the alerts array", () => {
    store().onSessionAlert("test-a", "Session A", "turn_complete");
    const alerts = useSessionStore.getState().alerts;
    expect(alerts).toHaveLength(1);
    expect(alerts[0]!.sessionId).toBe("test-a");
    expect(alerts[0]!.kind).toBe("turn_complete");
  });

  it("clearAlertsForSession removes only matching alerts", () => {
    store().onSessionAlert("test-a", "A", "turn_complete");
    store().onSessionAlert("test-b", "B", "permission_needed");
    store().onSessionAlert("test-a", "A2", "turn_failed");

    store().clearAlertsForSession("test-a");

    const alerts = useSessionStore.getState().alerts;
    expect(alerts).toHaveLength(1);
    expect(alerts[0]!.sessionId).toBe("test-b");
  });

  it("dismissAlert removes an entry by timestamp", async () => {
    useSessionStore.setState({ alerts: [] });

    store().onSessionAlert("test-a", "A", "turn_complete");
    // Small delay to ensure different timestamps.
    await new Promise((r) => setTimeout(r, 2));
    store().onSessionAlert("test-a", "A2", "turn_failed");

    expect(useSessionStore.getState().alerts).toHaveLength(2);

    const firstTs = useSessionStore.getState().alerts[0]!.timestamp;
    store().dismissAlert("test-a", firstTs);

    const alerts = useSessionStore.getState().alerts;
    expect(alerts).toHaveLength(1);
    expect(alerts[0]!.title).toBe("A2");
  });
});

describe("flagged files lifecycle", () => {
  beforeEach(() => {
    useSessionStore.setState({ sessionId: "sess-1" });
  });

  it("onFlaggedFilesChanged sets the flaggedFiles array", () => {
    store().onFlaggedFilesChanged([
      { path: "src/main.rs", reason: "included" },
    ]);
    expect(useSessionStore.getState().flaggedFiles).toHaveLength(1);
    expect(useSessionStore.getState().flaggedFiles[0]!.path).toBe("src/main.rs");
  });

  it("reset clears flaggedFiles", () => {
    store().onFlaggedFilesChanged([{ path: "test.rs" }]);
    store().reset();
    expect(useSessionStore.getState().flaggedFiles).toHaveLength(0);
  });
});

describe("session meta changes", () => {
  it("onSessionMetaChanged updates the session in availableSessions", () => {
    useSessionStore.setState({
      availableSessions: [
        {
          session_id: "s1",
          state: "active",
          created_at: 0,
          client_count: 0,
        },
      ],
    });

    store().onSessionMetaChanged("s1", true, false);
    const s = useSessionStore.getState().availableSessions[0]!;
    expect(s.archived).toBe(true);
    expect(s.pinned).toBe(false);
  });

  it("onSessionAttentionChanged updates pending counts", () => {
    useSessionStore.setState({
      availableSessions: [
        {
          session_id: "s1",
          state: "active",
          created_at: 0,
          client_count: 0,
        },
      ],
    });

    store().onSessionAttentionChanged("s1", 2, 1);
    const s = useSessionStore.getState().availableSessions[0]!;
    expect(s.pending_permissions).toBe(2);
    expect(s.pending_questions).toBe(1);
  });
});

describe("session activity + usage", () => {
  it("onSessionActivityChanged updates session state", () => {
    useSessionStore.setState({
      availableSessions: [
        {
          session_id: "s1",
          state: "idle",
          created_at: 0,
          client_count: 0,
        },
      ],
    });

    store().onSessionActivityChanged("s1", "running");
    expect(useSessionStore.getState().availableSessions[0]!.state).toBe("running");
  });

  it("onSessionUsageChanged updates usage on session", () => {
    useSessionStore.setState({
      availableSessions: [
        {
          session_id: "s1",
          state: "active",
          created_at: 0,
          client_count: 0,
        },
      ],
    });

    store().onSessionUsageChanged("s1", {
      input_tokens: 1000,
      output_tokens: 500,
      cache_read_tokens: 200,
      cache_write_tokens: 100,
      cost: 0.05,
      turns: 3,
    });

    const s = useSessionStore.getState().availableSessions[0]!;
    expect(s.usage).toBeDefined();
    expect(s.usage!.cost).toBe(0.05);
    expect(s.usage!.turns).toBe(3);
  });
});
