import { act, fireEvent, render, renderHook, screen } from "@testing-library/react";
import { createMemoryHistory, createRootRoute, createRoute, createRouter, RouterProvider } from "@tanstack/react-router";
import { MewClient, type ClientMessage, type MewWebSocket, type ServerMessage } from "@mew/web-client";
import { afterEach, expect, it, vi } from "vitest";
import { useMewConnection, useSessionAttach } from "../lib/hooks";
import { RightRail } from "../components/right-rail";
import { setClient } from "../lib/client-ref";
import { Route as SessionsSettingsRoute } from "../routes/settings.sessions";
import { bridgeClientToStore, useSessionStore } from "../stores/session";

let client: MewClient;
vi.mock("../lib/client", () => ({
  getClient: () => client,
  SESSION_ID_KEY: "mew.sessionId",
}));

class SocketFixture extends EventTarget {
  sent: ClientMessage[] = [];

  send(data: string) { this.sent.push(JSON.parse(data)); }
  close(code = 1000, reason = "") { this.dispatchEvent(new CloseEvent("close", { code, reason })); }
  open() { this.dispatchEvent(new Event("open")); }
  reply(message: ServerMessage) {
    this.dispatchEvent(new MessageEvent("message", { data: JSON.stringify(message) }));
  }
}

afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
  localStorage.removeItem("mew.sessionId");
});

it("keeps the selected conversation and waits for its fresh confirmation after reconnecting", async () => {
  vi.useFakeTimers();
  useSessionStore.setState({ sessionId: null, confirmedSessionId: null, connectionState: "disconnected" });
  const sockets: SocketFixture[] = [];
  client = new MewClient("ws://localhost/", {
    socketFactory: () => {
      const socket = new SocketFixture();
      sockets.push(socket);
      return socket as unknown as MewWebSocket;
    },
  });
  bridgeClientToStore(client);
  const { result, unmount } = renderHook(() => {
    useMewConnection();
    useSessionAttach("the-chat");
    return useSessionStore((state) => state.connectionState === "connected" && state.confirmedSessionId === "the-chat");
  });
  const attach = { type: "attach_session", session_id: "the-chat", client_kind: "web" };
  const protocol = { type: "pong", version: "test", protocol_version: 3 } as const;
  try {
    await act(async () => sockets[0]!.open());
    expect(sockets[0]!.sent).toEqual([{ type: "ping" }]);
    await act(async () => sockets[0]!.reply(protocol));
    expect(sockets[0]!.sent).toContainEqual(attach);
    expect(result.current).toBe(false);
    await act(async () => sockets[0]!.reply({ type: "session_ready", session_id: "the-chat" }));
    expect(result.current).toBe(true);

    await act(async () => sockets[0]!.close(1006));
    expect(result.current).toBe(false);
    expect(useSessionStore.getState().sessionId).toBe("the-chat");
    await act(async () => vi.advanceTimersByTimeAsync(5000));
    await act(async () => sockets[1]!.open());
    expect(sockets[1]!.sent).toEqual([{ type: "ping" }]);
    await act(async () => sockets[1]!.reply(protocol));
    expect(sockets[1]!.sent).toContainEqual(attach);
    expect(result.current).toBe(false);
    await act(async () => sockets[1]!.reply({ type: "session_ready", session_id: "the-chat" }));
    expect(result.current).toBe(true);
  } finally {
    unmount();
  }
});

it("keeps a chat in settings until its deletion is confirmed", async () => {
  vi.spyOn(window, "scrollTo").mockImplementation(() => {});
  const socket = new SocketFixture();
  client = new MewClient("ws://localhost/", { socketFactory: () => socket as unknown as MewWebSocket });
  bridgeClientToStore(client);
  const connected = client.connect();
  socket.open();
  await connected;
  useSessionStore.setState({
    connectionState: "connected",
    sessionId: "other",
    sessionTitles: new Map([["target", "Chat to remove"], ["other", "Other chat"]]),
    availableSessions: [
      { session_id: "target", state: "idle", created_at: 2, client_count: 0 },
      { session_id: "other", state: "idle", created_at: 1, client_count: 0 },
    ],
  });
  const root = createRootRoute();
  const settings = createRoute({
    getParentRoute: () => root,
    path: "/settings/sessions",
    component: SessionsSettingsRoute.options.component,
  });
  const router = createRouter({
    routeTree: root.addChildren([settings]),
    history: createMemoryHistory({ initialEntries: ["/settings/sessions"] }),
  });
  const { unmount } = render(<RouterProvider router={router} />);
  try {
    const [deleteButton] = await screen.findAllByTitle("Delete session");
    fireEvent.click(deleteButton!);
    fireEvent.click(screen.getByRole("button", { name: "Delete" }));
    expect(screen.getByRole<HTMLButtonElement>("button", { name: "Deleting…" }).disabled).toBe(true);
    expect(screen.getByText("Chat to remove")).toBeTruthy();
    const [request] = socket.sent.filter((message) => message.type === "delete_session_confirmed");
    expect(request?.session_id).toBe("target");
    await act(async () => socket.reply({
      type: "session_deletion_result", session_id: "target", request_id: request!.request_id, error: "This chat is still running.",
    }));
    expect(screen.getByRole("alert").textContent).toContain("This chat is still running.");
    expect(screen.getByText("Chat to remove")).toBeTruthy();
    expect(router.state.location.pathname).toBe("/settings/sessions");

    fireEvent.click(screen.getByRole("button", { name: "Delete" }));
    const retry = socket.sent.filter((message) => message.type === "delete_session_confirmed").at(-1);
    await act(async () => socket.reply({
      type: "session_deletion_result", session_id: "target", request_id: retry!.request_id, error: null,
    }));
    expect(screen.queryByText("Chat to remove")).toBeNull();
    expect(screen.getByText("Other chat")).toBeTruthy();
  } finally {
    unmount();
    client.disconnect();
  }
});

it("opens each agent's history and keeps rejected guidance with its original child", async () => {
  const socket = new SocketFixture();
  client = new MewClient("ws://localhost/", { socketFactory: () => socket as unknown as MewWebSocket });
  setClient(client);
  bridgeClientToStore(client);
  const connected = client.connect();
  socket.open();
  await connected;
  useSessionStore.setState({ connectionState: "connected", subagents: new Map(), messages: [] });
  socket.reply({ type: "session_ready", session_id: "test-parent" });
  for (const child of ["North", "South"]) {
    socket.reply({
      type: "subagent_start", parent_call_id: `${child}-call`, child_session_id: child, name: child, display_name: child,
    });
  }
  const tab = { id: "agents", title: "Agents", kind: "agents" as const, closable: false };
  const { unmount } = render(<RightRail
    mode="dock"
    open
    onOpenChange={() => {}}
    onWorkbenchTabsChange={() => {}}
    workbenchTabs={{ tabs: [tab], activeTabId: tab.id }}
  />);
  try {
    fireEvent.click(screen.getByRole("button", { name: "Open North" }));
    await screen.findByRole("button", { name: "Back to agents" });
    const history = socket.sent.find((message) => message.type === "read_subagent_history");
    expect(history?.child_session_id).toBe("North");
    const reply = {
      type: "subagent_history",
      session_id: "test-parent",
      child_session_id: "North",
      request_id: history!.request_id,
      messages: [{
        id: "north-message",
        session_id: "North",
        role: "assistant",
        parts: [{ id: "north-text", message_id: "north-message", session_id: "North", type: "text", text: "North transcript" }],
        time: { created: 0 },
      }],
      error: null,
    } satisfies ServerMessage;
    await act(async () => socket.reply({ ...reply, child_session_id: "South" }));
    expect(screen.queryByText("North transcript")).toBeNull();
    await act(async () => socket.reply(reply));
    expect(screen.getByText("North transcript")).toBeTruthy();
    expect(useSessionStore.getState().messages).toEqual([]);

    const composer = screen.getByRole<HTMLTextAreaElement>("textbox", { name: "Guidance for North" });
    fireEvent.change(composer, { target: { value: "Try another route" } });
    fireEvent.click(screen.getByRole("button", { name: "Send guidance" }));
    const guidance = socket.sent.find((message) => message.type === "guide_subagent");
    expect(guidance?.child_session_id).toBe("North");
    await act(async () => socket.reply({
      type: "subagent_message_receipt",
      session_id: "test-parent",
      child_session_id: "North",
      request_id: guidance!.request_id,
      status: "rejected",
      message: "The agent could not receive guidance.",
    }));
    expect(composer.value).toBe("Try another route");
    expect(screen.getByRole("alert").textContent).toContain("could not receive guidance");
    fireEvent.click(screen.getByRole("button", { name: "Back to agents" }));
    fireEvent.click(screen.getByRole("button", { name: "Open South" }));
    expect(screen.getByRole<HTMLTextAreaElement>("textbox", { name: "Guidance for South" }).value).toBe("");
    fireEvent.click(screen.getByRole("button", { name: "Back to agents" }));
    fireEvent.click(screen.getByRole("button", { name: "Open North" }));
    const resumed = screen.getByRole<HTMLTextAreaElement>("textbox", { name: "Guidance for North" });
    expect(resumed.value).toBe("Try another route");
    fireEvent.click(screen.getByRole("button", { name: "Send guidance" }));
    const retry = socket.sent.filter((message) => message.type === "guide_subagent").at(-1);
    await act(async () => socket.reply({
      type: "subagent_message_receipt",
      session_id: "test-parent",
      child_session_id: "North",
      request_id: retry!.request_id,
      status: "accepted",
    }));
    expect(resumed.value).toBe("");
    expect(screen.getByText("Guidance saved.")).toBeTruthy();
    await act(async () => socket.reply({
      type: "subagent_end", parent_call_id: "North-call", child_session_id: "North", outcome: { type: "completed" },
    }));
    expect(screen.queryByRole("button", { name: "Send guidance" })).toBeNull();
  } finally {
    unmount();
    client.disconnect();
    setClient(null);
  }
});

it("reconciles guidance after reattachment without overwriting edits and reuses an uncertain request on retry", async () => {
  const sockets: SocketFixture[] = [];
  client = new MewClient("ws://localhost/", { socketFactory: () => {
    const socket = new SocketFixture();
    sockets.push(socket);
    return socket as unknown as MewWebSocket;
  } });
  setClient(client);
  bridgeClientToStore(client);
  const connected = client.connect();
  const first = sockets[0]!;
  first.open();
  await connected;
  const parent = "await-parent";
  const child = "await-child";
  useSessionStore.setState({ connectionState: "connected", subagents: new Map(), subagentDrafts: new Map() });
  first.reply({ type: "session_ready", session_id: parent });
  first.reply({ type: "subagent_start", parent_call_id: "await-call", child_session_id: child, name: "Proof worker", display_name: null });
  const tab = { id: "agents", title: "Agents", kind: "agents" as const, closable: false };
  const { unmount } = render(<RightRail
    mode="dock"
    open
    onOpenChange={() => {}}
    onWorkbenchTabsChange={() => {}}
    workbenchTabs={{ tabs: [tab], activeTabId: tab.id }}
  />);
  const openWorker = async () => {
    fireEvent.click(screen.getByRole("button", { name: "Open Proof worker" }));
    await screen.findByRole("button", { name: "Back to agents" });
  };
  const back = () => fireEvent.click(screen.getByRole("button", { name: "Back to agents" }));
  const composer = () => screen.getByRole<HTMLTextAreaElement>("textbox", { name: "Guidance for Proof worker" });
  const send = () => fireEvent.click(screen.getByRole("button", { name: "Send guidance" }));
  try {
    await openWorker();
    fireEvent.change(composer(), { target: { value: "First guidance" } });
    send();
    const original = first.sent.find((message) => message.type === "guide_subagent")!;
    await act(async () => first.close(1006));
    const reopened = client.connect();
    const next = sockets[1]!;
    next.open();
    await reopened;
    await act(async () => {
      useSessionStore.setState({ connectionState: "connected" });
      next.reply({ type: "session_ready", session_id: "different-parent" });
    });
    expect(next.sent.some(({ type }) => type === "read_subagent_message_receipt")).toBe(false);
    await act(async () => next.reply({ type: "session_ready", session_id: parent }));
    expect(next.sent).toContainEqual({
      type: "read_subagent_message_receipt", session_id: parent, child_session_id: child, request_id: original.request_id,
    });
    expect(next.sent.some(({ type }) => type === "guide_subagent")).toBe(false);
    await act(async () => next.reply({
      type: "session_activity_snapshot", session_id: parent, jobs: [],
      subagents: [{
        task_id: "await-call", child_session_id: child, name: "Proof worker", display_name: null,
        status: "Reading the build log", tool_name: "read",
      }],
    }));
    expect(screen.getByText("↳ Reading the build log")).toBeTruthy();

    await openWorker();
    fireEvent.change(composer(), { target: { value: "A new instruction" } });
    await act(async () => next.reply({
      type: "subagent_end", parent_call_id: "await-call", child_session_id: "stale-child", outcome: { type: "completed" },
    }));
    expect(composer().readOnly).toBe(false);
    expect(screen.getByRole("button", { name: "Checking delivery…" })).toBeTruthy();
    back();
    const reply = {
      type: "subagent_message_receipt", session_id: parent, child_session_id: child, request_id: original.request_id,
    } as const;
    await act(async () => next.reply({ ...reply, status: "accepted" }));
    await openWorker();
    expect(composer().value).toBe("A new instruction");
    send();
    const updated = next.sent.filter((message) => message.type === "guide_subagent").at(-1)!;
    expect(updated.request_id).not.toBe(original.request_id);
    await act(async () => next.reply({ ...reply, request_id: updated.request_id, status: "unknown" }));
    expect(next.sent).toContainEqual({
      type: "read_subagent_message_receipt", session_id: parent, child_session_id: child, request_id: updated.request_id,
    });
    await act(async () => next.reply({ ...reply, request_id: updated.request_id, status: "unknown" }));
    expect(composer().value).toBe("A new instruction");
    send();
    expect(next.sent.filter(({ type }) => type === "guide_subagent").at(-1)).toEqual(updated);
    await act(async () => next.reply({ ...reply, request_id: updated.request_id, status: "accepted" }));
    expect(composer().value).toBe("");
  } finally {
    unmount();
    client.disconnect();
    setClient(null);
  }
});

it("preserves a stable session transcript when switching to another session fails to attach", async () => {
  const socket = new SocketFixture();
  client = new MewClient("ws://localhost/", { socketFactory: () => socket as unknown as MewWebSocket });
  bridgeClientToStore(client);
  const connected = client.connect();
  socket.open();
  await connected;
  socket.reply({ type: "pong", version: "test", protocol_version: 3, scoped_session_failures: true });

  useSessionStore.setState({ connectionState: "connected", sessionId: null, confirmedSessionId: null, messages: [] });
  const { result, rerender, unmount } = renderHook(
    ({ sessionId }: { sessionId: string }) => useSessionAttach(sessionId),
    { initialProps: { sessionId: "sess_a" } },
  );
  try {
    await act(async () => {});
    const attachA = socket.sent.find((message) => message.type === "attach_session");
    expect(attachA?.session_id).toBe("sess_a");
    await act(async () => socket.reply({ type: "session_ready", session_id: "sess_a" }));
    await act(async () => {
      useSessionStore.getState().addUserMessage("keep me");
    });
    expect(result.current.ready).toBe(true);
    expect(useSessionStore.getState().messages).toHaveLength(1);

    rerender({ sessionId: "sess_b" });
    await act(async () => {});
    await act(async () =>
      socket.reply({
        type: "session_operation_failed",
        operation: "attach",
        session_id: "sess_b",
        message: "session not found",
      }),
    );
    expect(result.current.error).toContain("session not found");
    expect(useSessionStore.getState().sessionId).toBe("sess_a");
    expect(useSessionStore.getState().messages).toHaveLength(1);

    rerender({ sessionId: "sess_a" });
    expect(result.current.ready).toBe(true);
    expect(useSessionStore.getState().messages).toHaveLength(1);
    expect(useSessionStore.getState().messages[0]?.parts).toEqual([{ type: "text", text: "keep me" }]);
  } finally {
    unmount();
    client.disconnect();
  }
});
