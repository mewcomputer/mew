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
    clientKind;
    remoteAuth;
    ws = null;
    listeners = new Map();
    /** Promise resolved when the WebSocket opens. */
    openPromise = null;
    connectionLifetime = null;
    protocolVersion = 0;
    sessionId = null;
    confirmedSessions = new Set();
    browserFocusPreflight = null;
    /** Messages currently projected from the most recent history pages. */
    sessionHistoryMessages = [];
    /** SessionReady has no request ID, so lifecycle requests share a queue. */
    sessionCommandTail = Promise.resolve();
    pendingSessionCommands = 0;
    scopedSessionFailures = false;
    pendingPrompts = new Map();
    constructor(url, opts = {}) {
        this.url = url;
        this.socketFactory = opts.socketFactory ?? defaultSocketFactory;
        this.debug = opts.debug ?? false;
        this.clientKind = opts.clientKind ?? "web";
        this.remoteAuth = opts.remoteAuth;
    }
    /** Open the WebSocket connection. Idempotent. */
    connect() {
        if (this.openPromise)
            return this.openPromise;
        let ws;
        try {
            ws = this.socketFactory(this.url);
        }
        catch (error) {
            return Promise.reject(error);
        }
        const lifetime = new AbortController();
        this.ws = ws;
        this.connectionLifetime = lifetime;
        this.openPromise = new Promise((resolve, reject) => {
            const cleanup = () => {
                clearTimeout(timeout);
                this.off("remote-ready", onReady);
                this.off("errorMessage", onRemoteError);
                lifetime.signal.removeEventListener("abort", onDisconnected);
            };
            const onReady = () => { cleanup(); resolve(); };
            const onRemoteError = (message) => {
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
                if (this.ws !== ws)
                    return;
                if (this.debug)
                    console.debug("[mew] open");
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
                if (this.ws !== ws)
                    return;
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
                if (!this.endConnection(ws, `The daemon disconnected (${ev.code} ${ev.reason}).`))
                    return;
                this.emit("close", ev.code, ev.reason);
            });
            ws.addEventListener("error", (ev) => {
                if (this.debug)
                    console.debug("[mew] error", ev);
                if (!this.endConnection(ws, "The daemon connection failed."))
                    return;
                this.emit("error", ev);
                this.emit("close", 1006, "The daemon connection failed.");
            });
        });
        return this.openPromise;
    }
    /** Close the current socket and discard its attachment confirmations. */
    disconnect(code = 1000, reason = "client disconnect") {
        const socket = this.ws;
        if (!socket || !this.endConnection(socket, "The daemon disconnected."))
            return;
        socket.close(code, reason);
        this.emit("close", code, reason);
    }
    endConnection(socket, message) {
        if (this.ws !== socket)
            return false;
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
        if (wasSwitching)
            this.emit("session-command-state");
        return true;
    }
    isConnected() {
        return this.ws !== null;
    }
    hasConfirmedSession(sessionId) {
        return this.confirmedSessions.has(sessionId);
    }
    /** Commands for the active session need its current socket's confirmation. */
    isSessionAttached(sessionId) {
        return this.sessionId === sessionId && this.hasConfirmedSession(sessionId);
    }
    /** True once the active attachment has no queued or in-flight lifecycle work. */
    isSessionStable(sessionId) {
        return this.pendingSessionCommands === 0 && this.isSessionAttached(sessionId);
    }
    requestSession(message) {
        const connecting = this.connect();
        const signal = this.connectionLifetime?.signal;
        if (!signal)
            return connecting.then(() => { throw new Error("No daemon connection."); });
        const wasIdle = this.pendingSessionCommands++ === 0;
        const run = this.sessionCommandTail.then(async () => {
            await connecting;
            signal.throwIfAborted();
            const sessionId = message.type === "attach_session" ? message.session_id : null;
            if (sessionId !== null && this.isSessionAttached(sessionId))
                return sessionId;
            return new Promise((resolve, reject) => {
                const cleanup = () => {
                    clearTimeout(timeout);
                    signal.removeEventListener("abort", onDisconnect);
                    this.off("session-ready", onReady);
                    this.off("session-operation-failed", onFailure);
                    this.off("errorMessage", onLegacyError);
                };
                const fail = (error) => { cleanup(); reject(error); };
                const onReady = (data) => {
                    if (sessionId !== null && data.session_id !== sessionId)
                        return;
                    cleanup();
                    resolve(data.session_id);
                };
                const onFailure = (data) => {
                    if (data.operation !== (sessionId === null ? "create" : "attach") || data.session_id !== sessionId)
                        return;
                    fail(new Error(data.message));
                };
                const onLegacyError = ({ message }) => {
                    if (!this.scopedSessionFailures)
                        fail(new Error(message));
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
                try {
                    this.send(message);
                }
                catch (error) {
                    fail(error);
                }
            });
        }).finally(() => {
            if (this.connectionLifetime?.signal !== signal)
                return;
            if (--this.pendingSessionCommands === 0)
                this.emit("session-command-state");
        });
        this.sessionCommandTail = run.then(() => undefined, () => undefined);
        if (wasIdle)
            this.emit("session-command-state");
        return run;
    }
    /** Send `new_session`. Resolves with the daemon-assigned session id. */
    newSession(cwd = null) {
        return this.requestSession({ type: "new_session", cwd, client_kind: this.clientKind });
    }
    /** Send `prompt`. Streaming events are emitted via the registered handlers. */
    prompt(text, attachments = []) {
        this.send({ type: "prompt", text, attachments });
    }
    /** Resolve only when the daemon accepts the prompt in the specified session. */
    async submitPrompt(sessionId, text, attachments = []) {
        const socket = this.ws;
        if (!socket || !this.isSessionAttached(sessionId))
            throw new Error("This conversation is not attached to the daemon.");
        if (this.protocolVersion < 3)
            throw new Error("Update the daemon to enable confirmed prompt submission.");
        if (this.pendingPrompts.has(sessionId))
            throw new Error("This conversation is already awaiting a prompt receipt.");
        const requestId = crypto.randomUUID();
        return new Promise((resolve, reject) => {
            const finish = (status, message) => {
                this.pendingPrompts.delete(sessionId);
                clearTimeout(pending.timeout);
                if (status === "accepted")
                    resolve();
                else
                    reject(new Error(message ?? (status === "unknown"
                        ? "Submission could not be confirmed. Check the conversation before retrying."
                        : "The daemon rejected the prompt.")));
            };
            const pending = { requestId, socket, finish, timeout: setTimeout(() => finish("unknown"), 15000) };
            this.pendingPrompts.set(sessionId, pending);
            try {
                this.send({ type: "submit_prompt", session_id: sessionId, request_id: requestId, text, attachments });
            }
            catch (error) {
                finish("rejected", error instanceof Error ? error.message : undefined);
            }
        });
    }
    /** Compare and save one todo; resolve only after the daemon confirms it. */
    updateTodo(sessionId, edit) {
        const requestId = crypto.randomUUID();
        return new Promise((resolve, reject) => {
            const cleanup = () => {
                clearTimeout(timer);
                this.off("todo-updated", onSaved);
                this.off("todo-update-error", onError);
                this.off("close", onClose);
            };
            const onSaved = (data) => {
                if (data.session_id !== sessionId || data.request_id !== requestId)
                    return;
                cleanup();
                resolve(data.todos);
            };
            const onError = (data) => {
                if (data.session_id !== sessionId || data.request_id !== requestId)
                    return;
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
            }
            catch (error) {
                cleanup();
                reject(error);
            }
        });
    }
    /** Send `cancel` to abort the current turn. */
    cancel() {
        this.send({ type: "cancel" });
    }
    /** Inject guidance into the running turn's next request (steer the LLM).
     *  If no turn is running, it is picked up by the next turn. */
    guide(text) {
        this.send({ type: "guide", text });
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
    /** Close one question while leaving the parent turn and other questions active. */
    cancelAskUser(request_id) {
        this.send({ type: "ask_user_response", request_id, answers: [], cancelled: true });
    }
    /** Accept or reject a proposed goal using its original request ID. */
    respondToGoal(request_id, accepted) {
        this.send({ type: "goal_response", request_id, accepted });
    }
    /** Attach to an existing session (active or idle). If the session is idle,
     *  the daemon loads its persisted history from disk and sends a
     *  `session-history` event. Resolves with the session id. */
    attachSession(session_id) {
        return this.requestSession({ type: "attach_session", session_id, client_kind: this.clientKind });
    }
    /** Request older messages for the attached session using the cursor from a
     *  previous history page. */
    loadSessionHistory(session_id, before, limit) {
        this.send({ type: "load_session_history", session_id, before, limit });
    }
    /** Read the next bounded UTF-8 chunk from a session-owned job. */
    readJobOutput(session_id, job_id, request_id, offset) {
        this.send({ type: "read_job_output", session_id, job_id, request_id, offset });
    }
    /** Request a recorded child transcript without changing the attached session. */
    readSubagentHistory(session_id, child_session_id, request_id) {
        this.send({ type: "read_subagent_history", session_id, child_session_id, request_id });
    }
    /** Send guidance to a running child of the attached conversation. */
    async guideSubagent(sessionId, childSessionId, text, requestId = crypto.randomUUID()) {
        if (!text.trim())
            throw new Error("Write a message before sending guidance.");
        return this.requestSubagentReceipt({ type: "guide_subagent", session_id: sessionId, child_session_id: childSessionId, request_id: requestId, text });
    }
    /** Read a receipt without sending new guidance. */
    readSubagentMessageReceipt(sessionId, childSessionId, requestId) {
        return this.requestSubagentReceipt({ type: "read_subagent_message_receipt", session_id: sessionId, child_session_id: childSessionId, request_id: requestId });
    }
    async requestSubagentReceipt(message) {
        const { session_id, child_session_id, request_id } = message;
        if (!this.isSessionAttached(session_id))
            throw new Error("The parent conversation is not attached.");
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
            const onReceipt = (receipt) => {
                if (receipt.session_id !== session_id || receipt.child_session_id !== child_session_id || receipt.request_id !== request_id)
                    return;
                cleanup();
                resolve(receipt);
            };
            const timeout = setTimeout(unconfirmed, 10000);
            signal?.addEventListener("abort", unconfirmed, { once: true });
            this.on("subagent-message-receipt", onReceipt);
            try {
                this.send(message);
            }
            catch (error) {
                cleanup();
                reject(error);
            }
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
    /** Delete a conversation and wait for its matching daemon confirmation. */
    deleteSessionConfirmed(sessionId) {
        const requestId = crypto.randomUUID();
        const signal = this.connectionLifetime?.signal;
        return new Promise((resolve, reject) => {
            const cleanup = () => {
                clearTimeout(timeout);
                signal?.removeEventListener("abort", onDisconnect);
                this.off("session-deletion-result", onReply);
            };
            const fail = (error) => { cleanup(); reject(error); };
            const unconfirmed = () => fail(new Error("Deletion could not be confirmed. Refresh sessions before retrying."));
            const onDisconnect = () => unconfirmed();
            const onReply = (result) => {
                if (result.session_id !== sessionId || result.request_id !== requestId)
                    return;
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
            try {
                this.send({ type: "delete_session_confirmed", session_id: sessionId, request_id: requestId });
            }
            catch (error) {
                fail(error);
            }
        });
    }
    /** Rename a session (set a custom title). Persists to disk and broadcasts. */
    renameSession(session_id, title) {
        this.send({ type: "rename_session", session_id, title });
    }
    /** Enable or disable auto-generated session titles. */
    setAutoTitle(enabled) {
        this.send({ type: "set_auto_title", enabled });
    }
    readDaemonSettings(requestId) {
        this.send({ type: "read_daemon_settings", request_id: requestId });
    }
    saveSessionDefaults(sessionId, requestId) {
        this.send({ type: "save_session_defaults", session_id: sessionId, request_id: requestId });
    }
    resetSessionDefaults(requestId) {
        this.send({ type: "reset_session_defaults", request_id: requestId });
    }
    revokeDevice(deviceId, requestId) {
        this.send({ type: "revoke_device", device_id: deviceId, request_id: requestId });
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
     *  "none" to disable. Numeric token budgets ride this call as the string
     *  convention `"budget:<n>"` (e.g. `"budget:8192"`); use
     *  `setThinkingBudget` for that. Resolves when the daemon confirms via
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
    /** Set a numeric token budget for thinking via `setThinkingVariant`
     *  (`"budget:<n>"`). Only valid for models that declare a
     *  `thinking_budget` range. */
    setThinkingBudget(tokens) {
        return this.setThinkingVariant(`budget:${tokens}`);
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
    listFilesystemDir(path) {
        this.send({ type: "list_filesystem_dir", ...(path ? { path } : {}) });
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
        return new Promise((resolve, reject) => {
            const signal = this.connectionLifetime?.signal;
            const cleanup = () => {
                clearTimeout(timeout);
                this.off("pong", onPong);
                signal?.removeEventListener("abort", onDisconnect);
            };
            const onPong = (data) => {
                cleanup();
                resolve(data.version);
            };
            const fail = (error) => { cleanup(); reject(error); };
            const onDisconnect = () => fail(signal?.reason);
            const timeout = setTimeout(() => fail(new Error("The daemon did not respond to its protocol check.")), 3000);
            signal?.addEventListener("abort", onDisconnect, { once: true });
            this.on("pong", onPong);
            try {
                this.send({ type: "ping" });
            }
            catch (error) {
                fail(error);
            }
        });
    }
    /** List known projects (recent session cwds). */
    listProjects() {
        this.send({ type: "list_projects" });
    }
    async registerBrowserPage(sessionId, pageInstance, cdpEndpoint, targetId, focusUrl) {
        if (focusUrl !== undefined) {
            const socket = this.ws;
            if (!socket || !this.hasConfirmedSession(sessionId))
                throw new Error("Browser input requires a confirmed session on this connection.");
            if (!this.browserFocusPreflight) {
                const check = new Promise((resolve, reject) => {
                    const cleanup = () => {
                        clearTimeout(timeout);
                        this.off("pong", onPong);
                        this.off("close", onConnectionChanged);
                        this.off("error", onConnectionChanged);
                    };
                    const onPong = (data) => { cleanup(); resolve(data.browser_focus_preflight); };
                    const onConnectionChanged = () => {
                        if (this.ws === socket)
                            return;
                        cleanup();
                        reject(new Error("The daemon disconnected before confirming browser focus support."));
                    };
                    const timeout = setTimeout(() => { cleanup(); reject(new Error("The daemon did not confirm browser focus support.")); }, 3000);
                    this.on("pong", onPong);
                    this.on("close", onConnectionChanged);
                    this.on("error", onConnectionChanged);
                    try {
                        this.send({ type: "ping" });
                    }
                    catch (error) {
                        cleanup();
                        reject(error);
                    }
                });
                this.browserFocusPreflight = check;
                void check.catch(() => { if (this.browserFocusPreflight === check)
                    this.browserFocusPreflight = null; });
            }
            const supported = await this.browserFocusPreflight;
            if (this.ws !== socket || !this.hasConfirmedSession(sessionId))
                throw new Error("The daemon connection changed before browser page registration.");
            if (!supported)
                throw new Error("Update the daemon to enable safe browser input.");
        }
        return this.browserPageRequest({ type: "register_browser_page", session_id: sessionId, page_instance: pageInstance, cdp_endpoint: cdpEndpoint, target_id: targetId, focus_url: focusUrl });
    }
    unregisterBrowserPage(sessionId, pageInstance) {
        return this.browserPageRequest({ type: "unregister_browser_page", session_id: sessionId, page_instance: pageInstance });
    }
    browserPageRequest(message) {
        return new Promise((resolve, reject) => {
            const cleanup = () => {
                clearTimeout(timeout);
                this.off("browser-page-registration", onReply);
                this.off("close", onClose);
            };
            const onReply = (reply) => {
                if (reply.session_id !== message.session_id || reply.page_instance !== message.page_instance)
                    return;
                cleanup();
                if (reply.error)
                    reject(new Error(reply.error));
                else
                    resolve();
            };
            const onClose = () => { cleanup(); reject(new Error("The daemon disconnected while updating the browser page.")); };
            const timeout = setTimeout(() => { cleanup(); reject(new Error("The daemon did not acknowledge the browser page.")); }, 10000);
            this.on("browser-page-registration", onReply);
            this.on("close", onClose);
            try {
                this.send(message);
            }
            catch (error) {
                cleanup();
                reject(error);
            }
        });
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
                if (!this.isSessionAttached(msg.session_id))
                    break;
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
                    if (!socket || pending.socket === socket)
                        continue;
                    if (this.protocolVersion < 3) {
                        pending.finish("unknown");
                        continue;
                    }
                    pending.socket = socket;
                    try {
                        this.send({ type: "read_prompt_receipt", session_id: sessionId, request_id: pending.requestId });
                    }
                    catch {
                        pending.finish("unknown");
                    }
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
                const _exhaustive = msg;
                throw new Error(`unhandled ServerMessage: ${_exhaustive.type}`);
            }
        }
    }
    send(msg) {
        if (this.pendingSessionCommands > 0 && [
            "prompt", "cancel", "guide", "slash_command", "switch_model", "switch_persona",
            "set_thinking_variant", "set_permission_mode", "yield_control", "set_auto_title", "set_auto_summary",
        ].includes(msg.type)) {
            throw new Error("Wait until switching conversations finishes.");
        }
        if (!this.ws)
            throw new Error("not connected");
        if (this.debug)
            console.debug("[mew] ->", msg);
        this.ws.send(JSON.stringify(msg));
    }
    /** Return the last confirmed session ID, which can outlive its connection. */
    getSessionId() {
        return this.sessionId;
    }
}
//# sourceMappingURL=index.js.map