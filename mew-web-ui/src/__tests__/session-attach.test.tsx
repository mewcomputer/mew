import { afterEach, expect, it, vi } from "vitest";
import { act, cleanup, renderHook, waitFor } from "@testing-library/react";
import { MewClient, type MewWebSocket, type ServerMessage } from "@mew/web-client";
import { useSessionStore } from "../stores/session";

const fixtures = vi.hoisted(() => ({ client: null as MewClient | null, router: { navigate: vi.fn(), state: { location: { pathname: "/session/wanted" } } } }));
vi.mock("@tanstack/react-router", () => ({ useRouter: () => fixtures.router }));
vi.mock("../lib/client", () => ({ getClient: () => fixtures.client!, SESSION_ID_KEY: "mew.sessionId" }));

import { useSessionAttach } from "../lib/hooks";

afterEach(() => { cleanup(); fixtures.client?.disconnect(); fixtures.client = null; vi.clearAllMocks(); });

class TestSocket extends EventTarget {
  sent: Record<string, unknown>[] = [];
  send(data: string) { this.sent.push(JSON.parse(data)); }
  close() { this.dispatchEvent(new Event("close")); }
  server(message: ServerMessage) { this.dispatchEvent(new MessageEvent("message", { data: JSON.stringify(message) })); }
}

it("reattaches the visible conversation after the connection is replaced", async () => {
  const sockets: TestSocket[] = [];
  const client = new MewClient("ws://test", { socketFactory: () => {
    const socket = new TestSocket();
    sockets.push(socket);
    return socket as unknown as MewWebSocket;
  } });
  fixtures.client = client;
  const first = client.connect();
  sockets[0]!.dispatchEvent(new Event("open"));
  await first;
  sockets[0]!.server({ type: "session_ready", session_id: "chat" });
  useSessionStore.setState({ connectionState: "connected", sessionId: "chat" });
  renderHook(() => useSessionAttach("chat"));
  act(() => { sockets[0]!.close(); useSessionStore.setState({ connectionState: "reconnecting" }); });
  const second = client.connect();
  sockets[1]!.dispatchEvent(new Event("open"));
  await second;
  act(() => useSessionStore.setState({ connectionState: "connected" }));
  await waitFor(() => expect(sockets[1]!.sent).toContainEqual({ type: "attach_session", session_id: "chat", client_kind: "web" }));
  act(() => sockets[1]!.server({ type: "session_ready", session_id: "chat" }));
  await waitFor(() => expect(client.hasConfirmedSession("chat")).toBe(true));
  expect(fixtures.router.navigate).not.toHaveBeenCalled();
});

it("returns to the latest selected session after an intervening attach completes", async () => {
  const socket = new TestSocket();
  const client = new MewClient("ws://test", { socketFactory: () => socket as unknown as MewWebSocket });
  fixtures.client = client;
  const connected = client.connect();
  socket.dispatchEvent(new Event("open"));
  await connected;
  socket.server({ type: "session_ready", session_id: "a" });
  useSessionStore.setState({ connectionState: "connected", sessionId: "a" });
  const hook = renderHook(({ id }) => useSessionAttach(id), { initialProps: { id: "a" } });
  hook.rerender({ id: "b" });
  await waitFor(() => expect(socket.sent).toContainEqual({ type: "attach_session", session_id: "b", client_kind: "web" }));
  hook.rerender({ id: "a" });
  expect(hook.result.current).toBe(false);
  fixtures.router.state.location.pathname = "/settings";
  act(() => {
    socket.server({ type: "session_ready", session_id: "b" });
    useSessionStore.setState({ sessionId: "b" });
  });
  await waitFor(() => expect(socket.sent).toContainEqual({ type: "attach_session", session_id: "a", client_kind: "web" }));
  act(() => socket.server({ type: "session_ready", session_id: "a" }));
  await waitFor(() => expect(client.getSessionId()).toBe("a"));
  expect(fixtures.router.navigate).not.toHaveBeenCalled();
  fixtures.router.state.location.pathname = "/session/wanted";
});
