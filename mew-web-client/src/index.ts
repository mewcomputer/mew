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
  | { type: "new_session"; cwd: string | null; client_kind: string }
  | { type: "attach_session"; session_id: string; client_kind: string }
  | { type: "list_sessions" }
  | { type: "delete_session"; session_id: string }
  | { type: "rename_session"; session_id: string; title: string }
  | { type: "set_auto_title"; enabled: boolean }
  | { type: "set_auto_summary"; enabled: boolean }
  | { type: "prompt"; text: string; attachments: Attachment[] }
  | { type: "cancel" }
  | {
      type: "permission_response";
      request_id: number;
      decision: PermissionDecision;
    }
  | { type: "ask_user_response"; request_id: number; answers: string[] }
  | { type: "slash_command"; command: string }
  | { type: "list_models" }
  | { type: "switch_model"; provider: string; model: string }
  | { type: "set_thinking_variant"; variant: string }
  | { type: "set_permission_mode"; mode: string }
  | { type: "yield_control" }
  | { type: "create_group"; name: string; color?: string }
  | {
      type: "update_group";
      group_id: string;
      name?: string;
      color?: string | null;
      order?: number;
    }
  | { type: "delete_group"; group_id: string }
  | {
      type: "assign_session_group";
      session_id: string;
      group_id?: string | null;
      position?: number;
    }
  | { type: "archive_session"; session_id: string; archived: boolean }
  | { type: "pin_session"; session_id: string; pinned: boolean }
  | { type: "list_dir"; session_id: string; path?: string }
  | {
      type: "read_file_preview";
      session_id: string;
      path: string;
      max_bytes?: number;
    }
  | { type: "git_status"; session_id: string }
  | { type: "watch_workspace"; session_id: string; enabled: boolean }
  | { type: "open_path"; session_id: string; path: string }
  | { type: "unflag_file"; session_id: string; path: string };

// Provider events — see mew_message::ProviderEventWire.
export type ProviderEventWire =
  | { type: "part_start"; part: Part }
  | { type: "part_delta"; part_id: string; field: string; delta: string }
  | { type: "part_end"; part_id: string }
  | {
      type: "message_end";
      finish: "stop" | "tool_use" | "length" | "content_filter" | "error";
      usage: { input: number; output: number };
      cost: number;
    }
  | {
      type: "retry_wait";
      attempt: number;
      max_attempts: number;
      delay_secs: number;
      reason: string;
    }
  | { type: "error"; error: MessageError };

export type Part =
  | { type: "text"; base: PartBase; text: string; synthetic: boolean }
  | { type: "reasoning"; base: PartBase; text: string; signature?: string }
  | {
      type: "tool_call";
      base: PartBase;
      tool_name: string;
      call_id: string;
      state: ToolState;
    }
  | { type: "tool_result"; base: PartBase; call_id: string; output?: string }
  | {
      type: "file";
      base: PartBase;
      mime: string;
      filename?: string;
      url: string;
    }
  | { type: "compaction"; base: PartBase; auto: boolean; overflow: boolean };

export interface PartBase {
  id: string;
  message_id: string;
  session_id: string;
}

export type ToolState =
  | {
      type: "pending";
      input: unknown;
      time: { start: number; end: number | null };
    }
  | {
      type: "running";
      input: unknown;
      output: string;
      time: { start: number; end: number | null };
    }
  | {
      type: "completed";
      input: unknown;
      output: string;
      time: { start: number; end: number | null };
    }
  | {
      type: "error";
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
  | { type: "completed" }
  | { type: "cancelled" }
  | { type: "failed"; reason: string };

// ---------------------------------------------------------------------------
// Session & model management types
// ---------------------------------------------------------------------------

/** Info about a single available model, returned by `list_models`. */
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
  /** Maximum context window in tokens, if known from the catalog. */
  context_window?: number;
}

/** A named thinking/reasoning variant (e.g. "high", "max", "thinking"). */
export interface ThinkingVariantInfo {
  name: string;
}

/** Session lifecycle state. */
export type SessionState = "active" | "idle" | "running";

/** Cumulative diff stats for a session. */
export interface ChangeStats {
  added: number;
  removed: number;
  files: string[];
}

/** Wire-format usage stats for a session. */
export interface SessionUsageWire {
  input_tokens: number;
  output_tokens: number;
  cache_read_tokens: number;
  cache_write_tokens: number;
  cost: number;
  turns: number;
}

/** Alert kind for cross-session notifications. */
export type AlertKind = "turn_complete" | "turn_failed" | "permission_needed" | "input_needed";

/** Wire-format info about a flagged file. */
export interface FlaggedFileWire {
  path: string;
  reason?: string;
}

/** Metadata returned by `list_sessions` for one session. */
export interface SessionInfo {
  session_id: string;
  state: SessionState;
  model?: string;
  provider?: string;
  created_at: number;
  last_message_at?: number;
  summary?: string;
  client_count: number;
  cwd?: string;
  last_turn_failed?: boolean;
  archived?: boolean;
  pinned?: boolean;
  group_id?: string;
  change_stats?: ChangeStats;
  usage?: SessionUsageWire;
  pending_permissions?: number;
  pending_questions?: number;
}

/** A session group. */
export interface GroupInfo {
  id: string;
  name: string;
  color?: string;
  order: number;
}

/** One entry in a directory listing. */
export interface DirEntry {
  name: string;
  is_dir: boolean;
  size?: number;
}

/** Git file status. */
export type GitFileStatus = "added" | "modified" | "deleted" | "renamed" | "untracked";

/** One entry in a git status result. */
export interface GitEntry {
  path: string;
  status: GitFileStatus;
}

/** A message role. */
export type Role = "user" | "assistant";

/** Timestamp metadata for a message. */
export interface Time {
  created: number;
  completed?: number;
}

/** A complete message, as returned in `session_history`. */
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
  | { type: "session_ready"; session_id: string; model?: string; provider?: string; permission_mode?: string }
  | { type: "error"; message: string }
  | { type: "provider"; event: ProviderEventWire }
  | { type: "user_message"; text: string }
  | { type: "tool_start"; call_id: string }
  | { type: "tool_end"; call_id: string; success: boolean }
  | { type: "part_updated"; part_id: string; part: Part }
  | { type: "tool_progress"; call_id: string; chunk: string }
  | { type: "error_event"; message: string }
  | {
      type: "permission_request";
      request_id: number;
      tool_name: string;
      input: Record<string, unknown>;
    }
  | { type: "workspace_permission_request"; request_id: number; path: string }
  | {
      type: "ask_user_request";
      request_id: number;
      call_id: string;
      questions: Question[];
    }
  | {
      type: "subagent_start";
      parent_call_id: string;
      name: string;
      child_session_id: string;
      display_name: string | null;
    }
  | {
      type: "subagent_status";
      parent_call_id: string;
      tool_name: string;
      message: string;
    }
  | {
      type: "subagent_end";
      parent_call_id: string;
      child_session_id: string;
      outcome: SubagentOutcome;
    }
  | {
      type: "subagent_permission_request";
      request_id: number;
      parent_call_id: string;
      tool_name: string;
      input: Record<string, unknown>;
    }
  | { type: "todos_updated"; todos: Todo[] }
  | { type: "persona_switch_requested"; name: string }
  | { type: "job_update"; job_id: string; command: string; state: string }
  | { type: "slash_result"; text: string }
  | { type: "request_resolved"; request_id: number }
  | { type: "session_cleared" }
  | { type: "session_list"; sessions: SessionInfo[] }
  | { type: "session_history"; messages: Message[] }
  | { type: "model_list"; models: ModelInfo[] }
  | { type: "model_switched"; provider: string; model: string }
  | { type: "thinking_variant_changed"; variant?: string }
  | { type: "permission_mode_changed"; mode: string }
  | { type: "client_attached"; client_id: number; client_kind: string }
  | { type: "client_detached"; client_id: number }
  | { type: "control_yielded"; client_id: number }
  | { type: "session_title_changed"; session_id: string; title: string }
  | { type: "session_summary_changed"; session_id: string; summary: string }
  | { type: "session_activity_changed"; session_id: string; activity: SessionState }
  | {
      type: "session_stats_changed";
      session_id: string;
      added: number;
      removed: number;
      files_changed: number;
    }
  | { type: "group_list"; groups: GroupInfo[] }
  | { type: "groups_changed"; groups: GroupInfo[] }
  | { type: "dir_listing"; path: string; entries: DirEntry[] }
  | {
      type: "file_preview";
      path: string;
      content: string;
      truncated: boolean;
      language?: string;
    }
  | { type: "git_status_result"; entries: GitEntry[] }
  | { type: "fs_changed"; paths: string[] }
  | { type: "session_usage_changed"; session_id: string; usage: SessionUsageWire }
  | {
      type: "session_alert";
      session_id: string;
      title: string;
      kind: AlertKind;
      detail?: string;
    }
  | { type: "flagged_files_changed"; session_id: string; files: FlaggedFileWire[] }
  | {
      type: "session_meta_changed";
      session_id: string;
      archived: boolean;
      pinned: boolean;
      group_id?: string;
    }
  | {
      type: "session_attention_changed";
      session_id: string;
      pending_permissions: number;
      pending_questions: number;
    };

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
    permission_mode?: string;
  }) => void;
  provider: (ev: ProviderEventWire) => void;
  "user-message": (data: { text: string }) => void;
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
  "permission-mode-changed": (data: { mode: string }) => void;
  "client-attached": (data: { client_id: number; client_kind: string }) => void;
  "client-detached": (data: { client_id: number }) => void;
  "control-yielded": (data: { client_id: number }) => void;
  "session-title-changed": (data: { session_id: string; title: string }) => void;
  "session-summary-changed": (data: { session_id: string; summary: string }) => void;
  "session-activity-changed": (data: { session_id: string; activity: SessionState }) => void;
  "session-stats-changed": (data: {
    session_id: string;
    added: number;
    removed: number;
    files_changed: number;
  }) => void;
  "group-list": (data: { groups: GroupInfo[] }) => void;
  "groups-changed": (data: { groups: GroupInfo[] }) => void;
  "dir-listing": (data: { path: string; entries: DirEntry[] }) => void;
  "file-preview": (data: {
    path: string;
    content: string;
    truncated: boolean;
    language?: string;
  }) => void;
  "git-status-result": (data: { entries: GitEntry[] }) => void;
  "fs-changed": (data: { paths: string[] }) => void;
  "session-usage-changed": (data: { session_id: string; usage: SessionUsageWire }) => void;
  "session-alert": (data: {
    session_id: string;
    title: string;
    kind: AlertKind;
    detail?: string;
  }) => void;
  "flagged-files-changed": (data: { session_id: string; files: FlaggedFileWire[] }) => void;
  "session-meta-changed": (data: {
    session_id: string;
    archived: boolean;
    pinned: boolean;
    group_id?: string;
  }) => void;
  "session-attention-changed": (data: {
    session_id: string;
    pending_permissions: number;
    pending_questions: number;
  }) => void;

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

  /** Send `new_session`. Resolves with the daemon-assigned session id. */
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
      this.send({ type: "new_session", cwd, client_kind: "web" });
    });
  }

  /** Send `prompt`. Streaming events are emitted via the registered handlers. */
  prompt(text: string, attachments: Attachment[] = []): void {
    this.send({ type: "prompt", text, attachments });
  }

  /** Send `cancel` to abort the current turn. */
  cancel(): void {
    this.send({ type: "cancel" });
  }

  /**
   * Send a slash command (e.g. `/clear`, `/compact`). Returns the
   * `slash_result.text` if the daemon produces one.
   */
  slashCommand(command: string): Promise<string | null> {
    return new Promise<string | null>((resolve) => {
      const onResult = (data: { text: string }) => {
        this.off("slash-result", onResult);
        resolve(data.text);
      };
      this.on("slash-result", onResult);
      this.send({ type: "slash_command", command });
      // Daemon may not produce a SlashResult for unknown commands; resolve
      // null after a short grace period if nothing arrived.
      setTimeout(() => {
        this.off("slash-result", onResult);
        resolve(null);
      }, 5000);
    });
  }

  /** Respond to a `permission_request`. The callback in `on("permission-request", ...)` calls this. */
  respondToPermission(request_id: number, decision: PermissionDecision): void {
    this.send({ type: "permission_response", request_id, decision });
  }

  /** Respond to an `ask_user_request`. The UI calls this after the user
   *  submits answers to the questions. */
  respondToAskUser(request_id: number, answers: string[]): void {
    this.send({ type: "ask_user_response", request_id, answers });
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
      this.send({ type: "attach_session", session_id, client_kind: "web" });
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
      this.send({ type: "list_sessions" });
    });
  }

  /** Delete a session from disk and remove it from the active list. */
  deleteSession(session_id: string): void {
    this.send({ type: "delete_session", session_id });
  }

  /** Rename a session (set a custom title). Persists to disk and broadcasts. */
  renameSession(session_id: string, title: string): void {
    this.send({ type: "rename_session", session_id, title });
  }

  /** Enable or disable auto-generated session titles. */
  setAutoTitle(enabled: boolean): void {
    this.send({ type: "set_auto_title", enabled });
  }

  setAutoSummary(enabled: boolean): void {
    this.send({ type: "set_auto_summary", enabled });
  }

  /** List available models from all configured providers. */
  listModels(): Promise<ModelInfo[]> {
    return new Promise<ModelInfo[]>((resolve) => {
      const onList = (data: { models: ModelInfo[] }) => {
        this.off("model-list", onList);
        resolve(data.models);
      };
      this.on("model-list", onList);
      this.send({ type: "list_models" });
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
      this.send({ type: "switch_model", provider, model });
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
      this.send({ type: "set_thinking_variant", variant });
    });
  }

  /** Set the permission mode for the active session. Mode is one of:
   *  "standard", "permissive", "auto", "auto_plus", "dangerous".
   *  Resolves when the daemon confirms via `permission-mode-changed`. */
  setPermissionMode(mode: string): Promise<string | null> {
    return new Promise<string | null>((resolve) => {
      const onChanged = (data: { mode: string }) => {
        this.off("permission-mode-changed", onChanged);
        resolve(data.mode);
      };
      this.on("permission-mode-changed", onChanged);
      this.send({ type: "set_permission_mode", mode });
    });
  }

  /** Yield control of the session. Advisory — other clients can become active. */
  yieldControl(): void {
    this.send({ type: "yield_control" });
  }

  // -- Phase 2: groups & archive --
  createGroup(name: string, color?: string): void {
    this.send({ type: "create_group", name, color });
  }
  updateGroup(
    groupId: string,
    opts: { name?: string; color?: string | null; order?: number },
  ): void {
    this.send({ type: "update_group", group_id: groupId, ...opts });
  }
  deleteGroup(groupId: string): void {
    this.send({ type: "delete_group", group_id: groupId });
  }
  assignSessionGroup(
    sessionId: string,
    groupId: string | null,
    position?: number,
  ): void {
    this.send({
      type: "assign_session_group",
      session_id: sessionId,
      group_id: groupId,
      position,
    });
  }
  archiveSession(sessionId: string, archived: boolean): void {
    this.send({ type: "archive_session", session_id: sessionId, archived });
  }
  pinSession(sessionId: string, pinned: boolean): void {
    this.send({ type: "pin_session", session_id: sessionId, pinned });
  }

  // -- Phase 3: file service --
  listDir(sessionId: string, path?: string): void {
    this.send({ type: "list_dir", session_id: sessionId, path });
  }
  readFilePreview(sessionId: string, path: string, maxBytes?: number): void {
    this.send({ type: "read_file_preview", session_id: sessionId, path, max_bytes: maxBytes });
  }
  gitStatus(sessionId: string): void {
    this.send({ type: "git_status", session_id: sessionId });
  }
  watchWorkspace(sessionId: string, enabled: boolean): void {
    this.send({ type: "watch_workspace", session_id: sessionId, enabled });
  }
  openPath(sessionId: string, path: string): void {
    this.send({ type: "open_path", session_id: sessionId, path });
  }

  unflagFile(sessionId: string, path: string): void {
    this.send({ type: "unflag_file", session_id: sessionId, path });
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
      case "session_ready":
        this.sessionId = msg.session_id;
        this.emit("session-ready", {
          session_id: msg.session_id,
          model: msg.model,
          provider: msg.provider,
        });
        break;
      case "provider":
        this.emit("provider", msg.event);
        break;
      case "user_message":
        this.emit("user-message", { text: msg.text });
        break;
      case "tool_start":
        this.emit("tool-start", { call_id: msg.call_id });
        break;
      case "tool_end":
        this.emit("tool-end", { call_id: msg.call_id, success: msg.success });
        break;
      case "part_updated":
        this.emit("part-updated", { part_id: msg.part_id, part: msg.part });
        break;
      case "tool_progress":
        this.emit("tool-progress", { call_id: msg.call_id, chunk: msg.chunk });
        break;
      case "permission_request":
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
      case "workspace_permission_request":
        this.emit(
          "workspace-permission-request",
          {
            request_id: msg.request_id,
            path: msg.path,
          },
          (decision) => this.respondToPermission(msg.request_id, decision),
        );
        break;
      case "ask_user_request":
        this.emit("ask-user-request", {
          request_id: msg.request_id,
          call_id: msg.call_id,
          questions: msg.questions,
        });
        break;
      case "subagent_start":
        this.emit("subagent-start", {
          parent_call_id: msg.parent_call_id,
          name: msg.name,
          child_session_id: msg.child_session_id,
          display_name: msg.display_name,
        });
        break;
      case "subagent_status":
        this.emit("subagent-status", {
          parent_call_id: msg.parent_call_id,
          tool_name: msg.tool_name,
          message: msg.message,
        });
        break;
      case "subagent_end":
        this.emit("subagent-end", {
          parent_call_id: msg.parent_call_id,
          child_session_id: msg.child_session_id,
          outcome: msg.outcome,
        });
        break;
      case "todos_updated":
        this.emit("todos-updated", { todos: msg.todos });
        break;
      case "persona_switch_requested":
        this.emit("persona-switch-requested", { name: msg.name });
        break;
      case "job_update":
        this.emit("job-update", {
          job_id: msg.job_id,
          command: msg.command,
          state: msg.state,
        });
        break;
      case "slash_result":
        this.emit("slash-result", { text: msg.text });
        break;
      case "request_resolved":
        this.emit("request-resolved", { request_id: msg.request_id });
        break;
      case "session_cleared":
        this.emit("session-cleared");
        break;
      case "session_list":
        this.emit("session-list", { sessions: msg.sessions });
        break;
      case "session_history":
        this.emit("session-history", { messages: msg.messages });
        break;
      case "model_list":
        this.emit("model-list", { models: msg.models });
        break;
      case "model_switched":
        this.emit("model-switched", {
          provider: msg.provider,
          model: msg.model,
        });
        break;
      case "thinking_variant_changed":
        this.emit("thinking-variant-changed", {
          variant: msg.variant ?? null,
        });
        break;
      case "permission_mode_changed":
        this.emit("permission-mode-changed", { mode: msg.mode });
        break;
      case "client_attached":
        this.emit("client-attached", {
          client_id: msg.client_id,
          client_kind: msg.client_kind,
        });
        break;
      case "client_detached":
        this.emit("client-detached", { client_id: msg.client_id });
        break;
      case "control_yielded":
        this.emit("control-yielded", { client_id: msg.client_id });
        break;
      case "session_title_changed":
        this.emit("session-title-changed", {
          session_id: msg.session_id,
          title: msg.title,
        });
        break;
      case "session_summary_changed":
        this.emit("session-summary-changed", {
          session_id: msg.session_id,
          summary: msg.summary,
        });
        break;
      case "session_activity_changed":
        this.emit("session-activity-changed", {
          session_id: msg.session_id,
          activity: msg.activity,
        });
        break;
      case "session_stats_changed":
        this.emit("session-stats-changed", {
          session_id: msg.session_id,
          added: msg.added,
          removed: msg.removed,
          files_changed: msg.files_changed,
        });
        break;
      case "group_list":
        this.emit("group-list", { groups: msg.groups });
        break;
      case "groups_changed":
        this.emit("groups-changed", { groups: msg.groups });
        break;
      case "dir_listing":
        this.emit("dir-listing", { path: msg.path, entries: msg.entries });
        break;
      case "file_preview":
        this.emit("file-preview", {
          path: msg.path,
          content: msg.content,
          truncated: msg.truncated,
          language: msg.language,
        });
        break;
      case "git_status_result":
        this.emit("git-status-result", { entries: msg.entries });
        break;
      case "fs_changed":
        this.emit("fs-changed", { paths: msg.paths });
        break;
      case "session_usage_changed":
        this.emit("session-usage-changed", {
          session_id: msg.session_id,
          usage: msg.usage,
        });
        break;
      case "session_alert":
        this.emit("session-alert", {
          session_id: msg.session_id,
          title: msg.title,
          kind: msg.kind,
          detail: msg.detail,
        });
        break;
      case "flagged_files_changed":
        this.emit("flagged-files-changed", {
          session_id: msg.session_id,
          files: msg.files,
        });
        break;
      case "session_meta_changed":
        this.emit("session-meta-changed", {
          session_id: msg.session_id,
          archived: msg.archived,
          pinned: msg.pinned,
          group_id: msg.group_id,
        });
        break;
      case "session_attention_changed":
        this.emit("session-attention-changed", {
          session_id: msg.session_id,
          pending_permissions: msg.pending_permissions,
          pending_questions: msg.pending_questions,
        });
        break;
      case "error":
        this.emit("errorMessage", { message: msg.message });
        break;
      case "error_event":
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
