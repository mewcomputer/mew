export interface Attachment {
    path: string;
    mime?: string;
}
export type PermissionDecision = "allow_once" | "allow_session" | "deny";
export type ClientMessage = {
    type: "NewSession";
    cwd: string | null;
} | {
    type: "AttachSession";
    session_id: string;
} | {
    type: "ListSessions";
} | {
    type: "Prompt";
    text: string;
    attachments: Attachment[];
} | {
    type: "Cancel";
} | {
    type: "PermissionResponse";
    request_id: number;
    decision: PermissionDecision;
} | {
    type: "AskUserResponse";
    request_id: number;
    answers: string[];
} | {
    type: "SlashCommand";
    command: string;
} | {
    type: "ListModels";
} | {
    type: "SwitchModel";
    provider: string;
    model: string;
};
export type ProviderEventWire = {
    type: "PartStart";
    part: Part;
} | {
    type: "PartDelta";
    part_id: string;
    field: string;
    delta: string;
} | {
    type: "PartEnd";
    part_id: string;
} | {
    type: "MessageEnd";
    finish: "stop" | "tool_use" | "length" | "content_filter" | "error";
    usage: {
        input: number;
        output: number;
    };
    cost: number;
} | {
    type: "RetryWait";
    attempt: number;
    max_attempts: number;
    delay_secs: number;
    reason: string;
} | {
    type: "Error";
    error: MessageError;
};
export type Part = {
    type: "Text";
    base: PartBase;
    text: string;
    synthetic: boolean;
} | {
    type: "Reasoning";
    base: PartBase;
    text: string;
    signature?: string;
} | {
    type: "ToolCall";
    base: PartBase;
    tool_name: string;
    call_id: string;
    state: ToolState;
} | {
    type: "ToolResult";
    base: PartBase;
    call_id: string;
    output?: string;
} | {
    type: "File";
    base: PartBase;
    mime: string;
    filename?: string;
    url: string;
} | {
    type: "Compaction";
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
    type: "Pending";
    input: unknown;
    time: {
        start: number;
        end: number | null;
    };
} | {
    type: "Running";
    input: unknown;
    output: string;
    time: {
        start: number;
        end: number | null;
    };
} | {
    type: "Completed";
    input: unknown;
    output: string;
    time: {
        start: number;
        end: number | null;
    };
} | {
    type: "Error";
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
    type: "Completed";
} | {
    type: "Cancelled";
} | {
    type: "Failed";
    reason: string;
};
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
export type ServerMessage = {
    type: "SessionReady";
    session_id: string;
    model?: string;
    provider?: string;
} | {
    type: "Error";
    message: string;
} | {
    type: "Provider";
    event: ProviderEventWire;
} | {
    type: "ToolStart";
    call_id: string;
} | {
    type: "ToolEnd";
    call_id: string;
    success: boolean;
} | {
    type: "PartUpdated";
    part_id: string;
    part: Part;
} | {
    type: "ToolProgress";
    call_id: string;
    chunk: string;
} | {
    type: "ErrorEvent";
    message: string;
} | {
    type: "PermissionRequest";
    request_id: number;
    tool_name: string;
    input: Record<string, unknown>;
} | {
    type: "WorkspacePermissionRequest";
    request_id: number;
    path: string;
} | {
    type: "AskUserRequest";
    request_id: number;
    call_id: string;
    questions: Question[];
} | {
    type: "SubagentStart";
    parent_call_id: string;
    name: string;
    child_session_id: string;
    display_name: string | null;
} | {
    type: "SubagentStatus";
    parent_call_id: string;
    tool_name: string;
    message: string;
} | {
    type: "SubagentEnd";
    parent_call_id: string;
    child_session_id: string;
    outcome: SubagentOutcome;
} | {
    type: "SubagentPermissionRequest";
    request_id: number;
    parent_call_id: string;
    tool_name: string;
    input: Record<string, unknown>;
} | {
    type: "TodosUpdated";
    todos: Todo[];
} | {
    type: "PersonaSwitchRequested";
    name: string;
} | {
    type: "JobUpdate";
    job_id: string;
    command: string;
    state: string;
} | {
    type: "SlashResult";
    text: string;
} | {
    type: "RequestResolved";
    request_id: number;
} | {
    type: "SessionCleared";
} | {
    type: "SessionList";
    sessions: SessionInfo[];
} | {
    type: "SessionHistory";
    messages: Message[];
} | {
    type: "ModelList";
    models: ModelInfo[];
} | {
    type: "ModelSwitched";
    provider: string;
    model: string;
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
    }) => void;
    provider: (ev: ProviderEventWire) => void;
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
    /** Send `NewSession`. Resolves with the daemon-assigned session id. */
    newSession(cwd?: string | null): Promise<string>;
    /** Send `Prompt`. Streaming events are emitted via the registered handlers. */
    prompt(text: string, attachments?: Attachment[]): void;
    /** Send `Cancel` to abort the current turn. */
    cancel(): void;
    /**
     * Send a slash command (e.g. `/clear`, `/compact`). Returns the
     * `SlashResult.text` if the daemon produces one.
     */
    slashCommand(command: string): Promise<string | null>;
    /** Respond to a `PermissionRequest`. The callback in `on("permission-request", ...)` calls this. */
    respondToPermission(request_id: number, decision: PermissionDecision): void;
    /** Respond to an `AskUserRequest`. The UI calls this after the user
     *  submits answers to the questions. */
    respondToAskUser(request_id: number, answers: string[]): void;
    /** Attach to an existing session (active or idle). If the session is idle,
     *  the daemon loads its persisted history from disk and sends a
     *  `session-history` event. Resolves with the session id. */
    attachSession(session_id: string): Promise<string>;
    /** List all sessions known to the daemon (active + persisted idle).
     *  The daemon responds with a `session-list` event. */
    listSessions(): Promise<SessionInfo[]>;
    /** List available models from all configured providers. */
    listModels(): Promise<ModelInfo[]>;
    /** Switch the active session to a different model. Resolves when the
     *  daemon confirms via `model-switched`. */
    switchModel(provider: string, model: string): Promise<{
        provider: string;
        model: string;
    }>;
    on<E extends MewEventName>(event: E, cb: MewClientEvents[E]): void;
    off<E extends MewEventName>(event: E, cb: MewClientEvents[E]): void;
    private emit;
    private dispatch;
    private send;
    /** Return the active session id, or null if `newSession` hasn't succeeded. */
    getSessionId(): string | null;
}
//# sourceMappingURL=index.d.ts.map