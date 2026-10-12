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
  client.on("session-history", ({ messages }) => {
    history.push(messages.map((message) => message.id));
  });
  const message = (id: string) => ({
    id,
    session_id: "sess_1",
    role: "user" as const,
    parts: [],
    time: { created: 0 },
  });

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
  assert.deepEqual(latest().sent, [{
    type: "load_session_history",
    session_id: "sess_1",
    before: 1,
    limit: 24,
  }]);
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
    assert.deepEqual(replies, [{ version: "0.1.0", protocol_version: expectedProtocol }]);
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

  const got: Array<{ session_id: string; context_tokens?: number }> = [];
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
