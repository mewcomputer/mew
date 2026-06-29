//! TypeScript client for the mew daemon wire protocol.
//!
//! Speaks `ClientMessage` / `ServerMessage` over WebSocket, mirroring the
//! Rust types in `crates/mew-protocol/src/lib.rs`. Browser-native — uses
//! the standard `WebSocket` API. For Node, pass any WebSocket-compatible
//! implementation via the `socketFactory` option.
//!
//! Example:
//! ```ts
//! import { MewClient } from "@mew/web-client";
//!
//! const client = new MewClient("ws://localhost:9847/");
//! await client.connect();
//! const sessionId = await client.newSession();
//! client.on("provider", (ev) => { /* render streaming text */ });
//! client.on("permission-request", (req, respond) => {
//!   if (confirm(`Allow ${req.tool_name}?`)) respond("allow_once");
//!   else respond("deny");
//! });
//! await client.prompt("Summarize the last commit");
//! ```

// ---------------------------------------------------------------------------
// Wire types — mirror crates/mew-protocol/src/lib.rs exactly. Keep these in
// sync if the Rust types change.
// ---------------------------------------------------------------------------

export interface Attachment {
  path: string;
  mime?: string;
}

export type PermissionDecision = "allow_once" | "allow_session" | "deny";

export type ClientMessage =
  | { type: "NewSession"; cwd: string | null }
  | { type: "AttachSession"; session_id: string }
  | { type: "ListSessions" }
  | { type: "Prompt"; text: string; attachments: Attachment[] }
  | { type: "Cancel" }
  | {
      type: "PermissionResponse";
      request_id: number;
      decision: PermissionDecision;
    }
  | { type: "AskUserResponse"; request_id: number; answers: string[] }
  | { type: "SlashCommand"; command: string }
  | { type: "ListModels" }
  | { type: "SwitchModel"; provider: string; model: string }
  | { type: "SetThinkingVariant"; variant: string };

// Provider events — see mew_message::ProviderEventWire.
export type ProviderEventWire =
  | { type: "PartStart"; part: Part }
  | { type: "PartDelta"; part_id: string; field: string; delta: string }
  | { type: "PartEnd"; part_id: string }
  | {
      type: "MessageEnd";
      finish: "stop" | "tool_use" | "length" | "content_filter" | "error";
      usage: { input: number; output: number };
      cost: number;
    }
  | {
      type: "RetryWait";
      attempt: number;
      max_attempts: number;
      delay_secs: number;
      reason: string;
    }
  | { type: "Error"; error: MessageError };

export type Part =
  | { type: "Text"; base: PartBase; text: string; synthetic: boolean }
  | { type: "Reasoning"; base: PartBase; text: string; signature?: string }
  | {
      type: "ToolCall";
      base: PartBase;
      tool_name: string;
      call_id: string;
      state: ToolState;
    }
  | { type: "ToolResult"; base: PartBase; call_id: string; output?: string }
  | {
      type: "File";
      base: PartBase;
      mime: string;
      filename?: string;
      url: string;
    }
  | { type: "Compaction"; base: PartBase; auto: boolean; overflow: boolean };

export interface PartBase {
  id: string;
  message_id: string;
  session_id: string;
}

export type ToolState =
  | {
      type: "Pending";
      input: unknown;
      time: { start: number; end: number | null };
    }
  | {
      type: "Running";
      input: unknown;
      output: string;
      time: { start: number; end: number | null };
    }
  | {
      type: "Completed";
      input: unknown;
      output: string;
      time: { start: number; end: number | null };
    }
  | {
      type: "Error";
      input: unknown;
      error: string;
      time: { start: number; end: number | null };
    };

export type ErrorKind =
  | "provider_auth"
  | "rate_limit"
  | "invalid_request"
  | "tool_exec"
  | "tool_timeout"
  | "mcp_transport"
  | "network"
  | "unknown";

export interface MessageError {
  kind: ErrorKind;
  message: string;
}

export interface QuestionOption {
  label: string;
  description: string;
}

export interface Question {
  prompt: string;
  options: QuestionOption[];
}

export interface Todo {
  id: number;
  content: string;
  status: string;
  depends_on: number[];
}

export type SubagentOutcome =
  | { type: "Completed" }
  | { type: "Cancelled" }
  | { type: "Failed"; reason: string };

// ---------------------------------------------------------------------------
// Session & model management types
// ---------------------------------------------------------------------------

/** Info about a single available model, returned by `ListModels`. */
export interface ModelInfo {
  /** Fully-qualified ID: "provider/model" (e.g. "deepseek/deepseek-v4-flash"). */
  id: string;
  /** Provider ID (e.g. "deepseek"). */
  provider: string;
  /** Model ID within the provider (e.g. "deepseek-v4-flash"). */
  model: string;
  /** Human-readable description for the picker UI. */
  description?: string;
  /** Available thinking/reasoning variants for this model. */
  thinking_variants?: ThinkingVariantInfo[];
}

/** A named thinking/reasoning variant (e.g. "high", "max", "thinking"). */
export interface ThinkingVariantInfo {
  name: string;
}

/** Session lifecycle state. */
export type SessionState = "active" | "idle";

/** Metadata returned by `ListSessions` for one session. */
export interface SessionInfo {
  session_id: string;
  state: SessionState;
  model?: string;
  provider?: string;
  created_at: number;
  last_message_at?: number;
  client_count: number;
}

/** A message role. */
export type Role = "user" | "assistant";

/** Timestamp metadata for a message. */
export interface Time {
  created: number;
  completed?: number;
}

/** A complete message, as returned in `SessionHistory`. */
export interface Message {
  id: string;
  session_id: string;
  role: Role;
  parts: Part[];
  time: Time;
  assistant?: AssistantMeta | null;
}

/** Provider/model metadata for an assistant message. */
export interface AssistantMeta {
  provider_id: string;
  model_id: string;
  cost: number;
  tokens: Tokens;
  finish?: string | null;
  error?: MessageError | null;
}

/** Token usage breakdown. */
export interface Tokens {
  input: number;
  output: number;
  reasoning: number;
  cache_read: number;
  cache_write: number;
}

export type ServerMessage =
  | { type: "SessionReady"; session_id: string; model?: string; provider?: string }
  | { type: "Error"; message: string }
  | { type: "Provider"; event: ProviderEventWire }
  | { type: "ToolStart"; call_id: string }
  | { type: "ToolEnd"; call_id: string; success: boolean }
  | { type: "PartUpdated"; part_id: string; part: Part }
  | { type: "ToolProgress"; call_id: string; chunk: string }
  | { type: "ErrorEvent"; message: string }
  | {
      type: "PermissionRequest";
      request_id: number;
      tool_name: string;
      input: Record<string, unknown>;
    }
  | { type: "WorkspacePermissionRequest"; request_id: number; path: string }
  | {
      type: "AskUserRequest";
      request_id: number;
      call_id: string;
      questions: Question[];
    }
  | {
      type: "SubagentStart";
      parent_call_id: string;
      name: string;
      child_session_id: string;
      display_name: string | null;
    }
  | {
      type: "SubagentStatus";
      parent_call_id: string;
      tool_name: string;
      message: string;
    }
  | {
      type: "SubagentEnd";
      parent_call_id: string;
      child_session_id: string;
      outcome: SubagentOutcome;
    }
  | {
      type: "SubagentPermissionRequest";
      request_id: number;
      parent_call_id: string;
      tool_name: string;
      input: Record<string, unknown>;
    }
  | { type: "TodosUpdated"; todos: Todo[] }
  | { type: "PersonaSwitchRequested"; name: string }
  | { type: "JobUpdate"; job_id: string; command: string; state: string }
  | { type: "SlashResult"; text: string }
  | { type: "RequestResolved"; request_id: number }
  | { type: "SessionCleared" }
  | { type: "SessionList"; sessions: SessionInfo[] }
  | { type: "SessionHistory"; messages: Message[] }
  | { type: "ModelList"; models: ModelInfo[] }
  | { type: "ModelSwitched"; provider: string; model: string }
  | { type: "ThinkingVariantChanged"; variant?: string }
  | { type: "SessionTitleChanged"; session_id: string; title: string };

// ---------------------------------------------------------------------------
// Minimal WebSocket interface — lets Node users pass `ws` while browsers
// pass the native WebSocket.
// ---------------------------------------------------------------------------

export interface MewWebSocket {
  send(data: string): void;
  close(code?: number, reason?: string): void;
  addEventListener(type: "open", listener: () => void): void;
  addEventListener(
    type: "close",
    listener: (ev: { code: number; reason: string }) => void,
  ): void;
  addEventListener(type: "error", listener: (ev: unknown) => void): void;
  addEventListener(
    type: "message",
    listener: (ev: { data: string }) => void,
  ): void;
  removeEventListener(
    type: string,
    listener: (...args: unknown[]) => void,
  ): void;
}

export type SocketFactory = (url: string) => MewWebSocket;

const defaultSocketFactory: SocketFactory = (url) => {
  if (typeof WebSocket === "undefined") {
    throw new Error(
      "No WebSocket implementation available. In Node, pass `socketFactory` using the `ws` package.",
    );
  }
  return new WebSocket(url) as unknown as MewWebSocket;
};

// ---------------------------------------------------------------------------
// Event handlers
// ---------------------------------------------------------------------------

export interface MewClientEvents {
  open: () => void;
  close: (code: number, reason: string) => void;
  error: (err: unknown) => void;

  "session-ready": (data: {
    session_id: string;
    model?: string;
    provider?: string;
  }) => void;
  provider: (ev: ProviderEventWire) => void;
  "tool-start": (data: { call_id: string }) => void;
  "tool-end": (data: { call_id: string; success: boolean }) => void;
  "part-updated": (data: { part_id: string; part: Part }) => void;
  "tool-progress": (data: { call_id: string; chunk: string }) => void;

  "permission-request": (
    data: {
      request_id: number;
      tool_name: string;
      input: Record<string, unknown>;
    },
    respond: (decision: PermissionDecision) => void,
  ) => void;
  "workspace-permission-request": (
    data: { request_id: number; path: string },
    respond: (decision: PermissionDecision) => void,
  ) => void;
  "ask-user-request": (data: {
    request_id: number;
    call_id: string;
    questions: Question[];
  }) => void;

  "subagent-start": (data: {
    parent_call_id: string;
    name: string;
    child_session_id: string;
    display_name: string | null;
  }) => void;
  "subagent-status": (data: {
    parent_call_id: string;
    tool_name: string;
    message: string;
  }) => void;
  "subagent-end": (data: {
    parent_call_id: string;
    child_session_id: string;
    outcome: SubagentOutcome;
  }) => void;
  "todos-updated": (data: { todos: Todo[] }) => void;
  "persona-switch-requested": (data: { name: string }) => void;
  "job-update": (data: {
    job_id: string;
    command: string;
    state: string;
  }) => void;
  "slash-result": (data: { text: string }) => void;
  "request-resolved": (data: { request_id: number }) => void;
  "session-cleared": () => void;
  "session-list": (data: { sessions: SessionInfo[] }) => void;
  "session-history": (data: { messages: Message[] }) => void;
  "model-list": (data: { models: ModelInfo[] }) => void;
  "model-switched": (data: { provider: string; model: string }) => void;
  "thinking-variant-changed": (data: { variant: string | null }) => void;
  "session-title-changed": (data: { session_id: string; title: string }) => void;

  errorMessage: (data: { message: string }) => void;
  errorEvent: (data: { message: string }) => void;
}

export type MewEventName = keyof MewClientEvents;

// ---------------------------------------------------------------------------
// MewClient
// ---------------------------------------------------------------------------

export interface MewClientOptions {
  /** Override how the WebSocket is constructed (e.g. inject `ws` in Node). */
  socketFactory?: SocketFactory;
  /** If true, log every wire message to the console. Useful for debugging. */
  debug?: boolean;
}

/**
 * Client for the mew daemon wire protocol. One client == one connection ==
 * one session. To run multiple sessions concurrently, create multiple
 * `MewClient` instances.
 */
export class MewClient {
  private readonly url: string;
  private readonly socketFactory: SocketFactory;
  private readonly debug: boolean;
  private ws: MewWebSocket | null = null;
  private listeners = new Map<
    MewEventName,
    Set<(...args: unknown[]) => void>
  >();
  /** Promise resolved when the WebSocket opens. */
  private openPromise: Promise<void> | null = null;
  /** Session id returned by `newSession`. */
  private sessionId: string | null = null;

  constructor(url: string, opts: MewClientOptions = {}) {
    this.url = url;
    this.socketFactory = opts.socketFactory ?? defaultSocketFactory;
    this.debug = opts.debug ?? false;
  }

  /** Open the WebSocket connection. Idempotent. */
  connect(): Promise<void> {
    if (this.openPromise) return this.openPromise;
    this.openPromise = new Promise<void>((resolve, reject) => {
      let settled = false;
      const ws = this.socketFactory(this.url);
      this.ws = ws;
      ws.addEventListener("open", () => {
        if (this.debug) console.debug("[mew] open");
        settled = true;
        this.emit("open");
        resolve();
      });
      ws.addEventListener("message", (ev) => {
        try {
          const msg = JSON.parse(ev.data) as ServerMessage;
          if (this.debug) console.debug("[mew] <-", msg);
          this.dispatch(msg);
        } catch (e) {
          this.emit("error", e);
        }
      });
      ws.addEventListener("close", (ev) => {
        if (this.debug) console.debug("[mew] close", ev);
        if (!settled) {
          settled = true;
          reject(new Error(`ws closed before open: ${ev.code} ${ev.reason}`));
        }
        this.emit("close", ev.code, ev.reason);
      });
      ws.addEventListener("error", (ev) => {
        if (this.debug) console.debug("[mew] error", ev);
        if (!settled) {
          settled = true;
          reject(ev);
        }
        this.emit("error", ev);
      });
    });
    return this.openPromise;
  }

  /** Close the WebSocket. After calling this, the client cannot be reused. */
  disconnect(code = 1000, reason = "client disconnect") {
    this.ws?.close(code, reason);
    this.ws = null;
    this.openPromise = null;
  }

  isConnected(): boolean {
    return this.ws !== null;
  }

  /** Send `NewSession`. Resolves with the daemon-assigned session id. */
  async newSession(cwd: string | null = null): Promise<string> {
    await this.connect();
    return new Promise<string>((resolve, reject) => {
      const onReady = (data: { session_id: string }) => {
        this.sessionId = data.session_id;
        this.off("session-ready", onReady);
        resolve(data.session_id);
      };
      const onError = (msg: { message: string }) => {
        this.off("session-ready", onReady);
        this.off("errorMessage", onError);
        reject(new Error(msg.message));
      };
      this.on("session-ready", onReady);
      this.on("errorMessage", onError);
      this.send({ type: "NewSession", cwd });
    });
  }

  /** Send `Prompt`. Streaming events are emitted via the registered handlers. */
  prompt(text: string, attachments: Attachment[] = []): void {
    this.send({ type: "Prompt", text, attachments });
  }

  /** Send `Cancel` to abort the current turn. */
  cancel(): void {
    this.send({ type: "Cancel" });
  }

  /**
   * Send a slash command (e.g. `/clear`, `/compact`). Returns the
   * `SlashResult.text` if the daemon produces one.
   */
  slashCommand(command: string): Promise<string | null> {
    return new Promise<string | null>((resolve) => {
      const onResult = (data: { text: string }) => {
        this.off("slash-result", onResult);
        resolve(data.text);
      };
      this.on("slash-result", onResult);
      this.send({ type: "SlashCommand", command });
      // Daemon may not produce a SlashResult for unknown commands; resolve
      // null after a short grace period if nothing arrived.
      setTimeout(() => {
        this.off("slash-result", onResult);
        resolve(null);
      }, 5000);
    });
  }

  /** Respond to a `PermissionRequest`. The callback in `on("permission-request", ...)` calls this. */
  respondToPermission(request_id: number, decision: PermissionDecision): void {
    this.send({ type: "PermissionResponse", request_id, decision });
  }

  /** Respond to an `AskUserRequest`. The UI calls this after the user
   *  submits answers to the questions. */
  respondToAskUser(request_id: number, answers: string[]): void {
    this.send({ type: "AskUserResponse", request_id, answers });
  }

  /** Attach to an existing session (active or idle). If the session is idle,
   *  the daemon loads its persisted history from disk and sends a
   *  `session-history` event. Resolves with the session id. */
  async attachSession(session_id: string): Promise<string> {
    await this.connect();
    return new Promise<string>((resolve, reject) => {
      const onReady = (data: { session_id: string }) => {
        this.sessionId = data.session_id;
        this.off("session-ready", onReady);
        this.off("errorMessage", onError);
        resolve(data.session_id);
      };
      const onError = (msg: { message: string }) => {
        this.off("session-ready", onReady);
        this.off("errorMessage", onError);
        reject(new Error(msg.message));
      };
      this.on("session-ready", onReady);
      this.on("errorMessage", onError);
      this.send({ type: "AttachSession", session_id });
    });
  }

  /** List all sessions known to the daemon (active + persisted idle).
   *  The daemon responds with a `session-list` event. */
  listSessions(): Promise<SessionInfo[]> {
    return new Promise<SessionInfo[]>((resolve) => {
      const onList = (data: { sessions: SessionInfo[] }) => {
        this.off("session-list", onList);
        resolve(data.sessions);
      };
      this.on("session-list", onList);
      this.send({ type: "ListSessions" });
    });
  }

  /** List available models from all configured providers. */
  listModels(): Promise<ModelInfo[]> {
    return new Promise<ModelInfo[]>((resolve) => {
      const onList = (data: { models: ModelInfo[] }) => {
        this.off("model-list", onList);
        resolve(data.models);
      };
      this.on("model-list", onList);
      this.send({ type: "ListModels" });
    });
  }

  /** Switch the active session to a different model. Resolves when the
   *  daemon confirms via `model-switched`. */
  switchModel(provider: string, model: string): Promise<{ provider: string; model: string }> {
    return new Promise<{ provider: string; model: string }>((resolve) => {
      const onSwitched = (data: { provider: string; model: string }) => {
        this.off("model-switched", onSwitched);
        resolve(data);
      };
      this.on("model-switched", onSwitched);
      this.send({ type: "SwitchModel", provider, model });
    });
  }

  /** Set or clear the thinking/reasoning variant. Pass empty string or
   *  "none" to disable. Resolves when the daemon confirms via
   *  `thinking-variant-changed`. Returns the resolved variant name, or
   *  null if thinking was disabled. */
  setThinkingVariant(variant: string): Promise<string | null> {
    return new Promise<string | null>((resolve) => {
      const onChanged = (data: { variant: string | null }) => {
        this.off("thinking-variant-changed", onChanged);
        resolve(data.variant ?? null);
      };
      this.on("thinking-variant-changed", onChanged);
      this.send({ type: "SetThinkingVariant", variant });
    });
  }

  // -------------------------------------------------------------------------
  // Event registration
  // -------------------------------------------------------------------------

  on<E extends MewEventName>(event: E, cb: MewClientEvents[E]): void {
    let set = this.listeners.get(event);
    if (!set) {
      set = new Set();
      this.listeners.set(event, set);
    }
    set.add(cb as (...args: unknown[]) => void);
  }

  off<E extends MewEventName>(event: E, cb: MewClientEvents[E]): void {
    this.listeners.get(event)?.delete(cb as (...args: unknown[]) => void);
  }

  private emit<E extends MewEventName>(
    event: E,
    ...args: Parameters<MewClientEvents[E]>
  ): void {
    const set = this.listeners.get(event);
    if (!set) return;
    for (const cb of set) {
      try {
        (cb as (...a: unknown[]) => void)(...args);
      } catch (e) {
        // Never let one listener's throw break the dispatch loop.
        console.error("[mew] listener for", event, "threw:", e);
      }
    }
  }

  // -------------------------------------------------------------------------
  // Wire dispatch
  // -------------------------------------------------------------------------

  private dispatch(msg: ServerMessage): void {
    switch (msg.type) {
      case "SessionReady":
        this.sessionId = msg.session_id;
        this.emit("session-ready", {
          session_id: msg.session_id,
          model: msg.model,
          provider: msg.provider,
        });
        break;
      case "Provider":
        this.emit("provider", msg.event);
        break;
      case "ToolStart":
        this.emit("tool-start", { call_id: msg.call_id });
        break;
      case "ToolEnd":
        this.emit("tool-end", { call_id: msg.call_id, success: msg.success });
        break;
      case "PartUpdated":
        this.emit("part-updated", { part_id: msg.part_id, part: msg.part });
        break;
      case "ToolProgress":
        this.emit("tool-progress", { call_id: msg.call_id, chunk: msg.chunk });
        break;
      case "PermissionRequest":
        this.emit(
          "permission-request",
          {
            request_id: msg.request_id,
            tool_name: msg.tool_name,
            input: msg.input,
          },
          (decision) => this.respondToPermission(msg.request_id, decision),
        );
        break;
      case "WorkspacePermissionRequest":
        this.emit(
          "workspace-permission-request",
          {
            request_id: msg.request_id,
            path: msg.path,
          },
          (decision) => this.respondToPermission(msg.request_id, decision),
        );
        break;
      case "AskUserRequest":
        this.emit("ask-user-request", {
          request_id: msg.request_id,
          call_id: msg.call_id,
          questions: msg.questions,
        });
        break;
      case "SubagentStart":
        this.emit("subagent-start", {
          parent_call_id: msg.parent_call_id,
          name: msg.name,
          child_session_id: msg.child_session_id,
          display_name: msg.display_name,
        });
        break;
      case "SubagentStatus":
        this.emit("subagent-status", {
          parent_call_id: msg.parent_call_id,
          tool_name: msg.tool_name,
          message: msg.message,
        });
        break;
      case "SubagentEnd":
        this.emit("subagent-end", {
          parent_call_id: msg.parent_call_id,
          child_session_id: msg.child_session_id,
          outcome: msg.outcome,
        });
        break;
      case "TodosUpdated":
        this.emit("todos-updated", { todos: msg.todos });
        break;
      case "PersonaSwitchRequested":
        this.emit("persona-switch-requested", { name: msg.name });
        break;
      case "JobUpdate":
        this.emit("job-update", {
          job_id: msg.job_id,
          command: msg.command,
          state: msg.state,
        });
        break;
      case "SlashResult":
        this.emit("slash-result", { text: msg.text });
        break;
      case "RequestResolved":
        this.emit("request-resolved", { request_id: msg.request_id });
        break;
      case "SessionCleared":
        this.emit("session-cleared");
        break;
      case "SessionList":
        this.emit("session-list", { sessions: msg.sessions });
        break;
      case "SessionHistory":
        this.emit("session-history", { messages: msg.messages });
        break;
      case "ModelList":
        this.emit("model-list", { models: msg.models });
        break;
      case "ModelSwitched":
        this.emit("model-switched", {
          provider: msg.provider,
          model: msg.model,
        });
        break;
      case "ThinkingVariantChanged":
        this.emit("thinking-variant-changed", {
          variant: msg.variant ?? null,
        });
        break;
      case "SessionTitleChanged":
        this.emit("session-title-changed", {
          session_id: msg.session_id,
          title: msg.title,
        });
        break;
      case "Error":
        this.emit("errorMessage", { message: msg.message });
        break;
      case "ErrorEvent":
        this.emit("errorEvent", { message: msg.message });
        break;
    }
  }

  private send(msg: ClientMessage): void {
    if (!this.ws) throw new Error("not connected");
    if (this.debug) console.debug("[mew] ->", msg);
    this.ws.send(JSON.stringify(msg));
  }

  /** Return the active session id, or null if `newSession` hasn't succeeded. */
  getSessionId(): string | null {
    return this.sessionId;
  }
}
