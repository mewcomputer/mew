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
