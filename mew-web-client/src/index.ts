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

export interface NewChatDefaults {
  model: string | null;
  provider: string | null;
  persona: string | null;
  thinking_variant: string | null;
  permission_mode: string;
}

export interface PairedDevice {
  id: string;
  name: string;
  node_id: string;
  scope: "observe" | "collaborate" | "control" | null;
  paired_at: number | null;
}

export interface DaemonSettings {
  request_id: string | null;
  defaults: NewChatDefaults | null;
  devices: PairedDevice[] | null;
}

export type ClientMessage =
  | { type: "remote_hello"; token?: string; device_name: string }
  | { type: "new_session"; cwd: string | null; client_kind: string }
  | { type: "attach_session"; session_id: string; client_kind: string }
  | { type: "load_session_history"; session_id: string; before: number; limit?: number }
  | { type: "read_job_output"; session_id: string; job_id: string; request_id: string; offset: number }
  | { type: "read_subagent_history"; session_id: string; child_session_id: string; request_id: string }
  | { type: "guide_subagent"; session_id: string; child_session_id: string; request_id: string; text: string }
  | { type: "read_subagent_message_receipt"; session_id: string; child_session_id: string; request_id: string }
  | { type: "list_sessions" }
  | { type: "delete_session"; session_id: string }
  | { type: "delete_session_confirmed"; session_id: string; request_id: string }
  | { type: "rename_session"; session_id: string; title: string }
  | { type: "set_auto_title"; enabled: boolean }
  | { type: "read_daemon_settings"; request_id: string }
  | { type: "save_session_defaults"; session_id: string; request_id: string }
  | { type: "reset_session_defaults"; request_id: string }
  | { type: "revoke_device"; device_id: string; request_id: string }
  | { type: "set_auto_summary"; enabled: boolean }
  | { type: "prompt"; text: string; attachments: Attachment[] }
  | { type: "submit_prompt"; session_id: string; request_id: string; text: string; attachments: Attachment[] }
  | { type: "read_prompt_receipt"; session_id: string; request_id: string }
  | { type: "cancel" }
  | { type: "guide"; text: string }
  | {
      type: "permission_response";
      request_id: string;
      decision: PermissionDecision;
    }
  | { type: "ask_user_response"; request_id: string; answers: string[]; cancelled?: boolean }
  | { type: "goal_response"; request_id: string; accepted: boolean }
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
  | ({ type: "update_todo"; session_id: string; request_id: string } & TodoEdit)
  | { type: "git_status"; session_id: string }
  | { type: "watch_workspace"; session_id: string; enabled: boolean }
  | { type: "open_path"; session_id: string; path: string }
  | { type: "unflag_file"; session_id: string; path: string }
  | { type: "ping" }
  | { type: "list_projects" }
  | { type: "list_filesystem_dir"; path?: string }
  | { type: "register_browser_page"; session_id: string; page_instance: string; cdp_endpoint: string; target_id: string; focus_url?: string }
  | { type: "unregister_browser_page"; session_id: string; page_instance: string }
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

export type Part = PartBase & (
  | { type: "text"; text: string; synthetic?: boolean }
  | { type: "reasoning"; text: string; signature?: string }
  | {
      type: "tool_call";
      tool_name: string;
      call_id: string;
      state: ToolState;
    }
  | { type: "tool_result"; call_id: string; output?: string }
  | {
      type: "file";
      mime: string;
      filename?: string;
      url: string;
    }
  | {
      type: "compaction";
      auto: boolean;
      overflow: boolean;
      summary?: string;
      removed_count?: number;
    });

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

export interface SubagentQuestionSource {
  parent_call_id: string;
  child_session_id: string;
  name: string;
  display_name: string | null;
}

export interface SubagentActivity {
  task_id: string;
  child_session_id: string | null;
  name: string;
  display_name: string | null;
  status: string;
  tool_name: string | null;
}

export interface JobActivity {
  job_id: string;
  command: string;
  state: string;
}

export interface JobOutput {
  session_id: string;
  job_id: string;
  request_id: string;
  offset: number;
  next_offset: number;
  output: string;
  done: boolean;
  incomplete: boolean;
}

export interface JobOutputError {
  session_id: string;
  job_id: string;
  request_id: string;
  message: string;
}

export interface SessionActivitySnapshot {
  session_id: string;
  subagents: SubagentActivity[];
  jobs: JobActivity[];
}

export interface QuestionOption {
  label: string;
  description: string;
}

export interface Question {
  prompt: string;
  options: QuestionOption[];
}

export interface TodoEdit {
  todo_id: number;
  expected_content: string;
  expected_status: string;
  content: string;
  status: string;
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
  /** Catalog-provided display name; absent for unnamed models and older daemons. */
  display_name?: string;
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
  /** Current context occupancy (latest request's prompt size), if known. */
  context_tokens?: number;
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

type SessionOperationFailed = {
  type: "session_operation_failed";
  operation: "create" | "attach";
  session_id: string | null;
  message: string;
};

type PromptReceipt = {
  type: "prompt_receipt";
  session_id: string;
  request_id: string;
  status: "accepted" | "rejected" | "unknown";
  message?: string;
};

export type SubagentMessageReceipt = {
  type: "subagent_message_receipt";
  session_id: string;
  child_session_id: string;
  request_id: string;
  status: PromptReceipt["status"];
  message?: string;
};

type SessionDeletionResult = {
  type: "session_deletion_result";
  session_id: string;
  request_id: string;
  error?: string | null;
};

export interface SessionHistoryPage {
  session_id: string;
  messages: Message[];
  next_cursor?: number;
  replace: boolean;
}

export type ServerMessage =
  | ({ type: "daemon_settings" } & DaemonSettings)
  | { type: "daemon_settings_failed"; request_id: string; message: string }
  | { type: "remote_ready"; scope: "observe" | "collaborate" | "control" }
  | { type: "session_ready"; session_id: string; cwd?: string; model?: string; provider?: string; permission_mode?: string }
  | SessionOperationFailed
  | PromptReceipt
  | SubagentMessageReceipt
  | SessionDeletionResult
  | { type: "error"; message: string }
  | { type: "provider"; event: ProviderEventWire }
  | { type: "user_message"; text: string; request_id?: string }
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
      subagent?: SubagentQuestionSource;
      request_id: string;
      call_id: string;
      questions: Question[];
    }
  | { type: "goal_proposed"; request_id: string; call_id: string; objective: string }
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
  | { type: "todo_updated"; session_id: string; request_id: string; todos: Todo[] }
  | { type: "todo_update_error"; session_id: string; request_id: string; message: string }
  | { type: "job_update"; job_id: string; command: string; state: string }
  | ({ type: "session_activity_snapshot" } & SessionActivitySnapshot)
  | { type: "slash_result"; text: string }
  | { type: "request_resolved"; request_id: string }
  | { type: "session_cleared" }
  | { type: "session_list"; sessions: SessionInfo[] }
  | { type: "session_history"; messages: Message[] }
  | ({ type: "job_output" } & JobOutput)
  | ({ type: "job_output_error" } & JobOutputError)
  | { type: "subagent_history"; session_id: string; child_session_id: string; request_id: string; messages: Message[]; error: string | null }
  | ({ type: "session_history_page" } & SessionHistoryPage)
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
  | { type: "fs_changed"; session_id?: string; paths: string[] }
  | {
      type: "session_usage_changed";
      session_id: string;
      usage: SessionUsageWire;
      context_tokens?: number;
    }
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
  | { type: "pong"; version: string; protocol_version?: number; browser_focus_preflight?: boolean; scoped_session_failures?: boolean }
  | { type: "project_list"; projects: ProjectInfo[] }
  | { type: "browser_page_registration"; session_id: string; page_instance: string; error: string | null }
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
  "daemon-settings": (data: DaemonSettings) => void;
  "daemon-settings-failed": (data: { request_id: string; message: string }) => void;
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
  "session-command-state": () => void;
  "session-operation-failed": (data: SessionOperationFailed) => void;
  "prompt-receipt": (data: PromptReceipt) => void;
  "session-deletion-result": (data: SessionDeletionResult) => void;
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
    subagent?: SubagentQuestionSource;
    request_id: string;
    call_id: string;
    questions: Question[];
  }) => void;
  "goal-proposed": (data: { request_id: string; call_id: string; objective: string }) => void;

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
  "todo-updated": (data: { session_id: string; request_id: string; todos: Todo[] }) => void;
  "todo-update-error": (data: { session_id: string; request_id: string; message: string }) => void;
  "todos-updated": (data: { todos: Todo[] }) => void;
  "job-update": (data: {
    job_id: string;
    command: string;
    state: string;
  }) => void;
  "session-activity-snapshot": (data: SessionActivitySnapshot) => void;
  "slash-result": (data: { text: string }) => void;
  "request-resolved": (data: { request_id: string }) => void;
  "session-cleared": () => void;
  "session-list": (data: { sessions: SessionInfo[] }) => void;
  "session-history": (data: { messages: Message[] }) => void;
  "session-history-page": (page: SessionHistoryPage) => void;
  "job-output": (data: JobOutput) => void;
  "job-output-error": (data: JobOutputError) => void;
  "subagent-history": (data: { session_id: string; child_session_id: string; request_id: string; messages: Message[]; error: string | null }) => void;
  "subagent-message-receipt": (receipt: SubagentMessageReceipt) => void;
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
  "session-usage-changed": (data: {
    session_id: string;
    usage: SessionUsageWire;
    context_tokens?: number;
  }) => void;
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
  pong: (data: { version: string; protocol_version: number; browser_focus_preflight: boolean }) => void;
  "project-list": (data: { projects: ProjectInfo[] }) => void;
  "browser-page-registration": (data: { session_id: string; page_instance: string; error: string | null }) => void;
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
  private connectionLifetime: AbortController | null = null;
  private protocolVersion = 0;
  private sessionId: string | null = null;
  private confirmedSessions = new Set<string>();
  private browserFocusPreflight: Promise<boolean> | null = null;
  /** Messages currently projected from the most recent history pages. */
  private sessionHistoryMessages: Message[] = [];
  /** SessionReady has no request ID, so lifecycle requests share a queue. */
  private sessionCommandTail: Promise<void> = Promise.resolve();
  private pendingSessionCommands = 0;
  private scopedSessionFailures = false;
  private pendingPrompts = new Map<string, {
    requestId: string;
    socket: MewWebSocket;
    timeout: ReturnType<typeof setTimeout>;
    finish: (status: PromptReceipt["status"], message?: string) => void;
  }>();

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
    let ws: MewWebSocket;
    try { ws = this.socketFactory(this.url); }
    catch (error) { return Promise.reject(error); }
    const lifetime = new AbortController();
    this.ws = ws;
    this.connectionLifetime = lifetime;
    this.openPromise = new Promise<void>((resolve, reject) => {
      const cleanup = () => {
        clearTimeout(timeout);
        this.off("remote-ready", onReady);
        this.off("errorMessage", onRemoteError);
        lifetime.signal.removeEventListener("abort", onDisconnected);
      };
      const onReady = () => { cleanup(); resolve(); };
      const onRemoteError = (message: { message: string }) => {
        cleanup();
        reject(new Error(message.message));
        this.disconnect();
      };
      const onDisconnected = () => { cleanup(); reject(lifetime.signal.reason); };
      const timeout = setTimeout(() => {
        cleanup();
        reject(new Error("The daemon connection did not open in time."));
        this.disconnect();
      }, 10000);
      lifetime.signal.addEventListener("abort", onDisconnected, { once: true });
      ws.addEventListener("open", () => {
        if (this.ws !== ws) return;
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
        onReady();
      });
      if (this.clientKind === "remote") {
        this.on("remote-ready", onReady);
        this.on("errorMessage", onRemoteError);
      }
      ws.addEventListener("message", (ev) => {
        if (this.ws !== ws) return;
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
        if (!this.endConnection(ws, `The daemon disconnected (${ev.code} ${ev.reason}).`)) return;
        this.emit("close", ev.code, ev.reason);
      });
      ws.addEventListener("error", (ev) => {
        if (this.debug) console.debug("[mew] error", ev);
        if (!this.endConnection(ws, "The daemon connection failed.")) return;
        this.emit("error", ev);
        this.emit("close", 1006, "The daemon connection failed.");
      });
    });
    return this.openPromise;
  }

  /** Close the current socket and discard its attachment confirmations. */
  disconnect(code = 1000, reason = "client disconnect") {
    const socket = this.ws;
    if (!socket || !this.endConnection(socket, "The daemon disconnected.")) return;
    socket.close(code, reason);
    this.emit("close", code, reason);
  }

  private endConnection(socket: MewWebSocket, message: string): boolean {
    if (this.ws !== socket) return false;
    this.ws = null;
    this.openPromise = null;
    this.confirmedSessions.clear();
    this.browserFocusPreflight = null;
    this.protocolVersion = 0;
    this.scopedSessionFailures = false;
    this.sessionHistoryMessages = [];
    const wasSwitching = this.pendingSessionCommands > 0;
    this.pendingSessionCommands = 0;
    this.sessionCommandTail = Promise.resolve();
    const lifetime = this.connectionLifetime;
    this.connectionLifetime = null;
    lifetime?.abort(new Error(message));
    if (wasSwitching) this.emit("session-command-state");
    return true;
  }

  isConnected(): boolean {
    return this.ws !== null;
  }

  hasConfirmedSession(sessionId: string): boolean {
    return this.confirmedSessions.has(sessionId);
  }

  /** Commands for the active session need its current socket's confirmation. */
  isSessionAttached(sessionId: string): boolean {
    return this.sessionId === sessionId && this.hasConfirmedSession(sessionId);
  }

  /** True once the active attachment has no queued or in-flight lifecycle work. */
  isSessionStable(sessionId: string): boolean {
    return this.pendingSessionCommands === 0 && this.isSessionAttached(sessionId);
  }

  private requestSession(message: Extract<ClientMessage, { type: "new_session" | "attach_session" }>): Promise<string> {
    const connecting = this.connect();
    const signal = this.connectionLifetime?.signal;
    if (!signal) return connecting.then(() => { throw new Error("No daemon connection."); });
    const wasIdle = this.pendingSessionCommands++ === 0;
    const run = this.sessionCommandTail.then(async () => {
      await connecting;
      signal.throwIfAborted();
      const sessionId = message.type === "attach_session" ? message.session_id : null;
      if (sessionId !== null && this.isSessionAttached(sessionId)) return sessionId;
      return new Promise<string>((resolve, reject) => {
        const cleanup = () => {
          clearTimeout(timeout);
          signal.removeEventListener("abort", onDisconnect);
          this.off("session-ready", onReady);
          this.off("session-operation-failed", onFailure);
          this.off("errorMessage", onLegacyError);
        };
        const fail = (error: unknown) => { cleanup(); reject(error); };
        const onReady = (data: { session_id: string }) => {
          if (sessionId !== null && data.session_id !== sessionId) return;
          cleanup();
          resolve(data.session_id);
        };
        const onFailure = (data: SessionOperationFailed) => {
          if (data.operation !== (sessionId === null ? "create" : "attach") || data.session_id !== sessionId) return;
          fail(new Error(data.message));
        };
        const onLegacyError = ({ message }: { message: string }) => {
          if (!this.scopedSessionFailures) fail(new Error(message));
        };
        const onDisconnect = () => fail(signal.reason);
        const timeout = setTimeout(() => {
          fail(new Error("The daemon did not confirm the conversation request."));
          this.disconnect();
        }, 10000);
        signal.addEventListener("abort", onDisconnect, { once: true });
        this.on("session-ready", onReady);
        this.on("session-operation-failed", onFailure);
        this.on("errorMessage", onLegacyError);
        try { this.send(message); }
        catch (error) { fail(error); }
      });
    }).finally(() => {
      if (this.connectionLifetime?.signal !== signal) return;
      if (--this.pendingSessionCommands === 0) this.emit("session-command-state");
    });
    this.sessionCommandTail = run.then(
      () => undefined,
      () => undefined,
    );
    if (wasIdle) this.emit("session-command-state");
    return run;
  }

  /** Send `new_session`. Resolves with the daemon-assigned session id. */
  newSession(cwd: string | null = null): Promise<string> {
    return this.requestSession({ type: "new_session", cwd, client_kind: this.clientKind });
  }

  /** Send `prompt`. Streaming events are emitted via the registered handlers. */
  prompt(text: string, attachments: Attachment[] = []): void {
    this.send({ type: "prompt", text, attachments });
  }

  /** Resolve only when the daemon accepts the prompt in the specified session. */
  async submitPrompt(sessionId: string, text: string, attachments: Attachment[] = []): Promise<void> {
    const socket = this.ws;
    if (!socket || !this.isSessionAttached(sessionId)) throw new Error("This conversation is not attached to the daemon.");
    if (this.protocolVersion < 3) throw new Error("Update the daemon to enable confirmed prompt submission.");
    if (this.pendingPrompts.has(sessionId)) throw new Error("This conversation is already awaiting a prompt receipt.");
    const requestId = crypto.randomUUID();
    return new Promise((resolve, reject) => {
      const finish = (status: PromptReceipt["status"], message?: string) => {
        this.pendingPrompts.delete(sessionId);
        clearTimeout(pending.timeout);
        if (status === "accepted") resolve();
        else reject(new Error(message ?? (status === "unknown"
          ? "Submission could not be confirmed. Check the conversation before retrying."
          : "The daemon rejected the prompt.")));
      };
      const pending = { requestId, socket, finish, timeout: setTimeout(() => finish("unknown"), 15000) };
      this.pendingPrompts.set(sessionId, pending);
      try { this.send({ type: "submit_prompt", session_id: sessionId, request_id: requestId, text, attachments }); }
      catch (error) { finish("rejected", error instanceof Error ? error.message : undefined); }
    });
  }

  /** Compare and save one todo; resolve only after the daemon confirms it. */
  updateTodo(sessionId: string, edit: TodoEdit): Promise<Todo[]> {
    const requestId = crypto.randomUUID();
    return new Promise<Todo[]>((resolve, reject) => {
      const cleanup = () => {
        clearTimeout(timer);
        this.off("todo-updated", onSaved);
        this.off("todo-update-error", onError);
        this.off("close", onClose);
      };
      const onSaved = (data: { session_id: string; request_id: string; todos: Todo[] }) => {
        if (data.session_id !== sessionId || data.request_id !== requestId) return;
        cleanup();
        resolve(data.todos);
      };
      const onError = (data: { session_id: string; request_id: string; message: string }) => {
        if (data.session_id !== sessionId || data.request_id !== requestId) return;
        cleanup();
        reject(new Error(data.message));
      };
      const onClose = () => {
        cleanup();
        reject(new Error("daemon disconnected before confirming todo save"));
      };
      const timer = setTimeout(() => {
        cleanup();
        reject(new Error("todo save was not confirmed; update or reconnect the daemon before retrying"));
      }, 3000);
      this.on("todo-updated", onSaved);
      this.on("todo-update-error", onError);
      this.on("close", onClose);
      try {
        this.send({ type: "update_todo", session_id: sessionId, request_id: requestId, ...edit });
      } catch (error) {
        cleanup();
        reject(error);
      }
    });
  }

  /** Send `cancel` to abort the current turn. */
  cancel(): void {
    this.send({ type: "cancel" });
  }

  /** Inject guidance into the running turn's next request (steer the LLM).
   *  If no turn is running, it is picked up by the next turn. */
  guide(text: string): void {
    this.send({ type: "guide", text });
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

  /** Close one question while leaving the parent turn and other questions active. */
  cancelAskUser(request_id: string): void {
    this.send({ type: "ask_user_response", request_id, answers: [], cancelled: true });
  }

  /** Accept or reject a proposed goal using its original request ID. */
  respondToGoal(request_id: string, accepted: boolean): void {
    this.send({ type: "goal_response", request_id, accepted });
  }

  /** Attach to an existing session (active or idle). If the session is idle,
   *  the daemon loads its persisted history from disk and sends a
   *  `session-history` event. Resolves with the session id. */
  attachSession(session_id: string): Promise<string> {
    return this.requestSession({ type: "attach_session", session_id, client_kind: this.clientKind });
  }

  /** Request older messages for the attached session using the cursor from a
   *  previous history page. */
  loadSessionHistory(session_id: string, before: number, limit?: number): void {
    this.send({ type: "load_session_history", session_id, before, limit });
  }

  /** Read the next bounded UTF-8 chunk from a session-owned job. */
  readJobOutput(session_id: string, job_id: string, request_id: string, offset: number): void {
    this.send({ type: "read_job_output", session_id, job_id, request_id, offset });
  }

  /** Request a recorded child transcript without changing the attached session. */
  readSubagentHistory(session_id: string, child_session_id: string, request_id: string): void {
    this.send({ type: "read_subagent_history", session_id, child_session_id, request_id });
  }

  /** Send guidance to a running child of the attached conversation. */
  async guideSubagent(sessionId: string, childSessionId: string, text: string, requestId: string = crypto.randomUUID()): Promise<SubagentMessageReceipt> {
    if (!text.trim()) throw new Error("Write a message before sending guidance.");
    return this.requestSubagentReceipt({ type: "guide_subagent", session_id: sessionId, child_session_id: childSessionId, request_id: requestId, text });
  }

  /** Read a receipt without sending new guidance. */
  readSubagentMessageReceipt(sessionId: string, childSessionId: string, requestId: string): Promise<SubagentMessageReceipt> {
    return this.requestSubagentReceipt({ type: "read_subagent_message_receipt", session_id: sessionId, child_session_id: childSessionId, request_id: requestId });
  }

  private async requestSubagentReceipt(message: Extract<ClientMessage, { type: "guide_subagent" | "read_subagent_message_receipt" }>): Promise<SubagentMessageReceipt> {
    const { session_id, child_session_id, request_id } = message;
    if (!this.isSessionAttached(session_id)) throw new Error("The parent conversation is not attached.");
    const signal = this.connectionLifetime?.signal;
    return new Promise((resolve, reject) => {
      const cleanup = () => {
        clearTimeout(timeout);
        signal?.removeEventListener("abort", unconfirmed);
        this.off("subagent-message-receipt", onReceipt);
      };
      const unconfirmed = () => {
        cleanup();
        resolve({ type: "subagent_message_receipt", session_id, child_session_id, request_id, status: "unknown" });
      };
      const onReceipt = (receipt: SubagentMessageReceipt) => {
        if (receipt.session_id !== session_id || receipt.child_session_id !== child_session_id || receipt.request_id !== request_id) return;
        cleanup();
        resolve(receipt);
      };
      const timeout = setTimeout(unconfirmed, 10000);
      signal?.addEventListener("abort", unconfirmed, { once: true });
      this.on("subagent-message-receipt", onReceipt);
      try {
        this.send(message);
      } catch (error) {
        cleanup();
        reject(error);
      }
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

  /** Delete a conversation and wait for its matching daemon confirmation. */
  deleteSessionConfirmed(sessionId: string): Promise<void> {
    const requestId = crypto.randomUUID();
    const signal = this.connectionLifetime?.signal;
    return new Promise((resolve, reject) => {
      const cleanup = () => {
        clearTimeout(timeout);
        signal?.removeEventListener("abort", onDisconnect);
        this.off("session-deletion-result", onReply);
      };
      const fail = (error: unknown) => { cleanup(); reject(error); };
      const unconfirmed = () => fail(new Error("Deletion could not be confirmed. Refresh sessions before retrying."));
      const onDisconnect = () => unconfirmed();
      const onReply = (result: SessionDeletionResult) => {
        if (result.session_id !== sessionId || result.request_id !== requestId) return;
        if (result.error != null) {
          fail(new Error(result.error));
          return;
        }
        cleanup();
        this.confirmedSessions.delete(sessionId);
        if (this.sessionId === sessionId) {
          this.sessionId = null;
          this.sessionHistoryMessages = [];
        }
        resolve();
      };
      const timeout = setTimeout(unconfirmed, 10000);
      this.on("session-deletion-result", onReply);
      signal?.addEventListener("abort", onDisconnect, { once: true });
      try { this.send({ type: "delete_session_confirmed", session_id: sessionId, request_id: requestId }); }
      catch (error) { fail(error); }
    });
  }

  /** Rename a session (set a custom title). Persists to disk and broadcasts. */
  renameSession(session_id: string, title: string): void {
    this.send({ type: "rename_session", session_id, title });
  }

  /** Enable or disable auto-generated session titles. */
  setAutoTitle(enabled: boolean): void {
    this.send({ type: "set_auto_title", enabled });
  }

  readDaemonSettings(requestId: string): void {
    this.send({ type: "read_daemon_settings", request_id: requestId });
  }

  saveSessionDefaults(sessionId: string, requestId: string): void {
    this.send({ type: "save_session_defaults", session_id: sessionId, request_id: requestId });
  }

  resetSessionDefaults(requestId: string): void {
    this.send({ type: "reset_session_defaults", request_id: requestId });
  }

  revokeDevice(deviceId: string, requestId: string): void {
    this.send({ type: "revoke_device", device_id: deviceId, request_id: requestId });
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
    return new Promise<string>((resolve, reject) => {
      const signal = this.connectionLifetime?.signal;
      const cleanup = () => {
        clearTimeout(timeout);
        this.off("pong", onPong);
        signal?.removeEventListener("abort", onDisconnect);
      };
      const onPong = (data: { version: string }) => {
        cleanup();
        resolve(data.version);
      };
      const fail = (error: unknown) => { cleanup(); reject(error); };
      const onDisconnect = () => fail(signal?.reason);
      const timeout = setTimeout(() => fail(new Error("The daemon did not respond to its protocol check.")), 3000);
      signal?.addEventListener("abort", onDisconnect, { once: true });
      this.on("pong", onPong);
      try { this.send({ type: "ping" }); }
      catch (error) { fail(error); }
    });
  }

  /** List known projects (recent session cwds). */
  listProjects(): void {
    this.send({ type: "list_projects" });
  }

  async registerBrowserPage(sessionId: string, pageInstance: string, cdpEndpoint: string, targetId: string, focusUrl?: string): Promise<void> {
    if (focusUrl !== undefined) {
      const socket = this.ws;
      if (!socket || !this.hasConfirmedSession(sessionId)) throw new Error("Browser input requires a confirmed session on this connection.");
      if (!this.browserFocusPreflight) {
        const check = new Promise<boolean>((resolve, reject) => {
          const cleanup = () => {
            clearTimeout(timeout);
            this.off("pong", onPong);
            this.off("close", onConnectionChanged);
            this.off("error", onConnectionChanged);
          };
          const onPong = (data: { browser_focus_preflight: boolean }) => { cleanup(); resolve(data.browser_focus_preflight); };
          const onConnectionChanged = () => {
            if (this.ws === socket) return;
            cleanup();
            reject(new Error("The daemon disconnected before confirming browser focus support."));
          };
          const timeout = setTimeout(() => { cleanup(); reject(new Error("The daemon did not confirm browser focus support.")); }, 3000);
          this.on("pong", onPong);
          this.on("close", onConnectionChanged);
          this.on("error", onConnectionChanged);
          try { this.send({ type: "ping" }); }
          catch (error) { cleanup(); reject(error); }
        });
        this.browserFocusPreflight = check;
        void check.catch(() => { if (this.browserFocusPreflight === check) this.browserFocusPreflight = null; });
      }
      const supported = await this.browserFocusPreflight;
      if (this.ws !== socket || !this.hasConfirmedSession(sessionId)) throw new Error("The daemon connection changed before browser page registration.");
      if (!supported) throw new Error("Update the daemon to enable safe browser input.");
    }
    return this.browserPageRequest({ type: "register_browser_page", session_id: sessionId, page_instance: pageInstance, cdp_endpoint: cdpEndpoint, target_id: targetId, focus_url: focusUrl });
  }

  unregisterBrowserPage(sessionId: string, pageInstance: string): Promise<void> {
    return this.browserPageRequest({ type: "unregister_browser_page", session_id: sessionId, page_instance: pageInstance });
  }

  private browserPageRequest(message: Extract<ClientMessage, { type: "register_browser_page" | "unregister_browser_page" }>): Promise<void> {
    return new Promise((resolve, reject) => {
      const cleanup = () => {
        clearTimeout(timeout);
        this.off("browser-page-registration", onReply);
        this.off("close", onClose);
      };
      const onReply = (reply: { session_id: string; page_instance: string; error: string | null }) => {
        if (reply.session_id !== message.session_id || reply.page_instance !== message.page_instance) return;
        cleanup();
        if (reply.error) reject(new Error(reply.error));
        else resolve();
      };
      const onClose = () => { cleanup(); reject(new Error("The daemon disconnected while updating the browser page.")); };
      const timeout = setTimeout(() => { cleanup(); reject(new Error("The daemon did not acknowledge the browser page.")); }, 10000);
      this.on("browser-page-registration", onReply);
      this.on("close", onClose);
      try { this.send(message); }
      catch (error) { cleanup(); reject(error); }
    });
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
      case "daemon_settings":
        this.emit("daemon-settings", { request_id: msg.request_id, defaults: msg.defaults, devices: msg.devices });
        break;
      case "daemon_settings_failed":
        this.emit("daemon-settings-failed", { request_id: msg.request_id, message: msg.message });
        break;
      case "session_ready":
        this.sessionId = msg.session_id;
        this.confirmedSessions.add(msg.session_id);
        this.sessionHistoryMessages = [];
        this.emit("session-ready", {
          session_id: msg.session_id,
          cwd: msg.cwd,
          model: msg.model,
          provider: msg.provider,
          permission_mode: msg.permission_mode,
        });
        break;
      case "session_operation_failed":
        this.scopedSessionFailures = true;
        this.emit("session-operation-failed", msg);
        break;
      case "session_deletion_result":
        this.emit("session-deletion-result", msg);
        break;
      case "prompt_receipt": {
        const pending = this.pendingPrompts.get(msg.session_id);
        if (pending && pending.socket === this.ws && pending.requestId === msg.request_id) {
          pending.finish(msg.status, msg.message);
        }
        this.emit("prompt-receipt", msg);
        break;
      }
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
          ...(msg.subagent ? { subagent: msg.subagent } : {}),
          request_id: msg.request_id,
          call_id: msg.call_id,
          questions: msg.questions,
        });
        break;
      case "goal_proposed":
        this.emit("goal-proposed", {
          request_id: msg.request_id,
          call_id: msg.call_id,
          objective: msg.objective,
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
      case "todo_updated":
        this.emit("todo-updated", msg);
        break;
      case "todo_update_error":
        this.emit("todo-update-error", msg);
        break;
      case "todos_updated":
        this.emit("todos-updated", { todos: msg.todos });
        break;
      case "job_update":
        this.emit("job-update", {
          job_id: msg.job_id,
          command: msg.command,
          state: msg.state,
        });
        break;
      case "session_activity_snapshot":
        this.emit("session-activity-snapshot", {
          session_id: msg.session_id,
          subagents: msg.subagents,
          jobs: msg.jobs,
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
        this.sessionHistoryMessages = msg.messages;
        this.emit("session-history", { messages: msg.messages });
        if (this.sessionId && this.isSessionAttached(this.sessionId)) {
          this.emit("session-history-page", { session_id: this.sessionId, messages: msg.messages, replace: true });
        }
        break;
      case "job_output":
        this.emit("job-output", { session_id: msg.session_id, job_id: msg.job_id, request_id: msg.request_id, offset: msg.offset, next_offset: msg.next_offset, output: msg.output, done: msg.done, incomplete: msg.incomplete });
        break;
      case "job_output_error":
        this.emit("job-output-error", { session_id: msg.session_id, job_id: msg.job_id, request_id: msg.request_id, message: msg.message });
        break;
      case "subagent_history":
        this.emit("subagent-history", { session_id: msg.session_id, child_session_id: msg.child_session_id, request_id: msg.request_id, messages: msg.messages, error: msg.error });
        break;
      case "subagent_message_receipt":
        this.emit("subagent-message-receipt", msg);
        break;
      case "session_history_page": {
        if (!this.isSessionAttached(msg.session_id)) break;
        this.emit("session-history-page", msg);
        if (this.listeners.get("session-history")?.size) {
          this.sessionHistoryMessages = msg.replace
            ? msg.messages
            : [...msg.messages, ...this.sessionHistoryMessages];
          this.emit("session-history", { messages: this.sessionHistoryMessages });
        }
        break;
      }
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
          context_tokens: msg.context_tokens,
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
      case "pong": {
        this.scopedSessionFailures ||= msg.scoped_session_failures ?? false;
        this.protocolVersion = msg.protocol_version ?? 0;
        const socket = this.ws;
        for (const [sessionId, pending] of this.pendingPrompts) {
          if (!socket || pending.socket === socket) continue;
          if (this.protocolVersion < 3) {
            pending.finish("unknown");
            continue;
          }
          pending.socket = socket;
          try { this.send({ type: "read_prompt_receipt", session_id: sessionId, request_id: pending.requestId }); }
          catch { pending.finish("unknown"); }
        }
        this.emit("pong", { version: msg.version, protocol_version: this.protocolVersion, browser_focus_preflight: msg.browser_focus_preflight ?? false });
        break;
      }
      case "project_list":
        this.emit("project-list", { projects: msg.projects });
        break;
      case "browser_page_registration":
        this.emit("browser-page-registration", { session_id: msg.session_id, page_instance: msg.page_instance, error: msg.error });
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
    if (this.pendingSessionCommands > 0 && [
      "prompt", "cancel", "guide", "slash_command", "switch_model", "switch_persona",
      "set_thinking_variant", "set_permission_mode", "yield_control", "set_auto_title", "set_auto_summary",
    ].includes(msg.type)) {
      throw new Error("Wait until switching conversations finishes.");
    }
    if (!this.ws) throw new Error("not connected");
    if (this.debug) console.debug("[mew] ->", msg);
    this.ws.send(JSON.stringify(msg));
  }

  /** Return the last confirmed session ID, which can outlive its connection. */
  getSessionId(): string | null {
    return this.sessionId;
  }
}
