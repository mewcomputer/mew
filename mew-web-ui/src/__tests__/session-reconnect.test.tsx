import { act, fireEvent, render, renderHook, screen } from "@testing-library/react";
import { createMemoryHistory, createRootRoute, createRoute, createRouter, RouterProvider } from "@tanstack/react-router";
import { MewClient, type ClientMessage, type MewWebSocket, type ServerMessage } from "@mew/web-client";
import { afterEach, expect, it, vi } from "vitest";
import { useMewConnection, useSessionAttach } from "../lib/hooks";
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
