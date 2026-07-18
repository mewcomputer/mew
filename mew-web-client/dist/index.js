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
const defaultSocketFactory = (url) => {
    if (typeof WebSocket === "undefined") {
        throw new Error("No WebSocket implementation available. In Node, pass `socketFactory` using the `ws` package.");
    }
    return new WebSocket(url);
};
/**
 * Client for the mew daemon wire protocol. One client == one connection ==
 * one session. To run multiple sessions concurrently, create multiple
 * `MewClient` instances.
 */
export class MewClient {
    url;
    socketFactory;
    debug;
    ws = null;
    listeners = new Map();
    /** Promise resolved when the WebSocket opens. */
    openPromise = null;
    /** Session id returned by `newSession`. */
    sessionId = null;
    /** Session lifecycle requests share uncorrelated daemon errors. */
    sessionCommandTail = Promise.resolve();
    constructor(url, opts = {}) {
        this.url = url;
        this.socketFactory = opts.socketFactory ?? defaultSocketFactory;
        this.debug = opts.debug ?? false;
    }
    /** Open the WebSocket connection. Idempotent. */
    connect() {
        if (this.openPromise)
            return this.openPromise;
        this.openPromise = new Promise((resolve, reject) => {
            let settled = false;
            const ws = this.socketFactory(this.url);
            this.ws = ws;
            ws.addEventListener("open", () => {
                if (this.debug)
                    console.debug("[mew] open");
                settled = true;
                this.emit("open");
                resolve();
            });
            ws.addEventListener("message", (ev) => {
                try {
                    const msg = JSON.parse(ev.data);
                    if (this.debug)
                        console.debug("[mew] <-", msg);
                    this.dispatch(msg);
                }
                catch (e) {
                    this.emit("error", e);
                }
            });
            ws.addEventListener("close", (ev) => {
                if (this.debug)
                    console.debug("[mew] close", ev);
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
                if (this.debug)
                    console.debug("[mew] error", ev);
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
    isConnected() {
        return this.ws !== null;
    }
    /** Serialize lifecycle requests because daemon errors have no request id. */
    enqueueSessionCommand(command) {
        const run = this.sessionCommandTail.then(command, command);
        this.sessionCommandTail = run.then(() => undefined, () => undefined);
        return run;
    }
    /** Send `new_session`. Resolves with the daemon-assigned session id. */
    newSession(cwd = null) {
        return this.enqueueSessionCommand(async () => {
            await this.connect();
            return new Promise((resolve, reject) => {
                const onReady = (data) => {
                    this.sessionId = data.session_id;
                    this.off("session-ready", onReady);
                    resolve(data.session_id);
                };
                const onError = (msg) => {
                    this.off("session-ready", onReady);
                    this.off("errorMessage", onError);
                    reject(new Error(msg.message));
                };
                this.on("session-ready", onReady);
                this.on("errorMessage", onError);
                this.send({ type: "new_session", cwd, client_kind: "web" });
            });
        });
    }
    /** Send `prompt`. Streaming events are emitted via the registered handlers. */
    prompt(text, attachments = []) {
        this.send({ type: "prompt", text, attachments });
    }
    /** Send `cancel` to abort the current turn. */
    cancel() {
        this.send({ type: "cancel" });
    }
    /**
     * Send a slash command (e.g. `/clear`, `/compact`). Returns the
     * `slash_result.text` if the daemon produces one.
     */
    slashCommand(command) {
        return new Promise((resolve) => {
            const onResult = (data) => {
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
    respondToPermission(request_id, decision) {
        this.send({ type: "permission_response", request_id, decision });
    }
    /** Respond to an `ask_user_request`. The UI calls this after the user
     *  submits answers to the questions. */
    respondToAskUser(request_id, answers) {
        this.send({ type: "ask_user_response", request_id, answers });
    }
    /** Respond to a `plan_approval_request`. `approved = false` with optional
     *  `feedback` requests changes to the plan. */
    respondToPlanApproval(request_id, approved, feedback) {
        this.send({ type: "plan_approval_response", request_id, approved, feedback });
    }
    /** Attach to an existing session (active or idle). If the session is idle,
     *  the daemon loads its persisted history from disk and sends a
     *  `session-history` event. Resolves with the session id. */
    attachSession(session_id) {
        return this.enqueueSessionCommand(async () => {
            await this.connect();
            return new Promise((resolve, reject) => {
                const onReady = (data) => {
                    this.sessionId = data.session_id;
                    this.off("session-ready", onReady);
                    this.off("errorMessage", onError);
                    resolve(data.session_id);
                };
                const onError = (msg) => {
                    this.off("session-ready", onReady);
                    this.off("errorMessage", onError);
                    reject(new Error(msg.message));
                };
                this.on("session-ready", onReady);
                this.on("errorMessage", onError);
                this.send({ type: "attach_session", session_id, client_kind: "web" });
            });
        });
    }
    /** List all sessions known to the daemon (active + persisted idle).
     *  The daemon responds with a `session-list` event. */
    listSessions() {
        return new Promise((resolve) => {
            const onList = (data) => {
                this.off("session-list", onList);
                resolve(data.sessions);
            };
            this.on("session-list", onList);
            this.send({ type: "list_sessions" });
        });
    }
    /** Delete a session from disk and remove it from the active list. */
    deleteSession(session_id) {
        this.send({ type: "delete_session", session_id });
    }
    /** Rename a session (set a custom title). Persists to disk and broadcasts. */
    renameSession(session_id, title) {
        this.send({ type: "rename_session", session_id, title });
    }
    /** Enable or disable auto-generated session titles. */
    setAutoTitle(enabled) {
        this.send({ type: "set_auto_title", enabled });
    }
    setAutoSummary(enabled) {
        this.send({ type: "set_auto_summary", enabled });
    }
    /** List available models from all configured providers. */
    listModels() {
        return new Promise((resolve) => {
            const onList = (data) => {
                this.off("model-list", onList);
                resolve(data.models);
            };
            this.on("model-list", onList);
            this.send({ type: "list_models" });
        });
    }
    /** Switch the active session to a different model. Resolves when the
     *  daemon confirms via `model-switched`. */
    switchModel(provider, model) {
        return new Promise((resolve) => {
            const onSwitched = (data) => {
                this.off("model-switched", onSwitched);
                resolve(data);
            };
            this.on("model-switched", onSwitched);
            this.send({ type: "switch_model", provider, model });
        });
    }
    /** List available personas for the active session. Resolves when the
     *  daemon replies with `persona-list`. */
    listPersonas() {
        return new Promise((resolve) => {
            const onList = (data) => {
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
    switchPersona(name) {
        this.send({ type: "switch_persona", name });
    }
    /** Set or clear the thinking/reasoning variant. Pass empty string or
     *  "none" to disable. Resolves when the daemon confirms via
     *  `thinking-variant-changed`. Returns the resolved variant name, or
     *  null if thinking was disabled. */
    setThinkingVariant(variant) {
        return new Promise((resolve) => {
            const onChanged = (data) => {
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
    setPermissionMode(mode) {
        return new Promise((resolve) => {
            const onChanged = (data) => {
                this.off("permission-mode-changed", onChanged);
                resolve(data.mode);
            };
            this.on("permission-mode-changed", onChanged);
            this.send({ type: "set_permission_mode", mode });
        });
    }
    /** Yield control of the session. Advisory — other clients can become active. */
    yieldControl() {
        this.send({ type: "yield_control" });
    }
    // -- Phase 2: groups & archive --
    createGroup(name, color) {
        this.send({ type: "create_group", name, color });
    }
    updateGroup(groupId, opts) {
        this.send({ type: "update_group", group_id: groupId, ...opts });
    }
    deleteGroup(groupId) {
        this.send({ type: "delete_group", group_id: groupId });
    }
    assignSessionGroup(sessionId, groupId, position) {
        this.send({
            type: "assign_session_group",
            session_id: sessionId,
            group_id: groupId,
            position,
        });
    }
    archiveSession(sessionId, archived) {
        this.send({ type: "archive_session", session_id: sessionId, archived });
    }
    pinSession(sessionId, pinned) {
        this.send({ type: "pin_session", session_id: sessionId, pinned });
    }
    /** Regenerate the session title from the first user message via LLM.
     *  The daemon broadcasts `session-title-changed` when done. */
    regenerateTitle(sessionId) {
        this.send({ type: "regenerate_title", session_id: sessionId });
    }
    // -- Phase 3: file service --
    listDir(sessionId, path) {
        this.send({ type: "list_dir", session_id: sessionId, path });
    }
    readFilePreview(sessionId, path, maxBytes) {
        this.send({ type: "read_file_preview", session_id: sessionId, path, max_bytes: maxBytes });
    }
    gitStatus(sessionId) {
        this.send({ type: "git_status", session_id: sessionId });
    }
    watchWorkspace(sessionId, enabled) {
        this.send({ type: "watch_workspace", session_id: sessionId, enabled });
    }
    openPath(sessionId, path) {
        this.send({ type: "open_path", session_id: sessionId, path });
    }
    unflagFile(sessionId, path) {
        this.send({ type: "unflag_file", session_id: sessionId, path });
    }
    /** Ping the daemon; resolves with the daemon version once a pong arrives. */
    ping() {
        return new Promise((resolve) => {
            const handler = (data) => {
                this.off("pong", handler);
                resolve(data.version);
            };
            this.on("pong", handler);
            this.send({ type: "ping" });
        });
    }
    /** List known projects (recent session cwds). */
    listProjects() {
        this.send({ type: "list_projects" });
    }
    browserOpen(url, tabId) { this.send({ type: "browser_open", url, tab_id: tabId }); }
    browserSnapshot(tabId) { this.send({ type: "browser_snapshot", tab_id: tabId }); }
    browserScreenshot(annotate = false, tabId) { this.send({ type: "browser_screenshot", annotate, tab_id: tabId }); }
    browserClick(selector, tabId) { this.send({ type: "browser_click", selector, tab_id: tabId }); }
    browserFill(selector, text, tabId) { this.send({ type: "browser_fill", selector, text, tab_id: tabId }); }
    browserPress(key, tabId) { this.send({ type: "browser_press", key, tab_id: tabId }); }
    browserClose(tabId) { this.send({ type: "browser_close", tab_id: tabId }); }
    // -------------------------------------------------------------------------
    // Event registration
    // -------------------------------------------------------------------------
    on(event, cb) {
        let set = this.listeners.get(event);
        if (!set) {
            set = new Set();
            this.listeners.set(event, set);
        }
        set.add(cb);
    }
    off(event, cb) {
        this.listeners.get(event)?.delete(cb);
    }
    emit(event, ...args) {
        const set = this.listeners.get(event);
        if (!set)
            return;
        for (const cb of set) {
            try {
                cb(...args);
            }
            catch (e) {
                // Never let one listener's throw break the dispatch loop.
                console.error("[mew] listener for", event, "threw:", e);
            }
        }
    }
    // -------------------------------------------------------------------------
    // Wire dispatch
    // -------------------------------------------------------------------------
    dispatch(msg) {
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
                this.emit("permission-request", {
                    request_id: msg.request_id,
                    tool_name: msg.tool_name,
                    input: msg.input,
                }, (decision) => this.respondToPermission(msg.request_id, decision));
                break;
            case "workspace_permission_request":
                this.emit("workspace-permission-request", {
                    request_id: msg.request_id,
                    path: msg.path,
                }, (decision) => this.respondToPermission(msg.request_id, decision));
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
                this.emit("permission-request", {
                    request_id: msg.request_id,
                    tool_name: msg.tool_name,
                    input: msg.input,
                }, (decision) => this.respondToPermission(msg.request_id, decision));
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
                const _exhaustive = msg;
                throw new Error(`unhandled ServerMessage: ${_exhaustive.type}`);
            }
        }
    }
    send(msg) {
        if (!this.ws)
            throw new Error("not connected");
        if (this.debug)
            console.debug("[mew] ->", msg);
        this.ws.send(JSON.stringify(msg));
    }
    /** Return the active session id, or null if `newSession` hasn't succeeded. */
    getSessionId() {
        return this.sessionId;
    }
}
//# sourceMappingURL=index.js.map