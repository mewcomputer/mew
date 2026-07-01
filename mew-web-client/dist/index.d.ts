export interface Attachment {
    path: string;
    mime?: string;
}
export type PermissionDecision = "allow_once" | "allow_session" | "deny";
export type ClientMessage = {
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
    type: "permission_response";
    request_id: number;
    decision: PermissionDecision;
} | {
    type: "ask_user_response";
    request_id: number;
    answers: string[];
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
    type: "set_thinking_variant";
    variant: string;
} | {
    type: "set_permission_mode";
    mode: string;
} | {
    type: "yield_control";
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
    finish: "stop" | "tool_use" | "length" | "content_filter" | "error";
    usage: {
        input: number;
        output: number;
    };
    cost: number;
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
}
/** A named thinking/reasoning variant (e.g. "high", "max", "thinking"). */
export interface ThinkingVariantInfo {
    name: string;
}
/** Session lifecycle state. */
export type SessionState = "active" | "idle";
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
export type ServerMessage = {
    type: "session_ready";
    session_id: string;
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
    request_id: number;
    tool_name: string;
    input: Record<string, unknown>;
} | {
    type: "workspace_permission_request";
    request_id: number;
    path: string;
} | {
    type: "ask_user_request";
    request_id: number;
    call_id: string;
    questions: Question[];
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
} | {
    type: "subagent_permission_request";
    request_id: number;
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
    request_id: number;
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
        request_id: number;
        tool_name: string;
        input: Record<string, unknown>;
    }, respond: (decision: PermissionDecision) => void) => void;
    "workspace-permission-request": (data: {
        request_id: number;
        path: string;
    }, respond: (decision: PermissionDecision) => void) => void;
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
        request_id: number;
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
    errorMessage: (data: {
        message: string;
    }) => void;
    errorEvent: (data: {
        message: string;
    }) => void;
}
export type MewEventName = keyof MewClientEvents;
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
export declare class MewClient {
    private readonly url;
    private readonly socketFactory;
    private readonly debug;
    private ws;
    private listeners;
    /** Promise resolved when the WebSocket opens. */
    private openPromise;
    /** Session id returned by `newSession`. */
    private sessionId;
    constructor(url: string, opts?: MewClientOptions);
    /** Open the WebSocket connection. Idempotent. */
    connect(): Promise<void>;
    /** Close the WebSocket. After calling this, the client cannot be reused. */
    disconnect(code?: number, reason?: string): void;
    isConnected(): boolean;
    /** Send `new_session`. Resolves with the daemon-assigned session id. */
    newSession(cwd?: string | null): Promise<string>;
    /** Send `prompt`. Streaming events are emitted via the registered handlers. */
    prompt(text: string, attachments?: Attachment[]): void;
    /** Send `cancel` to abort the current turn. */
    cancel(): void;
    /**
     * Send a slash command (e.g. `/clear`, `/compact`). Returns the
     * `slash_result.text` if the daemon produces one.
     */
    slashCommand(command: string): Promise<string | null>;
    /** Respond to a `permission_request`. The callback in `on("permission-request", ...)` calls this. */
    respondToPermission(request_id: number, decision: PermissionDecision): void;
    /** Respond to an `ask_user_request`. The UI calls this after the user
     *  submits answers to the questions. */
    respondToAskUser(request_id: number, answers: string[]): void;
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
    /** Set or clear the thinking/reasoning variant. Pass empty string or
     *  "none" to disable. Resolves when the daemon confirms via
     *  `thinking-variant-changed`. Returns the resolved variant name, or
     *  null if thinking was disabled. */
    setThinkingVariant(variant: string): Promise<string | null>;
    /** Set the permission mode for the active session. Mode is one of:
     *  "standard", "permissive", "auto", "auto_plus", "dangerous".
     *  Resolves when the daemon confirms via `permission-mode-changed`. */
    setPermissionMode(mode: string): Promise<string | null>;
    /** Yield control of the session. Advisory — other clients can become active. */
    yieldControl(): void;
    on<E extends MewEventName>(event: E, cb: MewClientEvents[E]): void;
    off<E extends MewEventName>(event: E, cb: MewClientEvents[E]): void;
    private emit;
    private dispatch;
    private send;
    /** Return the active session id, or null if `newSession` hasn't succeeded. */
    getSessionId(): string | null;
}
//# sourceMappingURL=index.d.ts.map