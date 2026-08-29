export interface Attachment {
    path: string;
    mime?: string;
}
export type PermissionDecision = "allow_once" | "allow_session" | "deny";
export type ClientMessage = {
    type: "remote_hello";
    token?: string;
    device_name: string;
} | {
    type: "new_session";
    cwd: string | null;
    client_kind: string;
} | {
    type: "attach_session";
    session_id: string;
    client_kind: string;
} | {
    type: "list_sessions";
} | {
    type: "delete_session";
    session_id: string;
} | {
    type: "rename_session";
    session_id: string;
    title: string;
} | {
    type: "set_auto_title";
    enabled: boolean;
} | {
    type: "set_auto_summary";
    enabled: boolean;
} | {
    type: "prompt";
    text: string;
    attachments: Attachment[];
} | {
    type: "cancel";
} | {
    type: "guide";
    text: string;
} | {
    type: "permission_response";
    request_id: string;
    decision: PermissionDecision;
} | {
    type: "ask_user_response";
    request_id: string;
    answers: string[];
} | {
    type: "plan_approval_response";
    request_id: string;
    approved: boolean;
    feedback?: string;
} | {
    type: "slash_command";
    command: string;
} | {
    type: "list_models";
} | {
    type: "switch_model";
    provider: string;
    model: string;
} | {
    type: "list_personas";
} | {
    type: "switch_persona";
    name: string;
} | {
    type: "set_thinking_variant";
    variant: string;
} | {
    type: "set_permission_mode";
    mode: string;
} | {
    type: "yield_control";
} | {
    type: "create_group";
    name: string;
    color?: string;
} | {
    type: "update_group";
    group_id: string;
    name?: string;
    color?: string | null;
    order?: number;
} | {
    type: "delete_group";
    group_id: string;
} | {
    type: "assign_session_group";
    session_id: string;
    group_id?: string | null;
    position?: number;
} | {
    type: "archive_session";
    session_id: string;
    archived: boolean;
} | {
    type: "pin_session";
    session_id: string;
    pinned: boolean;
} | {
    type: "regenerate_title";
    session_id: string;
} | {
    type: "list_dir";
    session_id: string;
    path?: string;
} | {
    type: "read_file_preview";
    session_id: string;
    path: string;
    max_bytes?: number;
} | {
    type: "git_status";
    session_id: string;
} | {
    type: "watch_workspace";
    session_id: string;
    enabled: boolean;
} | {
    type: "open_path";
    session_id: string;
    path: string;
} | {
    type: "unflag_file";
    session_id: string;
    path: string;
} | {
    type: "ping";
} | {
    type: "list_projects";
} | {
    type: "list_filesystem_dir";
    path?: string;
} | {
    type: "browser_open";
    url: string;
    tab_id?: string;
} | {
    type: "browser_snapshot";
    tab_id?: string;
} | {
    type: "browser_screenshot";
    annotate: boolean;
    tab_id?: string;
} | {
    type: "browser_click";
    selector: string;
    tab_id?: string;
} | {
    type: "browser_fill";
    selector: string;
    text: string;
    tab_id?: string;
} | {
    type: "browser_press";
    key: string;
    tab_id?: string;
} | {
    type: "browser_close";
    tab_id?: string;
};
export type ProviderEventWire = {
    type: "part_start";
    part: Part;
} | {
    type: "part_delta";
    part_id: string;
    field: string;
    delta: string;
} | {
    type: "part_end";
    part_id: string;
} | {
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
} | {
    type: "retry_wait";
    attempt: number;
    max_attempts: number;
    delay_secs: number;
    reason: string;
} | {
    type: "error";
    error: MessageError;
};
export type Part = {
    type: "text";
    base: PartBase;
    text: string;
    synthetic: boolean;
} | {
    type: "reasoning";
    base: PartBase;
    text: string;
    signature?: string;
} | {
    type: "tool_call";
    base: PartBase;
    tool_name: string;
    call_id: string;
    state: ToolState;
} | {
    type: "tool_result";
    base: PartBase;
    call_id: string;
    output?: string;
} | {
    type: "file";
    base: PartBase;
    mime: string;
    filename?: string;
    url: string;
} | {
    type: "compaction";
    base: PartBase;
    auto: boolean;
    overflow: boolean;
    summary?: string;
    removed_count?: number;
};
export interface PartBase {
    id: string;
    message_id: string;
    session_id: string;
}
export type ToolState = {
    type: "pending";
    input: unknown;
    time: {
        start: number;
        end: number | null;
    };
} | {
    type: "running";
    input: unknown;
    output: string;
    time: {
        start: number;
        end: number | null;
    };
} | {
    type: "completed";
    input: unknown;
    output: string;
    time: {
        start: number;
        end: number | null;
    };
} | {
    type: "error";
    input: unknown;
    error: string;
    time: {
        start: number;
        end: number | null;
    };
};
export type ErrorKind = "provider_auth" | "rate_limit" | "invalid_request" | "tool_exec" | "tool_timeout" | "mcp_transport" | "network" | "unknown";
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
export type SubagentOutcome = {
    type: "completed";
} | {
    type: "cancelled";
} | {
    type: "failed";
    reason: string;
};
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
export type ServerMessage = {
    type: "remote_ready";
    scope: "observe" | "collaborate" | "control";
} | {
    type: "session_ready";
    session_id: string;
    cwd?: string;
    model?: string;
    provider?: string;
    permission_mode?: string;
} | {
    type: "error";
    message: string;
} | {
    type: "provider";
    event: ProviderEventWire;
} | {
    type: "user_message";
    text: string;
} | {
    type: "tool_start";
    call_id: string;
} | {
    type: "tool_end";
    call_id: string;
    success: boolean;
} | {
    type: "part_updated";
    part_id: string;
    part: Part;
} | {
    type: "tool_progress";
    call_id: string;
    chunk: string;
} | {
    type: "error_event";
    message: string;
} | {
    type: "permission_request";
    request_id: string;
    tool_name: string;
    input: Record<string, unknown>;
} | {
    type: "workspace_permission_request";
    request_id: string;
    path: string;
} | {
    type: "ask_user_request";
    request_id: string;
    call_id: string;
    questions: Question[];
} | {
    type: "plan_approval_request";
    request_id: string;
    call_id: string;
    plan_path: string;
    plan_markdown: string;
    persona: string;
} | {
    type: "subagent_start";
    parent_call_id: string;
    name: string;
    child_session_id: string;
    display_name: string | null;
} | {
    type: "subagent_status";
    parent_call_id: string;
    tool_name: string;
    message: string;
} | {
    type: "subagent_end";
    parent_call_id: string;
    child_session_id: string;
    outcome: SubagentOutcome;
    manifests?: TurnManifest[];
} | {
    type: "subagent_permission_request";
    request_id: string;
    parent_call_id: string;
    tool_name: string;
    input: Record<string, unknown>;
} | {
    type: "todos_updated";
    todos: Todo[];
} | {
    type: "persona_switch_requested";
    name: string;
} | {
    type: "job_update";
    job_id: string;
    command: string;
    state: string;
} | {
    type: "slash_result";
    text: string;
} | {
    type: "request_resolved";
    request_id: string;
} | {
    type: "session_cleared";
} | {
    type: "session_list";
    sessions: SessionInfo[];
} | {
    type: "session_history";
    messages: Message[];
} | {
    type: "model_list";
    models: ModelInfo[];
} | {
    type: "model_switched";
    provider: string;
    model: string;
} | {
    type: "persona_list";
    personas: PersonaInfo[];
} | {
    type: "persona_switched";
    name: string;
} | {
    type: "thinking_variant_changed";
    variant?: string;
} | {
    type: "permission_mode_changed";
    mode: string;
} | {
    type: "client_attached";
    client_id: number;
    client_kind: string;
} | {
    type: "client_detached";
    client_id: number;
} | {
    type: "control_yielded";
    client_id: number;
} | {
    type: "session_title_changed";
    session_id: string;
    title: string;
} | {
    type: "session_summary_changed";
    session_id: string;
    summary: string;
} | {
    type: "session_activity_changed";
    session_id: string;
    activity: SessionState;
} | {
    type: "session_stats_changed";
    session_id: string;
    added: number;
    removed: number;
    files_changed: number;
} | {
    type: "group_list";
    groups: GroupInfo[];
} | {
    type: "groups_changed";
    groups: GroupInfo[];
} | {
    type: "dir_listing";
    path: string;
    entries: DirEntry[];
} | {
    type: "filesystem_dir_listing";
    path: string;
    entries: DirEntry[];
} | {
    type: "file_preview";
    path: string;
    content: string;
    truncated: boolean;
    language?: string;
} | {
    type: "git_status_result";
    entries: GitEntry[];
} | {
    type: "fs_changed";
    paths: string[];
} | {
    type: "session_usage_changed";
    session_id: string;
    usage: SessionUsageWire;
    context_tokens?: number;
} | {
    type: "session_alert";
    session_id: string;
    title: string;
    kind: AlertKind;
    detail?: string;
} | {
    type: "flagged_files_changed";
    session_id: string;
    files: FlaggedFileWire[];
} | {
    type: "session_meta_changed";
    session_id: string;
    archived: boolean;
    pinned: boolean;
    group_id?: string;
} | {
    type: "session_attention_changed";
    session_id: string;
    pending_permissions: number;
    pending_questions: number;
} | {
    type: "pong";
    version: string;
} | {
    type: "project_list";
    projects: ProjectInfo[];
} | {
    type: "browser_snapshot";
    snapshot: string;
    url: string;
    title: string;
    tab_id?: string;
} | {
    type: "browser_screenshot";
    data: string;
    url: string;
    tab_id?: string;
} | {
    type: "browser_state";
    open: boolean;
    url?: string;
    title?: string;
    tab_id?: string;
} | {
    type: "browser_error";
    message: string;
    tab_id?: string;
};
export interface MewWebSocket {
    send(data: string): void;
    close(code?: number, reason?: string): void;
    addEventListener(type: "open", listener: () => void): void;
    addEventListener(type: "close", listener: (ev: {
        code: number;
        reason: string;
    }) => void): void;
    addEventListener(type: "error", listener: (ev: unknown) => void): void;
    addEventListener(type: "message", listener: (ev: {
        data: string;
    }) => void): void;
    removeEventListener(type: string, listener: (...args: unknown[]) => void): void;
}
export type SocketFactory = (url: string) => MewWebSocket;
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
    "user-message": (data: {
        text: string;
    }) => void;
    "tool-start": (data: {
        call_id: string;
    }) => void;
    "tool-end": (data: {
        call_id: string;
        success: boolean;
    }) => void;
    "part-updated": (data: {
        part_id: string;
        part: Part;
    }) => void;
    "tool-progress": (data: {
        call_id: string;
        chunk: string;
    }) => void;
    "permission-request": (data: {
        request_id: string;
        tool_name: string;
        input: Record<string, unknown>;
    }, respond: (decision: PermissionDecision) => void) => void;
    "workspace-permission-request": (data: {
        request_id: string;
        path: string;
    }, respond: (decision: PermissionDecision) => void) => void;
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
    "todos-updated": (data: {
        todos: Todo[];
    }) => void;
    "persona-switch-requested": (data: {
        name: string;
    }) => void;
    "job-update": (data: {
        job_id: string;
        command: string;
        state: string;
    }) => void;
    "slash-result": (data: {
        text: string;
    }) => void;
    "request-resolved": (data: {
        request_id: string;
    }) => void;
    "session-cleared": () => void;
    "session-list": (data: {
        sessions: SessionInfo[];
    }) => void;
    "session-history": (data: {
        messages: Message[];
    }) => void;
    "model-list": (data: {
        models: ModelInfo[];
    }) => void;
    "model-switched": (data: {
        provider: string;
        model: string;
    }) => void;
    "persona-list": (data: {
        personas: PersonaInfo[];
    }) => void;
    "persona-switched": (data: {
        name: string;
    }) => void;
    "thinking-variant-changed": (data: {
        variant: string | null;
    }) => void;
    "permission-mode-changed": (data: {
        mode: string;
    }) => void;
    "client-attached": (data: {
        client_id: number;
        client_kind: string;
    }) => void;
    "client-detached": (data: {
        client_id: number;
    }) => void;
    "control-yielded": (data: {
        client_id: number;
    }) => void;
    "session-title-changed": (data: {
        session_id: string;
        title: string;
    }) => void;
    "session-summary-changed": (data: {
        session_id: string;
        summary: string;
    }) => void;
    "session-activity-changed": (data: {
        session_id: string;
        activity: SessionState;
    }) => void;
    "session-stats-changed": (data: {
        session_id: string;
        added: number;
        removed: number;
        files_changed: number;
    }) => void;
    "group-list": (data: {
        groups: GroupInfo[];
    }) => void;
    "groups-changed": (data: {
        groups: GroupInfo[];
    }) => void;
    "dir-listing": (data: {
        path: string;
        entries: DirEntry[];
    }) => void;
    "filesystem-dir-listing": (data: {
        path: string;
        entries: DirEntry[];
    }) => void;
    "file-preview": (data: {
        path: string;
        content: string;
        truncated: boolean;
        language?: string;
    }) => void;
    "git-status-result": (data: {
        entries: GitEntry[];
    }) => void;
    "fs-changed": (data: {
        paths: string[];
    }) => void;
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
    "flagged-files-changed": (data: {
        session_id: string;
        files: FlaggedFileWire[];
    }) => void;
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
    errorMessage: (data: {
        message: string;
    }) => void;
    errorEvent: (data: {
        message: string;
    }) => void;
    pong: (data: {
        version: string;
    }) => void;
    "project-list": (data: {
        projects: ProjectInfo[];
    }) => void;
    "browser-snapshot": (data: {
        snapshot: string;
        url: string;
        title: string;
        tabId?: string;
    }) => void;
    "browser-screenshot": (data: {
        data: string;
        url: string;
        tabId?: string;
    }) => void;
    "browser-state": (data: {
        open: boolean;
        url?: string;
        title?: string;
        tabId?: string;
    }) => void;
    "browser-error": (data: {
        message: string;
        tabId?: string;
    }) => void;
    "remote-ready": (data: {
        scope: "observe" | "collaborate" | "control";
    }) => void;
}
export type MewEventName = keyof MewClientEvents;
export interface MewClientOptions {
    /** Override how the WebSocket is constructed (e.g. inject `ws` in Node). */
    socketFactory?: SocketFactory;
    /** If true, log every wire message to the console. Useful for debugging. */
    debug?: boolean;
    /** Client identity used for capability-gated daemon features. */
    clientKind?: "web" | "desktop" | "remote";
    /** Pairing credentials for a client connecting to an explicit remote daemon. */
    remoteAuth?: {
        token: string;
        deviceName: string;
    };
}
/**
 * Client for the mew daemon wire protocol. One client == one connection ==
 * one session. To run multiple sessions concurrently, create multiple
 * `MewClient` instances.
 */
export declare class MewClient {
    private readonly url;
    private readonly socketFactory;
    private readonly debug;
    private readonly clientKind;
    private readonly remoteAuth?;
    private ws;
    private listeners;
    /** Promise resolved when the WebSocket opens. */
    private openPromise;
    /** Session id returned by `newSession`. */
    private sessionId;
    /** Session lifecycle requests share uncorrelated daemon errors. */
    private sessionCommandTail;
    constructor(url: string, opts?: MewClientOptions);
    /** Open the WebSocket connection. Idempotent. */
    connect(): Promise<void>;
    /** Close the WebSocket. After calling this, the client cannot be reused. */
    disconnect(code?: number, reason?: string): void;
    isConnected(): boolean;
    /** Serialize lifecycle requests because daemon errors have no request id. */
    private enqueueSessionCommand;
    /** Send `new_session`. Resolves with the daemon-assigned session id. */
    newSession(cwd?: string | null): Promise<string>;
    /** Send `prompt`. Streaming events are emitted via the registered handlers. */
    prompt(text: string, attachments?: Attachment[]): void;
    /** Send `cancel` to abort the current turn. */
    cancel(): void;
    /** Inject guidance into the running turn's next request (steer the LLM).
     *  If no turn is running, it is picked up by the next turn. */
    guide(text: string): void;
    /**
     * Send a slash command (e.g. `/clear`, `/compact`). Returns the
     * `slash_result.text` if the daemon produces one.
     */
    slashCommand(command: string): Promise<string | null>;
    /** Respond to a `permission_request`. The callback in `on("permission-request", ...)` calls this. */
    respondToPermission(request_id: string, decision: PermissionDecision): void;
    /** Respond to an `ask_user_request`. The UI calls this after the user
     *  submits answers to the questions. */
    respondToAskUser(request_id: string, answers: string[]): void;
    /** Respond to a `plan_approval_request`. `approved = false` with optional
     *  `feedback` requests changes to the plan. */
    respondToPlanApproval(request_id: string, approved: boolean, feedback?: string): void;
    /** Attach to an existing session (active or idle). If the session is idle,
     *  the daemon loads its persisted history from disk and sends a
     *  `session-history` event. Resolves with the session id. */
    attachSession(session_id: string): Promise<string>;
    /** List all sessions known to the daemon (active + persisted idle).
     *  The daemon responds with a `session-list` event. */
    listSessions(): Promise<SessionInfo[]>;
    /** Delete a session from disk and remove it from the active list. */
    deleteSession(session_id: string): void;
    /** Rename a session (set a custom title). Persists to disk and broadcasts. */
    renameSession(session_id: string, title: string): void;
    /** Enable or disable auto-generated session titles. */
    setAutoTitle(enabled: boolean): void;
    setAutoSummary(enabled: boolean): void;
    /** List available models from all configured providers. */
    listModels(): Promise<ModelInfo[]>;
    /** Switch the active session to a different model. Resolves when the
     *  daemon confirms via `model-switched`. */
    switchModel(provider: string, model: string): Promise<{
        provider: string;
        model: string;
    }>;
    /** List available personas for the active session. Resolves when the
     *  daemon replies with `persona-list`. */
    listPersonas(): Promise<PersonaInfo[]>;
    /** Switch the active session to a different persona. Fire-and-forget:
     *  the store is updated when the daemon confirms via `persona-switched`
     *  (handled by the bridge), so the caller doesn't need to await. */
    switchPersona(name: string): void;
    /** Set or clear the thinking/reasoning variant. Pass empty string or
     *  "none" to disable. Numeric token budgets ride this call as the string
     *  convention `"budget:<n>"` (e.g. `"budget:8192"`); use
     *  `setThinkingBudget` for that. Resolves when the daemon confirms via
     *  `thinking-variant-changed`. Returns the resolved variant name, or
     *  null if thinking was disabled. */
    setThinkingVariant(variant: string): Promise<string | null>;
    /** Set a numeric token budget for thinking via `setThinkingVariant`
     *  (`"budget:<n>"`). Only valid for models that declare a
     *  `thinking_budget` range. */
    setThinkingBudget(tokens: number): Promise<string | null>;
    /** Set the permission mode for the active session. Mode is one of:
     *  "standard", "permissive", "auto", "auto_plus", "dangerous".
     *  Resolves when the daemon confirms via `permission-mode-changed`. */
    setPermissionMode(mode: string): Promise<string | null>;
    /** Yield control of the session. Advisory — other clients can become active. */
    yieldControl(): void;
    createGroup(name: string, color?: string): void;
    updateGroup(groupId: string, opts: {
        name?: string;
        color?: string | null;
        order?: number;
    }): void;
    deleteGroup(groupId: string): void;
    assignSessionGroup(sessionId: string, groupId: string | null, position?: number): void;
    archiveSession(sessionId: string, archived: boolean): void;
    pinSession(sessionId: string, pinned: boolean): void;
    /** Regenerate the session title from the first user message via LLM.
     *  The daemon broadcasts `session-title-changed` when done. */
    regenerateTitle(sessionId: string): void;
    listDir(sessionId: string, path?: string): void;
    listFilesystemDir(path?: string): void;
    readFilePreview(sessionId: string, path: string, maxBytes?: number): void;
    gitStatus(sessionId: string): void;
    watchWorkspace(sessionId: string, enabled: boolean): void;
    openPath(sessionId: string, path: string): void;
    unflagFile(sessionId: string, path: string): void;
    /** Ping the daemon; resolves with the daemon version once a pong arrives. */
    ping(): Promise<string>;
    /** List known projects (recent session cwds). */
    listProjects(): void;
    browserOpen(url: string, tabId?: string): void;
    browserSnapshot(tabId?: string): void;
    browserScreenshot(annotate?: boolean, tabId?: string): void;
    browserClick(selector: string, tabId?: string): void;
    browserFill(selector: string, text: string, tabId?: string): void;
    browserPress(key: string, tabId?: string): void;
    browserClose(tabId?: string): void;
    on<E extends MewEventName>(event: E, cb: MewClientEvents[E]): void;
    off<E extends MewEventName>(event: E, cb: MewClientEvents[E]): void;
    private emit;
    private dispatch;
    private send;
    /** Return the active session id, or null if `newSession` hasn't succeeded. */
    getSessionId(): string | null;
}
//# sourceMappingURL=index.d.ts.map