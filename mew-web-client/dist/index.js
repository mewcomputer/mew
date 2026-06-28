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
                if (!settled) {
                    settled = true;
                    reject(new Error(`ws closed before open: ${ev.code} ${ev.reason}`));
                }
                this.emit("close", ev.code, ev.reason);
            });
            ws.addEventListener("error", (ev) => {
                if (this.debug)
                    console.debug("[mew] error", ev);
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
    /** Send `NewSession`. Resolves with the daemon-assigned session id. */
    async newSession(cwd = null) {
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
            this.send({ type: "NewSession", cwd });
        });
    }
    /** Send `Prompt`. Streaming events are emitted via the registered handlers. */
    prompt(text, attachments = []) {
        this.send({ type: "Prompt", text, attachments });
    }
    /** Send `Cancel` to abort the current turn. */
    cancel() {
        this.send({ type: "Cancel" });
    }
    /**
     * Send a slash command (e.g. `/clear`, `/compact`). Returns the
     * `SlashResult.text` if the daemon produces one.
     */
    slashCommand(command) {
        return new Promise((resolve) => {
            const onResult = (data) => {
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
    respondToPermission(request_id, decision) {
        this.send({ type: "PermissionResponse", request_id, decision });
    }
    /** Respond to an `AskUserRequest`. The UI calls this after the user
     *  submits answers to the questions. */
    respondToAskUser(request_id, answers) {
        this.send({ type: "AskUserResponse", request_id, answers });
    }
    /** Attach to an existing session (active or idle). If the session is idle,
     *  the daemon loads its persisted history from disk and sends a
     *  `session-history` event. Resolves with the session id. */
    async attachSession(session_id) {
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
            this.send({ type: "AttachSession", session_id });
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
            this.send({ type: "ListSessions" });
        });
    }
    /** List available models from all configured providers. */
    listModels() {
        return new Promise((resolve) => {
            const onList = (data) => {
                this.off("model-list", onList);
                resolve(data.models);
            };
            this.on("model-list", onList);
            this.send({ type: "ListModels" });
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
            this.send({ type: "SwitchModel", provider, model });
        });
    }
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
                this.emit("permission-request", {
                    request_id: msg.request_id,
                    tool_name: msg.tool_name,
                    input: msg.input,
                }, (decision) => this.respondToPermission(msg.request_id, decision));
                break;
            case "WorkspacePermissionRequest":
                this.emit("workspace-permission-request", {
                    request_id: msg.request_id,
                    path: msg.path,
                }, (decision) => this.respondToPermission(msg.request_id, decision));
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
            case "Error":
                this.emit("errorMessage", { message: msg.message });
                break;
            case "ErrorEvent":
                this.emit("errorEvent", { message: msg.message });
                break;
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