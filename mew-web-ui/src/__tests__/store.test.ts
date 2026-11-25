import { describe, it, expect, beforeEach, vi } from "vitest";
import {
  askUserResponders,
  bridgeClientToStore,
  permissionResponders,
  selectSessionRunning,
  useSessionStore,
  wireMessageToChatMessage,
} from "../stores/session";
import type { MewClient } from "@mew/web-client";
import { setClient } from "../lib/client-ref";

// Helper to get fresh store state.
function store() {
  return useSessionStore.getState();
}

describe("pending input recovery", () => {
  let handlers: Map<string, (...args: any[]) => void>;

  beforeEach(() => {
    store().reset();
    permissionResponders.clear();
    askUserResponders.clear();
    handlers = new Map();
    bridgeClientToStore({
      on: (event: string, handler: (...args: any[]) => void) => handlers.set(event, handler),
      listModels: () => Promise.resolve([]),
      listPersonas: () => Promise.resolve([]),
      loadSessionIndex: vi.fn(),
    } as unknown as MewClient);
  });

  it("ignores file previews belonging to another conversation", () => {
    useSessionStore.setState({ sessionId: "chat-b", filePreview: null });
    handlers.get("file-preview")?.({ session_id: "chat-a", path: "README.md", content: "wrong workspace", truncated: false });
    expect(store().filePreview).toBeNull();
    handlers.get("file-preview")?.({ session_id: "chat-b", path: "README.md", content: "right workspace", truncated: false });
    expect(store().filePreview?.content).toBe("right workspace");
  });

  it("clears late file data when the next conversation attaches", () => {
    useSessionStore.setState({ sessionId: "chat-a", filePreview: null });
    store().reset();
    handlers.get("file-preview")?.({ session_id: "chat-a", path: "README.md", content: "old", truncated: false });
    handlers.get("dir-listing")?.({ session_id: "chat-a", path: "", entries: [{ name: "old", is_dir: false }] });
    handlers.get("git-status-result")?.({ session_id: "chat-a", entries: [{ path: "old.md", status: "modified" }] });
    handlers.get("session-ready")?.({ session_id: "chat-b" });
    expect(store().filePreview).toBeNull();
    expect(store().dirListing).toBeNull();
    expect(store().gitStatus).toEqual([]);
    handlers.get("dir-listing")?.({ session_id: "chat-a", path: "", entries: [{ name: "old", is_dir: false }] });
    expect(store().dirListing).toBeNull();
    handlers.get("git-status-result")?.({ session_id: "chat-a", entries: [{ path: "old.md", status: "modified" }] });
    expect(store().gitStatus).toEqual([]);
    handlers.get("dir-listing")?.({ session_id: "chat-b", path: "", entries: [{ name: "new", is_dir: false }] });
    expect(store().dirListing?.[0]?.name).toBe("new");
  });

  it("explains why an older daemon's unscoped preview cannot be displayed", () => {
    useSessionStore.setState({ sessionId: "chat-b", filePreview: null });
    handlers.get("file-preview")?.({ path: "README.md", content: "unscoped", truncated: false });
    expect(store().filePreview?.content).toBe("");
    expect(store().filePreview?.error).toContain("update the daemon");
  });

  it("reports unavailable status from an older daemon instead of showing a clean tree", () => {
    useSessionStore.setState({ sessionId: "chat-b", gitStatus: [], gitStatusError: null });
    handlers.get("git-status-result")?.({ entries: [{ path: "changed.md", status: "modified" }] });
    expect(store().gitStatus).toEqual([]);
    expect(store().gitStatusError).toContain("update the daemon");
    handlers.get("git-status-result")?.({ session_id: "chat-b", entries: [] });
    expect(store().gitStatusError).toBeNull();
  });

  it("shows only status failures for the active conversation", () => {
    useSessionStore.setState({ sessionId: "chat-b", gitStatus: [], gitStatusError: null });
    handlers.get("git-status-result")?.({ session_id: "chat-a", entries: [], error: "old error" });
    expect(store().gitStatusError).toBeNull();
    handlers.get("git-status-result")?.({ session_id: "chat-b", entries: [], error: "not a git repository" });
    expect(store().gitStatusError).toContain("not a git repository");
  });

  it("explains when an older daemon returns an unscoped directory listing", () => {
    useSessionStore.setState({ sessionId: "chat-b", dirListing: null, dirListingError: null });
    handlers.get("dir-listing")?.({ path: "", entries: [{ name: "unscoped", is_dir: false }] });
    expect(store().dirListing).toBeNull();
    expect(store().dirListingError).toContain("update the daemon");
  });

  it("coalesces filesystem bursts without refreshing a different session", () => {
    vi.useFakeTimers();
    const listDir = vi.fn();
    const gitStatus = vi.fn();
    setClient({ listDir, gitStatus } as unknown as MewClient);
    useSessionStore.setState({ sessionId: "chat-a", connectionState: "connected", dirListingPath: "src" });
    store().onFsChanged([]);
    store().onFsChanged([]);
    store().onFsChanged([]);
    vi.advanceTimersByTime(200);
    expect(listDir).toHaveBeenCalledTimes(1);
    expect(gitStatus).toHaveBeenCalledTimes(1);
    store().onFsChanged([]);
    useSessionStore.setState({ sessionId: "chat-b" });
    vi.advanceTimersByTime(200);
    expect(gitStatus).toHaveBeenCalledTimes(1);
    setClient(null);
    vi.useRealTimers();
  });

  it("does not refresh a disconnected client after a filesystem event", () => {
    vi.useFakeTimers();
    const listDir = vi.fn(() => { throw new Error("closed"); });
    setClient({ listDir, gitStatus: vi.fn() } as unknown as MewClient);
    useSessionStore.setState({ sessionId: "chat-a", connectionState: "connected" });
    store().onFsChanged([]);
    useSessionStore.setState({ connectionState: "disconnected" });
    expect(() => vi.advanceTimersByTime(200)).not.toThrow();
    expect(listDir).not.toHaveBeenCalled();
    setClient(null);
    vi.useRealTimers();
  });

  it("keeps the changes tree stable for identical status responses", () => {
    const entries = [{ path: "a.txt", status: "modified" as const }];
    store().onGitStatus(entries);
    store().onGitStatus([{ path: "a.txt", status: "modified" }]);
    expect(store().gitStatus).toBe(entries);
  });

  it("shows consecutive identical prompts broadcast by the daemon", () => {
    handlers.get("user-message")?.({ text: "try again" });
    handlers.get("user-message")?.({ text: "try again" });
    expect(store().messages.map((message) => message.parts)).toEqual([
      [{ type: "text", text: "try again" }],
      [{ type: "text", text: "try again" }],
    ]);
  });

  it("deduplicates replayed permission, question, and goal requests", () => {
    for (let replay = 0; replay < 2; replay++) {
      handlers.get("permission-request")?.({ request_id: "p", tool_name: "bash", input: {} }, () => {});
      handlers.get("ask-user-request")?.({ request_id: "q", call_id: "call-q", questions: [] });
      handlers.get("goal-proposed")?.({ request_id: "g", call_id: "call-g", objective: "finish the task" });
    }

    expect(store().pendingPermissions).toHaveLength(1);
    expect(store().pendingAskUser).toHaveLength(1);
    expect(store().pendingGoals).toHaveLength(1);
  });

  it("retains child question identity across replay and resolves only that request", () => {
    const subagent = { parent_call_id: "task", child_session_id: "child", name: "worker", display_name: "Curie" };
    handlers.get("ask-user-request")?.({ request_id: "parent-q", call_id: "parent-ask", questions: [] });
    for (let replay = 0; replay < 2; replay++) {
      handlers.get("ask-user-request")?.({ request_id: "child-q", call_id: "child-ask", questions: [], subagent });
    }
    expect(store().pendingAskUser).toHaveLength(2);
    expect(store().pendingAskUser.find((request) => request.requestId === "child-q")?.subagent).toEqual(subagent);
    handlers.get("request-resolved")?.({ request_id: "child-q" });
    expect(store().pendingAskUser.map((request) => request.requestId)).toEqual(["parent-q"]);
  });

  it("clears resolved cards and response callbacks across all input kinds", () => {
    handlers.get("permission-request")?.({ request_id: "p", tool_name: "bash", input: {} }, () => {});
    handlers.get("ask-user-request")?.({ request_id: "q", call_id: "call-q", questions: [] });
    handlers.get("goal-proposed")?.({ request_id: "g", call_id: "call-g", objective: "finish the task" });
    askUserResponders.set("q", () => {});

    for (const request_id of ["p", "q", "g", "unknown"]) {
      handlers.get("request-resolved")?.({ request_id });
    }

    expect(store().pendingPermissions).toEqual([]);
    expect(store().pendingAskUser).toEqual([]);
    expect(store().pendingGoals).toEqual([]);
    expect(permissionResponders.size).toBe(0);
    expect(askUserResponders.size).toBe(0);
  });

  it("clears stale input on session-ready before replay, including reconnect to the same session", () => {
    for (const session_id of ["s1", "s2", "s2"]) {
      handlers.get("permission-request")?.({ request_id: "p", tool_name: "bash", input: {} }, () => {});
      handlers.get("ask-user-request")?.({ request_id: "q", call_id: "call-q", questions: [] });
      handlers.get("goal-proposed")?.({ request_id: "g", call_id: "call-g", objective: "finish the task" });
      askUserResponders.set("q", () => {});

      handlers.get("session-ready")?.({ session_id });

      expect(store().pendingPermissions).toEqual([]);
      expect(store().pendingAskUser).toEqual([]);
      expect(store().pendingGoals).toEqual([]);
      expect(permissionResponders.size).toBe(0);
      expect(askUserResponders.size).toBe(0);
    }
  });

  it("keeps live pending inputs when another history page arrives", () => {
    handlers.get("permission-request")?.({ request_id: "p", tool_name: "bash", input: {} }, () => {});
    handlers.get("ask-user-request")?.({ request_id: "q", call_id: "call-q", questions: [] });
    handlers.get("goal-proposed")?.({ request_id: "g", call_id: "call-g", objective: "finish the task" });

    handlers.get("session-history-page")?.({ session_id: store().sessionId, messages: [], replace: false });

    expect(store().pendingPermissions).toHaveLength(1);
    expect(store().pendingAskUser).toHaveLength(1);
    expect(store().pendingGoals).toHaveLength(1);
  });
});

describe("connection recovery", () => {
  beforeEach(() => {
    useSessionStore.setState({
      connectionState: "disconnected",
      connectionError: null,
      connectionRetryToken: 0,
    });
  });

  it("clears the error and requests a fresh connection attempt", () => {
    useSessionStore.setState({
      connectionState: "reconnecting",
      connectionError: "daemon stopped",
      connectionRetryToken: 4,
    });

    store().retryConnection();

    expect(useSessionStore.getState().connectionState).toBe("connecting");
    expect(useSessionStore.getState().connectionError).toBeNull();
    expect(useSessionStore.getState().connectionRetryToken).toBe(5);
  });
});

describe("session-ready bridge", () => {
  it("clears the socket's chat confirmation on close while retaining its selected chat", () => {
    const handlers = new Map<string, (data: any) => void>();
    const client = {
      on: (event: string, handler: (data: any) => void) => handlers.set(event, handler),
      off: vi.fn(),
      listModels: vi.fn(() => Promise.resolve([])),
      listPersonas: vi.fn(() => Promise.resolve([])),
      loadSessionIndex: vi.fn(),
    } as unknown as MewClient;
    bridgeClientToStore(client);
    handlers.get("session-ready")?.({ session_id: "chat" });
    expect(store().confirmedSessionId).toBe("chat");
    handlers.get("close")?.(1006);
    expect(store().confirmedSessionId).toBeNull();
    expect(store().sessionId).toBe("chat");
  });

  it("resets previous session messages only when session-ready switches to a different session", () => {
    const handlers = new Map<string, (data: any) => void>();
    const client = {
      on: (event: string, handler: (data: any) => void) => handlers.set(event, handler),
      off: vi.fn(),
      listModels: vi.fn(() => Promise.resolve([])),
      listPersonas: vi.fn(() => Promise.resolve([])),
      loadSessionIndex: vi.fn(),
    } as unknown as MewClient;
    bridgeClientToStore(client);

    useSessionStore.setState({
      sessionId: "chat-a",
      messages: [],
      projects: [{ path: "/tmp/proj", display_name: "proj", session_count: 1, last_used_at: 1 }],
    });
    store().addUserMessage("existing prompt");

    const seenTransitions: Array<{ sessionId: string | null; msgCount: number }> = [];
    const unsubscribe = useSessionStore.subscribe((state) => {
      seenTransitions.push({ sessionId: state.sessionId, msgCount: state.messages.length });
    });
    try {
      // Reconnecting to the same session preserves messages before replay.
      handlers.get("session-ready")?.({ session_id: "chat-a" });
      expect(store().sessionId).toBe("chat-a");
      expect(store().messages).toHaveLength(1);

      // Switching to another session resets the old conversation state atomically
      // without emitting an intermediate blank state under the old sessionId
      // or wiping daemon-global projects.
      seenTransitions.length = 0;
      handlers.get("session-ready")?.({
        session_id: "chat-b",
        provider: "antigravity",
        model: "gemini-4-hydrogen",
        permission_mode: "permissive",
      });
      expect(seenTransitions).toEqual([{ sessionId: "chat-b", msgCount: 0 }]);
      expect(store().sessionId).toBe("chat-b");
      expect(store().messages).toHaveLength(0);
      expect(store().currentProvider).toBe("antigravity");
      expect(store().currentModel).toBe("gemini-4-hydrogen");
      expect(store().permissionMode).toBe("permissive");
      expect(store().projects).toHaveLength(1);

      // Explicit session deletion clears sessionId + messages atomically while keeping projects.
      store().addUserMessage("temp");
      store().clearSession();
      expect(store().sessionId).toBeNull();
      expect(store().confirmedSessionId).toBeNull();
      expect(store().messages).toHaveLength(0);
      expect(store().projects).toHaveLength(1);
    } finally {
      unsubscribe();
    }
  });

  it("adds each authoritative user message even when consecutive prompts have identical text", () => {
    const handlers = new Map<string, (data: any) => void>();
    bridgeClientToStore({
      on: (event: string, handler: (data: any) => void) => handlers.set(event, handler),
    } as unknown as MewClient);
    useSessionStore.setState({ messages: [] });
    handlers.get("user-message")?.({ text: "same prompt" });
    handlers.get("user-message")?.({ text: "same prompt" });
    expect(store().messages).toHaveLength(2);
  });

  it("prepends older history without replacing live state or accepting another chat's page", () => {
    const handlers = new Map<string, (data: any) => void>();
    bridgeClientToStore({
      on: (event: string, handler: (data: any) => void) => handlers.set(event, handler),
    } as unknown as MewClient);
    useSessionStore.setState({ sessionId: "chat", messages: [], streamingPartId: null });
    const message = (id: string) => ({
      id,
      session_id: "chat",
      role: "user",
      parts: [{ id: `part-${id}`, type: "text", text: id }],
      time: { created: 0 },
    });
    const history = handlers.get("session-history-page");
    history?.({
      session_id: "chat", replace: true, messages: [message("recent")], next_cursor: 1,
    });
    expect(store().historyCursor).toBe(1);
    store().addUserMessage("live prompt");
    useSessionStore.setState({ streamingPartId: "live-response" });
    history?.({ session_id: "other", replace: true, messages: [] });
    history?.({
      session_id: "chat", replace: false, messages: [message("old"), message("recent")],
    });
    expect(store().messages.map((entry) => entry.parts[0])).toEqual([
      { type: "text", text: "old" },
      { type: "text", text: "recent" },
      { type: "text", text: "live prompt" },
    ]);
    expect(store().streamingPartId).toBe("live-response");
    expect(store().historyCursor).toBeNull();
  });

  it("changes persona only after the daemon confirms an explicit switch", () => {
    const handlers = new Map<string, (data: any) => void>();
    const client = {
      on: (event: string, handler: (data: any) => void) => handlers.set(event, handler),
      off: vi.fn(),
    } as unknown as MewClient;

    useSessionStore.setState({ currentPersona: "planner" });
    bridgeClientToStore(client);

    handlers.get("persona-switch-requested")?.({ name: "builder" });
    expect(useSessionStore.getState().currentPersona).toBe("planner");

    handlers.get("persona-switched")?.({ name: "builder" });
    expect(useSessionStore.getState().currentPersona).toBe("builder");
  });

  it("refreshes session metadata only after a session is ready", () => {
    const handlers = new Map<string, (data: any) => void>();
    const client = {
      on: (event: string, handler: (data: any) => void) => handlers.set(event, handler),
      off: vi.fn(),
      listModels: vi.fn(() => Promise.resolve([])),
      listPersonas: vi.fn(() => Promise.resolve([])),
      loadSessionIndex: vi.fn(),
    } as unknown as MewClient & {
      listModels: ReturnType<typeof vi.fn>;
      listPersonas: ReturnType<typeof vi.fn>;
      loadSessionIndex: ReturnType<typeof vi.fn>;
    };

    useSessionStore.setState({ sessionId: null, sessionCwd: null, messages: [] });
    bridgeClientToStore(client);

    expect(client.listModels).not.toHaveBeenCalled();
    expect(client.listPersonas).not.toHaveBeenCalled();

    handlers.get("session-ready")?.({
      session_id: "sess-ready",
      provider: "test",
      model: "model",
      permission_mode: "auto",
      cwd: "/projects/mew",
    });

    expect(client.listModels).toHaveBeenCalledOnce();
    expect(client.listPersonas).toHaveBeenCalledOnce();
    expect(client.loadSessionIndex).toHaveBeenCalledWith("sess-ready");
    expect(useSessionStore.getState().sessionId).toBe("sess-ready");
    expect(useSessionStore.getState().sessionCwd).toBe("/projects/mew");
  });

  it("does not add a pre-session no-session error to the chat", () => {
    const handlers = new Map<string, (data: any) => void>();
    const client = {
      on: (event: string, handler: (data: any) => void) => handlers.set(event, handler),
      off: vi.fn(),
    } as unknown as MewClient;

    useSessionStore.setState({ sessionId: null, sessionCwd: null, messages: [] });
    bridgeClientToStore(client);
    handlers.get("errorMessage")?.({ message: "no session" });

    expect(useSessionStore.getState().messages).toHaveLength(0);
  });
});

describe("alert lifecycle", () => {
  beforeEach(() => {
    // Reset alerts between tests.
    store().clearAlertsForSession("test-a");
    store().clearAlertsForSession("test-b");
  });

  it("onSessionAlert stores actionable alerts but ignores completion noise", () => {
    store().onSessionAlert("test-a", "Session A", "turn_complete");
    expect(useSessionStore.getState().alerts).toHaveLength(0);
    store().onSessionAlert("test-a", "Session A", "permission_needed");
    const alerts = useSessionStore.getState().alerts;
    expect(alerts).toHaveLength(1);
    expect(alerts[0]!.sessionId).toBe("test-a");
    expect(alerts[0]!.kind).toBe("permission_needed");
  });

  it("clearAlertsForSession removes only matching alerts", () => {
    store().onSessionAlert("test-a", "A", "permission_needed");
    store().onSessionAlert("test-b", "B", "permission_needed");
    store().onSessionAlert("test-a", "A2", "turn_failed");

    store().clearAlertsForSession("test-a");

    const alerts = useSessionStore.getState().alerts;
    expect(alerts).toHaveLength(1);
    expect(alerts[0]!.sessionId).toBe("test-b");
  });

  it("dismissAlert removes an entry by timestamp", async () => {
    useSessionStore.setState({ alerts: [] });

    store().onSessionAlert("test-a", "A", "permission_needed");
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

  it("a successful turn clears the previous failure alert", () => {
    store().onSessionAlert("test-a", "A", "turn_failed");
    expect(useSessionStore.getState().alerts).toHaveLength(1);
    store().onSessionAlert("test-a", "A", "turn_complete");
    expect(useSessionStore.getState().alerts).toHaveLength(0);
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

  it("tracks active session running state directly even before listSessions resolves or when a stale list arrives", () => {
    const handlers = new Map<string, (data: any) => void>();
    const client = {
      on: (event: string, handler: (data: any) => void) => handlers.set(event, handler),
      off: vi.fn(),
      listModels: vi.fn(() => Promise.resolve([])),
      listPersonas: vi.fn(() => Promise.resolve([])),
      loadSessionIndex: vi.fn(),
    } as unknown as MewClient;
    bridgeClientToStore(client);

    useSessionStore.setState({ availableSessions: [], sessionId: null, confirmedSessionId: null });
    handlers.get("session-ready")?.({ session_id: "new-sess" });
    expect(selectSessionRunning(useSessionStore.getState())).toBe(false);

    // session_activity_changed arrives before listSessions populates availableSessions.
    handlers.get("session-activity-changed")?.({ session_id: "new-sess", activity: "running" });
    expect(selectSessionRunning(useSessionStore.getState())).toBe(true);

    // An in-flight listSessions reply saying "idle" must not overwrite live "running".
    store().setAvailableSessions([
      { session_id: "new-sess", state: "idle", created_at: 1, client_count: 1 },
    ]);
    expect(selectSessionRunning(useSessionStore.getState())).toBe(true);
    expect(useSessionStore.getState().availableSessions[0]!.state).toBe("running");

    // Switching sessions clears sessionActivity so running state does not leak.
    handlers.get("session-ready")?.({ session_id: "other-sess" });
    expect(selectSessionRunning(useSessionStore.getState())).toBe(false);

    // Re-attaching and idling clears running state back to false.
    handlers.get("session-activity-changed")?.({ session_id: "other-sess", activity: "running" });
    expect(selectSessionRunning(useSessionStore.getState())).toBe(true);
    handlers.get("session-activity-changed")?.({ session_id: "other-sess", activity: "idle" });
    expect(selectSessionRunning(useSessionStore.getState())).toBe(false);
  });

  it("onSessionUsageChanged updates usage on session", () => {
    useSessionStore.setState({
      sessionId: "s1",
      contextTokens: 900,
      contextTokensKnown: true,
      availableSessions: [
        {
          session_id: "s1",
          state: "active",
          created_at: 0,
          client_count: 0,
        },
      ],
    });

    store().onSessionUsageChanged(
      "s1",
      {
        input_tokens: 1000,
        output_tokens: 500,
        cache_read_tokens: 200,
        cache_write_tokens: 100,
        cost: 0.05,
        turns: 3,
      },
      900,
    );

    const s = useSessionStore.getState().availableSessions[0]!;
    expect(s.usage).toBeDefined();
    expect(s.usage!.cost).toBe(0.05);
    expect(s.usage!.turns).toBe(3);
    expect(s.context_tokens).toBe(900);
    store().onSessionUsageChanged("s1", s.usage!);
    expect(useSessionStore.getState().availableSessions[0]!.context_tokens).toBe(900);
    store().onSessionUsageChanged("s1", s.usage!, null);
    expect(useSessionStore.getState().availableSessions[0]!.context_tokens).toBeUndefined();
    expect(useSessionStore.getState().contextTokensKnown).toBe(false);
  });

  it("restores the active context reading from the session list", () => {
    useSessionStore.setState({ sessionId: "s1", confirmedSessionId: "s1", contextTokens: 0, contextTokensKnown: true });
    const info = { session_id: "s1", state: "active" as const, created_at: 0, client_count: 0 };
    store().setAvailableSessions([info]);
    expect(useSessionStore.getState().contextTokensKnown).toBe(false);
    store().setAvailableSessions([{ ...info, context_tokens: 4200 }]);
    expect(useSessionStore.getState().contextTokensKnown).toBe(true);
    expect(useSessionStore.getState().contextTokens).toBe(4200);
  });

  it("seeds session titles from the session list", () => {
    useSessionStore.setState({ sessionTitles: new Map() });
    store().setAvailableSessions([
      { session_id: "s1", state: "idle", created_at: 0, client_count: 0, title: "Fix the flaky test" },
      { session_id: "s2", state: "idle", created_at: 0, client_count: 0 },
    ]);
    expect(useSessionStore.getState().sessionTitles.get("s1")).toBe("Fix the flaky test");
    expect(useSessionStore.getState().sessionTitles.has("s2")).toBe(false);
  });
});

describe("assistant metadata preservation", () => {
  beforeEach(() => {
    useSessionStore.setState({
      messages: [],
      totalInputTokens: 0,
      totalOutputTokens: 0,
      totalCost: 0,
    });
  });

  it("onSessionHistory preserves assistantMeta", () => {
    const manifest = {
      model: "gpt-4o",
      context_window: 128000,
      input_tokens: 5000,
      output_tokens: 200,
      cache_read_tokens: 1000,
      cache_write_tokens: 0,
      reasoning_tokens: 0,
      segments: [],
    };

    store().onSessionHistory([
      {
        id: "msg-1",
        session_id: "sess-1",
        role: "assistant",
        parts: [
          {
            type: "text",
            id: "part-1",
            message_id: "msg-1",
            session_id: "sess-1",
            text: "Hello",
            synthetic: false,
          },
        ],
        time: { created: 0, completed: undefined },
        assistant: {
          provider_id: "openai",
          model_id: "gpt-4o",
          cost: 0.01,
          tokens: {
            input: 5000,
            output: 200,
            reasoning: 0,
            cache_read: 1000,
            cache_write: 0,
          },
          finish: "stop",
          manifest,
        },
      },
    ]);

    const messages = useSessionStore.getState().messages;
    expect(messages).toHaveLength(1);
    expect(messages[0]!.assistantMeta).toBeDefined();
    expect(messages[0]!.assistantMeta!.manifest).toBeDefined();
    expect(messages[0]!.assistantMeta!.manifest!.model).toBe("gpt-4o");
  });

  it("message_end attaches assistantMeta to last assistant message", () => {
    useSessionStore.setState({
      messages: [
        {
          id: "msg-1",
          role: "assistant" as const,
          parts: [{ type: "text" as const, text: "Hello" }],
          timestamp: 0,
        },
      ],
    });

    const manifest = {
      model: "deepseek-v3",
      context_window: 64000,
      input_tokens: 3000,
      output_tokens: 150,
      cache_read_tokens: 0,
      cache_write_tokens: 0,
      reasoning_tokens: 0,
      segments: [],
    };

    store().onProviderEvent({
      type: "message_end",
      finish: "stop",
      usage: {
        input: 3000,
        output: 150,
        reasoning: 0,
        cache_read: 0,
        cache_write: 0,
      },
      cost: 0.02,
      manifest,
    });

    const messages = useSessionStore.getState().messages;
    expect(messages[0]!.assistantMeta).toBeDefined();
    expect(messages[0]!.assistantMeta!.cost).toBe(0.02);
    expect(messages[0]!.assistantMeta!.manifest).toBeDefined();
    expect(messages[0]!.assistantMeta!.manifest!.model).toBe("deepseek-v3");
    expect(useSessionStore.getState().totalInputTokens).toBe(3000);
    expect(useSessionStore.getState().totalCost).toBe(0.02);
    expect(useSessionStore.getState().contextTokens).toBe(3000);
    expect(useSessionStore.getState().contextWindow).toBe(64000);
  });

  it("message_end snapshots context occupancy instead of accumulating", () => {
    const fire = (input: number, contextWindow?: number) =>
      store().onProviderEvent({
        type: "message_end",
        finish: "stop",
        usage: { input, output: 10, reasoning: 0, cache_read: 0, cache_write: 0 },
        cost: 0,
        manifest: contextWindow
          ? {
              model: "m",
              context_window: contextWindow,
              input_tokens: input,
              output_tokens: 10,
              cache_read_tokens: 0,
              cache_write_tokens: 0,
              reasoning_tokens: 0,
              segments: [],
            }
          : undefined,
      });

    fire(10_000, 200_000);
    expect(useSessionStore.getState().contextTokens).toBe(10_000);
    expect(useSessionStore.getState().contextWindow).toBe(200_000);

    // A larger follow-up context replaces the reading, and a manifest without
    // a window keeps the last known one.
    fire(180_000);
    expect(useSessionStore.getState().contextTokens).toBe(180_000);
    expect(useSessionStore.getState().contextWindow).toBe(200_000);
  });

  it("message_end attaches assistantMeta even when last message is not assistant", () => {
    // A user message was appended after the assistant message (e.g. user typed
    // while streaming was finishing). The handler should search backwards for
    // the last assistant message, not just check messages[length-1].
    useSessionStore.setState({
      messages: [
        {
          id: "msg-1",
          role: "assistant" as const,
          parts: [{ type: "text" as const, text: "Hello" }],
          timestamp: 0,
        },
        {
          id: "msg-2",
          role: "user" as const,
          parts: [{ type: "text" as const, text: "Follow up" }],
          timestamp: 1,
        },
      ],
    });

    const manifest = {
      model: "deepseek-v3",
      context_window: 64000,
      input_tokens: 3000,
      output_tokens: 150,
      cache_read_tokens: 0,
      cache_write_tokens: 0,
      reasoning_tokens: 0,
      segments: [],
    };

    store().onProviderEvent({
      type: "message_end",
      finish: "stop",
      usage: {
        input: 3000,
        output: 150,
        reasoning: 0,
        cache_read: 0,
        cache_write: 0,
      },
      cost: 0.02,
      manifest,
    });

    const messages = useSessionStore.getState().messages;
    // The assistant message (index 0) should have assistantMeta, not the user message.
    expect(messages[0]!.assistantMeta).toBeDefined();
    expect(messages[0]!.assistantMeta!.manifest).toBeDefined();
    expect(messages[0]!.assistantMeta!.manifest!.model).toBe("deepseek-v3");
    // The user message (index 1) should NOT have assistantMeta.
    expect(messages[1]!.assistantMeta).toBeUndefined();
  });

  it("onSessionHistory preserves messages with assistantMeta but no visible parts", () => {
    // An assistant message with only an empty text part (model returned empty)
    // would normally be filtered out, but it has assistantMeta with a manifest
    // that should be preserved for the inspector.
    const manifest = {
      model: "gpt-4o",
      context_window: 128000,
      input_tokens: 5000,
      output_tokens: 0,
      cache_read_tokens: 0,
      cache_write_tokens: 0,
      reasoning_tokens: 0,
      segments: [],
    };

    store().onSessionHistory([
      {
        id: "msg-empty",
        session_id: "sess-1",
        role: "assistant",
        parts: [
          {
            type: "text",
            id: "part-empty", message_id: "msg-empty", session_id: "sess-1",
            text: "",
            synthetic: false,
          },
        ],
        time: { created: 0, completed: undefined },
        assistant: {
          provider_id: "openai",
          model_id: "gpt-4o",
          cost: 0.01,
          tokens: {
            input: 5000,
            output: 0,
            reasoning: 0,
            cache_read: 0,
            cache_write: 0,
          },
          finish: "stop",
          manifest,
        },
      },
    ]);

    const messages = useSessionStore.getState().messages;
    // The message should be preserved despite having no visible parts,
    // because it carries assistantMeta.
    expect(messages).toHaveLength(1);
    expect(messages[0]!.assistantMeta).toBeDefined();
    expect(messages[0]!.assistantMeta!.manifest).toBeDefined();
  });
});

describe("authoritative activity recovery", () => {
  beforeEach(() => store().reset());

  it("replaces stale activity only for the confirmed session", () => {
    store().setSessionId("active");
    store().onSubagentStart({ parent_call_id: "stale", child_session_id: "old-child", name: "worker", display_name: null });
    store().onJobUpdate({ jobId: "stale-job", command: "old", state: "running" });
    const snapshot = {
      session_id: "active",
      subagents: [{ task_id: "task", child_session_id: "child", name: "worker", display_name: "Curie", status: "Reading the build log", tool_name: "read" }],
      jobs: [{ job_id: "job", command: "cargo test", state: "running" }],
    };
    store().onSessionActivitySnapshot({ ...snapshot, session_id: "other" });
    expect([...store().subagents.keys()]).toEqual(["stale"]);
    expect([...store().jobs.keys()]).toEqual(["stale-job"]);
    store().onSessionActivitySnapshot(snapshot);
    expect([...store().subagents.keys()]).toEqual(["task"]);
    expect(store().subagents.get("task")).toMatchObject({
      childSessionId: "child", displayName: "Curie", status: "running", lastProgress: "Reading the build log",
    });
    expect([...store().jobs.values()]).toEqual([{ jobId: "job", command: "cargo test", state: "running" }]);
    store().onSessionActivitySnapshot({ session_id: "active", subagents: [], jobs: [] });
    expect(store().subagents.size).toBe(0);
    expect(store().jobs.size).toBe(0);
  });

  it("ignores completion for a different child under the same call", () => {
    store().onSubagentStart({ parent_call_id: "task", child_session_id: "current-child", name: "worker", display_name: null });
    store().onSubagentEnd({ parent_call_id: "task", child_session_id: "old-child", outcome: { type: "completed" } });
    expect(store().subagents.get("task")).toMatchObject({ childSessionId: "current-child", status: "running" });
  });
});

describe("nested Python tools", () => {
  let handlers: Map<string, (...args: any[]) => void>;
  beforeEach(() => {
    store().reset();
    handlers = new Map();
    bridgeClientToStore({ on: (event: string, handler: (...args: any[]) => void) => handlers.set(event, handler), listModels: () => Promise.resolve([]), listPersonas: () => Promise.resolve([]) } as unknown as MewClient);
    useSessionStore.setState({ sessionId: "chat", messages: [{ id: "msg", role: "assistant", timestamp: 0, parts: [{ type: "tool-call", callId: "py", toolName: "python", input: { source: "pass" }, state: "running" }] }] });
  });
  it("updates the streamed cell when the daemon fills in its call identity", () => {
    useSessionStore.setState({ messages: [] });
    const part = { type: "tool_call", id: "part-1", message_id: "message-1", session_id: "chat", tool_name: "", call_id: "", state: { status: "pending", input: {}, time: { start: 0, end: null } } };
    handlers.get("provider")?.({ type: "part_start", part });
    handlers.get("part-updated")?.({ part_id: "part-1", part: { ...part, tool_name: "python", call_id: "py", state: { status: "running", input: { source: "print(1)" }, output: "", time: { start: 1, end: null } } } });
    handlers.get("nested-tool-start")?.({ parent_call_id: "py", call_id: "child", tool_name: "read", input: { path: "notes.txt" } });
    const cell = store().messages[0]!.parts[0]!;
    expect(cell.type === "tool-call" && { name: cell.toolName, callId: cell.callId, state: cell.state, input: cell.input, nested: cell.nestedCalls?.length }).toEqual({ name: "python", callId: "py", state: "running", input: { source: "print(1)" }, nested: 1 });
  });
  it("tracks and deduplicates nested calls without polluting Python output", () => {
    const start = { parent_call_id: "py", call_id: "child", tool_name: "write", input: { path: "report.md" } };
    handlers.get("nested-tool-start")?.(start);
    handlers.get("nested-tool-start")?.(start);
    handlers.get("nested-tool-end")?.({ parent_call_id: "py", call_id: "child", success: false, error: "permission denied" });
    handlers.get("nested-tool-start")?.(start);
    const part = store().messages[0]!.parts[0]!;
    expect(part.type).toBe("tool-call");
    if (part.type !== "tool-call") throw new Error("expected tool call");
    expect(part.nestedCalls).toEqual([{ ...start, status: "error", error: "permission denied" }]);
    expect(store().toolOutputs.get("py")).toBeUndefined();
  });
  it("restores nested calls from history under the right Python cell", () => {
    const child = { parent_call_id: "py", call_id: "child", tool_name: "read", input: { path: "notes.txt" }, status: "completed", output_bytes: 12 };
    handlers.get("session-history-page")?.({ session_id: "chat", replace: true, messages: [{ id: "m", session_id: "chat", role: "assistant", time: { created: 0 }, assistant: { nested_tool_calls: [child] }, parts: [{ type: "tool_call", id: "p", message_id: "m", session_id: "chat", tool_name: "python", call_id: "py", state: { status: "completed", input: { source: "pass" }, output: "done", time: { start: 0, end: 1 } } }] }] });
    const part = store().messages[0]!.parts[0]!;
    expect(part.type === "tool-call" && part.nestedCalls).toEqual([child]);
    expect(part.type === "tool-call" && { state: part.state, output: part.output }).toEqual({ state: "completed", output: "done" });
  });
});

describe("queued messages state", () => {
  let handlers: Map<string, (...args: any[]) => void>;
  beforeEach(() => {
    store().reset();
    handlers = new Map();
    bridgeClientToStore({
      on: (event: string, handler: (...args: any[]) => void) => handlers.set(event, handler),
      listModels: () => Promise.resolve([]),
      listPersonas: () => Promise.resolve([]),
      loadSessionIndex: vi.fn(),
    } as unknown as MewClient);
  });

  it("applies session-queue-changed only for the active session and clears on reset", () => {
    useSessionStore.setState({ sessionId: "chat-1" });
    const item = { id: "q-1", text: "after this turn", mode: "follow_up" as const, created_at: 100 };
    handlers.get("session-queue-changed")?.({ session_id: "chat-2", items: [item] });
    expect(store().queuedMessages).toEqual([]);

    handlers.get("session-queue-changed")?.({ session_id: "chat-1", items: [item] });
    expect(store().queuedMessages).toEqual([item]);

    store().onSessionCleared();
    expect(store().queuedMessages).toEqual([]);
  });
});

describe("turn completion finalization on error, slash result, and idle", () => {
  beforeEach(() => store().reset());

  it("finalizes open assistant messages and clears streaming fields on onError", () => {
    useSessionStore.setState({
      streamingPartId: "p-1",
      streamingText: "partial",
      streamingReasoningId: "r-1",
      streamingReasoningText: "thinking",
      messages: [
        {
          id: "m-open",
          role: "assistant",
          timestamp: 1000,
          parts: [{ type: "text", text: "partial", streaming: true }],
        },
      ],
    });

    store().onError("SSE stream ended before the response was complete");

    const msgs = store().messages;
    expect(msgs).toHaveLength(2);
    expect(msgs[0]!.completedAt).toBeTypeOf("number");
    expect(msgs[0]!.parts[0]).toEqual({ type: "text", text: "partial", streaming: false });
    expect(msgs[1]!.completedAt).toBeTypeOf("number");
    expect(msgs[1]!.parts[0]).toEqual({
      type: "error",
      message: "SSE stream ended before the response was complete",
    });
    expect(store().streamingPartId).toBeNull();
    expect(store().streamingText).toBe("");
    expect(store().streamingReasoningId).toBeNull();
    expect(store().streamingReasoningText).toBe("");
  });

  it("stamps completedAt on synthetic slash result messages", () => {
    store().onSlashResult("compacted 1925 messages");
    const msg = store().messages[0]!;
    expect(msg.completedAt).toBeTypeOf("number");
    expect(msg.parts[0]).toEqual({ type: "text", text: "compacted 1925 messages" });
  });

  it("finalizes open assistant messages and clears streaming state when active session becomes idle", () => {
    useSessionStore.setState({
      sessionId: "sess-1",
      availableSessions: [{ session_id: "sess-1", state: "running" } as never],
      streamingPartId: "p-1",
      streamingText: "half",
      messages: [
        {
          id: "m-open",
          role: "assistant",
          timestamp: 1000,
          parts: [{ type: "reasoning", text: "half", streaming: true }],
        },
      ],
    });

    store().onSessionActivityChanged("sess-1", "idle");

    expect(store().messages[0]!.completedAt).toBeTypeOf("number");
    expect(store().messages[0]!.parts[0]).toEqual({
      type: "reasoning",
      text: "half",
      streaming: false,
    });
    expect(store().streamingPartId).toBeNull();
    expect(store().streamingText).toBe("");
  });
});

describe("compacting, retry_wait, steered, and cancelled lifecycle", () => {
  let handlers: Map<string, (...args: any[]) => void>;

  beforeEach(() => {
    store().reset();
    handlers = new Map();
    bridgeClientToStore({
      on: (event: string, handler: (...args: any[]) => void) => handlers.set(event, handler),
      listModels: () => Promise.resolve([]),
      listPersonas: () => Promise.resolve([]),
      loadSessionIndex: vi.fn(),
    } as unknown as MewClient);
    useSessionStore.setState({
      sessionId: "sess-1",
      confirmedSessionId: "sess-1",
      availableSessions: [{ session_id: "sess-1", state: "running" } as never],
    });
  });

  it("tracks session-compacting state and deduplicates /compact slash output", () => {
    handlers.get("session-compacting")?.({ session_id: "other", active: true });
    expect(store().compacting).toBe(false);
    expect(store().compactingStartedAt).toBeNull();

    handlers.get("session-compacting")?.({ session_id: "sess-1", active: true });
    expect(store().compacting).toBe(true);
    expect(store().compactingStartedAt).toBeTypeOf("number");

    handlers.get("session-compacting")?.({
      session_id: "sess-1",
      active: false,
      removed: 12,
      summary: "## Goal & Constraints\nLive summary",
    });
    expect(store().compacting).toBe(false);
    expect(store().compactingStartedAt).toBeNull();
    expect(store().messages).toHaveLength(1);
    expect(store().messages[0]!.role).toBe("system");
    expect(store().messages[0]!.parts[0]).toEqual({
      type: "compaction",
      summary: "## Goal & Constraints\nLive summary",
      removed: 12,
    });

    // /compact slash output arriving right after should not add a duplicate text card.
    handlers.get("slash-result")?.({ text: "compacted 12 messages" });
    expect(store().messages).toHaveLength(1);

    // Reverse ordering: slash-result before session-compacting replaces the plain text bubble.
    store().reset();
    useSessionStore.setState({ sessionId: "sess-1" });
    handlers.get("slash-result")?.({ text: "compacted 5 messages" });
    expect(store().messages).toHaveLength(1);
    handlers.get("session-compacting")?.({ session_id: "sess-1", active: false, removed: 5 });
    expect(store().messages).toHaveLength(1);
    expect(store().messages[0]!.parts[0]).toEqual({ type: "compaction", summary: "", removed: 5 });
  });

  it("tracks retry_wait state and clears it on next token, error, or idle", () => {
    handlers.get("provider")?.({
      type: "retry_wait",
      attempt: 1,
      max_attempts: 3,
      delay_secs: 4,
      reason: "overloaded",
    });
    expect(store().retryWait).toEqual({
      attempt: 1,
      maxAttempts: 3,
      delaySecs: 4,
      reason: "overloaded",
    });

    handlers.get("provider")?.({
      type: "part_start",
      part: { id: "p-1", message_id: "m-1", session_id: "sess-1", type: "text", text: "" },
    });
    expect(store().retryWait).toBeNull();

    handlers.get("provider")?.({
      type: "retry_wait",
      attempt: 2,
      max_attempts: 3,
      delay_secs: 8,
      reason: "timeout",
    });
    expect(store().retryWait?.attempt).toBe(2);

    handlers.get("provider")?.({ type: "error", error: { message: "boom" } });
    expect(store().retryWait).toBeNull();
  });

  it("marks steered user messages and cancelled turns", () => {
    handlers.get("user-message")?.({ text: "normal prompt", steered: false });
    handlers.get("session-activity-changed")?.({
      session_id: "sess-1",
      activity: "idle",
      cancelled: true,
    });
    expect(store().messages).toHaveLength(2);
    expect(store().messages[0]!.steered).toBeUndefined();
    expect(store().messages[1]!.role).toBe("assistant");
    expect(store().messages[1]!.cancelled).toBe(true);

    handlers.get("user-message")?.({ text: "course correct", steered: true });
    expect(store().messages[2]!.steered).toBe(true);
    handlers.get("provider")?.({
      type: "part_start",
      part: { id: "p-2", message_id: "m-2", session_id: "sess-1", type: "text", text: "" },
    });
    handlers.get("session-activity-changed")?.({
      session_id: "sess-1",
      activity: "idle",
      cancelled: true,
    });
    expect(store().messages[3]!.cancelled).toBe(true);
  });

  it("maps wire compaction parts from session_history", () => {
    store().onSessionHistory([
      {
        id: "comp-1",
        session_id: "sess-1",
        role: "system",
        time: { created: 1000, completed: 1000 },
        parts: [
          {
            id: "cp-1",
            message_id: "comp-1",
            session_id: "sess-1",
            type: "compaction",
            auto: true,
            overflow: false,
            summary: "Summary of dropped turns",
            removed_count: 8,
          },
        ],
      },
    ]);
    expect(store().messages).toHaveLength(1);
    expect(store().messages[0]!.parts[0]).toEqual({
      type: "compaction",
      summary: "Summary of dropped turns",
      removed: 8,
    });
  });

  it("hides synthetic flagged-file preservation messages while keeping normal user text", () => {
    const preservedMsg = {
      id: "flag-1",
      session_id: "sess-1",
      role: "user" as const,
      time: { created: 1000 },
      parts: [
        {
          id: "p-1",
          message_id: "flag-1",
          session_id: "sess-1",
          type: "text" as const,
          text: "Flagged file (preserved across compaction) — /Users/natalie/code/mew/PLAN.md:\n\n# Plan",
          synthetic: true,
        },
      ],
    };
    expect(wireMessageToChatMessage(preservedMsg).parts).toEqual([]);

    const referencedMsg = {
      id: "flag-2",
      session_id: "sess-1",
      role: "user" as const,
      time: { created: 1001 },
      parts: [
        {
          id: "p-2",
          message_id: "flag-2",
          session_id: "sess-1",
          type: "text" as const,
          text: "Flagged file (referenced — re-read with the read tool when needed): /Users/natalie/code/mew/PLAN.md",
          synthetic: true,
        },
      ],
    };
    expect(wireMessageToChatMessage(referencedMsg).parts).toEqual([]);

    const userTypedMsg = {
      id: "user-1",
      session_id: "sess-1",
      role: "user" as const,
      time: { created: 1002 },
      parts: [
        {
          id: "p-3",
          message_id: "user-1",
          session_id: "sess-1",
          type: "text" as const,
          text: "Flagged file (preserved across compaction) shouldn't be hidden when typed by a user",
          synthetic: false,
        },
      ],
    };
    expect(wireMessageToChatMessage(userTypedMsg).parts).toHaveLength(1);

    store().onSessionHistory([preservedMsg, referencedMsg, userTypedMsg]);
    expect(store().messages.map((m) => m.id)).toEqual(["user-1"]);
  });
});
