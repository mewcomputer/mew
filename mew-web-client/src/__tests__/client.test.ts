//! Tests for the MewClient. Uses an in-memory mock WebSocket so the suite
//! runs without a live daemon.

import { strict as assert } from "node:assert";
import { test } from "node:test";
import { MewClient } from "../index.ts";
import type { MewWebSocket, ClientMessage, ServerMessage } from "../index.ts";

type Listener<E> = (ev: E) => void;

class MockWebSocket implements MewWebSocket {
  static instances: MockWebSocket[] = [];

  readyState = 0; // CONNECTING
  url: string;
  sent: ClientMessage[] = [];

  private listeners = new Map<string, Set<Listener<unknown>>>();
  /** Peer-side messages queued for this socket; drain via `peerSend`. */
  peerQueue: ServerMessage[] = [];

  constructor(url: string) {
    this.url = url;
    MockWebSocket.instances.push(this);
  }

  send(data: string) {
    this.sent.push(JSON.parse(data) as ClientMessage);
  }
  close(code = 1000, reason = "") {
    this.readyState = 3; // CLOSED
    this.fire("close", { code, reason });
  }
  addEventListener(type: string, listener: Listener<unknown>) {
    let set = this.listeners.get(type);
    if (!set) {
      set = new Set();
      this.listeners.set(type, set);
    }
    set.add(listener);
  }
  removeEventListener(type: string, listener: Listener<unknown>) {
    this.listeners.get(type)?.delete(listener);
  }
  private fire(type: string, ev: unknown) {
    const set = this.listeners.get(type);
    if (!set) return;
    for (const l of [...set]) l(ev);
  }

  /** Test helpers */
  open() {
    this.readyState = 1; // OPEN
    this.fire("open", undefined);
  }
  peerSend(msg: ServerMessage) {
    this.fire("message", { data: JSON.stringify(msg) });
  }
  peerClose(code = 1000, reason = "peer") {
    this.close(code, reason);
  }
  peerError(error = "peer error") {
    this.fire("error", error);
  }
}

function makeFactory(): {
  factory: (url: string) => MewWebSocket;
  latest: () => MockWebSocket;
} {
  let last: MockWebSocket | null = null;
  return {
    factory: (url: string) => {
      last = new MockWebSocket(url);
      return last;
    },
    latest: () => {
      if (!last) throw new Error("no socket created");
      return last;
    },
  };
}

test("selected diff requests preserve file, session, and request identity", async () => {
  const { factory, latest } = makeFactory();
  const client = new MewClient("ws://test/", { socketFactory: factory });
  const connected = client.connect();
  latest().open();
  await connected;
  const replies: unknown[] = [];
  client.on("session-diff", (data) => replies.push(data));
  client.readSessionDiff("chat-a", "request-a", "src/main.ts");
  assert.deepEqual(latest().sent.at(-1), { type: "read_session_file_diff", session_id: "chat-a", request_id: "request-a", path: "src/main.ts" });
  latest().peerSend({ type: "session_diff", session_id: "chat-a", request_id: "request-a", files: [] });
  assert.deepEqual(replies, [{ session_id: "chat-a", request_id: "request-a", files: [] }]);
});

test("automation settings can be read and follow daemon updates", async () => {
  const { factory, latest } = makeFactory();
  const client = new MewClient("ws://test/", { socketFactory: factory });
  const connected = client.connect();
  latest().open();
  await connected;
  const settings: unknown[] = [];
  client.on("automation-settings", (data) => settings.push(data));
  client.on("auto-summary-changed", (data) => settings.push(data));
  client.readAutomationSettings();
  assert.deepEqual(latest().sent.at(-1), { type: "read_automation_settings" });
  latest().peerSend({ type: "automation_settings", auto_title_enabled: false, auto_summary_enabled: true });
  latest().peerSend({ type: "auto_summary_changed", enabled: false });
  assert.deepEqual(settings, [{ auto_title_enabled: false, auto_summary_enabled: true }, { enabled: false }]);
});

test("daemon settings retain request identity and unavailable device management", async () => {
  const { factory, latest } = makeFactory();
  const client = new MewClient("ws://test/", { socketFactory: factory });
  const connected = client.connect();
  latest().open();
  await connected;
  const settings: unknown[] = [];
  const failures: unknown[] = [];
  client.on("daemon-settings", (data) => settings.push(data));
  client.on("daemon-settings-failed", (data) => failures.push(data));
  client.readDaemonSettings("read");
  client.saveSessionDefaults("chat", "save");
  client.resetSessionDefaults("reset");
  client.revokeDevice("phone", "revoke");
  client.setRemoteAccess(true, "remote-on");
  client.createPairingInvite("pair", { scope: "control", ttlSecs: 120 });
  assert.deepEqual(latest().sent, [
    { type: "read_daemon_settings", request_id: "read" },
    { type: "save_session_defaults", session_id: "chat", request_id: "save" },
    { type: "reset_session_defaults", request_id: "reset" },
    { type: "revoke_device", device_id: "phone", request_id: "revoke" },
    { type: "set_remote_access", enabled: true, request_id: "remote-on" },
    { type: "create_pairing_invite", request_id: "pair", scope: "control", ttl_secs: 120 },
  ]);
  const invites: unknown[] = [];
  client.on("pairing-invite", (data) => invites.push(data));
  latest().peerSend({
    type: "daemon_settings",
    request_id: "read",
    defaults: null,
    devices: null,
    remote: { supported: true, enabled: true, node_id: "node-1" },
  });
  latest().peerSend({
    type: "pairing_invite",
    request_id: "pair",
    node_id: "node-1",
    payload: "computer.mew.mew://node-1?token=tok",
    expires_at: 1234,
  });
  latest().peerSend({ type: "daemon_settings_failed", request_id: "save", message: "read only" });
  assert.deepEqual(settings, [
    {
      request_id: "read",
      defaults: null,
      devices: null,
      remote: { supported: true, enabled: true, node_id: "node-1" },
    },
  ]);
  assert.deepEqual(invites, [
    {
      request_id: "pair",
      node_id: "node-1",
      payload: "computer.mew.mew://node-1?token=tok",
      expires_at: 1234,
    },
  ]);
  assert.deepEqual(failures, [{ request_id: "save", message: "read only" }]);
  client.disconnect();
});

test("connect resolves when the socket opens", async () => {
  const { factory, latest } = makeFactory();
  const client = new MewClient("ws://test/", { socketFactory: factory });
  const p = client.connect();
  // Simulate async open
  setImmediate(() => latest().open());
  await p;
  assert.equal(client.isConnected(), true);
});

test("remote clients authenticate before connect resolves", async () => {
  const { factory, latest } = makeFactory();
  const client = new MewClient("iroh://remote", {
    socketFactory: factory,
    clientKind: "remote",
    remoteAuth: { token: "pairing-token", deviceName: "test laptop" },
  });
  const connectP = client.connect();
  const socket = latest();
  socket.open();
  assert.deepEqual(socket.sent, [{
    type: "remote_hello",
    token: "pairing-token",
    device_name: "test laptop",
  }]);

  let resolved = false;
  void connectP.then(() => { resolved = true; });
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(resolved, false);
  socket.peerSend({ type: "remote_ready", scope: "control" });
  await connectP;
  assert.equal(client.isConnected(), true);
});

test("connect can be retried after the socket fails before opening", async () => {
  const { factory, latest } = makeFactory();
  const client = new MewClient("ws://test/", { socketFactory: factory });

  const first = client.connect();
  const firstSocket = latest();
  firstSocket.peerClose(1006, "refused");
  await assert.rejects(first);

  const second = client.connect();
  const secondSocket = latest();
  assert.notEqual(secondSocket, firstSocket);
  setImmediate(() => secondSocket.open());
  await second;
  assert.equal(client.isConnected(), true);
});

test("a session request cannot hang behind an unopened socket", { timeout: 1000 }, async (context) => {
  context.mock.timers.enable({ apis: ["setTimeout"] });
  const { factory, latest } = makeFactory();
  const client = new MewClient("ws://test/", { socketFactory: factory });
  const waiting = assert.rejects(client.attachSession("old"), /did not open/i);
  const stalled = latest();
  context.mock.timers.tick(10000);
  await waiting;
  assert.equal(client.isConnected(), false);

  const replacement = client.attachSession("new");
  const socket = latest();
  assert.notEqual(socket, stalled);
  socket.open();
  await new Promise((resolve) => setImmediate(resolve));
  assert.deepEqual(socket.sent, [{ type: "attach_session", session_id: "new", client_kind: "web" }]);
  socket.peerSend({ type: "session_ready", session_id: "new" });
  assert.equal(await replacement, "new");
});

test("connect creates a fresh socket after an established socket closes", async () => {
  const { factory, latest } = makeFactory();
  const client = new MewClient("ws://test/", { socketFactory: factory });

  const first = client.connect();
  const firstSocket = latest();
  setImmediate(() => firstSocket.open());
  await first;
  firstSocket.peerClose(1006, "daemon stopped");

  const second = client.connect();
  const secondSocket = latest();
  assert.notEqual(secondSocket, firstSocket);
  setImmediate(() => secondSocket.open());
  await second;
  assert.equal(client.isConnected(), true);
});

for (const operation of ["attach", "create"] as const) {
  test(`${operation} interrupted by disconnect does not block later session commands`, async () => {
    const { factory, latest } = makeFactory();
    const client = new MewClient("ws://test/", { socketFactory: factory });
    const initial = client.connect();
    const first = latest();
    first.open();
    await initial;
    const interrupted = operation === "attach" ? client.attachSession("old") : client.newSession();
    const rejected = assert.rejects(interrupted, /connection|disconnect|closed/i);
    await new Promise((resolve) => setImmediate(resolve));
    first.peerClose(1006, "disconnected");
    await Promise.race([
      rejected,
      new Promise<never>((_, reject) => setTimeout(() => reject(new Error("session command did not reject")), 500)),
    ]);

    const reconnect = client.connect();
    const second = latest();
    second.open();
    await reconnect;
    const attached = client.attachSession("new");
    await new Promise((resolve) => setImmediate(resolve));
    assert.ok(second.sent.some((message) => message.type === "attach_session" && message.session_id === "new"));
    second.peerSend({ type: "session_ready", session_id: "new" });
    assert.equal(await attached, "new");
    client.disconnect();
  });
}

test("unscoped commands cannot reach a different session while an attach is pending", async () => {
  const { factory, latest } = makeFactory();
  const client = new MewClient("ws://test/", { socketFactory: factory });
  const connected = client.connect();
  const socket = latest();
  socket.open();
  await connected;
  socket.peerSend({ type: "session_ready", session_id: "a" });
  const commandStates: boolean[] = [];
  client.on("session-command-state", () => commandStates.push(
    client.isSessionStable(client.getSessionId()!),
  ));
  assert.equal(client.isSessionStable("a"), true);
  const attaching = client.attachSession("b");
  const queued = client.attachSession("c");
  assert.equal(client.isSessionAttached("a"), true);
  assert.equal(client.isSessionStable("a"), false);
  await new Promise((resolve) => setImmediate(resolve));
  await assert.rejects(client.slashCommand("/clear"), /switching conversations/i);
  assert.throws(() => client.cancel(), /switching conversations/i);
  assert.ok(!socket.sent.some((message) => message.type === "slash_command" || message.type === "cancel"));
  socket.peerSend({ type: "session_ready", session_id: "b" });
  await attaching;
  assert.equal(client.isSessionStable("b"), false);
  await new Promise((resolve) => setImmediate(resolve));
  socket.peerSend({ type: "session_ready", session_id: "c" });
  await queued;
  assert.equal(client.isSessionStable("c"), true);
  assert.deepEqual(commandStates, [false, true]);
  client.disconnect();
});

test("concurrent requests to attach the same session do not send a duplicate", async () => {
  const { factory, latest } = makeFactory();
  const client = new MewClient("ws://test/", { socketFactory: factory });
  const connected = client.connect();
  const socket = latest();
  socket.open();
  await connected;
  const first = client.attachSession("chat");
  const second = client.attachSession("chat");
  await new Promise((resolve) => setImmediate(resolve));
  socket.peerSend({ type: "session_ready", session_id: "chat" });
  assert.deepEqual(await Promise.race([
    Promise.all([first, second]),
    new Promise<never>((_, reject) => setTimeout(() => reject(new Error("duplicate attach waited forever")), 500)),
  ]), ["chat", "chat"]);
  assert.equal(socket.sent.filter((message) => message.type === "attach_session").length, 1);
  client.disconnect();
});

test("a legacy error following a scoped failure does not reject the next session request", async () => {
  const { factory, latest } = makeFactory();
  const client = new MewClient("ws://test/", { socketFactory: factory });
  const connected = client.connect();
  const socket = latest();
  socket.open();
  await connected;
  const first = client.attachSession("missing");
  const firstRejected = assert.rejects(first, /missing session/);
  const second = client.attachSession("valid");
  await new Promise((resolve) => setImmediate(resolve));
  socket.peerSend({ type: "session_operation_failed", operation: "attach", session_id: "missing", message: "missing session" });
  await firstRejected;
  await new Promise((resolve) => setImmediate(resolve));
  socket.peerSend({ type: "error", message: "missing session" });
  socket.peerSend({ type: "session_ready", session_id: "valid" });
  assert.equal(await second, "valid");
  client.disconnect();
});

test("a daemon advertising scoped failures ignores unrelated errors during attach", async () => {
  const { factory, latest } = makeFactory();
  const client = new MewClient("ws://test/", { socketFactory: factory });
  const connected = client.connect();
  const socket = latest();
  socket.open();
  await connected;
  socket.peerSend({ type: "pong", version: "test", protocol_version: 3, scoped_session_failures: true });
  const pending = client.attachSession("valid");
  await new Promise((resolve) => setImmediate(resolve));
  socket.peerSend({ type: "error", message: "unrelated command failed" });
  socket.peerSend({ type: "session_ready", session_id: "valid" });
  assert.equal(await pending, "valid");
  client.disconnect();
});

test("a legacy v3 daemon rejects a failed attach and releases the queue", async () => {
  const { factory, latest } = makeFactory();
  const client = new MewClient("ws://test/", { socketFactory: factory });
  const connected = client.connect();
  const socket = latest();
  socket.open();
  await connected;
  socket.peerSend({ type: "pong", version: "old", protocol_version: 3 });
  const missing = client.attachSession("missing");
  const rejected = assert.rejects(missing, /missing session/);
  const valid = client.attachSession("valid");
  await new Promise((resolve) => setImmediate(resolve));
  socket.peerSend({ type: "error", message: "missing session" });
  await Promise.race([rejected, new Promise<never>((_, reject) => setTimeout(() => reject(new Error("legacy error ignored")), 500))]);
  await new Promise((resolve) => setImmediate(resolve));
  socket.peerSend({ type: "session_ready", session_id: "valid" });
  assert.equal(await valid, "valid");
  client.disconnect();
});

for (const operation of ["attach", "create"] as const) {
  test(`a failed ${operation} releases the session command queue`, async () => {
    const { factory, latest } = makeFactory();
    const client = new MewClient("ws://test/", { socketFactory: factory });
    const connected = client.connect();
    const socket = latest();
    socket.open();
    await connected;
    const request = operation === "attach" ? client.attachSession("missing") : client.newSession();
    await new Promise((resolve) => setImmediate(resolve));
    socket.peerSend({ type: "session_operation_failed", operation, session_id: operation === "attach" ? "missing" : null, message: "cannot open session" });
    await Promise.race([
      assert.rejects(request, /cannot open session/),
      new Promise<never>((_, reject) => setTimeout(() => reject(new Error("session failure was ignored")), 500)),
    ]);
    const next = client.attachSession("available");
    await new Promise((resolve) => setImmediate(resolve));
    assert.ok(socket.sent.some((message) => message.type === "attach_session" && message.session_id === "available"));
    socket.peerSend({ type: "session_ready", session_id: "available" });
    assert.equal(await next, "available");
    client.disconnect();
  });
}

test("scoped prompts wait for the matching receipt and reject a different session", async () => {
  const { factory, latest } = makeFactory();
  const client = new MewClient("ws://test/", { socketFactory: factory });
  const connected = client.connect();
  const socket = latest();
  socket.open();
  await connected;
  socket.peerSend({ type: "pong", version: "test", protocol_version: 3 });
  socket.peerSend({ type: "session_ready", session_id: "first" });
  await assert.rejects(client.submitPrompt("other", "wrong workspace"), /attached|session/i);
  const pending = client.submitPrompt("first", "hello");
  const request = socket.sent.at(-1);
  assert.equal(request?.type, "submit_prompt");
  if (request?.type !== "submit_prompt") throw new Error("expected a scoped prompt");
  assert.equal(request.session_id, "first");
  socket.peerSend({ type: "prompt_receipt", session_id: "other", request_id: request.request_id, status: "accepted" });
  let settled = false;
  void pending.then(() => { settled = true; });
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(settled, false);
  socket.peerSend({ type: "prompt_receipt", session_id: "first", request_id: request.request_id, status: "accepted" });
  await pending;

  const rejected = client.submitPrompt("first", "busy");
  const retry = socket.sent.at(-1);
  if (retry?.type !== "submit_prompt") throw new Error("expected a scoped prompt");
  socket.peerSend({ type: "prompt_receipt", session_id: "first", request_id: retry.request_id, status: "rejected", message: "turn already running" });
  await assert.rejects(rejected, /turn already running/);
  client.disconnect();
});

test("a disconnected prompt without a replacement socket reports uncertainty at its deadline", async (context) => {
  context.mock.timers.enable({ apis: ["setTimeout"] });
  const { factory, latest } = makeFactory();
  const client = new MewClient("ws://test/", { socketFactory: factory });
  const connected = client.connect();
  const first = latest();
  first.open();
  await connected;
  first.peerSend({ type: "pong", version: "test", protocol_version: 3 });
  first.peerSend({ type: "session_ready", session_id: "first" });
  const pending = client.submitPrompt("first", "may have arrived");
  const rejected = assert.rejects(pending, /could not be confirmed/i);
  first.peerClose(1006, "dropped");
  context.mock.timers.tick(15000);
  await rejected;
});

test("connect can recover if an established socket emits an error", async () => {
  const { factory, latest } = makeFactory();
  const client = new MewClient("ws://test/", { socketFactory: factory });

  const first = client.connect();
  const firstSocket = latest();
  setImmediate(() => firstSocket.open());
  await first;
  firstSocket.peerError();

  const second = client.connect();
  const secondSocket = latest();
  assert.notEqual(secondSocket, firstSocket);
  setImmediate(() => secondSocket.open());
  await second;
  assert.equal(client.isConnected(), true);
});

test("newSession sends new_session and resolves with session_id", async () => {
  const { factory, latest } = makeFactory();
  const client = new MewClient("ws://test/", { socketFactory: factory });
  const connectP = client.connect();
  setImmediate(() => latest().open());
  await connectP;

  const sessionP = client.newSession();
  // Simulate server reply
  setImmediate(() => {
    const ws = latest();
    assert.equal(ws.sent.length, 1);
    assert.equal(ws.sent[0]!.type, "new_session");
    ws.peerSend({ type: "session_ready", session_id: "sess_abc" });
  });
  const sessionId = await sessionP;
  assert.equal(sessionId, "sess_abc");
  assert.equal(client.getSessionId(), "sess_abc");
});

test("history pages prepend older messages and can be requested by cursor", async () => {
  const { factory, latest } = makeFactory();
  const client = new MewClient("ws://test/", { socketFactory: factory });
  const connectP = client.connect();
  setImmediate(() => latest().open());
  await connectP;

  const history: string[][] = [];
  const cursors: (number | undefined)[] = [];
  client.on("session-history", ({ messages }) => {
    history.push(messages.map((message) => message.id));
  });
  client.on("session-history-page", (page) => cursors.push(page.next_cursor));
  const message = (id: string) => ({
    id,
    session_id: "sess_1",
    role: "user" as const,
    parts: [],
    time: { created: 0 },
  });

  latest().peerSend({ type: "session_ready", session_id: "sess_1" });
  latest().peerSend({
    type: "session_history_page",
    session_id: "sess_1",
    messages: [message("new")],
    next_cursor: 1,
    replace: true,
  });
  latest().peerSend({
    type: "session_history_page",
    session_id: "sess_1",
    messages: [message("old")],
    next_cursor: undefined,
    replace: false,
  });
  client.loadSessionHistory("sess_1", 1, 24);

  assert.deepEqual(history, [["new"], ["old", "new"]]);
  assert.deepEqual(cursors, [1, undefined]);
  assert.deepEqual(latest().sent, [{
    type: "load_session_history",
    session_id: "sess_1",
    before: 1,
    limit: 24,
  }]);
});

test("session index emits turn summaries for the attached session only", async () => {
  const { factory, latest } = makeFactory();
  const client = new MewClient("ws://test/", { socketFactory: factory });
  const connectP = client.connect();
  setImmediate(() => latest().open());
  await connectP;

  const indexes: { session_id: string; turns: { id: string; index: number; message_count: number; prompt: string; reply: string }[] }[] = [];
  client.on("session-index", (data) => indexes.push(data));
  const turn = { id: "user-1", index: 0, message_count: 2, prompt: "hello", reply: "hi" };

  latest().peerSend({ type: "session_ready", session_id: "sess_1" });
  latest().peerSend({ type: "session_index", session_id: "other", turns: [turn] });
  latest().peerSend({ type: "session_index", session_id: "sess_1", turns: [turn] });
  client.loadSessionIndex("sess_1");

  assert.deepEqual(indexes, [{ session_id: "sess_1", turns: [turn] }]);
  assert.deepEqual(latest().sent, [{ type: "load_session_index", session_id: "sess_1" }]);
});

test("history from a previously attached session cannot replace the current session", async () => {
  const { factory, latest } = makeFactory();
  const client = new MewClient("ws://test/", { socketFactory: factory });
  const connected = client.connect();
  latest().open();
  await connected;
  const updates: string[] = [];
  client.on("session-history-page", (page) => updates.push(page.session_id));
  latest().peerSend({ type: "session_ready", session_id: "old" });
  latest().peerSend({ type: "session_ready", session_id: "current" });
  latest().peerSend({ type: "session_history_page", session_id: "old", messages: [], next_cursor: 1, replace: true });
  latest().peerSend({ type: "session_history_page", session_id: "current", messages: [], next_cursor: 1, replace: true });
  assert.deepEqual(updates, ["current"]);
});

test("manual persona selection waits for the daemon's confirmed persona", async () => {
  const { factory, latest } = makeFactory();
  const client = new MewClient("ws://test/", { socketFactory: factory });
  const connectP = client.connect();
  setImmediate(() => latest().open());
  await connectP;
  const personas: string[] = [];
  client.on("persona-switched", ({ name }) => personas.push(name));

  client.switchPersona("builder");
  assert.deepEqual(latest().sent, [{ type: "switch_persona", name: "builder" }]);
  assert.deepEqual(personas, []);

  latest().peerSend({ type: "persona_switched", name: "builder" });
  assert.deepEqual(personas, ["builder"]);
});

test("proposed goals can be accepted or rejected using the request ID", async () => {
  const { factory, latest } = makeFactory();
  const client = new MewClient("ws://test/", { socketFactory: factory });
  const connectP = client.connect();
  setImmediate(() => latest().open());
  await connectP;
  const goals: unknown[] = [];
  client.on("goal-proposed", (data) => goals.push(data));

  latest().peerSend({ type: "goal_proposed", request_id: "g1", call_id: "c1", objective: "ship the change" });
  client.respondToGoal("g1", true);
  client.respondToGoal("g2", false);

  assert.deepEqual(goals, [{ request_id: "g1", call_id: "c1", objective: "ship the change" }]);
  assert.deepEqual(latest().sent, [
    { type: "goal_response", request_id: "g1", accepted: true },
    { type: "goal_response", request_id: "g2", accepted: false },
  ]);
});

for (const [response, expectedProtocol] of [
  [{ type: "pong", version: "0.1.0", protocol_version: 1 }, 1],
  [{ type: "pong", version: "0.1.0" }, 0],
] as const) {
  test(`ping preserves the daemon version and exposes protocol ${expectedProtocol}`, async () => {
    const { factory, latest } = makeFactory();
    const client = new MewClient("ws://test/", { socketFactory: factory });
    const connectP = client.connect();
    setImmediate(() => latest().open());
    await connectP;
    const replies: unknown[] = [];
    client.on("pong", (data) => replies.push(data));

    const ping = client.ping();
    assert.deepEqual(latest().sent, [{ type: "ping" }]);
    latest().peerSend(response);

    assert.equal(await ping, "0.1.0");
    assert.deepEqual(replies, [{ version: "0.1.0", protocol_version: expectedProtocol, browser_focus_preflight: false }]);
  });
}

test("serializes overlapping session requests so errors stay with their request", async () => {
  const { factory, latest } = makeFactory();
  const client = new MewClient("ws://test/", { socketFactory: factory });
  const connectP = client.connect();
  setImmediate(() => latest().open());
  await connectP;

  const missingP = client.attachSession("sess_missing");
  const newP = client.newSession();
  setImmediate(() => {
    const ws = latest();
    assert.equal(ws.sent[0]!.type, "attach_session");
    ws.peerSend({ type: "error", message: "session not found" });
    setImmediate(() => {
      assert.equal(ws.sent[1]!.type, "new_session");
      ws.peerSend({ type: "session_ready", session_id: "sess_new" });
    });
  });

  await assert.rejects(missingP, /session not found/);
  assert.equal(await newP, "sess_new");
});

test("a disconnect rejects session commands from that socket and does not block its replacement", { timeout: 2000 }, async () => {
  const { factory, latest } = makeFactory();
  const client = new MewClient("ws://test/", { socketFactory: factory });
  const connected = client.connect();
  const first = latest();
  first.open();
  await connected;
  const attach = assert.rejects(client.attachSession("old"), /disconnected|connection changed/i);
  const queuedCreate = assert.rejects(client.newSession(), /disconnected|connection changed/i);
  await new Promise((resolve) => setImmediate(resolve));
  assert.deepEqual(first.sent, [{ type: "attach_session", session_id: "old", client_kind: "web" }]);
  first.peerClose(1006);
  await Promise.all([attach, queuedCreate]);

  const replacement = client.connect();
  latest().open();
  await replacement;
  const created = client.newSession();
  await new Promise((resolve) => setImmediate(resolve));
  assert.deepEqual(latest().sent, [{ type: "new_session", cwd: null, client_kind: "web" }]);
  first.peerSend({ type: "session_ready", session_id: "stale" });
  latest().peerSend({ type: "session_ready", session_id: "new" });
  assert.equal(await created, "new");
});

test("a scoped attach ignores unrelated errors and requires its own confirmation", async () => {
  const { factory, latest } = makeFactory();
  const client = new MewClient("ws://test/", { socketFactory: factory });
  const connected = client.connect();
  latest().open();
  await connected;
  latest().peerSend({ type: "pong", version: "test", protocol_version: 3, scoped_session_failures: true });
  const first = client.attachSession("old");
  await new Promise((resolve) => setImmediate(resolve));
  latest().peerSend({ type: "error", message: "unrelated operation failed" });
  latest().peerSend({ type: "session_ready", session_id: "wrong" });
  latest().peerSend({ type: "session_ready", session_id: "old" });
  assert.equal(await first, "old");
  assert.equal(await client.attachSession("old"), "old");
  assert.equal(latest().sent.length, 1);

  const missing = assert.rejects(client.attachSession("missing"), /could not find chat/);
  await new Promise((resolve) => setImmediate(resolve));
  latest().peerSend({ type: "session_operation_failed", operation: "attach", session_id: "missing", message: "could not find chat" });
  await missing;
});

test("a scoped prompt requires a current attachment and settles only on its own receipt", async () => {
  const { factory, latest } = makeFactory();
  const client = new MewClient("ws://test/", { socketFactory: factory });
  const connected = client.connect();
  latest().open();
  await connected;
  latest().peerSend({ type: "pong", version: "test", protocol_version: 3 });
  latest().peerSend({ type: "session_ready", session_id: "old" });
  latest().peerSend({ type: "session_ready", session_id: "active" });
  await assert.rejects(client.submitPrompt("old", "wrong owner"), /attached/i);
  const attachments = [{ path: "draft.png", mime: "image/png" }];
  const submitting = client.submitPrompt("active", "hello", attachments);
  const prompt = latest().sent[0];
  assert.ok(prompt?.type === "submit_prompt");
  assert.deepEqual({ ...prompt, request_id: "id" }, { type: "submit_prompt", session_id: "active", request_id: "id", text: "hello", attachments });
  await assert.rejects(client.submitPrompt("active", "duplicate"), /awaiting/i);

  latest().peerSend({ type: "prompt_receipt", session_id: "old", request_id: prompt.request_id, status: "accepted" });
  latest().peerSend({ type: "prompt_receipt", session_id: "active", request_id: "unrelated", status: "accepted" });
  latest().peerSend({ type: "prompt_receipt", session_id: "active", request_id: prompt.request_id, status: "rejected", message: "session is busy" });
  await assert.rejects(submitting, /session is busy/);
});

for (const status of ["accepted", "unknown"] as const) {
  test(`an interrupted prompt reads its ${status} receipt without resubmitting`, async () => {
    const { factory, latest } = makeFactory();
    const client = new MewClient("ws://test/", { socketFactory: factory });
    const connected = client.connect();
    const first = latest();
    first.open();
    await connected;
    first.peerSend({ type: "pong", version: "test", protocol_version: 3 });
    first.peerSend({ type: "session_ready", session_id: "active" });
    const submitting = client.submitPrompt("active", "one turn");
    const outcome = status === "accepted" ? submitting : assert.rejects(submitting, /could not be confirmed/i);
    const prompt = first.sent[0];
    assert.ok(prompt?.type === "submit_prompt");
    first.peerClose(1006);
    assert.equal(client.isSessionAttached("active"), false);

    const replacement = client.connect();
    latest().open();
    await replacement;
    first.peerSend({ type: "prompt_receipt", session_id: "active", request_id: prompt.request_id, status: "accepted" });
    latest().peerSend({ type: "pong", version: "test", protocol_version: 3 });
    assert.deepEqual(latest().sent, [{ type: "read_prompt_receipt", session_id: "active", request_id: prompt.request_id }]);
    latest().peerSend({ type: "prompt_receipt", session_id: "active", request_id: prompt.request_id, status });
    await outcome;
  });
}

test("prompt sends prompt with text and attachments", async () => {
  const { factory, latest } = makeFactory();
  const client = new MewClient("ws://test/", { socketFactory: factory });
  const connectP = client.connect();
  setImmediate(() => latest().open());
  await connectP;

  client.prompt("hello", [{ path: "/x.png", mime: "image/png" }]);
  const sent = latest().sent;
  assert.equal(sent.length, 1);
  assert.equal(sent[0]!.type, "prompt");
  assert.equal((sent[0] as any).text, "hello");
  assert.equal((sent[0] as any).attachments.length, 1);
});

test("confirmed deletion resolves only for the matching session and request", async () => {
  const { factory, latest } = makeFactory();
  const client = new MewClient("ws://test/", { socketFactory: factory });
  const connected = client.connect();
  latest().open();
  await connected;
  const removed = client.deleteSessionConfirmed("target");
  const request = latest().sent[0];
  assert.ok(request?.type === "delete_session_confirmed");
  assert.equal(request.session_id, "target");
  latest().peerSend({ type: "session_deletion_result", session_id: "other", request_id: request.request_id, error: null });
  latest().peerSend({ type: "session_deletion_result", session_id: "target", request_id: "unrelated", error: null });
  latest().peerSend({ type: "session_deletion_result", session_id: "target", request_id: request.request_id, error: "conversation is still running" });
  await assert.rejects(removed, /conversation is still running/);

  const retry = client.deleteSessionConfirmed("target");
  const retried = latest().sent[1];
  assert.ok(retried?.type === "delete_session_confirmed");
  assert.notEqual(retried.request_id, request.request_id);
  latest().peerSend({ type: "session_deletion_result", session_id: "target", request_id: retried.request_id, error: null });
  await retry;
});

test("a dropped deletion remains uncertain and sends no commands on the replacement socket", async () => {
  const { factory, latest } = makeFactory();
  const client = new MewClient("ws://test/", { socketFactory: factory });
  const connected = client.connect();
  latest().open();
  await connected;
  const failed = assert.rejects(client.deleteSessionConfirmed("target"), /could not be confirmed/i);
  latest().peerClose(1006);
  await failed;
  const replacement = client.connect();
  latest().open();
  await replacement;
  latest().peerSend({ type: "pong", version: "test", protocol_version: 3 });
  assert.deepEqual(latest().sent, []);
});

test("direct guidance requires the attached parent and accepts only its exact child receipt", async () => {
  const { factory, latest } = makeFactory();
  const client = new MewClient("ws://test/", { socketFactory: factory, clientKind: "desktop" });
  const connected = client.connect();
  latest().open();
  await connected;
  latest().peerSend({ type: "session_ready", session_id: "other-parent" });
  latest().peerSend({ type: "session_ready", session_id: "parent" });
  await assert.rejects(client.guideSubagent("other-parent", "child", "wrong parent"), /attached/i);
  const accepted = client.guideSubagent("parent", "child", "direct guidance");
  const command = latest().sent[0];
  assert.ok(command?.type === "guide_subagent");
  assert.deepEqual({ ...command, request_id: "id" }, {
    type: "guide_subagent", session_id: "parent", child_session_id: "child", request_id: "id", text: "direct guidance",
  });
  const receipt = {
    type: "subagent_message_receipt", session_id: "parent", child_session_id: "child", request_id: command.request_id, status: "accepted",
  } as const;
  latest().peerSend({ ...receipt, session_id: "other-parent", status: "rejected" });
  latest().peerSend({ ...receipt, child_session_id: "other-child", status: "rejected" });
  latest().peerSend({ ...receipt, request_id: "other-request", status: "rejected" });
  latest().peerSend(receipt);
  assert.equal((await accepted).status, "accepted");
});

test("a disconnected guidance receipt can be checked after the exact parent reattaches", async () => {
  const { factory, latest } = makeFactory();
  const client = new MewClient("ws://test/", { socketFactory: factory });
  const connected = client.connect();
  latest().open();
  await connected;
  latest().peerSend({ type: "session_ready", session_id: "parent" });
  const submitted = client.guideSubagent("parent", "child", "keep this draft");
  latest().peerClose(1006);
  const uncertain = await submitted;
  assert.equal(uncertain.status, "unknown");
  const replacement = client.connect();
  latest().open();
  await replacement;
  await assert.rejects(client.readSubagentMessageReceipt("parent", "child", uncertain.request_id), /attached/i);
  latest().peerSend({ type: "session_ready", session_id: "parent" });
  const requested = client.readSubagentMessageReceipt("parent", "child", uncertain.request_id);
  assert.deepEqual(latest().sent, [{
    type: "read_subagent_message_receipt", session_id: "parent", child_session_id: "child", request_id: uncertain.request_id,
  }]);
  latest().peerSend({ ...uncertain, status: "accepted" });
  assert.equal((await requested).status, "accepted");
});

test("guidance timeout returns the original request as uncertain", async (context) => {
  context.mock.timers.enable({ apis: ["setTimeout"] });
  const { factory, latest } = makeFactory();
  const client = new MewClient("ws://test/", { socketFactory: factory });
  const connected = client.connect();
  latest().open();
  await connected;
  latest().peerSend({ type: "session_ready", session_id: "parent" });
  const result = client.guideSubagent("parent", "child", "review this", "original-request");
  context.mock.timers.tick(10000);
  assert.deepEqual(await result, {
    type: "subagent_message_receipt", session_id: "parent", child_session_id: "child",
    request_id: "original-request", status: "unknown",
  });
  assert.equal(latest().sent.length, 1);
});

test("provider events are dispatched to listeners", async () => {
  const { factory, latest } = makeFactory();
  const client = new MewClient("ws://test/", { socketFactory: factory });
  const connectP = client.connect();
  setImmediate(() => latest().open());
  await connectP;

  const got: ServerMessage[] = [];
  client.on("provider", (ev) => got.push({ type: "provider", event: ev } as any));

  latest().peerSend({
    type: "provider",
    event: {
      type: "part_delta",
      part_id: "p1",
      field: "text",
      delta: "hi",
    },
  });

  await new Promise((r) => setImmediate(r));
  assert.equal(got.length, 1);
  assert.equal((got[0] as any).event.delta, "hi");
});

test("browser messages are dispatched by the SDK", async () => {
  const { factory, latest } = makeFactory();
  const client = new MewClient("ws://test/", { socketFactory: factory });
  const connectP = client.connect();
  setImmediate(() => latest().open());
  await connectP;

  const states: Array<{ open: boolean; url?: string; title?: string; tabId?: string }> = [];
  client.on("browser-state", (data) => states.push(data));
  const errors: Array<{ message: string; tabId?: string }> = [];
  client.on("browser-error", (data) => errors.push(data));

  client.browserOpen("https://example.com", "browser-2");
  assert.deepEqual(latest().sent[0], {
    type: "browser_open",
    url: "https://example.com",
    tab_id: "browser-2",
  });

  assert.doesNotThrow(() => {
    latest().peerSend({
      type: "browser_state",
      open: true,
      url: "https://example.com",
      title: "Example Domain",
      tab_id: "browser-2",
    });
  });
  await new Promise((r) => setImmediate(r));

  assert.deepEqual(states, [{
    open: true,
    url: "https://example.com",
    title: "Example Domain",
    tabId: "browser-2",
  }]);

  latest().peerSend({
    type: "browser_error",
    message: "browser open failed",
    tab_id: "browser-1",
  });
  await new Promise((r) => setImmediate(r));
  assert.deepEqual(errors, [{ message: "browser open failed", tabId: "browser-1" }]);
});

test("permission-request handler can respond", async () => {
  const { factory, latest } = makeFactory();
  const client = new MewClient("ws://test/", { socketFactory: factory });
  const connectP = client.connect();
  setImmediate(() => latest().open());
  await connectP;

  client.on("permission-request", (req, respond) => {
    assert.equal(req.tool_name, "bash");
    respond("allow_once");
  });

  latest().peerSend({
    type: "permission_request",
    request_id: "uuid-7",
    tool_name: "bash",
    input: { command: "ls" },
  });

  await new Promise((r) => setImmediate(r));
  // The PermissionResponse is auto-sent by the library via respond().
  const last = latest().sent[latest().sent.length - 1]!;
  assert.equal(last.type, "permission_response");
  assert.equal((last as any).request_id, "uuid-7");
  assert.equal((last as any).decision, "allow_once");
});

test("slashCommand resolves with the slash_result text", async () => {
  const { factory, latest } = makeFactory();
  const client = new MewClient("ws://test/", { socketFactory: factory });
  const connectP = client.connect();
  setImmediate(() => latest().open());
  await connectP;

  const resultP = client.slashCommand("/clear");
  setImmediate(() => {
    latest().peerSend({ type: "slash_result", text: "context cleared" });
  });
  const result = await resultP;
  assert.equal(result, "context cleared");
});

test("off() unsubscribes a handler", async () => {
  const { factory, latest } = makeFactory();
  const client = new MewClient("ws://test/", { socketFactory: factory });
  const connectP = client.connect();
  setImmediate(() => latest().open());
  await connectP;

  let count = 0;
  const cb = () => { count++; };
  client.on("tool-start", cb);
  latest().peerSend({ type: "tool_start", call_id: "c1" });
  await new Promise((r) => setImmediate(r));
  client.off("tool-start", cb);
  latest().peerSend({ type: "tool_start", call_id: "c2" });
  await new Promise((r) => setImmediate(r));
  assert.equal(count, 1);
});

test("dispatches nested tool events without interrupting the connection", async () => {
  const { factory, latest } = makeFactory();
  const client = new MewClient("ws://test/", { socketFactory: factory });
  const connectP = client.connect();
  setImmediate(() => latest().open());
  await connectP;
  const events: unknown[] = [];
  client.on("nested-tool-start", (call) => events.push(call));
  client.on("nested-tool-end", (call) => events.push(call));
  latest().peerSend({ type: "nested_tool_start", parent_call_id: "py", call_id: "py:1", tool_name: "read", input: { path: "a" } });
  latest().peerSend({ type: "nested_tool_end", parent_call_id: "py", call_id: "py:1", success: true });
  await new Promise((r) => setImmediate(r));
  assert.deepEqual(events, [
    { parent_call_id: "py", call_id: "py:1", tool_name: "read", input: { path: "a" } },
    { parent_call_id: "py", call_id: "py:1", success: true, error: undefined, output_bytes: undefined },
  ]);
});

test("cancel sends a cancel message", async () => {
  const { factory, latest } = makeFactory();
  const client = new MewClient("ws://test/", { socketFactory: factory });
  const connectP = client.connect();
  setImmediate(() => latest().open());
  await connectP;

  client.cancel();
  const sent = latest().sent;
  assert.equal(sent[sent.length - 1]!.type, "cancel");
});

test("guide sends a guide message", async () => {
  const { factory, latest } = makeFactory();
  const client = new MewClient("ws://test/", { socketFactory: factory });
  const connectP = client.connect();
  setImmediate(() => latest().open());
  await connectP;

  client.guide("steer now");
  const sent = latest().sent;
  assert.equal(sent[sent.length - 1]!.type, "guide");
  assert.equal((sent[sent.length - 1] as any).text, "steer now");
});

test("malformed JSON from peer surfaces as an error event", async () => {
  const { factory, latest } = makeFactory();
  const client = new MewClient("ws://test/", { socketFactory: factory });
  const connectP = client.connect();
  setImmediate(() => latest().open());
  await connectP;

  let errored = false;
  client.on("error", () => { errored = true; });
  latest().fire("message", { data: "not json" });
  await new Promise((r) => setImmediate(r));
  assert.equal(errored, true);
});

test("error and error_event are routed to distinct handlers", async () => {
  const { factory, latest } = makeFactory();
  const client = new MewClient("ws://test/", { socketFactory: factory });
  const connectP = client.connect();
  setImmediate(() => latest().open());
  await connectP;

  let errorMessage = "";
  let errorEvent = "";
  client.on("errorMessage", (d) => { errorMessage = d.message; });
  client.on("errorEvent", (d) => { errorEvent = d.message; });

  latest().peerSend({ type: "error", message: "before-session" });
  latest().peerSend({ type: "error_event", message: "during-stream" });
  await new Promise((r) => setImmediate(r));
  assert.equal(errorMessage, "before-session");
  assert.equal(errorEvent, "during-stream");
});

test("session-usage-changed carries the current context reading", async () => {
  const { factory, latest } = makeFactory();
  const client = new MewClient("ws://test/", { socketFactory: factory });
  const connectP = client.connect();
  setImmediate(() => latest().open());
  await connectP;

  const got: Array<{ session_id: string; context_tokens?: number | null }> = [];
  client.on("session-usage-changed", (data) => got.push(data));

  latest().peerSend({
    type: "session_usage_changed",
    session_id: "sess_1",
    usage: {
      input_tokens: 5000,
      output_tokens: 700,
      cache_read_tokens: 0,
      cache_write_tokens: 0,
      cost: 0.02,
      turns: 3,
    },
    context_tokens: 4200,
  });
  await new Promise((r) => setImmediate(r));
  assert.equal(got.length, 1);
  assert.equal(got[0]!.session_id, "sess_1");
  assert.equal(got[0]!.context_tokens, 4200);

  // The field is optional on the wire (older daemons omit it).
  latest().peerSend({
    type: "session_usage_changed",
    session_id: "sess_1",
    usage: {
      input_tokens: 1,
      output_tokens: 1,
      cache_read_tokens: 0,
      cache_write_tokens: 0,
      cost: 0,
      turns: 1,
    },
  });
  await new Promise((r) => setImmediate(r));
  assert.equal(got.length, 2);
  assert.equal(got[1]!.context_tokens, undefined);
  latest().peerSend({ type: "session_usage_changed", session_id: "sess_1", usage: {
    input_tokens: 1, output_tokens: 1, cache_read_tokens: 0, cache_write_tokens: 0, cost: 0, turns: 1,
  }, context_tokens: null });
  await new Promise((r) => setImmediate(r));
  assert.equal(got[2]!.context_tokens, null);
});

test("todo edits wait for matching session and request acknowledgements", async () => {
  const { factory, latest } = makeFactory();
  const client = new MewClient("ws://test/", { socketFactory: factory });
  const connect = client.connect();
  latest().open();
  await connect;
  const edit = { todo_id: 1, expected_content: "original", expected_status: "pending", content: "changed", status: "blocked" };
  let finished = false;
  const pending = client.updateTodo("session", edit).then((todos) => { finished = true; return todos; });
  const request = latest().sent.at(-1)!;
  assert.equal(request.type, "update_todo");
  if (request.type !== "update_todo") throw new Error("unexpected request");
  latest().peerSend({ type: "todo_updated", session_id: "other", request_id: request.request_id, todos: [] });
  latest().peerSend({ type: "todo_updated", session_id: "session", request_id: "other", todos: [] });
  await Promise.resolve();
  assert.equal(finished, false);
  const authoritative = [{ id: 1, content: "changed", status: "blocked", depends_on: [4] }];
  latest().peerSend({ type: "todo_updated", session_id: "session", request_id: request.request_id, todos: authoritative });
  assert.deepEqual(await pending, authoritative);
  const failure = client.updateTodo("session", edit);
  const second = latest().sent.at(-1)!;
  if (second.type !== "update_todo") throw new Error("unexpected request");
  latest().peerSend({ type: "todo_update_error", session_id: "session", request_id: second.request_id, message: "changed while editing" });
  await assert.rejects(failure, /changed while editing/);
});

test("todo edits report disconnect and reject sends without a connection", async () => {
  const { factory, latest } = makeFactory();
  const client = new MewClient("ws://test/", { socketFactory: factory });
  const edit = { todo_id: 1, expected_content: "original", expected_status: "pending", content: "changed", status: "blocked" };
  await assert.rejects(client.updateTodo("session", edit), /not connected/);
  const connect = client.connect();
  latest().open();
  await connect;
  const pending = client.updateTodo("session", edit);
  latest().peerClose();
  await assert.rejects(pending, /disconnected before confirming/);
});

test("unsupported todo editing times out while unrelated errors remain visible", async () => {
  const { factory, latest } = makeFactory();
  const client = new MewClient("ws://test/", { socketFactory: factory });
  const connect = client.connect();
  latest().open();
  await connect;
  const errors: string[] = [];
  client.on("errorMessage", (data) => errors.push(data.message));
  const pending = client.updateTodo("session", { todo_id: 1, expected_content: "original", expected_status: "pending", content: "changed", status: "blocked" });
  latest().peerSend({ type: "error", message: "unknown variant update_todo" });
  await assert.rejects(pending, /not confirmed; update or reconnect/);
  assert.deepEqual(errors, ["unknown variant update_todo"]);
});

test("subagent history preserves viewer identity and never replaces session history", async () => {
  const { factory, latest } = makeFactory();
  const client = new MewClient("ws://test/", { socketFactory: factory });
  const connect = client.connect();
  latest().open();
  await connect;
  const replies: unknown[] = [];
  let sessionHistory = false;
  client.on("subagent-history", (reply) => replies.push(reply));
  client.on("session-history", () => { sessionHistory = true; });
  client.readSubagentHistory("parent", "child", "request");
  assert.deepEqual(latest().sent.at(-1), { type: "read_subagent_history", session_id: "parent", child_session_id: "child", request_id: "request" });
  const reply = { type: "subagent_history" as const, session_id: "parent", child_session_id: "child", request_id: "request", messages: [], error: "missing transcript" };
  latest().peerSend(reply);
  assert.deepEqual(replies, [{ session_id: "parent", child_session_id: "child", request_id: "request", messages: [], error: "missing transcript" }]);
  assert.equal(sessionHistory, false);
});

test("child questions retain source identity and can close only their request", async () => {
  const { factory, latest } = makeFactory();
  const client = new MewClient("ws://test/", { socketFactory: factory });
  const connected = client.connect();
  setImmediate(() => latest().open());
  await connected;
  const requests: unknown[] = [];
  client.on("ask-user-request", (request) => requests.push(request));
  const subagent = {
    parent_call_id: "parent-task",
    child_session_id: "child-session",
    name: "worker",
    display_name: "Curie",
  };
  latest().peerSend({ type: "ask_user_request", request_id: "child-q", call_id: "ask", questions: [], subagent });
  assert.deepEqual(requests, [{ request_id: "child-q", call_id: "ask", questions: [], subagent }]);
  client.cancelAskUser("child-q");
  assert.deepEqual(latest().sent, [{ type: "ask_user_response", request_id: "child-q", answers: [], cancelled: true }]);
});

test("activity snapshots preserve their session and exact task identity", async () => {
  const { factory, latest } = makeFactory();
  const client = new MewClient("ws://test/", { socketFactory: factory });
  const connected = client.connect();
  latest().open();
  await connected;
  const snapshots: unknown[] = [];
  client.on("session-activity-snapshot", (snapshot) => snapshots.push(snapshot));
  const snapshot = {
    session_id: "active",
    subagents: [{ task_id: "task", child_session_id: "child", name: "worker", display_name: "Curie", status: "running", tool_name: "read" }],
    jobs: [{ job_id: "job", command: "cargo test", state: "running" }],
  };
  latest().peerSend({ type: "session_activity_snapshot", ...snapshot });
  assert.deepEqual(snapshots, [snapshot]);
});

test("job output chunks preserve request scope, offsets and incomplete status", async () => {
  const { factory, latest } = makeFactory();
  const client = new MewClient("ws://test/", { socketFactory: factory });
  const connected = client.connect();
  latest().open();
  await connected;
  const replies: unknown[] = [];
  const errors: unknown[] = [];
  client.on("job-output", (reply) => replies.push(reply));
  client.on("job-output-error", (error) => errors.push(error));
  client.readJobOutput("session", "job", "read", 65_535);
  assert.deepEqual(latest().sent.at(-1), { type: "read_job_output", session_id: "session", job_id: "job", request_id: "read", offset: 65_535 });
  const chunk = { session_id: "session", job_id: "job", request_id: "read", offset: 65_535, next_offset: 65_538, output: "猫", done: true, incomplete: true };
  latest().peerSend({ type: "job_output", ...chunk });
  assert.deepEqual(replies, [chunk]);
  const error = { session_id: "session", job_id: "job", request_id: "next", message: "invalid output offset" };
  latest().peerSend({ type: "job_output_error", ...error });
  assert.deepEqual(errors, [error]);
});


test("browser page registration drains its matching acknowledgement before revocation", async () => {
  const { factory, latest } = makeFactory();
  const client = new MewClient("ws://test/", { socketFactory: factory });
  const connected = client.connect();
  latest().open();
  await connected;
  const registered = client.registerBrowserPage("chat", "page-1", "http://127.0.0.1:9222", "target-1");
  assert.deepEqual(latest().sent.at(-1), { type: "register_browser_page", session_id: "chat", page_instance: "page-1", cdp_endpoint: "http://127.0.0.1:9222", target_id: "target-1" });
  latest().peerSend({ type: "browser_page_registration", session_id: "other", page_instance: "page-1", error: null });
  latest().peerSend({ type: "browser_page_registration", session_id: "chat", page_instance: "page-1", error: null });
  await registered;
  const revoked = client.unregisterBrowserPage("chat", "page-1");
  assert.deepEqual(latest().sent.at(-1), { type: "unregister_browser_page", session_id: "chat", page_instance: "page-1" });
  latest().peerSend({ type: "browser_page_registration", session_id: "chat", page_instance: "page-1", error: null });
  await revoked;
  const rejected = client.registerBrowserPage("chat", "page-2", "http://127.0.0.1:9222", "target-2");
  latest().peerSend({ type: "browser_page_registration", session_id: "chat", page_instance: "page-2", error: "page rejected" });
  await assert.rejects(rejected, /page rejected/);
  client.disconnect();
});

test("browser page registration includes an optional native focus endpoint", async () => {
  const { factory, latest } = makeFactory();
  const client = new MewClient("ws://test/", { socketFactory: factory });
  const connected = client.connect();
  latest().open();
  await connected;
  latest().peerSend({ type: "session_ready", session_id: "chat" });
  const registered = client.registerBrowserPage("chat", "page-1", "http://127.0.0.1:9222", "target-1", "http://127.0.0.1:9223/focus/page-1");
  const preflight = latest().sent.slice();
  latest().peerSend({ type: "pong", version: "0.1.0", protocol_version: 3, browser_focus_preflight: true });
  await new Promise((resolve) => setImmediate(resolve));
  const request = latest().sent.at(-1);
  latest().peerSend({ type: "browser_page_registration", session_id: "chat", page_instance: "page-1", error: null });
  await registered;
  client.disconnect();
  assert.deepEqual(preflight, [{ type: "ping" }]);
  assert.deepEqual(request, { type: "register_browser_page", session_id: "chat", page_instance: "page-1", cdp_endpoint: "http://127.0.0.1:9222", target_id: "target-1", focus_url: "http://127.0.0.1:9223/focus/page-1" });
});

for (const capability of [undefined, false]) {
  test(`browser registration rejects a daemon with focus preflight capability ${capability}`, async () => {
    const { factory, latest } = makeFactory();
    const client = new MewClient("ws://test/", { socketFactory: factory, clientKind: "desktop" });
    const connected = client.connect();
    latest().open();
    await connected;
    latest().peerSend({ type: "session_ready", session_id: "chat" });
    const registered = client.registerBrowserPage("chat", "page", "http://127.0.0.1:9222", "target", "http://127.0.0.1:9223/focus");
    latest().peerSend({ type: "pong", version: "old", protocol_version: 3, browser_focus_preflight: capability });
    latest().peerSend({ type: "browser_page_registration", session_id: "chat", page_instance: "page", error: null });
    await assert.rejects(registered, /update.*daemon/i);
    assert.deepEqual(latest().sent, [{ type: "ping" }]);
    client.disconnect();
  });
}

test("browser focus capability is shared by concurrent registrations and refreshed after reconnect", async () => {
  const { factory, latest } = makeFactory();
  const client = new MewClient("ws://test/", { socketFactory: factory, clientKind: "desktop" });
  const connected = client.connect();
  const first = latest();
  first.open();
  await connected;
  first.peerSend({ type: "session_ready", session_id: "chat" });
  const register = (page: string) => client.registerBrowserPage("chat", page, "http://127.0.0.1:9222", `target-${page}`, `http://127.0.0.1:9223/${page}`);
  const one = register("one");
  const two = register("two");
  assert.deepEqual(first.sent, [{ type: "ping" }]);
  first.peerSend({ type: "pong", version: "new", protocol_version: 3, browser_focus_preflight: true });
  await new Promise((resolve) => setImmediate(resolve));
  assert.deepEqual(first.sent.slice(1).map((message) => message.type), ["register_browser_page", "register_browser_page"]);
  for (const page_instance of ["one", "two"]) first.peerSend({ type: "browser_page_registration", session_id: "chat", page_instance, error: null });
  await Promise.all([one, two]);
  client.disconnect();
  const connectedAgain = client.connect();
  const second = latest();
  second.open();
  await connectedAgain;
  await assert.rejects(register("unconfirmed"), /confirmed.*session/i);
  assert.deepEqual(second.sent, []);
  second.peerSend({ type: "session_ready", session_id: "chat" });
  const three = register("three");
  assert.deepEqual(second.sent, [{ type: "ping" }]);
  first.peerSend({ type: "pong", version: "new", browser_focus_preflight: true });
  second.peerSend({ type: "pong", version: "old", protocol_version: 3 });
  await assert.rejects(three, /update.*daemon/i);
  assert.deepEqual(second.sent, [{ type: "ping" }]);
  client.disconnect();
});

test("closing a connection during focus capability discovery cannot register on a replacement", async () => {
  const { factory, latest } = makeFactory();
  const client = new MewClient("ws://test/", { socketFactory: factory, clientKind: "desktop" });
  const connected = client.connect();
  latest().open();
  await connected;
  latest().peerSend({ type: "session_ready", session_id: "chat" });
  const closed = client.registerBrowserPage("chat", "closed", "http://127.0.0.1:9222", "target", "http://127.0.0.1:9223/focus");
  const rejected = assert.rejects(closed, /disconnect/i);
  client.disconnect();
  await rejected;
  const connectedAgain = client.connect();
  latest().open();
  await connectedAgain;
  latest().peerSend({ type: "session_ready", session_id: "chat" });
  const registered = client.registerBrowserPage("chat", "changed", "http://127.0.0.1:9222", "target", "http://127.0.0.1:9223/focus");
  latest().peerSend({ type: "pong", version: "new", browser_focus_preflight: true });
  client.disconnect();
  await assert.rejects(registered, /connection changed/i);
});

test("focus capability discovery times out cleanly and can be retried", async (context) => {
  context.mock.timers.enable({ apis: ["setTimeout"] });
  const { factory, latest } = makeFactory();
  const client = new MewClient("ws://test/", { socketFactory: factory, clientKind: "desktop" });
  const connected = client.connect();
  latest().open();
  await connected;
  latest().peerSend({ type: "session_ready", session_id: "chat" });
  const register = () => client.registerBrowserPage("chat", "page", "http://127.0.0.1:9222", "target", "http://127.0.0.1:9223/focus");
  const timedOut = assert.rejects(register(), /did not confirm.*focus/i);
  context.mock.timers.tick(3000);
  await timedOut;
  latest().peerSend({ type: "pong", version: "late", browser_focus_preflight: true });
  assert.deepEqual(latest().sent, [{ type: "ping" }]);
  const retried = register();
  assert.deepEqual(latest().sent, [{ type: "ping" }, { type: "ping" }]);
  latest().peerSend({ type: "pong", version: "new", browser_focus_preflight: true });
  await new Promise((resolve) => setImmediate(resolve));
  latest().peerSend({ type: "browser_page_registration", session_id: "chat", page_instance: "page", error: null });
  await retried;
  client.disconnect();
});

test("session confirmations belong to the current connection and retain background sessions", async () => {
  const { factory, latest } = makeFactory();
  const client = new MewClient("ws://test/", { socketFactory: factory, clientKind: "desktop" });
  const connected = client.connect();
  const first = latest();
  first.open();
  await connected;
  assert.equal(client.hasConfirmedSession("chat-a"), false);
  let confirmedBeforeReady = false;
  client.on("session-ready", ({ session_id }) => { confirmedBeforeReady = client.hasConfirmedSession(session_id); });
  first.peerSend({ type: "session_ready", session_id: "chat-a" });
  first.peerSend({ type: "session_ready", session_id: "chat-b" });
  assert.equal(confirmedBeforeReady, true);
  assert.equal(client.hasConfirmedSession("chat-a"), true);
  assert.equal(client.hasConfirmedSession("chat-b"), true);
  first.peerClose();
  assert.equal(client.hasConfirmedSession("chat-a"), false);
  assert.equal(client.hasConfirmedSession("chat-b"), false);
  assert.equal(client.getSessionId(), "chat-b");
  const reconnected = client.connect();
  latest().open();
  await reconnected;
  assert.equal(client.hasConfirmedSession("chat-b"), false);
  latest().peerSend({ type: "session_ready", session_id: "chat-a" });
  first.peerSend({ type: "session_ready", session_id: "chat-b" });
  first.peerClose();
  assert.equal(client.hasConfirmedSession("chat-a"), true);
  assert.equal(client.hasConfirmedSession("chat-b"), false);
  client.disconnect();
  assert.equal(client.hasConfirmedSession("chat-a"), false);
  assert.equal(client.getSessionId(), "chat-a");
});

test("a failed socket loses its session confirmations before a replacement opens", async () => {
  const { factory, latest } = makeFactory();
  const client = new MewClient("ws://test/", { socketFactory: factory, clientKind: "desktop" });
  const connected = client.connect();
  latest().open();
  await connected;
  latest().peerSend({ type: "session_ready", session_id: "chat" });
  assert.equal(client.hasConfirmedSession("chat"), true);
  latest().peerError();
  assert.equal(client.hasConfirmedSession("chat"), false);
  const reconnected = client.connect();
  latest().open();
  await reconnected;
  assert.equal(client.hasConfirmedSession("chat"), false);
  client.disconnect();
});

test("queue helpers send wire messages and dispatch session-queue-changed events", async () => {
  const { factory, latest } = makeFactory();
  const client = new MewClient("ws://test/", { socketFactory: factory });
  const connected = client.connect();
  latest().open();
  await connected;

  const updates: unknown[] = [];
  client.on("session-queue-changed", (data) => updates.push(data));

  client.enqueueMessage("chat-1", "follow up", "follow_up", []);
  client.enqueueMessage("chat-1", "steer now", "steer", []);
  client.updateQueuedMessage("chat-1", "q-1", { mode: "steer" });
  client.removeQueuedMessage("chat-1", "q-1");

  const recent = latest().sent.slice(-4);
  assert.equal(typeof (recent[0] as { request_id?: string }).request_id, "string");
  assert.equal(typeof (recent[1] as { request_id?: string }).request_id, "string");
  assert.deepEqual(recent, [
    {
      type: "enqueue_message",
      session_id: "chat-1",
      request_id: (recent[0] as { request_id: string }).request_id,
      text: "follow up",
      attachments: [],
      mode: "follow_up",
    },
    {
      type: "enqueue_message",
      session_id: "chat-1",
      request_id: (recent[1] as { request_id: string }).request_id,
      text: "steer now",
      attachments: [],
      mode: "steer",
    },
    {
      type: "update_queued_message",
      session_id: "chat-1",
      id: "q-1",
      mode: "steer",
    },
    {
      type: "remove_queued_message",
      session_id: "chat-1",
      id: "q-1",
    },
  ]);

  const item = { id: "q-1", text: "follow up", mode: "follow_up" as const, created_at: 42 };
  latest().peerSend({ type: "session_queue_changed", session_id: "chat-1", items: [item] });
  assert.deepEqual(updates, [{ session_id: "chat-1", items: [item] }]);
  client.disconnect();
});

test("compacting, steered user messages, and cancelled idle activity dispatch expected payloads", async () => {
  const { factory, latest } = makeFactory();
  const client = new MewClient("ws://test/", { socketFactory: factory });
  const connected = client.connect();
  latest().open();
  await connected;

  const compacting: unknown[] = [];
  const userMessages: unknown[] = [];
  const activity: unknown[] = [];
  client.on("session-compacting", (d) => compacting.push(d));
  client.on("user-message", (d) => userMessages.push(d));
  client.on("session-activity-changed", (d) => activity.push(d));

  latest().peerSend({ type: "session_compacting", session_id: "chat-1", active: true });
  latest().peerSend({
    type: "session_compacting",
    session_id: "chat-1",
    active: false,
    removed: 14,
    summary: "## Goal & Constraints\nShip summary",
  });
  latest().peerSend({ type: "user_message", text: "change course", steered: true });
  latest().peerSend({ type: "session_activity_changed", session_id: "chat-1", activity: "idle", cancelled: true });

  assert.deepEqual(compacting, [
    { session_id: "chat-1", active: true },
    { session_id: "chat-1", active: false, removed: 14, summary: "## Goal & Constraints\nShip summary" },
  ]);
  assert.deepEqual(userMessages, [{ text: "change course", steered: true }]);
  assert.deepEqual(activity, [{ session_id: "chat-1", activity: "idle", cancelled: true }]);
  client.disconnect();
});
