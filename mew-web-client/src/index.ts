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
  | { type: "remote_hello"; token?: string; device_name: string }
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
      request_id: string;
      decision: PermissionDecision;
    }
  | { type: "ask_user_response"; request_id: string; answers: string[] }
  | {
      type: "plan_approval_response";
      request_id: string;
      approved: boolean;
      feedback?: string;
    }
  | { type: "slash_command"; command: string }
  | { type: "list_models" }
  | { type: "switch_model"; provider: string; model: string }
  | { type: "list_personas" }
  | { type: "switch_persona"; name: string }
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
  | { type: "regenerate_title"; session_id: string }
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
  | { type: "unflag_file"; session_id: string; path: string }
  | { type: "ping" }
  | { type: "list_projects" }
  | { type: "list_filesystem_dir"; path?: string }
  | { type: "browser_open"; url: string; tab_id?: string }
  | { type: "browser_snapshot"; tab_id?: string }
  | { type: "browser_screenshot"; annotate: boolean; tab_id?: string }
  | { type: "browser_click"; selector: string; tab_id?: string }
  | { type: "browser_fill"; selector: string; text: string; tab_id?: string }
  | { type: "browser_press"; key: string; tab_id?: string }
  | { type: "browser_close"; tab_id?: string };

// Provider events — see mew_message::ProviderEventWire.
export type ProviderEventWire =
  | { type: "part_start"; part: Part }
  | { type: "part_delta"; part_id: string; field: string; delta: string }
  | { type: "part_end"; part_id: string }
  | {
      type: "message_end";
      finish: string;
      usage: {
        input: number;
        output: number;
        reasoning: number;
        cache_read: number;
        cache_write: number;
      };
      cost: number;
      manifest?: TurnManifest | null;
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
  /** Numeric thinking-budget range, when the model accepts a
   *  `thinking_budget` token cap (e.g. Qwen3.8-max). Absent when the model
   *  has no configurable budget. */
  thinking_budget?: ThinkingBudgetInfo | null;
  /** Maximum context window in tokens, if known from the catalog. */
  context_window?: number;
}

/** Info about an available persona, returned by `list_personas`. */
export interface PersonaInfo {
  /** Persona name (unique identifier). */
  name: string;
  /** Human-readable description. */
  description: string;
  /** Optional color token for UI display. */
  color?: string;
  /** Whether this persona is currently active. */
  active: boolean;
}

/** A named thinking/reasoning variant (e.g. "high", "max", "thinking"). */
export interface ThinkingVariantInfo {
  name: string;
}

/** Numeric thinking-budget range for models that accept a `thinking_budget`
 *  token cap. Budget selection rides `setThinkingVariant` as the string
 *  convention `"budget:<n>"` (clamped/snapped to `min..=max` by `step` by
 *  the daemon); see `setThinkingBudget`. */
export interface ThinkingBudgetInfo {
  min: number;
  max: number;
  step: number;
  default: number;
  /** Canonical budget (in tokens) for each named effort variant, so UIs can
   *  seed a slider position from the active effort level. */
  by_effort: [string, number][];
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

/** A known project directory, returned by `list_projects`. */
export interface ProjectInfo {
  /** Absolute path to the project directory. */
  path: string;
  /** Human-friendly display name (last path component). */
  display_name: string;
  /** Number of sessions in this project. */
  session_count: number;
  /** Timestamp of the last activity (epoch seconds). */
  last_used_at: number | null;
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
  /** First user message text (truncated), used as a display title fallback. */
  first_message?: string;
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
export type Role = "user" | "assistant" | "system";

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
  manifest?: TurnManifest | null;
}

/** Token usage breakdown. */
export interface Tokens {
  input: number;
  output: number;
  reasoning: number;
  cache_read: number;
  cache_write: number;
}

/** Per-turn context window manifest. Captured at prompt assembly time. */
export interface TurnManifest {
  model: string;
  context_window: number;
  input_tokens?: number;
  output_tokens?: number;
  cache_read_tokens?: number;
  cache_write_tokens?: number;
  reasoning_tokens?: number;
  segments: Segment[];
}

/** A segment of the assembled prompt (system, tools, history, etc.). */
export interface Segment {
  label: string;
  kind: string;
  source_id?: string | null;
  tokens: number;
  tokens_scaled: number;
  children: Segment[];
}

export type ServerMessage =
  | { type: "remote_ready"; scope: "observe" | "collaborate" | "control" }
  | { type: "session_ready"; session_id: string; cwd?: string; model?: string; provider?: string; permission_mode?: string }
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
      request_id: string;
      tool_name: string;
      input: Record<string, unknown>;
    }
  | { type: "workspace_permission_request"; request_id: string; path: string }
  | {
      type: "ask_user_request";
      request_id: string;
      call_id: string;
      questions: Question[];
    }
  | {
      type: "plan_approval_request";
      request_id: string;
      call_id: string;
      plan_path: string;
      plan_markdown: string;
      persona: string;
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
      manifests?: TurnManifest[];
    }
  | {
      type: "subagent_permission_request";
      request_id: string;
      parent_call_id: string;
      tool_name: string;
      input: Record<string, unknown>;
    }
  | { type: "todos_updated"; todos: Todo[] }
  | { type: "persona_switch_requested"; name: string }
  | { type: "job_update"; job_id: string; command: string; state: string }
  | { type: "slash_result"; text: string }
  | { type: "request_resolved"; request_id: string }
  | { type: "session_cleared" }
  | { type: "session_list"; sessions: SessionInfo[] }
  | { type: "session_history"; messages: Message[] }
  | { type: "model_list"; models: ModelInfo[] }
  | { type: "model_switched"; provider: string; model: string }
  | { type: "persona_list"; personas: PersonaInfo[] }
  | { type: "persona_switched"; name: string }
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
  | { type: "filesystem_dir_listing"; path: string; entries: DirEntry[] }
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
    }
  | { type: "pong"; version: string }
  | { type: "project_list"; projects: ProjectInfo[] }
  | { type: "browser_snapshot"; snapshot: string; url: string; title: string; tab_id?: string }
  | { type: "browser_screenshot"; data: string; url: string; tab_id?: string }
  | { type: "browser_state"; open: boolean; url?: string; title?: string; tab_id?: string }
  | { type: "browser_error"; message: string; tab_id?: string };

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
    cwd?: string;
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
      request_id: string;
      tool_name: string;
      input: Record<string, unknown>;
    },
    respond: (decision: PermissionDecision) => void,
  ) => void;
  "workspace-permission-request": (
    data: { request_id: string; path: string },
    respond: (decision: PermissionDecision) => void,
  ) => void;
  "ask-user-request": (data: {
    request_id: string;
    call_id: string;
    questions: Question[];
  }) => void;

  "plan-approval-request": (data: {
    request_id: string;
    call_id: string;
    plan_path: string;
    plan_markdown: string;
    persona: string;
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
    manifests?: TurnManifest[];
  }) => void;
  "todos-updated": (data: { todos: Todo[] }) => void;
  "persona-switch-requested": (data: { name: string }) => void;
  "job-update": (data: {
    job_id: string;
    command: string;
    state: string;
  }) => void;
  "slash-result": (data: { text: string }) => void;
  "request-resolved": (data: { request_id: string }) => void;
  "session-cleared": () => void;
  "session-list": (data: { sessions: SessionInfo[] }) => void;
  "session-history": (data: { messages: Message[] }) => void;
  "model-list": (data: { models: ModelInfo[] }) => void;
  "model-switched": (data: { provider: string; model: string }) => void;
  "persona-list": (data: { personas: PersonaInfo[] }) => void;
  "persona-switched": (data: { name: string }) => void;
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
  "filesystem-dir-listing": (data: { path: string; entries: DirEntry[] }) => void;
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
  pong: (data: { version: string }) => void;
  "project-list": (data: { projects: ProjectInfo[] }) => void;
  "browser-snapshot": (data: { snapshot: string; url: string; title: string; tabId?: string }) => void;
  "browser-screenshot": (data: { data: string; url: string; tabId?: string }) => void;
  "browser-state": (data: { open: boolean; url?: string; title?: string; tabId?: string }) => void;
  "browser-error": (data: { message: string; tabId?: string }) => void;
  "remote-ready": (data: { scope: "observe" | "collaborate" | "control" }) => void;
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
  /** Client identity used for capability-gated daemon features. */
  clientKind?: "web" | "desktop" | "remote";
  /** Pairing credentials for a client connecting to an explicit remote daemon. */
  remoteAuth?: { token: string; deviceName: string };
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
  private readonly clientKind: "web" | "desktop" | "remote";
  private readonly remoteAuth?: { token: string; deviceName: string };
  private ws: MewWebSocket | null = null;
  private listeners = new Map<
    MewEventName,
    Set<(...args: unknown[]) => void>
  >();
  /** Promise resolved when the WebSocket opens. */
  private openPromise: Promise<void> | null = null;
  /** Session id returned by `newSession`. */
  private sessionId: string | null = null;
  /** Session lifecycle requests share uncorrelated daemon errors. */
  private sessionCommandTail: Promise<void> = Promise.resolve();

  constructor(url: string, opts: MewClientOptions = {}) {
    this.url = url;
    this.socketFactory = opts.socketFactory ?? defaultSocketFactory;
    this.debug = opts.debug ?? false;
    this.clientKind = opts.clientKind ?? "web";
    this.remoteAuth = opts.remoteAuth;
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
        this.emit("open");
        if (this.clientKind === "remote") {
          this.send({
            type: "remote_hello",
            token: this.remoteAuth?.token,
            device_name: this.remoteAuth?.deviceName ?? "remote client",
          });
          return;
        }
        settled = true;
        resolve();
      });
      if (this.clientKind === "remote") {
        this.on("remote-ready", () => {
          if (!settled) {
            settled = true;
            resolve();
          }
        });
        this.on("errorMessage", (message) => {
          if (!settled) {
            settled = true;
            reject(new Error(message.message));
          }
        });
      }
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
        if (this.ws === ws) {
          this.ws = null;
          this.openPromise = null;
        }
        if (!settled) {
          settled = true;
          reject(new Error(`ws closed before open: ${ev.code} ${ev.reason}`));
        }
        this.emit("close", ev.code, ev.reason);
      });
      ws.addEventListener("error", (ev) => {
        if (this.debug) console.debug("[mew] error", ev);
        if (this.ws === ws) {
          this.ws = null;
          this.openPromise = null;
        }
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

  /** Serialize lifecycle requests because daemon errors have no request id. */
  private enqueueSessionCommand<T>(command: () => Promise<T>): Promise<T> {
    const run = this.sessionCommandTail.then(command, command);
    this.sessionCommandTail = run.then(
      () => undefined,
      () => undefined,
    );
    return run;
  }

  /** Send `new_session`. Resolves with the daemon-assigned session id. */
  newSession(cwd: string | null = null): Promise<string> {
    return this.enqueueSessionCommand(async () => {
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
        this.send({ type: "new_session", cwd, client_kind: this.clientKind });
      });
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
  respondToPermission(request_id: string, decision: PermissionDecision): void {
    this.send({ type: "permission_response", request_id, decision });
  }

  /** Respond to an `ask_user_request`. The UI calls this after the user
   *  submits answers to the questions. */
  respondToAskUser(request_id: string, answers: string[]): void {
    this.send({ type: "ask_user_response", request_id, answers });
  }

  /** Respond to a `plan_approval_request`. `approved = false` with optional
   *  `feedback` requests changes to the plan. */
  respondToPlanApproval(
    request_id: string,
    approved: boolean,
    feedback?: string,
  ): void {
    this.send({ type: "plan_approval_response", request_id, approved, feedback });
  }

  /** Attach to an existing session (active or idle). If the session is idle,
   *  the daemon loads its persisted history from disk and sends a
   *  `session-history` event. Resolves with the session id. */
  attachSession(session_id: string): Promise<string> {
    return this.enqueueSessionCommand(async () => {
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
        this.send({ type: "attach_session", session_id, client_kind: this.clientKind });
      });
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

  /** List available personas for the active session. Resolves when the
   *  daemon replies with `persona-list`. */
  listPersonas(): Promise<PersonaInfo[]> {
    return new Promise<PersonaInfo[]>((resolve) => {
      const onList = (data: { personas: PersonaInfo[] }) => {
        this.off("persona-list", onList);
        resolve(data.personas);
      };
      this.on("persona-list", onList);
      this.send({ type: "list_personas" });
    });
  }

  /** Switch the active session to a different persona. Fire-and-forget:
   *  the store is updated when the daemon confirms via `persona-switched`
   *  (handled by the bridge), so the caller doesn't need to await. */
  switchPersona(name: string): void {
    this.send({ type: "switch_persona", name });
  }

  /** Set or clear the thinking/reasoning variant. Pass empty string or
   *  "none" to disable. Numeric token budgets ride this call as the string
   *  convention `"budget:<n>"` (e.g. `"budget:8192"`); use
   *  `setThinkingBudget` for that. Resolves when the daemon confirms via
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

  /** Set a numeric token budget for thinking via `setThinkingVariant`
   *  (`"budget:<n>"`). Only valid for models that declare a
   *  `thinking_budget` range. */
  setThinkingBudget(tokens: number): Promise<string | null> {
    return this.setThinkingVariant(`budget:${tokens}`);
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

  /** Regenerate the session title from the first user message via LLM.
   *  The daemon broadcasts `session-title-changed` when done. */
  regenerateTitle(sessionId: string): void {
    this.send({ type: "regenerate_title", session_id: sessionId });
  }

  // -- Phase 3: file service --
  listDir(sessionId: string, path?: string): void {
    this.send({ type: "list_dir", session_id: sessionId, path });
  }

  listFilesystemDir(path?: string): void {
    this.send({ type: "list_filesystem_dir", ...(path ? { path } : {}) });
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

  /** Ping the daemon; resolves with the daemon version once a pong arrives. */
  ping(): Promise<string> {
    return new Promise<string>((resolve) => {
      const handler = (data: { version: string }) => {
        this.off("pong", handler);
        resolve(data.version);
      };
      this.on("pong", handler);
      this.send({ type: "ping" });
    });
  }

  /** List known projects (recent session cwds). */
  listProjects(): void {
    this.send({ type: "list_projects" });
  }

  browserOpen(url: string, tabId?: string): void { this.send({ type: "browser_open", url, tab_id: tabId }); }
  browserSnapshot(tabId?: string): void { this.send({ type: "browser_snapshot", tab_id: tabId }); }
  browserScreenshot(annotate = false, tabId?: string): void { this.send({ type: "browser_screenshot", annotate, tab_id: tabId }); }
  browserClick(selector: string, tabId?: string): void { this.send({ type: "browser_click", selector, tab_id: tabId }); }
  browserFill(selector: string, text: string, tabId?: string): void { this.send({ type: "browser_fill", selector, text, tab_id: tabId }); }
  browserPress(key: string, tabId?: string): void { this.send({ type: "browser_press", key, tab_id: tabId }); }
  browserClose(tabId?: string): void { this.send({ type: "browser_close", tab_id: tabId }); }

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
          cwd: msg.cwd,
          model: msg.model,
          provider: msg.provider,
          permission_mode: msg.permission_mode,
        });
        break;
      case "remote_ready":
        this.emit("remote-ready", { scope: msg.scope });
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
      case "plan_approval_request":
        this.emit("plan-approval-request", {
          request_id: msg.request_id,
          call_id: msg.call_id,
          plan_path: msg.plan_path,
          plan_markdown: msg.plan_markdown,
          persona: msg.persona,
        });
        break;
      case "subagent_permission_request":
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
          manifests: msg.manifests ?? [],
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
      case "persona_list":
        this.emit("persona-list", { personas: msg.personas });
        break;
      case "persona_switched":
        this.emit("persona-switched", { name: msg.name });
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
      case "filesystem_dir_listing":
        this.emit("filesystem-dir-listing", { path: msg.path, entries: msg.entries });
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
      case "pong":
        this.emit("pong", { version: msg.version });
        break;
      case "project_list":
        this.emit("project-list", { projects: msg.projects });
        break;
      case "browser_snapshot":
        this.emit("browser-snapshot", { snapshot: msg.snapshot, url: msg.url, title: msg.title, tabId: msg.tab_id });
        break;
      case "browser_screenshot":
        this.emit("browser-screenshot", { data: msg.data, url: msg.url, tabId: msg.tab_id });
        break;
      case "browser_state":
        this.emit("browser-state", { open: msg.open, url: msg.url, title: msg.title, tabId: msg.tab_id });
        break;
      case "browser_error":
        this.emit("browser-error", { message: msg.message, tabId: msg.tab_id });
        break;
      default: {
        // Exhaustiveness check: adding a new ServerMessage variant
        // without handling it here becomes a TypeScript error.
        const _exhaustive: never = msg;
        throw new Error(`unhandled ServerMessage: ${(_exhaustive as { type: string }).type}`);
      }
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
