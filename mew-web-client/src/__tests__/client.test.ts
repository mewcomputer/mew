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

test("newSession sends NewSession and resolves with session_id", async () => {
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
    assert.equal(ws.sent[0]!.type, "NewSession");
    ws.peerSend({ type: "SessionReady", session_id: "sess_abc" });
  });
  const sessionId = await sessionP;
  assert.equal(sessionId, "sess_abc");
  assert.equal(client.getSessionId(), "sess_abc");
});

test("prompt sends Prompt with text and attachments", async () => {
  const { factory, latest } = makeFactory();
  const client = new MewClient("ws://test/", { socketFactory: factory });
  const connectP = client.connect();
  setImmediate(() => latest().open());
  await connectP;

  client.prompt("hello", [{ path: "/x.png", mime: "image/png" }]);
  const sent = latest().sent;
  assert.equal(sent.length, 1);
  assert.equal(sent[0]!.type, "Prompt");
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
  client.on("provider", (ev) => got.push({ type: "Provider", event: ev } as any));

  latest().peerSend({
    type: "Provider",
    event: {
      type: "PartDelta",
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
    type: "PermissionRequest",
    request_id: "uuid-7",
    tool_name: "bash",
    input: { command: "ls" },
  });

  await new Promise((r) => setImmediate(r));
  // The PermissionResponse is auto-sent by the library via respond().
  const last = latest().sent[latest().sent.length - 1]!;
  assert.equal(last.type, "PermissionResponse");
  assert.equal((last as any).request_id, "uuid-7");
  assert.equal((last as any).decision, "allow_once");
});

test("slashCommand resolves with the SlashResult text", async () => {
  const { factory, latest } = makeFactory();
  const client = new MewClient("ws://test/", { socketFactory: factory });
  const connectP = client.connect();
  setImmediate(() => latest().open());
  await connectP;

  const resultP = client.slashCommand("/clear");
  setImmediate(() => {
    latest().peerSend({ type: "SlashResult", text: "context cleared" });
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
  latest().peerSend({ type: "ToolStart", call_id: "c1" });
  await new Promise((r) => setImmediate(r));
  client.off("tool-start", cb);
  latest().peerSend({ type: "ToolStart", call_id: "c2" });
  await new Promise((r) => setImmediate(r));
  assert.equal(count, 1);
});

test("cancel sends a Cancel message", async () => {
  const { factory, latest } = makeFactory();
  const client = new MewClient("ws://test/", { socketFactory: factory });
  const connectP = client.connect();
  setImmediate(() => latest().open());
  await connectP;

  client.cancel();
  const sent = latest().sent;
  assert.equal(sent[sent.length - 1]!.type, "Cancel");
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

test("Error and ErrorEvent are routed to distinct handlers", async () => {
  const { factory, latest } = makeFactory();
  const client = new MewClient("ws://test/", { socketFactory: factory });
  const connectP = client.connect();
  setImmediate(() => latest().open());
  await connectP;

  let errorMessage = "";
  let errorEvent = "";
  client.on("errorMessage", (d) => { errorMessage = d.message; });
  client.on("errorEvent", (d) => { errorEvent = d.message; });

  latest().peerSend({ type: "Error", message: "before-session" });
  latest().peerSend({ type: "ErrorEvent", message: "during-stream" });
  await new Promise((r) => setImmediate(r));
  assert.equal(errorMessage, "before-session");
  assert.equal(errorEvent, "during-stream");
});