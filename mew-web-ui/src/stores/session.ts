import { create } from "zustand";
import type {
  MewClient,
  ProviderEventWire,
  Part,
  PermissionDecision,
  ModelInfo,
  SessionInfo,
  Message,
  Question,
  SubagentOutcome,
  Todo as WireTodo,
} from "@mew/web-client";

// ---------------------------------------------------------------------------
// Display types
// ---------------------------------------------------------------------------

export interface ChatMessage {
  id: string;
  role: "user" | "assistant";
  parts: MessagePart[];
  timestamp: number;
}

export type MessagePart =
  | { type: "text"; text: string; streaming?: boolean }
  | { type: "reasoning"; text: string; streaming?: boolean }
  | { type: "tool-call"; toolName: string; callId: string; input: unknown; state: ToolDisplayState; output?: string }
  | { type: "error"; message: string };

export type ToolDisplayState = "pending" | "running" | "completed" | "error";

/** Map the wire ToolState.type to our display state. */
function matchToolState(type: string): ToolDisplayState {
  switch (type) {
    case "Pending":
      return "pending";
    case "Running":
      return "running";
    case "Completed":
      return "completed";
    case "Error":
      return "error";
    default:
      return "pending";
  }
}

/** Convert a wire Part (from SessionHistory) into the store's MessagePart
 *  shape, or null if the part should be skipped (e.g. ToolResult parts
 *  are absorbed into the preceding ToolCall). Wire parts are discriminated
 *  by PascalCase `type`; we map each to the display representation. */
function wirePartToMessagePart(part: Part): MessagePart | null {
  switch (part.type) {
    case "Text":
      if (!part.text || part.text.trim() === "") return null;
      return { type: "text", text: part.text };
    case "Reasoning":
      return { type: "reasoning", text: part.text };
    case "ToolCall": {
      const state: ToolDisplayState = matchToolState(part.state.type);
      const output =
        part.state.type === "Completed" || part.state.type === "Running"
          ? part.state.output
          : part.state.type === "Error"
            ? part.state.error
            : undefined;
      return {
        type: "tool-call",
        toolName: part.tool_name,
        callId: part.call_id,
        input: part.state.input,
        state,
        output,
      };
    }
    case "ToolResult":
      // ToolResult parts are absorbed into the preceding ToolCall's
      // state; the ToolCall part already carries the output. Skip.
      return null;
    case "File":
      return { type: "text", text: `[file: ${part.url}]` };
    case "Compaction":
      return { type: "text", text: "[context compacted]" };
  }
}

export interface PendingPermission {
  requestId: number;
  toolName: string;
  input: Record<string, unknown>;
}

export interface SubagentInfo {
  parentCallId: string;
  name: string;
  childSessionId: string;
  displayName: string | null;
  status: "running" | "completed" | "cancelled" | "failed";
  lastProgress: string | null;
  outcome: SubagentOutcome | null;
}

export interface PendingAskUser {
  requestId: number;
  callId: string;
  questions: Question[];
}

export interface TodoItem {
  id: number;
  content: string;
  status: "pending" | "in_progress" | "done" | "blocked";
  dependsOn: number[];
}

export type ConnectionState = "disconnected" | "connecting" | "connected" | "reconnecting";

// ---------------------------------------------------------------------------
// Store
// ---------------------------------------------------------------------------

interface SessionState {
  // Connection
  connectionState: ConnectionState;
  sessionId: string | null;

  // Messages
  messages: ChatMessage[];
  streamingPartId: string | null;
  streamingText: string;
  streamingReasoningId: string | null;
  streamingReasoningText: string;

  // Tool states
  toolStates: Map<string, ToolDisplayState>;
  toolOutputs: Map<string, string>;

  // Permissions
  pendingPermissions: PendingPermission[];

  // Cost tracking (accumulated from MessageEnd events)
  totalInputTokens: number;
  totalOutputTokens: number;
  totalCost: number;

  // Model management
  availableModels: ModelInfo[];
  currentModel: string | null;
  currentProvider: string | null;
  currentThinkingVariant: string | null;

  // Session list
  availableSessions: SessionInfo[];
  sessionsLoading: boolean;

  // Session titles (session_id → title)
  sessionTitles: Map<string, string>;

  // Subagents
  subagents: Map<string, SubagentInfo>;

  // Ask-user requests
  pendingAskUser: PendingAskUser[];

  // Todo list
  todos: TodoItem[];

  // Actions
  setConnectionState: (s: ConnectionState) => void;
  setSessionId: (id: string | null) => void;

  addUserMessage: (text: string) => void;

  // Event handlers (called by the mew-web-client bridge)
  onProviderEvent: (ev: ProviderEventWire) => void;
  onToolStart: (callId: string) => void;
  onToolEnd: (callId: string, success: boolean) => void;
  onToolProgress: (callId: string, chunk: string) => void;
  onPartUpdated: (partId: string, part: Part) => void;
  onPermissionRequest: (req: PendingPermission) => void;
  resolvePermission: (requestId: number) => void;
  onError: (message: string) => void;
  onSlashResult: (text: string) => void;

  setAvailableModels: (models: ModelInfo[]) => void;
  setCurrentModel: (provider: string, model: string) => void;
  setCurrentThinkingVariant: (variant: string | null) => void;
  onSessionTitleChanged: (sessionId: string, title: string) => void;

  // Shared-session actions
  setAvailableSessions: (sessions: SessionInfo[]) => void;
  setSessionsLoading: (loading: boolean) => void;
  onSessionHistory: (messages: Message[]) => void;
  onSessionCleared: () => void;

  // Subagent actions
  onSubagentStart: (data: { parent_call_id: string; name: string; child_session_id: string; display_name: string | null }) => void;
  onSubagentStatus: (data: { parent_call_id: string; tool_name: string; message: string }) => void;
  onSubagentEnd: (data: { parent_call_id: string; child_session_id: string; outcome: SubagentOutcome }) => void;

  // Ask-user actions
  onAskUserRequest: (data: { request_id: number; call_id: string; questions: Question[] }) => void;
  resolveAskUser: (requestId: number) => void;

  // Todo actions
  onTodosUpdated: (todos: WireTodo[]) => void;

  reset: () => void;
}

export const useSessionStore = create<SessionState>((set, get) => ({
  connectionState: "disconnected",
  sessionId: null,
  messages: [],
  streamingPartId: null,
  streamingText: "",
  streamingReasoningId: null,
  streamingReasoningText: "",
  toolStates: new Map(),
  toolOutputs: new Map(),
  pendingPermissions: [],
  totalInputTokens: 0,
  totalOutputTokens: 0,
  totalCost: 0,
  availableModels: [],
  currentModel: null,
  currentProvider: null,
  currentThinkingVariant: null,
  availableSessions: [],
  sessionsLoading: false,
  sessionTitles: new Map(),
  subagents: new Map(),
  pendingAskUser: [],
  todos: [],

  setConnectionState: (s) => set({ connectionState: s }),
  setSessionId: (id) => set({ sessionId: id }),

  addUserMessage: (text) =>
    set((state) => ({
      messages: [
        ...state.messages,
        {
          id: crypto.randomUUID(),
          role: "user" as const,
          parts: [{ type: "text" as const, text }],
          timestamp: Date.now(),
        },
      ],
    })),

  onProviderEvent: (ev) => {
    const state = get();
    switch (ev.type) {
      case "PartStart": {
        if (ev.part.type === "Text") {
          // Start a new streaming text part.
          const partId = ev.part.base.id;
          set({
            streamingPartId: partId,
            streamingText: "",
          });
          set((s) => {
            const msgs = [...s.messages];
            const last = msgs[msgs.length - 1];
            // Determine if we need a new assistant message. We create a
            // fresh one if: there's no last message, the last message
            // isn't assistant, or the last assistant message has no
            // streaming parts (meaning the previous turn is finalized).
            const needsNewMessage =
              !last ||
              last.role !== "assistant" ||
              !last.parts.some((p) => (p.type === "text" || p.type === "reasoning") && p.streaming);

            if (needsNewMessage) {
              msgs.push({
                id: crypto.randomUUID(),
                role: "assistant" as const,
                parts: [{ type: "text" as const, text: "", streaming: true }],
                timestamp: Date.now(),
              });
            } else {
              // Append to existing message (e.g. text after reasoning
              // in the same turn).
              const hasStreaming = last.parts.some(
                (p) => p.type === "text" && p.streaming,
              );
              if (!hasStreaming) {
                last.parts.push({
                  type: "text" as const,
                  text: "",
                  streaming: true,
                });
              }
            }
            return { messages: msgs };
          });
        } else if (ev.part.type === "Reasoning") {
          // Start streaming reasoning text
          const partId = ev.part.base.id;
          set({
            streamingReasoningId: partId,
            streamingReasoningText: ev.part.text || "",
          });
          // Add a reasoning part to the current assistant message,
          // creating a new one if the last message isn't a streaming
          // assistant message (i.e. a new turn started).
          set((s) => {
            const msgs = [...s.messages];
            const last = msgs[msgs.length - 1];
            const isActiveAssistant =
              last &&
              last.role === "assistant" &&
              last.parts.some((p) => (p.type === "text" || p.type === "reasoning") && p.streaming);
            if (isActiveAssistant) {
              last.parts.push({
                type: "reasoning" as const,
                text: "",
                streaming: true,
              });
            } else {
              // New turn — create a fresh assistant message.
              msgs.push({
                id: crypto.randomUUID(),
                role: "assistant" as const,
                parts: [{ type: "reasoning" as const, text: "", streaming: true }],
                timestamp: Date.now(),
              });
            }
            return { messages: msgs };
          });
        } else if (ev.part.type === "ToolCall") {
          // Add a tool call part to the current assistant message,
          // creating a new one if needed (new turn).
          const tc = ev.part;
          set((s) => {
            const msgs = [...s.messages];
            const last = msgs[msgs.length - 1];
            const isActiveAssistant =
              last &&
              last.role === "assistant" &&
              last.parts.some((p) => (p.type === "text" || p.type === "reasoning") && p.streaming);
            if (isActiveAssistant) {
              last.parts.push({
                type: "tool-call",
                toolName: tc.tool_name,
                callId: tc.call_id,
                input: tc.state.input,
                state: "pending",
              });
            } else {
              // New turn — create a fresh assistant message.
              msgs.push({
                id: crypto.randomUUID(),
                role: "assistant" as const,
                parts: [{
                  type: "tool-call",
                  toolName: tc.tool_name,
                  callId: tc.call_id,
                  input: tc.state.input,
                  state: "pending",
                }],
                timestamp: Date.now(),
              });
            }
            const newToolStates = new Map(s.toolStates);
            newToolStates.set(tc.call_id, "pending");
            return { messages: msgs, toolStates: newToolStates };
          });
        }
        break;
      }
      case "PartDelta": {
        if (ev.field === "text" && state.streamingPartId === ev.part_id) {
          set((s) => ({ streamingText: s.streamingText + ev.delta }));
        } else if (
          ev.field === "text" &&
          state.streamingReasoningId === ev.part_id
        ) {
          set((s) => ({
            streamingReasoningText: s.streamingReasoningText + ev.delta,
          }));
        }
        break;
      }
      case "PartEnd": {
        if (state.streamingPartId === ev.part_id) {
          // Finalize the streaming text into the message
          set((s) => {
            const msgs = [...s.messages];
            const last = msgs[msgs.length - 1];
            if (last && last.role === "assistant") {
              const textPart = last.parts.find(
                (p) => p.type === "text" && p.streaming,
              );
              if (textPart && textPart.type === "text") {
                textPart.text = s.streamingText;
                textPart.streaming = false;
              }
            }
            return {
              messages: msgs,
              streamingPartId: null,
              streamingText: "",
            };
          });
        } else if (state.streamingReasoningId === ev.part_id) {
          // Finalize the streaming reasoning into the message
          set((s) => {
            const msgs = [...s.messages];
            const last = msgs[msgs.length - 1];
            if (last && last.role === "assistant") {
              const reasoningPart = last.parts.find(
                (p) => p.type === "reasoning" && p.streaming,
              );
              if (reasoningPart && reasoningPart.type === "reasoning") {
                reasoningPart.text = s.streamingReasoningText;
                reasoningPart.streaming = false;
              }
            }
            return {
              messages: msgs,
              streamingReasoningId: null,
              streamingReasoningText: "",
            };
          });
        }
        break;
      }
      case "MessageEnd": {
        // Accumulate cost
        set((s) => ({
          totalInputTokens: s.totalInputTokens + ev.usage.input,
          totalOutputTokens: s.totalOutputTokens + ev.usage.output,
          totalCost: s.totalCost + ev.cost,
        }));
        break;
      }
      case "RetryWait": {
        // Could show a toast; for now just log
        break;
      }
      case "Error": {
        set((s) => ({
          messages: [
            ...s.messages,
            {
              id: crypto.randomUUID(),
              role: "assistant" as const,
              parts: [{ type: "error", message: ev.error.message }],
              timestamp: Date.now(),
            },
          ],
        }));
        break;
      }
    }
  },

  onToolStart: (callId) =>
    set((s) => {
      const ts = new Map(s.toolStates);
      ts.set(callId, "running");
      return { toolStates: ts };
    }),

  onToolEnd: (callId, success) =>
    set((s) => {
      const ts = new Map(s.toolStates);
      ts.set(callId, success ? "completed" : "error");
      return { toolStates: ts };
    }),

  onToolProgress: (callId, chunk) =>
    set((s) => {
      const outs = new Map(s.toolOutputs);
      outs.set(callId, (outs.get(callId) ?? "") + chunk);
      return { toolOutputs: outs };
    }),

  onPartUpdated: (partId, part) => {
    // Handle tool state transitions: PartUpdated arrives with the updated
    // ToolCall part (state.type: Running/Completed/Error) and ToolResult parts.
    if (part.type === "ToolCall") {
      const callId = part.call_id;
      const newState: ToolDisplayState = matchToolState(part.state.type);
      const output =
        part.state.type === "Completed" || part.state.type === "Running"
          ? part.state.output
          : part.state.type === "Error"
            ? part.state.error
            : undefined;

      set((s) => {
        const msgs = [...s.messages];
        // Find the tool-call part in the last assistant message and update it
        for (let i = msgs.length - 1; i >= 0; i--) {
          const msg = msgs[i];
          if (!msg || msg.role !== "assistant") break;
          const tcPart = msg.parts.find(
            (p) => p.type === "tool-call" && p.callId === callId,
          );
          if (tcPart && tcPart.type === "tool-call") {
            tcPart.state = newState;
            // Update input from the PartUpdated event (the initial PartStart
            // may not have the full input if it arrived before parsing).
            if (part.state.input !== undefined) {
              tcPart.input = part.state.input;
            }
            if (output) tcPart.output = output;
          }
        }
        const ts = new Map(s.toolStates);
        ts.set(callId, newState);
        const outs = new Map(s.toolOutputs);
        if (output) outs.set(callId, output);
        return { messages: msgs, toolStates: ts, toolOutputs: outs };
      });
    } else if (part.type === "ToolResult") {
      // ToolResult part just confirms the tool finished. The output is
      // already in the ToolCallPart's state (Completed/Running has output).
      // Just mark the tool as completed if not already.
      const callId = part.call_id;
      set((s) => {
        const ts = new Map(s.toolStates);
        if (ts.get(callId) !== "error") {
          ts.set(callId, "completed");
        }
        return { toolStates: ts };
      });
    }
    // partId is available for future use (e.g. matching streaming parts)
    void partId;
  },

  onPermissionRequest: (req) =>
    set((s) => ({ pendingPermissions: [...s.pendingPermissions, req] })),

  resolvePermission: (requestId) =>
    set((s) => ({
      pendingPermissions: s.pendingPermissions.filter((p) => p.requestId !== requestId),
    })),

  onError: (message) =>
    set((s) => ({
      messages: [
        ...s.messages,
        {
          id: crypto.randomUUID(),
          role: "assistant" as const,
          parts: [{ type: "error", message }],
          timestamp: Date.now(),
        },
      ],
    })),

  onSlashResult: (text) =>
    set((s) => ({
      messages: [
        ...s.messages,
        {
          id: crypto.randomUUID(),
          role: "assistant" as const,
          parts: [{ type: "text", text }],
          timestamp: Date.now(),
        },
      ],
    })),

  setAvailableModels: (models) => set({ availableModels: models }),

  setCurrentModel: (provider, model) =>
    set({ currentProvider: provider, currentModel: model }),

  setCurrentThinkingVariant: (variant) =>
    set({ currentThinkingVariant: variant }),

  onSessionTitleChanged: (sessionId, title) =>
    set((state) => {
      const sessionTitles = new Map(state.sessionTitles);
      sessionTitles.set(sessionId, title);
      return { sessionTitles };
    }),

  setAvailableSessions: (sessions) => set({ availableSessions: sessions }),

  setSessionsLoading: (loading) => set({ sessionsLoading: loading }),

  onSessionHistory: (messages) =>
    set({
      // Replace the message list with the resumed history. Map wire parts
      // to display parts, filtering out nulls (ToolResult, empty text)
      // and skipping messages that end up with no visible parts.
      messages: messages
        .map((m) => ({
          id: m.id,
          role: m.role as "user" | "assistant",
          parts: m.parts
            .map(wirePartToMessagePart)
            .filter((p): p is MessagePart => p !== null),
          timestamp: m.time.created,
        }))
        .filter((m) => m.parts.length > 0),
      streamingPartId: null,
      streamingText: "",
      streamingReasoningId: null,
      streamingReasoningText: "",
      toolStates: new Map(),
      toolOutputs: new Map(),
      pendingPermissions: [],
    }),

  onSessionCleared: () =>
    set({
      messages: [],
      streamingPartId: null,
      streamingText: "",
      streamingReasoningId: null,
      streamingReasoningText: "",
      toolStates: new Map(),
      toolOutputs: new Map(),
      pendingPermissions: [],
      pendingAskUser: [],
      subagents: new Map(),
      todos: [],
      totalInputTokens: 0,
      totalOutputTokens: 0,
      totalCost: 0,
    }),

  onSubagentStart: (data) =>
    set((s) => {
      const subs = new Map(s.subagents);
      subs.set(data.parent_call_id, {
        parentCallId: data.parent_call_id,
        name: data.name,
        childSessionId: data.child_session_id,
        displayName: data.display_name,
        status: "running",
        lastProgress: null,
        outcome: null,
      });
      return { subagents: subs };
    }),

  onSubagentStatus: (data) =>
    set((s) => {
      const subs = new Map(s.subagents);
      const sub = subs.get(data.parent_call_id);
      if (sub) {
        subs.set(data.parent_call_id, {
          ...sub,
          lastProgress: data.message,
        });
      }
      return { subagents: subs };
    }),

  onSubagentEnd: (data) =>
    set((s) => {
      const subs = new Map(s.subagents);
      const sub = subs.get(data.parent_call_id);
      if (sub) {
        let status: SubagentInfo["status"] = "completed";
        if (data.outcome.type === "Cancelled") status = "cancelled";
        else if (data.outcome.type === "Failed") status = "failed";
        subs.set(data.parent_call_id, {
          ...sub,
          status,
          outcome: data.outcome,
        });
      }
      return { subagents: subs };
    }),

  onAskUserRequest: (data) =>
    set((s) => ({
      pendingAskUser: [
        ...s.pendingAskUser,
        {
          requestId: data.request_id,
          callId: data.call_id,
          questions: data.questions,
        },
      ],
    })),

  resolveAskUser: (requestId) =>
    set((s) => ({
      pendingAskUser: s.pendingAskUser.filter((a) => a.requestId !== requestId),
    })),

  onTodosUpdated: (wireTodos) =>
    set({
      todos: wireTodos.map((t) => ({
        id: t.id,
        content: t.content,
        status: t.status as TodoItem["status"],
        dependsOn: t.depends_on,
      })),
    }),

  reset: () =>
    set({
      messages: [],
      streamingPartId: null,
      streamingText: "",
      streamingReasoningId: null,
      streamingReasoningText: "",
      toolStates: new Map(),
      toolOutputs: new Map(),
      pendingPermissions: [],
      pendingAskUser: [],
      subagents: new Map(),
      todos: [],
      totalInputTokens: 0,
      totalOutputTokens: 0,
      totalCost: 0,
      availableModels: [],
      currentModel: null,
      currentProvider: null,
      currentThinkingVariant: null,
      availableSessions: [],
      sessionsLoading: false,
      sessionTitles: new Map(),
    }),
}));

// ---------------------------------------------------------------------------
// Bridge: wire mew-web-client events into the store
// ---------------------------------------------------------------------------

export function bridgeClientToStore(client: MewClient) {
  const store = useSessionStore;

  client.on("open", () => store.getState().setConnectionState("connected"));
  client.on("close", () => store.getState().setConnectionState("disconnected"));

  client.on("session-ready", (data) => {
    store.getState().setSessionId(data.session_id);
    if (data.model && data.provider) {
      store.getState().setCurrentModel(data.provider, data.model);
    }
  });

  client.on("provider", (ev) => store.getState().onProviderEvent(ev));

  client.on("tool-start", (data) => store.getState().onToolStart(data.call_id));
  client.on("tool-end", (data) => store.getState().onToolEnd(data.call_id, data.success));
  client.on("tool-progress", (data) => store.getState().onToolProgress(data.call_id, data.chunk));

  client.on("part-updated", (data) => store.getState().onPartUpdated(data.part_id, data.part));

  client.on("permission-request", (req, respond) => {
    store.getState().onPermissionRequest({
      requestId: req.request_id,
      toolName: req.tool_name,
      input: req.input,
    });
    permissionResponders.set(req.request_id, respond);
  });

  client.on("workspace-permission-request", (req, respond) => {
    store.getState().onPermissionRequest({
      requestId: req.request_id,
      toolName: "bash (workspace escape)",
      input: { path: req.path },
    });
    permissionResponders.set(req.request_id, respond);
  });

  client.on("slash-result", (data) => store.getState().onSlashResult(data.text));

  client.on("subagent-start", (data) => store.getState().onSubagentStart(data));
  client.on("subagent-status", (data) => store.getState().onSubagentStatus(data));
  client.on("subagent-end", (data) => store.getState().onSubagentEnd(data));

  client.on("ask-user-request", (data) => {
    store.getState().onAskUserRequest(data);
  });

  client.on("todos-updated", (data) => store.getState().onTodosUpdated(data.todos));

  client.on("model-list", (data) => store.getState().setAvailableModels(data.models));
  client.on("model-switched", (data) =>
    store.getState().setCurrentModel(data.provider, data.model),
  );
  client.on("thinking-variant-changed", (data) =>
    store.getState().setCurrentThinkingVariant(data.variant),
  );
  client.on("session-title-changed", (data) =>
    store.getState().onSessionTitleChanged(data.session_id, data.title),
  );

  client.on("session-list", (data) => {
    store.getState().setAvailableSessions(data.sessions);
    store.getState().setSessionsLoading(false);
  });
  client.on("session-history", (data) => store.getState().onSessionHistory(data.messages));
  client.on("session-cleared", () => store.getState().onSessionCleared());
  client.on("request-resolved", (data) => {
    store.getState().resolvePermission(data.request_id);
  });

  client.on("errorMessage", (data) => store.getState().onError(data.message));
  client.on("errorEvent", (data) => store.getState().onError(data.message));
}

/** Side-channel map: request_id → respond callback. The UI reads from this
 * when the user clicks Allow/Deny. */
export const permissionResponders = new Map<number, (decision: PermissionDecision) => void>();

/** Side-channel map: request_id → respond callback for AskUserRequest.
 * The new protocol sends responses via a separate AskUserResponse message,
 * so the UI calls client.respondToAskUser(request_id, answers) directly. */
export const askUserResponders = new Map<number, (answers: string[]) => void>();
