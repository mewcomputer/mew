import { create } from "zustand";
import { navigateToSession } from "../lib/router-ref";
import { getClient } from "../lib/client-ref";
import type {
  MewClient,
  ProviderEventWire,
  Part,
  PermissionDecision,
  ModelInfo,
  MentionCandidate,
  SessionInfo,
  SessionHistoryPage,
  Message,
  TurnSummary,
  Question,
  SubagentQuestionSource,
  SubagentOutcome,
  SubagentMessageReceipt,
  SessionActivitySnapshot,
  TurnManifest,
  Todo as WireTodo,
  GroupInfo,
  DirEntry,
  GitEntry,
  SessionUsageWire,
  AlertKind,
  ProjectInfo,
  PersonaInfo,
  AssistantMeta,
  QueuedMessageWire,
  NestedToolCall,
} from "@mew/web-client";

// ---------------------------------------------------------------------------
// Display types
// ---------------------------------------------------------------------------

export interface ChatMessage {
  id: string;
  role: "user" | "assistant" | "system";
  parts: MessagePart[];
  timestamp: number;
  /** When the provider finished this message. Missing while it is still running. */
  completedAt?: number;
  assistantMeta?: AssistantMeta;
  steered?: boolean;
  cancelled?: boolean;
}

export interface RetryWaitState {
  attempt: number;
  maxAttempts: number;
  delaySecs: number;
  reason: string;
}

export type MessagePart =
  | { type: "text"; text: string; streaming?: boolean }
  | { type: "reasoning"; text: string; streaming?: boolean }
  | {
      type: "tool-call";
      partId?: string;
      toolName: string;
      callId: string;
      input: unknown;
      state: ToolDisplayState;
      output?: string;
      nestedCalls?: NestedToolCall[];
      time?: { start: number; end: number | null };
    }
  | { type: "file"; mime: string; filename?: string; url: string }
  | { type: "compaction"; summary: string; removed?: number }
  | { type: "error"; message: string };

export type ToolDisplayState = "pending" | "running" | "completed" | "error";

/** Map the wire ToolState.status to our display state. */
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

function finalizeOpenAssistantMessages(messages: ChatMessage[], now: number): ChatMessage[] {
  let changed = false;
  const next = messages.map((msg) => {
    if (msg.role !== "assistant") return msg;
    const hasStreamingPart = msg.parts.some(
      (part) => (part.type === "text" || part.type === "reasoning") && part.streaming,
    );
    if (msg.completedAt != null && !hasStreamingPart) return msg;
    changed = true;
    return {
      ...msg,
      completedAt: msg.completedAt ?? now,
      parts: hasStreamingPart
        ? msg.parts.map((part) =>
            (part.type === "text" || part.type === "reasoning") && part.streaming
              ? { ...part, streaming: false }
              : part,
          )
        : msg.parts,
    };
  });
  return changed ? next : messages;
}

function withAppendedError(state: SessionState, message: string) {
  const now = Date.now();
  return {
    messages: [
      ...finalizeOpenAssistantMessages(state.messages, now),
      {
        id: crypto.randomUUID(),
        role: "assistant" as const,
        parts: [{ type: "error" as const, message }],
        timestamp: now,
        completedAt: now,
      },
    ],
    streamingPartId: null,
    streamingText: "",
    streamingReasoningId: null,
    streamingReasoningText: "",
    retryWait: null,
  };
}

/** Convert a wire Part (from session_history) into the store's MessagePart
 *  shape, or null if the part should be skipped (e.g. tool_result parts
 *  are absorbed into the preceding tool_call). Wire parts are discriminated
 *  by snake_case `type`; we map each to the display representation. */
function wirePartToMessagePart(part: Part, role?: Message["role"]): MessagePart | null {
  switch (part.type) {
    case "text":
      if (!part.text || part.text.trim() === "") return null;
      if (role === "user" && part.synthetic && part.text.trimStart().startsWith("Flagged file (")) {
        return null;
      }
      return { type: "text", text: part.text };
    case "reasoning":
      return { type: "reasoning", text: part.text };
    case "tool_call": {
      const state: ToolDisplayState = matchToolState(part.state.status);
      const output =
        part.state.status === "completed" || part.state.status === "running"
          ? part.state.output
          : part.state.status === "error"
            ? part.state.error
            : undefined;
      return {
        type: "tool-call",
        partId: part.id,
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
      return { type: "file", mime: part.mime, filename: part.filename, url: part.url };
    case "compaction":
      return {
        type: "compaction",
        summary: part.summary ?? "",
        ...(part.removed_count != null ? { removed: part.removed_count } : {}),
      };
  }
}

export function wireMessageToChatMessage(message: Message): ChatMessage {
  const nested = message.assistant?.nested_tool_calls ?? [];
  return {
    id: message.id,
    role: message.role,
    parts: message.parts
      .map((part) => wirePartToMessagePart(part, message.role))
      .filter((part): part is MessagePart => part !== null)
      .map((part) => {
      if (part.type !== "tool-call") return part;
      const calls = nested.filter((call) => call.parent_call_id === part.callId);
      return calls.length ? { ...part, nestedCalls: calls } : part;
    }),
    timestamp: message.time.created,
    completedAt: message.time.completed,
    assistantMeta: message.assistant ?? undefined,
  };
}

function updateNestedCalls(
  messages: ChatMessage[],
  parentCallId: string,
  update: (calls: NestedToolCall[]) => NestedToolCall[] | null,
): ChatMessage[] | null {
  for (let messageIndex = messages.length - 1; messageIndex >= 0; messageIndex--) {
    const message = messages[messageIndex]!;
    const partIndex = message.parts.findIndex((part) => part.type === "tool-call" && part.callId === parentCallId);
    if (partIndex < 0) continue;
    const part = message.parts[partIndex]!;
    if (part.type !== "tool-call") return null;
    const calls = update(part.nestedCalls ?? []);
    if (!calls) return null;
    const parts = [...message.parts];
    parts[partIndex] = { ...part, nestedCalls: calls };
    const result = [...messages];
    result[messageIndex] = { ...message, parts };
    return result;
  }
  return null;
}

export interface PendingPermission {
  requestId: string;
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
  manifests: TurnManifest[];
}

export interface SubagentDraft {
  text: string;
  error?: string;
  savedRequestId?: string;
  submission?: {
    parentId: string;
    childId: string;
    requestId: string;
    text: string;
    status: "sending" | "checking" | "unknown";
    checks: number;
  };
}

export const EMPTY_SUBAGENT_DRAFT: SubagentDraft = { text: "" };

export function subagentDraftKey(parentId: string, childId: string): string {
  return JSON.stringify([parentId, childId]);
}

export interface PendingAskUser {
  subagent?: SubagentQuestionSource;
  requestId: string;
  callId: string;
  questions: Question[];
}

export interface PendingGoal {
  requestId: string;
  callId: string;
  objective: string;
}

export interface TodoItem {
  id: number;
  content: string;
  status: "pending" | "in_progress" | "done" | "blocked";
  dependsOn: number[];
}

/** A background job tracked via the job-update wire stream. */
export interface JobInfo {
  jobId: string;
  command: string;
  state: string;
}

export type ConnectionState = "disconnected" | "connecting" | "connected" | "reconnecting";

export function selectSessionRunning(
  state: Pick<SessionState, "confirmedSessionId" | "sessionActivity" | "availableSessions">,
): boolean {
  if (state.confirmedSessionId == null) return false;
  if (state.sessionActivity !== null) return state.sessionActivity === "running";
  return state.availableSessions.some(
    (sess) => sess.session_id === state.confirmedSessionId && sess.state === "running",
  );
}

// ---------------------------------------------------------------------------
// Store
// ---------------------------------------------------------------------------

interface SessionState {
  // Connection
  connectionState: ConnectionState;
  connectionError: string | null;
  connectionRetryToken: number;
  sessionId: string | null;
  confirmedSessionId: string | null;
  sessionActivity: SessionInfo["state"] | null;
  sessionCwd: string | null;

  // Messages
  messages: ChatMessage[];
  sessionIndex: TurnSummary[];
  historyCursor: number | null;
  streamingPartId: string | null;
  streamingText: string;
  streamingReasoningId: string | null;
  streamingReasoningText: string;

  // Tool states
  toolStates: Map<string, ToolDisplayState>;
  toolOutputs: Map<string, string>;

  // Permissions
  pendingPermissions: PendingPermission[];
  queuedMessages: QueuedMessageWire[];
  compacting: boolean;
  compactingStartedAt: number | null;
  retryWait: RetryWaitState | null;

  // Cost tracking (accumulated from MessageEnd events)
  totalInputTokens: number;
  totalOutputTokens: number;
  totalCost: number;

  // Current context occupancy for the active session (snapshot, not
  // cumulative): the latest request's prompt size and the model's window.
  contextTokens: number;
  contextTokensKnown: boolean;
  contextWindow: number;

  // Model management
  availableModels: ModelInfo[];
  mentionSkills: MentionCandidate[];
  mentionSubagents: MentionCandidate[];
  currentModel: string | null;
  currentProvider: string | null;
  currentThinkingVariant: string | null;

  // Persona
  currentPersona: string | null;
  availablePersonas: PersonaInfo[];

  // Permission mode
  permissionMode: string;

  // Client presence
  attachedClients: { id: number; kind: string }[];

  // Session list
  availableSessions: SessionInfo[];
  sessionsLoading: boolean;

  // Session titles (session_id → title)
  sessionTitles: Map<string, string>;

  // Session groups
  groups: GroupInfo[];

  // File tree state
  dirListing: DirEntry[] | null;
  dirResults: Map<string, { entries: DirEntry[] | null; error: string | null }>;
  dirListingError: string | null;
  dirListingPath: string | null;
  filePreview: { path: string; content: string; truncated: boolean; language?: string; error?: string } | null;
  gitStatus: GitEntry[];
  gitStatusError: string | null;
  gitStatusLoaded: boolean;

  // Per-session usage map (session_id → usage)
  sessionUsage: Map<string, SessionUsageWire>;

  // Cross-session alerts
  alerts: { sessionId: string; title: string; kind: AlertKind; detail?: string; timestamp: number }[];

  // Flagged files for the current session
  flaggedFiles: { path: string; reason?: string }[];

  // Subagents
  subagents: Map<string, SubagentInfo>;
  subagentDrafts: Map<string, SubagentDraft>;

  // Ask-user requests
  pendingAskUser: PendingAskUser[];
  pendingGoals: PendingGoal[];


  // Todo list
  todos: TodoItem[];

  // Background jobs (job_id → latest update)
  jobs: Map<string, JobInfo>;

  // Daemon version (from pong reply; null until received)
  daemonVersion: string | null;

  // Projects (recent session cwds from the daemon)
  projects: ProjectInfo[];
  projectsLoading: boolean;

  // Prompt history (most-recent-last)
  promptHistory: string[];

  // Actions
  setConnectionState: (s: ConnectionState) => void;
  setConnectionError: (message: string | null) => void;
  retryConnection: () => void;
  setSessionId: (id: string | null) => void;

  addUserMessage: (text: string, steered?: boolean) => void;

  // Event handlers (called by the mew-web-client bridge)
  onProviderEvent: (ev: ProviderEventWire) => void;
  onToolStart: (callId: string) => void;
  onToolEnd: (callId: string, success: boolean) => void;
  onToolProgress: (callId: string, chunk: string) => void;
  onNestedToolStart: (call: Pick<NestedToolCall, "parent_call_id" | "call_id" | "tool_name" | "input">) => void;
  onNestedToolEnd: (call: { parent_call_id: string; call_id: string; success: boolean; error?: string; output_bytes?: number }) => void;
  onPartUpdated: (partId: string, part: Part) => void;
  onPermissionRequest: (req: PendingPermission) => void;
  resolvePermission: (requestId: string) => void;
  onError: (message: string) => void;
  onSlashResult: (text: string) => void;

  setAvailableModels: (models: ModelInfo[]) => void;
  setMentionCandidates: (skills: MentionCandidate[], subagents: MentionCandidate[]) => void;
  setCurrentModel: (provider: string, model: string) => void;
  setCurrentThinkingVariant: (variant: string | null) => void;
  setCurrentPersona: (name: string | null) => void;
  selectPersona: (name: string) => void;
  setAvailablePersonas: (personas: PersonaInfo[]) => void;
  setPermissionMode: (mode: string) => void;
  onClientAttached: (clientId: number, clientKind: string) => void;
  onClientDetached: (clientId: number) => void;
  onSessionTitleChanged: (sessionId: string, title: string) => void;
  sessionSummaries: Map<string, string>;
  onSessionSummaryChanged: (sessionId: string, summary: string) => void;

  // Phase 1-3 + misc new actions
  onSessionActivityChanged: (sessionId: string, activity: string, cancelled?: boolean) => void;
  onSessionCompacting: (sessionId: string, active: boolean, removed?: number | null, summary?: string | null) => void;
  setQueuedMessages: (sessionId: string, items: QueuedMessageWire[]) => void;
  onSessionStatsChanged: (sessionId: string, added: number, removed: number, filesChanged: number) => void;
  setGroups: (groups: GroupInfo[]) => void;
  onGroupsChanged: (groups: GroupInfo[]) => void;
  onDirListing: (path: string, entries: DirEntry[]) => void;
  onDirError: (path: string, error: string) => void;
  onFilePreview: (path: string, content: string, truncated: boolean, language?: string, error?: string) => void;
  onGitStatus: (entries: GitEntry[]) => void;
  onFsChanged: (paths: string[]) => void;
  onSessionUsageChanged: (sessionId: string, usage: SessionUsageWire, contextTokens?: number | null) => void;
  onSessionAlert: (sessionId: string, title: string, kind: AlertKind, detail?: string) => void;
  clearAlertsForSession: (sessionId: string) => void;
  dismissAlert: (sessionId: string, timestamp: number) => void;
  onFlaggedFilesChanged: (files: { path: string; reason?: string }[]) => void;
  onSessionMetaChanged: (sessionId: string, archived: boolean, pinned: boolean, groupId?: string) => void;
  onSessionAttentionChanged: (sessionId: string, pendingPermissions: number, pendingQuestions: number) => void;

  // Shared-session actions
  setAvailableSessions: (sessions: SessionInfo[]) => void;
  setSessionsLoading: (loading: boolean) => void;
  onSessionHistory: (messages: Message[], page?: SessionHistoryPage) => void;
  onSessionIndex: (sessionId: string, turns: TurnSummary[]) => void;
  onSessionCleared: () => void;

  // Subagent actions
  setSubagentDraft: (key: string, draft: SubagentDraft) => void;
  onSubagentStart: (data: { parent_call_id: string; name: string; child_session_id: string; display_name: string | null }) => void;
  onSubagentStatus: (data: { parent_call_id: string; tool_name: string; message: string }) => void;
  onSubagentEnd: (data: { parent_call_id: string; child_session_id: string; outcome: SubagentOutcome; manifests?: TurnManifest[] }) => void;

  // Ask-user actions
  onAskUserRequest: (data: { request_id: string; call_id: string; questions: Question[]; subagent?: SubagentQuestionSource }) => void;
  resolveAskUser: (requestId: string) => void;
  onGoalProposed: (data: { request_id: string; call_id: string; objective: string }) => void;
  resolveGoal: (requestId: string) => void;

  // Todo actions
  onTodosUpdated: (todos: WireTodo[]) => void;

  // Job actions
  onJobUpdate: (data: { jobId: string; command: string; state: string }) => void;
  onSessionActivitySnapshot: (data: SessionActivitySnapshot) => void;

  // Daemon version
  setDaemonVersion: (version: string) => void;

  // Projects
  onProjectList: (projects: ProjectInfo[]) => void;
  setProjectsLoading: (loading: boolean) => void;

  // Prompt history
  pushPromptHistory: (text: string) => void;

  reset: () => void;
  clearSession: () => void;
}

function transientSessionState() {
  return {
    sessionIndex: [],
    filePreview: null,
    dirListing: null,
    dirResults: new Map(),
    dirListingError: null,
    dirListingPath: null,
    gitStatus: [],
    gitStatusError: null,
    gitStatusLoaded: false,
    pendingPermissions: [],
    queuedMessages: [],
    compacting: false,
    compactingStartedAt: null,
    retryWait: null,
    pendingAskUser: [],
    pendingGoals: [],
  };
}

function resetSessionState() {
  return {
    ...transientSessionState(),
    messages: [],
    historyCursor: null,
    streamingPartId: null,
    streamingText: "",
    streamingReasoningId: null,
    streamingReasoningText: "",
    toolStates: new Map(),
    toolOutputs: new Map(),
    subagents: new Map(),
    todos: [],
    jobs: new Map(),
    totalInputTokens: 0,
    totalOutputTokens: 0,
    totalCost: 0,
    contextTokens: 0,
    contextTokensKnown: true,
    contextWindow: 0,
    sessionCwd: null,
    sessionActivity: null,
    flaggedFiles: [],
    attachedClients: [],
  };
}

let workspaceRefresh: { sessionId: string; timer: ReturnType<typeof setTimeout> } | undefined;

export const useSessionStore = create<SessionState>((set, get) => ({
  connectionState: "disconnected",
  connectionError: null,
  connectionRetryToken: 0,
  sessionId: null,
  confirmedSessionId: null,
  sessionActivity: null,
  sessionCwd: null,
  messages: [],
  sessionIndex: [],
  historyCursor: null,
  streamingPartId: null,
  streamingText: "",
  streamingReasoningId: null,
  streamingReasoningText: "",
  toolStates: new Map(),
  toolOutputs: new Map(),
  pendingPermissions: [],
  queuedMessages: [],
  compacting: false,
  compactingStartedAt: null,
  retryWait: null,
  totalInputTokens: 0,
  totalOutputTokens: 0,
  totalCost: 0,
  contextTokens: 0,
  contextTokensKnown: true,
  contextWindow: 0,
  availableModels: [],
  mentionSkills: [],
  mentionSubagents: [],
  currentModel: null,
  currentProvider: null,
  currentThinkingVariant: null,
  currentPersona: null,
  availablePersonas: [],
  permissionMode: "standard",
  attachedClients: [],
  availableSessions: [],
  sessionsLoading: false,
  sessionTitles: new Map(),
  sessionSummaries: new Map(),
  groups: [],
  dirListing: null,
  dirResults: new Map(),
  dirListingError: null,
  dirListingPath: null,
  filePreview: null,
  gitStatus: [],
  gitStatusError: null,
  gitStatusLoaded: false,
  sessionUsage: new Map(),
  alerts: [],
  flaggedFiles: [],
  subagents: new Map(),
  subagentDrafts: new Map(),
  pendingAskUser: [],
  pendingGoals: [],
  todos: [],
  jobs: new Map(),
  daemonVersion: null,
  projects: [],
  projectsLoading: false,
  promptHistory: [],

  setConnectionState: (s) => set({ connectionState: s }),
  setConnectionError: (message) => set({ connectionError: message }),
  retryConnection: () =>
    set((state) => ({
      connectionState: "connecting",
      connectionError: null,
      connectionRetryToken: state.connectionRetryToken + 1,
    })),
  setSessionId: (id) => set({ sessionId: id }),

  addUserMessage: (text, steered) =>
    set((state) => ({
      messages: [
        ...state.messages,
        {
          id: crypto.randomUUID(),
          role: "user" as const,
          parts: [{ type: "text" as const, text }],
          timestamp: Date.now(),
          ...(steered ? { steered: true } : {}),
        },
      ],
    })),

  onProviderEvent: (ev) => {
    const state = get();
    switch (ev.type) {
      case "part_start": {
        if (ev.part.type === "text") {
          // Start a new streaming text part.
          const partId = ev.part.id;
          set({
            streamingPartId: partId,
            streamingText: "",
            retryWait: null,
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
                msgs[msgs.length - 1] = {
                  ...last,
                  parts: [
                    ...last.parts,
                    {
                      type: "text" as const,
                      text: "",
                      streaming: true,
                    },
                  ],
                };
              }
            }
            return { messages: msgs };
          });
        } else if (ev.part.type === "reasoning") {
          // Start streaming reasoning text
          const partId = ev.part.id;
          set({
            streamingReasoningId: partId,
            streamingReasoningText: ev.part.text || "",
            retryWait: null,
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
              msgs[msgs.length - 1] = {
                ...last,
                parts: [
                  ...last.parts,
                  {
                    type: "reasoning" as const,
                    text: "",
                    streaming: true,
                  },
                ],
              };
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
              msgs[msgs.length - 1] = {
                ...last,
                parts: [
                  ...last.parts,
                  {
                    type: "tool-call",
                    partId: tc.id,
                    toolName: tc.tool_name,
                    callId: tc.call_id,
                    input: tc.state.input,
                    state: "pending",
                  },
                ],
              };
            } else {
              // New turn — create a fresh assistant message.
              msgs.push({
                id: crypto.randomUUID(),
                role: "assistant" as const,
                parts: [{
                  type: "tool-call",
                  partId: tc.id,
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
            return { messages: msgs, toolStates: newToolStates, retryWait: null };
          });
        }
        break;
      }
      case "part_delta": {
        if (ev.field === "text" && state.streamingPartId === ev.part_id) {
          set((s) => ({ streamingText: s.streamingText + ev.delta, retryWait: null }));
        } else if (
          ev.field === "text" &&
          state.streamingReasoningId === ev.part_id
        ) {
          set((s) => ({
            streamingReasoningText: s.streamingReasoningText + ev.delta,
            retryWait: null,
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
        set((s) => {
          const totalInputTokens = s.totalInputTokens + ev.usage.input;
          const totalOutputTokens = s.totalOutputTokens + ev.usage.output;
          const totalCost = s.totalCost + ev.cost;

          // Current context occupancy: this request's prompt size, plus the
          // window the manifest reports (keep the last known window if this
          // manifest lacks one).
          const contextTokens = ev.usage.input > 0 ? ev.usage.input : s.contextTokens;
          const contextWindow = ev.manifest?.context_window
            ? ev.manifest.context_window
            : s.contextWindow;

          // Find the last assistant message (search backwards, not by
          // position — an error or tool-result message may have been
          // appended after the assistant message).
          const lastAssistantIdx = [...s.messages]
            .reverse()
            .findIndex((m) => m.role === "assistant");
          if (lastAssistantIdx === -1) {
            return {
              totalInputTokens,
              totalOutputTokens,
              totalCost,
              contextTokens,
              contextTokensKnown: ev.usage.input > 0 || s.contextTokensKnown,
              contextWindow,
              retryWait: null,
            };
          }
          const idx = s.messages.length - 1 - lastAssistantIdx;

          return {
            totalInputTokens,
            totalOutputTokens,
            totalCost,
            contextTokens,
            contextTokensKnown: ev.usage.input > 0 || s.contextTokensKnown,
            contextWindow,
            retryWait: null,
            messages: s.messages.map((m, i) =>
              i === idx
                ? {
                    ...m,
                    completedAt: Date.now(),
                    assistantMeta: {
                      provider_id: "",
                      model_id: ev.manifest?.model ?? "",
                      cost: ev.cost,
                      tokens: ev.usage,
                      finish: ev.finish,
                      manifest: ev.manifest ?? undefined,
                    },
                  }
                : m,
            ),
          };
        });
        const { sessionId, confirmedSessionId } = get();
        if (sessionId && confirmedSessionId === sessionId) {
          const client = getClient();
          if (client?.isSessionAttached(sessionId)) {
            client.loadSessionIndex(sessionId);
          }
        }
        break;
      }
      case "retry_wait": {
        set({
          retryWait: {
            attempt: ev.attempt,
            maxAttempts: ev.max_attempts,
            delaySecs: ev.delay_secs,
            reason: ev.reason,
          },
        });
        break;
      }
      case "error": {
        set((s) => withAppendedError(s, ev.error.message));
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

  onNestedToolStart: (call) => set((state) => {
    const messages = updateNestedCalls(state.messages, call.parent_call_id, (calls) =>
      calls.some((child) => child.call_id === call.call_id) ? null : [...calls, { ...call, status: "running" }],
    );
    return messages ? { messages } : state;
  }),

  onNestedToolEnd: (call) => set((state) => {
    const messages = updateNestedCalls(state.messages, call.parent_call_id, (calls) => {
      const index = calls.findIndex((child) => child.call_id === call.call_id);
      if (index < 0) return null;
      const next = [...calls];
      next[index] = { ...calls[index]!, status: call.success ? "completed" : "error", error: call.error, output_bytes: call.output_bytes };
      return next;
    });
    return messages ? { messages } : state;
  }),

  onPartUpdated: (partId, part) => {
    // Handle tool state transitions: part_updated arrives with the updated
    // tool_call part (state.status: running/completed/error) and tool_result parts.
    if (part.type === "tool_call") {
      const callId = part.call_id;
      const newState: ToolDisplayState = matchToolState(part.state.status);
      const output =
        part.state.status === "completed" || part.state.status === "running"
          ? part.state.output
          : part.state.status === "error"
            ? part.state.error
            : undefined;

      set((s) => {
        const msgs = [...s.messages];
        let previousCallId: string | undefined;
        for (let i = msgs.length - 1; i >= 0; i--) {
          const msg = msgs[i];
          if (!msg || msg.role !== "assistant") continue;
          const index = msg.parts.findIndex((p) => p.type === "tool-call" &&
            (p.partId === partId || (callId !== "" && p.callId === callId)));
          if (index < 0) continue;
          const existing = msg.parts[index]!;
          if (existing.type !== "tool-call") continue;
          previousCallId = existing.callId;
          const parts = [...msg.parts];
          parts[index] = { ...existing, partId, callId, toolName: part.tool_name, state: newState,
            input: part.state.input, time: part.state.time, ...(output !== undefined ? { output } : {}) };
          msgs[i] = { ...msg, parts };
          break;
        }
        const ts = new Map(s.toolStates);
        if (previousCallId !== undefined && previousCallId !== callId) ts.delete(previousCallId);
        ts.set(callId, newState);
        const outs = new Map(s.toolOutputs);
        if (previousCallId !== undefined && previousCallId !== callId) outs.delete(previousCallId);
        if (output !== undefined) outs.set(callId, output);
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
  },

  onPermissionRequest: (req) =>
    set((s) => ({
      pendingPermissions: [...s.pendingPermissions.filter((p) => p.requestId !== req.requestId), req],
    })),

  resolvePermission: (requestId) =>
    set((s) => ({
      pendingPermissions: s.pendingPermissions.filter((p) => p.requestId !== requestId),
    })),

  onError: (message) => set((s) => withAppendedError(s, message)),

  onSlashResult: (text) =>
    set((s) => {
      const last = s.messages[s.messages.length - 1];
      if (
        /^compacted \d+ messages\b/i.test(text.trim()) &&
        last?.parts.some((part) => part.type === "compaction")
      ) {
        return {};
      }
      const now = Date.now();
      return {
        messages: [
          ...s.messages,
          {
            id: crypto.randomUUID(),
            role: "assistant" as const,
            parts: [{ type: "text", text }],
            timestamp: now,
            completedAt: now,
          },
        ],
      };
    }),

  setAvailableModels: (models) => set({ availableModels: models }),
  setMentionCandidates: (skills, subagents) => set({ mentionSkills: skills, mentionSubagents: subagents }),

  setCurrentModel: (provider, model) =>
    set({ currentProvider: provider, currentModel: model }),

  setCurrentThinkingVariant: (variant) =>
    set({ currentThinkingVariant: variant }),

  setCurrentPersona: (name) => set({ currentPersona: name }),
  selectPersona: (name) => {
    const target = name === "default" ? null : name;
    set({ currentPersona: target });
    if (target) {
      getClient()?.switchPersona(target);
    }
  },
  setAvailablePersonas: (personas) => set({ availablePersonas: personas }),

  setPermissionMode: (mode) => set({ permissionMode: mode }),

  onClientAttached: (clientId, clientKind) =>
    set((s) => ({
      attachedClients: [...s.attachedClients, { id: clientId, kind: clientKind }],
    })),

  onClientDetached: (clientId) =>
    set((s) => ({
      attachedClients: s.attachedClients.filter((c) => c.id !== clientId),
    })),

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
  onSessionActivityChanged: (sessionId, activity, cancelled) =>
    set((state) => {
      const availableSessions = state.availableSessions.map((s) =>
        s.session_id === sessionId
          ? { ...s, state: activity as SessionInfo["state"] }
          : s,
      );
      const isCurrent =
        sessionId === state.sessionId || sessionId === state.confirmedSessionId;
      const nextActivity = isCurrent
        ? (activity as SessionInfo["state"])
        : state.sessionActivity;
      if (sessionId === state.sessionId && activity === "idle") {
        const now = Date.now();
        let messages = finalizeOpenAssistantMessages(state.messages, now);
        if (cancelled) {
          let turnStart = 0;
          for (let i = messages.length - 1; i >= 0; i--) {
            const msg = messages[i]!;
            if (msg.role === "user" && msg.parts.some((part) => part.type === "text" && part.text.trim())) {
              turnStart = i + 1;
              break;
            }
          }
          let marked = false;
          for (let i = messages.length - 1; i >= turnStart; i--) {
            if (messages[i]!.role === "assistant") {
              messages = messages.map((msg, idx) => (idx === i ? { ...msg, cancelled: true } : msg));
              marked = true;
              break;
            }
          }
          if (!marked) {
            messages = [
              ...messages,
              {
                id: crypto.randomUUID(),
                role: "assistant" as const,
                parts: [],
                timestamp: now,
                completedAt: now,
                cancelled: true,
              },
            ];
          }
        }
        return {
          availableSessions,
          sessionActivity: nextActivity,
          messages,
          streamingPartId: null,
          streamingText: "",
          streamingReasoningId: null,
          streamingReasoningText: "",
          compacting: false,
          compactingStartedAt: null,
          retryWait: null,
        };
      }
      return isCurrent
        ? { availableSessions, sessionActivity: nextActivity }
        : { availableSessions };
    }),

  onSessionCompacting: (sessionId, active, removed, summary) =>
    set((state) => {
      if (sessionId !== state.sessionId) return state;
      if (active || removed == null || removed <= 0) {
        return {
          compacting: active,
          compactingStartedAt: active ? (state.compactingStartedAt ?? Date.now()) : null,
        };
      }
      const now = Date.now();
      const compactionMsg: ChatMessage = {
        id: crypto.randomUUID(),
        role: "system",
        parts: [{ type: "compaction", summary: summary ?? "", removed }],
        timestamp: now,
        completedAt: now,
      };
      // If /compact's slash-result arrived first, replace that text bubble with the divider.
      const last = state.messages[state.messages.length - 1];
      const lastPart = last?.parts[0];
      const isSlashCompactText =
        last?.role === "assistant" &&
        last.parts.length === 1 &&
        lastPart?.type === "text" &&
        /^compacted \d+ messages\b/i.test(lastPart.text.trim());
      const messages = isSlashCompactText
        ? [...state.messages.slice(0, -1), compactionMsg]
        : [...state.messages, compactionMsg];
      return { compacting: false, compactingStartedAt: null, messages };
    }),

  setQueuedMessages: (sessionId, items) =>
    set((state) => (state.sessionId === sessionId ? { queuedMessages: items } : state)),

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
    set((state) => ({ dirListing: entries, dirListingPath: path, dirListingError: null, dirResults: new Map(state.dirResults).set(path, { entries, error: null }) })),

  onDirError: (path, error) => set((state) => ({ dirListing: null, dirListingPath: path, dirListingError: error, dirResults: new Map(state.dirResults).set(path, { entries: null, error }) })),

  onFilePreview: (path, content, truncated, language, error) =>
    set({ filePreview: { path, content, truncated, language, error } }),

  onGitStatus: (entries) => set((state) => ({
    gitStatus: entries.length === state.gitStatus.length && entries.every((entry, index) => entry.path === state.gitStatus[index]?.path && entry.status === state.gitStatus[index]?.status) ? state.gitStatus : entries,
    gitStatusError: null,
    gitStatusLoaded: true,
  })),

  // When the filesystem changes (watchWorkspace), re-fetch the current dir
  // listing and git status so the Files/Changes tabs stay live. The client
  // is reached through the module-level ref set in App.tsx.
  onFsChanged: () => {
    const sid = get().sessionId;
    if (!sid || workspaceRefresh?.sessionId === sid) return;
    if (workspaceRefresh) clearTimeout(workspaceRefresh.timer);
    const timer = setTimeout(() => {
      workspaceRefresh = undefined;
      const client = getClient();
      if (!client || get().sessionId !== sid || get().connectionState !== "connected") return;
      try {
        client.listDir(sid, get().dirListingPath ?? undefined);
        client.gitStatus(sid);
      } catch {
        // The connection can close between checking it and sending.
      }
    }, 150);
    workspaceRefresh = { sessionId: sid, timer };
  },

  onSessionUsageChanged: (sessionId, usage, contextTokens) =>
    set((state) => {
      const sessionUsage = new Map(state.sessionUsage);
      sessionUsage.set(sessionId, usage);
      // Also update availableSessions with the usage and the current
      // context reading.
      return {
        sessionUsage,
        ...(state.sessionId === sessionId && contextTokens !== undefined ? {
          contextTokensKnown: contextTokens !== null,
          ...(contextTokens !== null ? { contextTokens } : {}),
        } : {}),
        availableSessions: state.availableSessions.map((s) =>
          s.session_id === sessionId
            ? { ...s, usage, context_tokens: contextTokens === null ? undefined : contextTokens ?? s.context_tokens }
            : s,
        ),
      };
    }),

  onSessionAlert: (sessionId, title, kind, detail) =>
    set((state) => {
      // Turn completion is useful for OS delivery but is not actionable
      // attention. A successful follow-up also resolves an earlier failure.
      const alerts = state.alerts.filter(
        (alert) =>
          alert.sessionId !== sessionId ||
          (alert.kind !== "turn_failed" && kind !== "turn_complete"),
      );
      if (kind === "turn_complete") {
        syncTitleBadge(alerts);
        return { alerts };
      }
      alerts.push({ sessionId, title, kind, detail, timestamp: Date.now() });
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
          ? { ...s, archived, pinned, group_id: groupId }
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

  setAvailableSessions: (sessions) => set((state) => {
    const preservedSessions =
      state.confirmedSessionId != null && state.sessionActivity !== null
        ? sessions.map((session) =>
            session.session_id === state.confirmedSessionId
              ? { ...session, state: state.sessionActivity! }
              : session,
          )
        : sessions;
    const active = preservedSessions.find((session) => session.session_id === state.confirmedSessionId);
    // Titles otherwise only arrive as change broadcasts. Seeding them from the
    // list keeps a reloaded client showing the persisted title.
    const sessionTitles = new Map(state.sessionTitles);
    for (const session of preservedSessions) {
      if (session.title) sessionTitles.set(session.session_id, session.title);
    }
    return {
      availableSessions: preservedSessions,
      sessionTitles,
      ...(active && active.state !== "running" ? {
        contextTokensKnown: active.context_tokens !== undefined,
        contextTokens: active.context_tokens ?? 0,
      } : {}),
    };
  }),

  setSessionsLoading: (loading) => set({ sessionsLoading: loading }),

  onSessionHistory: (messages, page) =>
    set((state) => {
      if (page && page.session_id !== state.sessionId) return state;
      const history = (page?.messages ?? messages)
        .map(wireMessageToChatMessage)
        .filter((m) => m.parts.length > 0 || m.assistantMeta);
      const historyCursor = page?.next_cursor ?? null;
      if (page && !page.replace) {
        const existing = new Set(state.messages.map(({ id }) => id));
        return { historyCursor, messages: [...history.filter(({ id }) => !existing.has(id)), ...state.messages] };
      }
      return {
        messages: history,
        historyCursor,
        streamingPartId: null,
        streamingText: "",
        streamingReasoningId: null,
        streamingReasoningText: "",
        toolStates: new Map(),
        toolOutputs: new Map(),
      };
    }),

  onSessionIndex: (sessionId, turns) =>
    set((state) => (state.sessionId === sessionId ? { sessionIndex: turns } : state)),

  onSessionCleared: () =>
    set({
      messages: [],
      sessionIndex: [],
      historyCursor: null,
      streamingPartId: null,
      streamingText: "",
      streamingReasoningId: null,
      streamingReasoningText: "",
      toolStates: new Map(),
      toolOutputs: new Map(),
      pendingPermissions: [],
      queuedMessages: [],
      compacting: false,
      compactingStartedAt: null,
      retryWait: null,
      pendingAskUser: [],
      pendingGoals: [],
      subagents: new Map(),
      todos: [],
      jobs: new Map(),
      totalInputTokens: 0,
      totalOutputTokens: 0,
      totalCost: 0,
    }),

  setSubagentDraft: (key, draft) =>
    set((state) => ({ subagentDrafts: new Map(state.subagentDrafts).set(key, draft) })),

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
        manifests: [],
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
      const sub = s.subagents.get(data.parent_call_id);
      if (!sub || sub.childSessionId !== data.child_session_id) return s;
      const subs = new Map(s.subagents);
      subs.set(data.parent_call_id, {
        ...sub,
        status: data.outcome.type,
        outcome: data.outcome,
        manifests: data.manifests ?? [],
      });
      return { subagents: subs };
    }),

  onAskUserRequest: (data) =>
    set((s) => ({
      pendingAskUser: [
        ...s.pendingAskUser.filter((a) => a.requestId !== data.request_id),
        {
          requestId: data.request_id,
          callId: data.call_id,
          questions: data.questions,
          subagent: data.subagent,
        },
      ],
    })),

  resolveAskUser: (requestId) =>
    set((s) => ({
      pendingAskUser: s.pendingAskUser.filter((a) => a.requestId !== requestId),
    })),

  onGoalProposed: (data) =>
    set((s) => ({
      pendingGoals: [
        ...s.pendingGoals.filter((goal) => goal.requestId !== data.request_id),
        { requestId: data.request_id, callId: data.call_id, objective: data.objective },
      ],
    })),

  resolveGoal: (requestId) =>
    set((s) => ({ pendingGoals: s.pendingGoals.filter((goal) => goal.requestId !== requestId) })),

  onTodosUpdated: (wireTodos) =>
    set({
      todos: wireTodos.map((t) => ({
        id: t.id,
        content: t.content,
        status: t.status as TodoItem["status"],
        dependsOn: t.depends_on,
      })),
    }),

  onJobUpdate: (data) =>
    set((state) => {
      const jobs = new Map(state.jobs);
      jobs.set(data.jobId, {
        jobId: data.jobId,
        command: data.command,
        state: data.state,
      });
      // Drop terminal jobs after 5s so the panel stays tidy.
      if (data.state === "done" || data.state === "failed" || data.state === "cancelled") {
        setTimeout(() => {
          useSessionStore.setState((s) => {
            const next = new Map(s.jobs);
            next.delete(data.jobId);
            return { jobs: next };
          });
        }, 5000);
      }
      return { jobs };
    }),

  onSessionActivitySnapshot: (data) =>
    set((state) => {
      if (data.session_id !== state.sessionId) return {};
      const subagents = new Map<string, SubagentInfo>();
      for (const task of data.subagents) {
        const previous = state.subagents.get(task.task_id);
        const sameChild = previous?.childSessionId === task.child_session_id;
        subagents.set(task.task_id, {
          parentCallId: task.task_id,
          childSessionId: task.child_session_id ?? "",
          name: task.name,
          displayName: task.display_name,
          status: "running",
          lastProgress: task.status && task.status !== "running"
            ? task.status
            : task.tool_name ?? (sameChild ? previous.lastProgress : null),
          outcome: sameChild ? previous.outcome : null,
          manifests: sameChild ? previous.manifests : [],
        });
      }
      const jobs = new Map(data.jobs.map((job) => [job.job_id, {
        jobId: job.job_id, command: job.command, state: job.state,
      }]));
      return { subagents, jobs };
    }),

  setDaemonVersion: (version) => set({ daemonVersion: version }),

  onProjectList: (projects) => set({ projects, projectsLoading: false }),
  setProjectsLoading: (loading) => set({ projectsLoading: loading }),

  pushPromptHistory: (text) =>
    set((state) => {
      // Skip empty or exact-duplicates of the last entry.
      if (!text.trim()) return {};
      const hist = state.promptHistory;
      if (hist.length > 0 && hist[hist.length - 1] === text) return {};
      return { promptHistory: [...hist, text] };
    }),

  reset: () => set(resetSessionState()),
  clearSession: () =>
    set({
      ...resetSessionState(),
      sessionId: null,
      confirmedSessionId: null,
    }),
}));

// ---------------------------------------------------------------------------
// Bridge: wire mew-web-client events into the store
// ---------------------------------------------------------------------------

const UNCONFIRMED_GUIDANCE = "Could not confirm guidance. Check the agent's history before sending again.";
const MAX_GUIDANCE_RECEIPT_CHECKS = 2;

function applySubagentReceipt(receipt: SubagentMessageReceipt) {
  const key = subagentDraftKey(receipt.session_id, receipt.child_session_id);
  const store = useSessionStore.getState();
  const draft = store.subagentDrafts.get(key);
  const submission = draft?.submission;
  if (!draft || submission?.requestId !== receipt.request_id) return;
  if (receipt.status === "accepted") {
    const text = draft.text === submission.text ? "" : draft.text;
    store.setSubagentDraft(key, { text, savedRequestId: receipt.request_id });
  } else if (receipt.status === "rejected") {
    store.setSubagentDraft(key, { text: draft.text, error: receipt.message ?? "The agent could not receive guidance." });
  } else {
    store.setSubagentDraft(key, { ...draft, submission: { ...submission, status: "unknown" }, error: receipt.message ?? UNCONFIRMED_GUIDANCE });
  }
}

function checkSubagentReceipt(client: MewClient, key: string) {
  const store = useSessionStore.getState();
  const draft = store.subagentDrafts.get(key);
  const submission = draft?.submission;
  if (!draft || submission?.status !== "unknown" || submission.checks >= MAX_GUIDANCE_RECEIPT_CHECKS || !client.isSessionAttached(submission.parentId)) return;
  const { parentId, childId, requestId } = submission;
  store.setSubagentDraft(key, { ...draft, submission: { ...submission, status: "checking", checks: submission.checks + 1 } });
  void client.readSubagentMessageReceipt(parentId, childId, requestId).then(applySubagentReceipt, () => {
    applySubagentReceipt({ type: "subagent_message_receipt", session_id: parentId, child_session_id: childId, request_id: requestId, status: "unknown" });
  });
}

export async function sendSubagentGuidance(client: MewClient, parentId: string, childId: string, parentCallId: string) {
  const key = subagentDraftKey(parentId, childId);
  const store = useSessionStore.getState();
  const agent = store.subagents.get(parentCallId);
  const draft = store.subagentDrafts.get(key) ?? EMPTY_SUBAGENT_DRAFT;
  const text = draft.text.trim();
  const submission = draft.submission;
  if (!text || store.connectionState !== "connected" || store.confirmedSessionId !== parentId
    || !client.isSessionAttached(parentId) || agent?.childSessionId !== childId || agent.status !== "running"
    || submission?.status === "sending" || submission?.status === "checking") return;
  const retry = submission?.status === "unknown" && submission.text.trim() === text;
  const requestId = retry ? submission.requestId : crypto.randomUUID();
  store.setSubagentDraft(key, {
    text: draft.text,
    submission: { parentId, childId, requestId, text: draft.text, status: "sending", checks: retry ? submission.checks : 0 },
  });
  try {
    const receipt = await client.guideSubagent(parentId, childId, text, requestId);
    applySubagentReceipt(receipt);
    if (receipt.status === "unknown") checkSubagentReceipt(client, key);
  } catch (error) {
    applySubagentReceipt({
      type: "subagent_message_receipt", session_id: parentId, child_session_id: childId, request_id: requestId,
      status: retry ? "unknown" : "rejected",
      message: error instanceof Error ? error.message : "Could not send guidance.",
    });
  }
}

export function bridgeClientToStore(client: MewClient) {
  const store = useSessionStore;

  client.on("close", () =>
    store.setState({
      connectionState: "disconnected",
      confirmedSessionId: null,
      sessionActivity: null,
    }),
  );
  client.on("subagent-message-receipt", (receipt) => {
    if (receipt.status !== "unknown") applySubagentReceipt(receipt);
  });

  client.on("session-ready", (data) => {
    const state = store.getState();
    const switching = state.sessionId !== data.session_id;
    const active = state.availableSessions.find((session) => session.session_id === data.session_id);
    useSessionStore.setState({
      ...(switching ? resetSessionState() : transientSessionState()),
      sessionId: data.session_id,
      confirmedSessionId: data.session_id,
      sessionCwd: data.cwd ?? null,
      contextTokensKnown: active?.context_tokens !== undefined,
      contextTokens: active?.context_tokens ?? 0,
      ...(data.model && data.provider
        ? { currentProvider: data.provider, currentModel: data.model }
        : {}),
      ...(data.permission_mode ? { permissionMode: data.permission_mode } : {}),
    });
    // The whole-session tape should be available immediately on attach, before
    // the reader has scrolled back through every history page.
    client.loadSessionIndex(data.session_id);
    for (const [key, draft] of store.getState().subagentDrafts) {
      if (draft.submission?.parentId === data.session_id) checkSubagentReceipt(client, key);
    }
    permissionResponders.clear();
    askUserResponders.clear();
    // Model and persona discovery is session-scoped. Waiting until the
    // daemon confirms a session prevents startup errors from being rendered
    // as chat messages on the home screen.
    void client.listModels();
    void client.listPersonas();
    void client.listMentionCandidates?.();
  });

  client.on("provider", (ev) => store.getState().onProviderEvent(ev));

  client.on("user-message", ({ text, steered }) => store.getState().addUserMessage(text, steered));

  client.on("tool-start", (data) => store.getState().onToolStart(data.call_id));
  client.on("tool-end", (data) => store.getState().onToolEnd(data.call_id, data.success));
  client.on("tool-progress", (data) => store.getState().onToolProgress(data.call_id, data.chunk));
  client.on("nested-tool-start", (data) => store.getState().onNestedToolStart(data));
  client.on("nested-tool-end", (data) => store.getState().onNestedToolEnd(data));

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
  client.on("session-activity-snapshot", (data) => store.getState().onSessionActivitySnapshot(data));
  client.on("subagent-status", (data) => store.getState().onSubagentStatus(data));
  client.on("subagent-end", (data) => store.getState().onSubagentEnd(data));

  client.on("ask-user-request", (data) => {
    store.getState().onAskUserRequest(data);
  });
  client.on("goal-proposed", (data) => store.getState().onGoalProposed(data));

  client.on("todos-updated", (data) => store.getState().onTodosUpdated(data.todos));

  client.on("job-update", (data) =>
    store.getState().onJobUpdate({
      jobId: data.job_id,
      command: data.command,
      state: data.state,
    }),
  );

  client.on("pong", (data) => store.getState().setDaemonVersion(data.version));

  client.on("project-list", (data) => store.getState().onProjectList(data.projects));

  client.on("model-list", (data) => store.getState().setAvailableModels(data.models));
  client.on("mention-candidates", (data) => store.getState().setMentionCandidates(data.skills, data.subagents));
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
  client.on("persona-list", (data) =>
    store.getState().setAvailablePersonas(data.personas),
  );
  client.on("persona-switched", (data) =>
    store.getState().setCurrentPersona(data.name),
  );
  client.on("session-title-changed", (data) =>
    store.getState().onSessionTitleChanged(data.session_id, data.title),
  );

  client.on("session-summary-changed", (data) =>
    store.getState().onSessionSummaryChanged(data.session_id, data.summary),
  );

  client.on("session-activity-changed", (data) =>
    store.getState().onSessionActivityChanged(data.session_id, data.activity, data.cancelled),
  );

  client.on("session-compacting", (data) =>
    store.getState().onSessionCompacting(data.session_id, data.active, data.removed, data.summary),
  );

  client.on("session-queue-changed", (data) =>
    store.getState().setQueuedMessages(data.session_id, data.items),
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
  client.on("dir-listing", (data) => {
    if (!data.session_id) {
      store.getState().onDirError("", "Please update the daemon to view files safely.");
      return;
    }
    if (data.session_id !== store.getState().sessionId) return;
    if (data.error) store.getState().onDirError(data.path, data.error);
    else store.getState().onDirListing(data.path, data.entries);
  });
  client.on("file-preview", (data) => {
    if (!data.session_id) {
      store.getState().onFilePreview(data.path, "", false, undefined, "Please update the daemon to view file previews safely.");
      return;
    }
    if (data.session_id !== store.getState().sessionId) return;
    store.getState().onFilePreview(data.path, data.content, data.truncated, data.language, data.error);
  });
  client.on("git-status-result", (data) => {
    if (!data.session_id) {
      store.setState({ gitStatus: [], gitStatusError: "Please update the daemon to view changes safely.", gitStatusLoaded: true });
      return;
    }
    if (data.session_id !== store.getState().sessionId) return;
    if (data.error) {
      store.setState({ gitStatus: [], gitStatusError: data.error, gitStatusLoaded: true });
      return;
    }
    store.getState().onGitStatus(data.entries);
  });
  client.on("fs-changed", (data) => {
    if (data.session_id && data.session_id !== store.getState().sessionId) return;
    store.getState().onFsChanged(data.paths);
  });

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
    store.getState().onSessionUsageChanged(data.session_id, data.usage, data.context_tokens),
  );

  client.on("session-alert", (data) => {
    const currentSessionId = store.getState().sessionId;
    store.getState().onSessionAlert(
      data.session_id,
      data.title,
      data.kind,
      data.detail,
    );

    // Keep the in-app alert for every session. Only background sessions need
    // an OS notification because the active session is already visible.
    if (data.session_id !== currentSessionId && typeof Notification !== "undefined") {
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
  client.on("session-history-page", (page) => store.getState().onSessionHistory(page.messages, page));
  client.on("session-index", (data) => store.getState().onSessionIndex(data.session_id, data.turns));
  client.on("session-cleared", () => store.getState().onSessionCleared());
  client.on("request-resolved", (data) => {
    store.getState().resolvePermission(data.request_id);
    store.getState().resolveAskUser(data.request_id);
    store.getState().resolveGoal(data.request_id);
    permissionResponders.delete(data.request_id);
    askUserResponders.delete(data.request_id);
  });

  const handleDaemonError = (data: { message: string }) => {
    if (data.message === "no session" && !store.getState().sessionId) return;
    store.getState().onError(data.message);
  };
  client.on("errorMessage", handleDaemonError);
  client.on("errorEvent", handleDaemonError);
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
export const permissionResponders = new Map<string, (decision: PermissionDecision) => void>();

/** Side-channel map: request_id → respond callback for AskUserRequest.
 * The new protocol sends responses via a separate AskUserResponse message,
 * so the UI calls client.respondToAskUser(request_id, answers) directly. */
export const askUserResponders = new Map<string, (answers: string[]) => void>();