import { create } from "zustand";
import { navigateToSession } from "../lib/router-ref";
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
  GroupInfo,
  DirEntry,
  GitEntry,
  SessionUsageWire,
  AlertKind,
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
  | {
      type: "tool-call";
      toolName: string;
      callId: string;
      input: unknown;
      state: ToolDisplayState;
      output?: string;
      time?: { start: number; end: number | null };
    }
  | { type: "error"; message: string };

export type ToolDisplayState = "pending" | "running" | "completed" | "error";

/** Map the wire ToolState.type to our display state. */
function matchToolState(type: string): ToolDisplayState {
  switch (type) {
    case "pending":
      return "pending";
    case "running":
      return "running";
    case "completed":
      return "completed";
    case "error":
      return "error";
    default:
      return "pending";
  }
}

/** Convert a wire Part (from session_history) into the store's MessagePart
 *  shape, or null if the part should be skipped (e.g. tool_result parts
 *  are absorbed into the preceding tool_call). Wire parts are discriminated
 *  by snake_case `type`; we map each to the display representation. */
function wirePartToMessagePart(part: Part): MessagePart | null {
  switch (part.type) {
    case "text":
      if (!part.text || part.text.trim() === "") return null;
      return { type: "text", text: part.text };
    case "reasoning":
      return { type: "reasoning", text: part.text };
    case "tool_call": {
      const state: ToolDisplayState = matchToolState(part.state.type);
      const output =
        part.state.type === "completed" || part.state.type === "running"
          ? part.state.output
          : part.state.type === "error"
            ? part.state.error
            : undefined;
      return {
        type: "tool-call",
        toolName: part.tool_name,
        callId: part.call_id,
        input: part.state.input,
        state,
        output,
        time: part.state.time,
      };
    }
    case "tool_result":
      // tool_result parts are absorbed into the preceding tool_call's
      // state; the tool_call part already carries the output. Skip.
      return null;
    case "file":
      return { type: "text", text: `[file: ${part.url}]` };
    case "compaction":
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
  // Input tokens from the last message_end — approximates current context fill
  lastInputTokens: number;

  // Model management
  availableModels: ModelInfo[];
  currentModel: string | null;
  currentProvider: string | null;
  currentThinkingVariant: string | null;

  // Persona
  currentPersona: string | null;

  // Permission mode
  permissionMode: string;

  // Client presence
  attachedClients: { id: number; kind: string }[];

  // Control yielded
  yieldedByClient: number | null;

  // Session list
  availableSessions: SessionInfo[];
  sessionsLoading: boolean;

  // Session titles (session_id → title)
  sessionTitles: Map<string, string>;

  // Session groups
  groups: GroupInfo[];

  // File tree state
  dirListing: DirEntry[] | null;
  dirListingPath: string | null;
  filePreview: { path: string; content: string; truncated: boolean; language?: string } | null;
  gitStatus: GitEntry[];

  // Per-session usage map (session_id → usage)
  sessionUsage: Map<string, SessionUsageWire>;

  // Cross-session alerts
  alerts: { sessionId: string; title: string; kind: AlertKind; detail?: string; timestamp: number }[];

  // Flagged files for the current session
  flaggedFiles: { path: string; reason?: string }[];

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
  setCurrentPersona: (name: string | null) => void;
  setPermissionMode: (mode: string) => void;
  onClientAttached: (clientId: number, clientKind: string) => void;
  onClientDetached: (clientId: number) => void;
  onControlYielded: (clientId: number) => void;
  clearYieldedControl: () => void;
  onSessionTitleChanged: (sessionId: string, title: string) => void;
  sessionSummaries: Map<string, string>;
  onSessionSummaryChanged: (sessionId: string, summary: string) => void;

  // Phase 1-3 + misc new actions
  onSessionActivityChanged: (sessionId: string, activity: string) => void;
  onSessionStatsChanged: (sessionId: string, added: number, removed: number, filesChanged: number) => void;
  setGroups: (groups: GroupInfo[]) => void;
  onGroupsChanged: (groups: GroupInfo[]) => void;
  onDirListing: (path: string, entries: DirEntry[]) => void;
  onFilePreview: (path: string, content: string, truncated: boolean, language?: string) => void;
  onGitStatus: (entries: GitEntry[]) => void;
  onFsChanged: (paths: string[]) => void;
  onSessionUsageChanged: (sessionId: string, usage: SessionUsageWire) => void;
  onSessionAlert: (sessionId: string, title: string, kind: AlertKind, detail?: string) => void;
  clearAlertsForSession: (sessionId: string) => void;
  dismissAlert: (sessionId: string, timestamp: number) => void;
  onFlaggedFilesChanged: (files: { path: string; reason?: string }[]) => void;
  onSessionMetaChanged: (sessionId: string, archived: boolean | null, pinned: boolean | null, groupId?: string) => void;
  onSessionAttentionChanged: (sessionId: string, pendingPermissions: number, pendingQuestions: number) => void;

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
  lastInputTokens: 0,
  availableModels: [],
  currentModel: null,
  currentProvider: null,
  currentThinkingVariant: null,
  currentPersona: null,
  permissionMode: "standard",
  attachedClients: [],
  yieldedByClient: null,
  availableSessions: [],
  sessionsLoading: false,
  sessionTitles: new Map(),
  sessionSummaries: new Map(),
  groups: [],
  dirListing: null,
  dirListingPath: null,
  filePreview: null,
  gitStatus: [],
  sessionUsage: new Map(),
  alerts: [],
  flaggedFiles: [],
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
      case "part_start": {
        if (ev.part.type === "text") {
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
        } else if (ev.part.type === "reasoning") {
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
        } else if (ev.part.type === "tool_call") {
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
      case "part_delta": {
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
      case "part_end": {
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
      case "message_end": {
        // Accumulate cost
        set((s) => ({
          totalInputTokens: s.totalInputTokens + ev.usage.input,
          totalOutputTokens: s.totalOutputTokens + ev.usage.output,
          totalCost: s.totalCost + ev.cost,
          lastInputTokens: ev.usage.input,
        }));
        break;
      }
      case "retry_wait": {
        // Could show a toast; for now just log
        break;
      }
      case "error": {
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
    // Handle tool state transitions: part_updated arrives with the updated
    // tool_call part (state.type: running/completed/error) and tool_result parts.
    if (part.type === "tool_call") {
      const callId = part.call_id;
      const newState: ToolDisplayState = matchToolState(part.state.type);
      const output =
        part.state.type === "completed" || part.state.type === "running"
          ? part.state.output
          : part.state.type === "error"
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
            // Update input from the part_updated event (the initial part_start
            // may not have the full input if it arrived before parsing).
            if (part.state.input !== undefined) {
              tcPart.input = part.state.input;
            }
            if (part.state.time) {
              tcPart.time = part.state.time;
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
    } else if (part.type === "tool_result") {
      // tool_result part just confirms the tool finished. The output is
      // already in the tool_call part's state (completed/running has output).
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

  setCurrentPersona: (name) => set({ currentPersona: name }),

  setPermissionMode: (mode) => set({ permissionMode: mode }),

  onClientAttached: (clientId, clientKind) =>
    set((s) => ({
      attachedClients: [...s.attachedClients, { id: clientId, kind: clientKind }],
    })),

  onClientDetached: (clientId) =>
    set((s) => ({
      attachedClients: s.attachedClients.filter((c) => c.id !== clientId),
    })),

  onControlYielded: (clientId) => set({ yieldedByClient: clientId }),

  clearYieldedControl: () => set({ yieldedByClient: null }),

  onSessionTitleChanged: (sessionId, title) =>
    set((state) => {
      const sessionTitles = new Map(state.sessionTitles);
      sessionTitles.set(sessionId, title);
      return { sessionTitles };
    }),

  onSessionSummaryChanged: (sessionId, summary) =>
    set((state) => {
      const sessionSummaries = new Map(state.sessionSummaries);
      sessionSummaries.set(sessionId, summary);
      return { sessionSummaries };
    }),

  // Phase 1-3 + misc new actions
  onSessionActivityChanged: (sessionId, activity) =>
    set((state) => ({
      availableSessions: state.availableSessions.map((s) =>
        s.session_id === sessionId
          ? { ...s, state: activity as SessionInfo["state"] }
          : s,
      ),
    })),

  onSessionStatsChanged: (sessionId, added, removed, filesChanged) =>
    set((state) => ({
      availableSessions: state.availableSessions.map((s) =>
        s.session_id === sessionId
          ? {
              ...s,
              change_stats: {
                added,
                removed,
                files: Array.from({ length: filesChanged }, (_, i) => `file_${i}`),
              },
            }
          : s,
      ),
    })),

  setGroups: (groups) => set({ groups }),

  onGroupsChanged: (groups) => set({ groups }),

  onDirListing: (path, entries) =>
    set({ dirListing: entries, dirListingPath: path }),

  onFilePreview: (path, content, truncated, language) =>
    set({ filePreview: { path, content, truncated, language } }),

  onGitStatus: (entries) => set({ gitStatus: entries }),

  onFsChanged: () => {},

  onSessionUsageChanged: (sessionId, usage) =>
    set((state) => {
      const sessionUsage = new Map(state.sessionUsage);
      sessionUsage.set(sessionId, usage);
      // Also update availableSessions with the usage.
      return {
        sessionUsage,
        availableSessions: state.availableSessions.map((s) =>
          s.session_id === sessionId ? { ...s, usage } : s,
        ),
      };
    }),

  onSessionAlert: (sessionId, title, kind, detail) =>
    set((state) => {
      const alerts = [
        ...state.alerts,
        { sessionId, title, kind, detail, timestamp: Date.now() },
      ];
      syncTitleBadge(alerts);
      return { alerts };
    }),

  clearAlertsForSession: (sessionId) =>
    set((state) => {
      const alerts = state.alerts.filter((a) => a.sessionId !== sessionId);
      syncTitleBadge(alerts);
      return { alerts };
    }),

  dismissAlert: (sessionId, timestamp) =>
    set((state) => {
      const alerts = state.alerts.filter(
        (a) => !(a.sessionId === sessionId && a.timestamp === timestamp),
      );
      syncTitleBadge(alerts);
      return { alerts };
    }),

  onFlaggedFilesChanged: (files) => set({ flaggedFiles: files }),

  onSessionMetaChanged: (sessionId, archived, pinned, groupId) =>
    set((state) => ({
      availableSessions: state.availableSessions.map((s) =>
        s.session_id === sessionId
          ? {
              ...s,
              archived: archived ?? s.archived,
              pinned: pinned ?? s.pinned,
              group_id: groupId ?? s.group_id,
            }
          : s,
      ),
    })),

  onSessionAttentionChanged: (sessionId, pendingPermissions, pendingQuestions) =>
    set((state) => ({
      availableSessions: state.availableSessions.map((s) =>
        s.session_id === sessionId
          ? { ...s, pending_permissions: pendingPermissions, pending_questions: pendingQuestions }
          : s,
      ),
    })),

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
      lastInputTokens: 0,
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
        if (data.outcome.type === "cancelled") status = "cancelled";
        else if (data.outcome.type === "failed") status = "failed";
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
      lastInputTokens: 0,
      // Clear per-session state that shouldn't leak across sessions.
      flaggedFiles: [],
      dirListing: null,
      dirListingPath: null,
      filePreview: null,
      gitStatus: [],
      // Keep global caches and per-session model info; they are repopulated
      // by wire events (model-list, session-list, session-ready).
      // currentModel is intentionally preserved to avoid model picker/footer blanks.
      attachedClients: [],
      yieldedByClient: null,
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
    if (data.permission_mode) {
      store.getState().setPermissionMode(data.permission_mode);
    }
  });

  client.on("provider", (ev) => store.getState().onProviderEvent(ev));

  client.on("user-message", (data) => {
    const store = useSessionStore.getState();
    // Deduplicate: the sending client already added the message locally.
    const last = store.messages[store.messages.length - 1];
    if (last && last.role === "user" && last.parts.some((p) => p.type === "text" && p.text === data.text)) {
      return;
    }
    store.addUserMessage(data.text);
  });

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
  client.on("permission-mode-changed", (data) =>
    store.getState().setPermissionMode(data.mode),
  );
  client.on("client-attached", (data) =>
    store.getState().onClientAttached(data.client_id, data.client_kind),
  );
  client.on("client-detached", (data) =>
    store.getState().onClientDetached(data.client_id),
  );
  client.on("control-yielded", (data) =>
    store.getState().onControlYielded(data.client_id),
  );
  client.on("persona-switch-requested", (data) =>
    store.getState().setCurrentPersona(data.name),
  );
  client.on("session-title-changed", (data) =>
    store.getState().onSessionTitleChanged(data.session_id, data.title),
  );

  client.on("session-summary-changed", (data) =>
    store.getState().onSessionSummaryChanged(data.session_id, data.summary),
  );

  client.on("session-activity-changed", (data) =>
    store.getState().onSessionActivityChanged(data.session_id, data.activity),
  );

  client.on("session-stats-changed", (data) =>
    store.getState().onSessionStatsChanged(
      data.session_id,
      data.added,
      data.removed,
      data.files_changed,
    ),
  );

  client.on("group-list", (data) => store.getState().setGroups(data.groups));
  client.on("groups-changed", (data) => store.getState().onGroupsChanged(data.groups));
  client.on("dir-listing", (data) => store.getState().onDirListing(data.path, data.entries));
  client.on("file-preview", (data) =>
    store.getState().onFilePreview(data.path, data.content, data.truncated, data.language),
  );
  client.on("git-status-result", (data) => store.getState().onGitStatus(data.entries));
  client.on("fs-changed", (data) => store.getState().onFsChanged(data.paths));

  client.on("flagged-files-changed", (data) => {
    if (data.session_id !== store.getState().sessionId) return;
    store.getState().onFlaggedFilesChanged(data.files);
  });

  client.on("session-meta-changed", (data) =>
    store.getState().onSessionMetaChanged(
      data.session_id,
      data.archived,
      data.pinned,
      data.group_id,
    ),
  );

  client.on("session-attention-changed", (data) =>
    store.getState().onSessionAttentionChanged(
      data.session_id,
      data.pending_permissions,
      data.pending_questions,
    ),
  );

  client.on("session-usage-changed", (data) =>
    store.getState().onSessionUsageChanged(data.session_id, data.usage),
  );

  client.on("session-alert", (data) => {
    // Suppress alerts for the currently viewed session.
    const currentSessionId = store.getState().sessionId;
    if (data.session_id === currentSessionId) return;

    store.getState().onSessionAlert(
      data.session_id,
      data.title,
      data.kind,
      data.detail,
    );

    // OS notification delivery.
    if (typeof Notification !== "undefined") {
      if (Notification.permission === "default") {
        // Request permission lazily (Safari needs a user gesture, but
        // Chrome/Firefox honor this from background events).
        Notification.requestPermission().then((perm) => {
          if (perm === "granted") {
            showNotification(data.session_id, data.title, data.kind, data.detail);
          }
        });
      } else if (Notification.permission === "granted") {
        showNotification(data.session_id, data.title, data.kind, data.detail);
      }
    }
  });

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

/** Sync document.title with the alert count. Called after any alerts mutation. */
function syncTitleBadge(alerts: { sessionId: string }[]) {
  if (typeof document === "undefined") return;
  if (alerts.length > 0) {
    document.title = `(${alerts.length}) mew`;
  } else {
    document.title = "mew";
  }
}

/** Show an OS notification for a session alert. Uses router-ref for navigation. */
function showNotification(
  sessionId: string,
  title: string,
  kind: string,
  detail?: string,
) {
  const kindLabel = kind.replace(/_/g, " ");
  const n = new Notification(`${title}: ${kindLabel}`, {
    body: detail ?? "",
  });
  n.onclick = () => {
    window.focus();
    navigateToSession(sessionId);
  };
}

/** Side-channel map: request_id → respond callback. The UI reads from this
 * when the user clicks Allow/Deny. */
export const permissionResponders = new Map<number, (decision: PermissionDecision) => void>();

/** Side-channel map: request_id → respond callback for AskUserRequest.
 * The new protocol sends responses via a separate AskUserResponse message,
 * so the UI calls client.respondToAskUser(request_id, answers) directly. */
export const askUserResponders = new Map<number, (answers: string[]) => void>();
