import { describe, it, expect, beforeEach, vi } from "vitest";
import { bridgeClientToStore, useSessionStore } from "../stores/session";
import type { MewClient } from "@mew/web-client";

// Helper to get fresh store state.
function store() {
  return useSessionStore.getState();
}

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
  it("refreshes session metadata only after a session is ready", () => {
    const handlers = new Map<string, (data: any) => void>();
    const client = {
      on: (event: string, handler: (data: any) => void) => handlers.set(event, handler),
      off: vi.fn(),
      listModels: vi.fn(() => Promise.resolve([])),
      listPersonas: vi.fn(() => Promise.resolve([])),
    } as unknown as MewClient & {
      listModels: ReturnType<typeof vi.fn>;
      listPersonas: ReturnType<typeof vi.fn>;
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

describe("plan approval lifecycle", () => {
  beforeEach(() => {
    store().reset();
  });

  it("onPlanApprovalRequest pushes a pending approval", () => {
    store().onPlanApprovalRequest({
      request_id: "r1",
      call_id: "c1",
      plan_path: "/repo/PLAN.md",
      plan_markdown: "# Goal\n\n1. do it",
      persona: "builder",
    });
    const pending = useSessionStore.getState().pendingPlanApprovals;
    expect(pending).toHaveLength(1);
    expect(pending[0]!.requestId).toBe("r1");
    expect(pending[0]!.persona).toBe("builder");
    expect(pending[0]!.planMarkdown).toContain("do it");
  });

  it("resolvePlanApproval removes the matching entry", () => {
    store().onPlanApprovalRequest({
      request_id: "r1",
      call_id: "c1",
      plan_path: "/repo/PLAN.md",
      plan_markdown: "plan",
      persona: "builder",
    });
    store().onPlanApprovalRequest({
      request_id: "r2",
      call_id: "c2",
      plan_path: "/repo/PLAN.md",
      plan_markdown: "plan2",
      persona: "builder",
    });
    store().resolvePlanApproval("r1");
    const pending = useSessionStore.getState().pendingPlanApprovals;
    expect(pending).toHaveLength(1);
    expect(pending[0]!.requestId).toBe("r2");
  });

  it("reset clears pending plan approvals", () => {
    store().onPlanApprovalRequest({
      request_id: "r1",
      call_id: "c1",
      plan_path: "/repo/PLAN.md",
      plan_markdown: "plan",
      persona: "builder",
    });
    store().reset();
    expect(useSessionStore.getState().pendingPlanApprovals).toHaveLength(0);
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
            base: {
              id: "part-1",
              message_id: "msg-1",
              session_id: "sess-1",
            },
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
            base: { id: "part-empty", message_id: "msg-empty", session_id: "sess-1" },
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
