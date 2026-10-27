import { afterEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, waitFor } from "@testing-library/react";
import { NativeBrowserPage } from "../components/native-browser-page";
import { MewClient, type MewWebSocket, type ServerMessage } from "@mew/web-client";
import { initializeHost, resetHost } from "../lib/host";

afterEach(() => { cleanup(); resetHost(); delete window.mewQuery; vi.restoreAllMocks(); document.querySelectorAll('[data-test-overlay], [role="dialog"]').forEach((node) => node.remove()); });

describe("native browser surface", () => {
  it("waits for its daemon session confirmation and requires a fresh confirmation after reconnect", async () => {
    const sockets: Array<EventTarget & { sent: Record<string, unknown>[]; server: (message: ServerMessage) => void }> = [];
    const client = new MewClient("ws://test/", { clientKind: "desktop", socketFactory: () => {
      const socket = Object.assign(new EventTarget(), {
        sent: [] as Record<string, unknown>[],
        send(data: string) { this.sent.push(JSON.parse(data)); },
        close() { socket.dispatchEvent(new CloseEvent("close", { code: 1000, reason: "test disconnect" })); },
        server(message: ServerMessage) { socket.dispatchEvent(new MessageEvent("message", { data: JSON.stringify(message) })); },
      });
      sockets.push(socket);
      return socket as unknown as MewWebSocket;
    } });
    const connect = async () => {
      const connected = client.connect();
      const socket = sockets.at(-1)!;
      socket.dispatchEvent(new Event("open"));
      await connected;
      return socket;
    };
    const first = await connect();
    window.mewQuery = ({ onSuccess }) => onSuccess("{}");
    const props = { client, connected: true, sessionId: "confirmed-chat", tabId: "tab", url: "https://example.com", active: true, onChange: vi.fn(), onError: vi.fn() };
    const ready = () => fireEvent(window, new CustomEvent("mew:browser-event", { detail: { sessionId: props.sessionId, tabId: props.tabId, pageInstance: "confirmed-page", kind: "ready", url: props.url, title: "Example", cdpEndpoint: "http://127.0.0.1:9222", targetId: "target" } }));
    const waiting = render(<NativeBrowserPage {...props} />);
    ready();
    await act(async () => { await new Promise((resolve) => setTimeout(resolve, 0)); });
    expect(first.sent).toEqual([]);
    waiting.unmount();
    await act(async () => { await new Promise((resolve) => setTimeout(resolve, 0)); });
    expect(first.sent).toEqual([]);
    const view = render(<NativeBrowserPage {...props} />);
    ready();
    act(() => first.server({ type: "session_ready", session_id: "other-chat" }));
    expect(first.sent).toEqual([]);
    act(() => first.server({ type: "session_ready", session_id: props.sessionId }));
    const registration = { type: "register_browser_page", session_id: props.sessionId, page_instance: "confirmed-page", cdp_endpoint: "http://127.0.0.1:9222", target_id: "target" };
    await waitFor(() => expect(first.sent).toEqual([registration]));
    expect(props.onChange).not.toHaveBeenCalled();
    act(() => first.server({ type: "browser_page_registration", session_id: props.sessionId, page_instance: "confirmed-page", error: null }));
    await waitFor(() => expect(props.onChange).toHaveBeenCalledOnce());
    act(() => first.server({ type: "session_ready", session_id: props.sessionId }));
    expect(first.sent).toEqual([registration]);
    act(() => { client.disconnect(); });
    view.rerender(<NativeBrowserPage {...props} connected={false} />);
    const second = await connect();
    view.rerender(<NativeBrowserPage {...props} />);
    await act(async () => { await new Promise((resolve) => setTimeout(resolve, 0)); });
    expect(second.sent).toEqual([]);
    act(() => second.server({ type: "session_ready", session_id: "other-chat" }));
    expect(second.sent).toEqual([]);
    act(() => second.server({ type: "session_ready", session_id: props.sessionId }));
    await waitFor(() => expect(second.sent).toEqual([registration]));
    act(() => second.server({ type: "browser_page_registration", session_id: props.sessionId, page_instance: "confirmed-page", error: null }));
    await waitFor(() => expect(props.onChange).toHaveBeenCalledTimes(2));
    view.unmount();
    await waitFor(() => expect(second.sent.at(-1)).toEqual({ type: "unregister_browser_page", session_id: props.sessionId, page_instance: "confirmed-page" }));
    act(() => second.server({ type: "browser_page_registration", session_id: props.sessionId, page_instance: "confirmed-page", error: null }));
    expect(props.onError).not.toHaveBeenCalled();
    client.disconnect();
  });

  it("creates an Electron DOM guest only after its first visible permit and keeps its initial src stable", async () => {
    vi.spyOn(Element.prototype, "getBoundingClientRect").mockReturnValue(new DOMRect(800, 80, 400, 600));
    const calls: Record<string, unknown>[] = [];
    let permit: ((response: string) => void) | undefined;
    window.mewQuery = ({ request, onSuccess }) => {
      const value = JSON.parse(request);
      if (value.type === "bootstrap") { onSuccess(JSON.stringify({ browserMode: "electron", daemonUrl: "ws://127.0.0.1:25566", themeMode: "dark", lightTheme: "light", darkTheme: "dark", themes: [] })); return; }
      calls.push(value);
      if (value.type === "browser_update" && value.visible) permit = onSuccess;
      else onSuccess("{}");
    };
    await initializeHost();
    const props = { client: { on: vi.fn(), off: vi.fn(), hasConfirmedSession: () => false } as unknown as MewClient, connected: false, sessionId: "electron-chat", tabId: "electron-tab", url: "https://example.com", onChange: vi.fn(), onError: vi.fn() };
    const view = render(<NativeBrowserPage {...props} active={false} present={false} />);
    expect(view.container.querySelector("webview")).toBeNull();
    expect(calls.at(-1)).toMatchObject({ visible: false });
    view.rerender(<NativeBrowserPage {...props} active present />);
    expect(view.container.querySelector("webview")).toBeNull();
    expect(permit).toBeDefined();
    permit!(JSON.stringify({ partition: "approved-electron-guest", pageInstance: "approved-page" }));
    await waitFor(() => expect(view.container.querySelector("webview")).not.toBeNull());
    const guest = view.container.querySelector("webview")!;
    expect(guest.getAttribute("partition")).toBe("approved-electron-guest");
    expect(guest.getAttribute("src")).toBe(props.url);
    expect(guest.hasAttribute("preload")).toBe(false);
    Object.assign(guest, { getWebContentsId: () => 42 });
    fireEvent(guest, new Event("dom-ready"));
    expect(calls).toContainEqual({ type: "browser_bind", sessionId: props.sessionId, tabId: props.tabId, pageInstance: "approved-page", guestId: 42 });
    const send = (kind: string, pageInstance: string) => fireEvent(window, new CustomEvent("mew:browser-event", { detail: { sessionId: props.sessionId, tabId: props.tabId, pageInstance, kind, url: props.url, title: "Example" } }));
    send("ready", "stale-page");
    send("changed", "stale-page");
    expect(props.onChange).not.toHaveBeenCalled();
    send("ready", "approved-page");
    send("changed", "approved-page");
    expect(props.onChange).toHaveBeenCalledWith(expect.objectContaining({ kind: "changed", pageInstance: "approved-page" }));
    view.rerender(<NativeBrowserPage {...props} url="https://example.com/redirected" active present obscured />);
    expect(view.container.querySelector("webview")).toBe(guest);
    expect(guest.getAttribute("src")).toBe(props.url);
    expect(calls.at(-1)).toMatchObject({ visible: true, url: "https://example.com/redirected" });
    const overlay = document.createElement("div"); overlay.role = "dialog"; overlay.setAttribute("aria-modal", "true"); document.body.append(overlay);
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(calls.at(-1)).toMatchObject({ visible: true });
    view.rerender(<NativeBrowserPage {...props} active={false} present={false} />);
    expect(view.container.querySelector("webview")).toBe(guest);
    expect(calls.at(-1)).toMatchObject({ visible: false, interactive: false });
    expect(calls.some((call) => call.type === "browser_close")).toBe(false);
  });

  it("rejects an Electron attachment permit without its page generation", async () => {
    vi.spyOn(Element.prototype, "getBoundingClientRect").mockReturnValue(new DOMRect(800, 80, 400, 600));
    window.mewQuery = ({ request, onSuccess }) => onSuccess(JSON.stringify(JSON.parse(request).type === "bootstrap"
      ? { browserMode: "electron", daemonUrl: "ws://127.0.0.1:25566", themeMode: "dark", lightTheme: "light", darkTheme: "dark", themes: [] }
      : { partition: "missing-generation" }));
    await initializeHost();
    const onError = vi.fn();
    const view = render(<NativeBrowserPage client={{ on: vi.fn(), off: vi.fn(), hasConfirmedSession: () => false } as unknown as MewClient} connected={false} sessionId="bad-permit-chat" tabId="tab" url="https://example.com" active onChange={vi.fn()} onError={onError} />);
    await waitFor(() => expect(onError).toHaveBeenCalledWith("The desktop host returned an invalid browser permit."));
    expect(view.container.querySelector("webview")).toBeNull();
  });

  it("registers a native focus endpoint only for its issued Electron page owner", async () => {
    vi.spyOn(Element.prototype, "getBoundingClientRect").mockReturnValue(new DOMRect(800, 80, 400, 600));
    window.mewQuery = ({ request, onSuccess }) => {
      const value = JSON.parse(request);
      onSuccess(JSON.stringify(value.type === "bootstrap"
        ? { browserMode: "electron", daemonUrl: "ws://127.0.0.1:25566", themeMode: "dark", lightTheme: "light", darkTheme: "dark", themes: [] }
        : value.type === "browser_update" && value.visible ? { partition: "focus-permit", pageInstance: "focus-page" } : {}));
    };
    await initializeHost();
    const client = { on: vi.fn(), off: vi.fn(), hasConfirmedSession: () => true, registerBrowserPage: vi.fn().mockResolvedValue(undefined), unregisterBrowserPage: vi.fn().mockResolvedValue(undefined), isConnected: () => true } as unknown as MewClient;
    const props = { client, connected: true, sessionId: "focus-chat", tabId: "focus-tab", url: "https://example.com", active: true, onChange: vi.fn(), onError: vi.fn() };
    const view = render(<NativeBrowserPage {...props} />);
    await waitFor(() => expect(view.container.querySelector("webview")).not.toBeNull());
    const ready = (sessionId: string, tabId: string, pageInstance: string, focusUrl: string) => fireEvent(window, new CustomEvent("mew:browser-event", { detail: {
      sessionId, tabId, pageInstance, kind: "ready", url: props.url, title: "Example", cdpEndpoint: "http://127.0.0.1:9222", targetId: "focus-target", focusUrl,
    } }));
    ready("other-chat", props.tabId, "focus-page", "http://127.0.0.1:9223/wrong-chat");
    ready(props.sessionId, "other-tab", "focus-page", "http://127.0.0.1:9223/wrong-tab");
    ready(props.sessionId, props.tabId, "old-page", "http://127.0.0.1:9223/old-page");
    expect(client.registerBrowserPage).not.toHaveBeenCalled();
    ready(props.sessionId, props.tabId, "focus-page", "http://127.0.0.1:9223/focus-page");
    await waitFor(() => expect(client.registerBrowserPage).toHaveBeenCalledExactlyOnceWith(props.sessionId, "focus-page", "http://127.0.0.1:9222", "focus-target", "http://127.0.0.1:9223/focus-page"));
    expect(props.onChange).toHaveBeenCalledWith(expect.objectContaining({ pageInstance: "focus-page", focusUrl: "http://127.0.0.1:9223/focus-page" }));
  });

  it("waits for a new Electron guest generation after an immediate DOM remount", async () => {
    vi.spyOn(Element.prototype, "getBoundingClientRect").mockReturnValue(new DOMRect(800, 80, 400, 600));
    let generation = 0;
    window.mewQuery = ({ request, onSuccess }) => {
      const value = JSON.parse(request);
      if (value.type === "bootstrap") { onSuccess(JSON.stringify({ browserMode: "electron", daemonUrl: "ws://127.0.0.1:25566", themeMode: "dark", lightTheme: "light", darkTheme: "dark", themes: [] })); return; }
      onSuccess(JSON.stringify(value.type === "browser_update" && value.visible ? { partition: `permit-${++generation}`, pageInstance: `generation-${generation}` } : {}));
    };
    await initializeHost();
    let revoke!: () => void;
    const revocation = new Promise<void>((resolve) => { revoke = resolve; });
    const client = { on: vi.fn(), off: vi.fn(), hasConfirmedSession: () => true, registerBrowserPage: vi.fn().mockResolvedValue(undefined), unregisterBrowserPage: vi.fn(() => revocation), isConnected: () => true } as unknown as MewClient;
    const props = { client, connected: true, sessionId: "electron-remount", tabId: "tab", url: "https://example.com", active: true, onChange: vi.fn(), onError: vi.fn() };
    const first = render(<NativeBrowserPage {...props} />);
    await waitFor(() => expect(first.container.querySelector("webview")).not.toBeNull());
    const ready = (pageInstance: string, targetId: string) => fireEvent(window, new CustomEvent("mew:browser-event", { detail: { sessionId: props.sessionId, tabId: props.tabId, pageInstance, kind: "ready", url: props.url, title: "Example", cdpEndpoint: "http://127.0.0.1:9222", targetId } }));
    ready("generation-1", "target-1");
    await waitFor(() => expect(client.registerBrowserPage).toHaveBeenCalledOnce());
    first.unmount();
    const second = render(<NativeBrowserPage {...props} />);
    await waitFor(() => expect(second.container.querySelector('webview[partition="permit-2"]')).not.toBeNull());
    await act(async () => { revoke(); await new Promise((resolve) => setTimeout(resolve, 0)); });
    expect(client.registerBrowserPage).toHaveBeenCalledTimes(1);
    ready("generation-1", "target-1");
    expect(client.registerBrowserPage).toHaveBeenCalledTimes(1);
    ready("generation-2", "target-2");
    await waitFor(() => expect(client.registerBrowserPage).toHaveBeenCalledTimes(2));
    expect(client.registerBrowserPage).toHaveBeenLastCalledWith(props.sessionId, "generation-2", "http://127.0.0.1:9222", "target-2");
  });

  it("keeps a closing page visible while revoking its tools and input immediately", async () => {
    vi.spyOn(Element.prototype, "getBoundingClientRect").mockReturnValue(new DOMRect(800, 80, 400, 600));
    const calls: Record<string, unknown>[] = [];
    window.mewQuery = ({ request, onSuccess }) => { calls.push(JSON.parse(request)); onSuccess("{}"); };
    const client = { on: vi.fn(), off: vi.fn(), hasConfirmedSession: () => true, registerBrowserPage: vi.fn().mockResolvedValue(undefined), unregisterBrowserPage: vi.fn().mockResolvedValue(undefined), isConnected: () => true } as unknown as MewClient;
    const props = { client, connected: true, sessionId: "closing-page", tabId: "tab", url: "https://example.com", onChange: vi.fn(), onError: vi.fn() };
    const { rerender } = render(<NativeBrowserPage {...props} active present />);
    window.dispatchEvent(new CustomEvent("mew:browser-event", { detail: { sessionId: props.sessionId, tabId: props.tabId, pageInstance: "closing-generation", kind: "ready", url: props.url, cdpEndpoint: "http://127.0.0.1:9222", targetId: "target" } }));
    await waitFor(() => expect(client.registerBrowserPage).toHaveBeenCalledOnce());
    expect(calls.at(-1)).toMatchObject({ visible: true, interactive: true });
    rerender(<NativeBrowserPage {...props} active={false} present />);
    expect(calls.at(-1)).toMatchObject({ visible: true, interactive: false });
    await waitFor(() => expect(client.unregisterBrowserPage).toHaveBeenCalledWith(props.sessionId, "closing-generation"));
    rerender(<NativeBrowserPage {...props} active={false} present={false} />);
    expect(calls.at(-1)).toMatchObject({ visible: false, interactive: false });
    expect(calls.some((call) => call.type === "browser_close")).toBe(false);
  });

  it.each(["menu", "dialog"])("keeps the browser visible beside a non-overlapping %s", async (role) => {
    const calls: Record<string, unknown>[] = [];
    window.mewQuery = ({ request, onSuccess }) => { calls.push(JSON.parse(request)); onSuccess("{}"); };
    const { getByLabelText } = render(<NativeBrowserPage client={{ on: vi.fn(), off: vi.fn(), hasConfirmedSession: () => false } as unknown as MewClient} connected={false} sessionId={`chat-overlay-${role}`} tabId="tab" url="https://example.com" active onChange={() => {}} onError={() => {}} />);
    vi.spyOn(getByLabelText("Browser page"), "getBoundingClientRect").mockReturnValue(new DOMRect(800, 80, 400, 600));
    window.dispatchEvent(new Event("resize"));
    expect(calls.at(-1)).toMatchObject({ visible: true });
    const overlay = document.createElement("div");
    overlay.role = role;
    overlay.dataset.testOverlay = "";
    vi.spyOn(overlay, "getBoundingClientRect").mockReturnValue(new DOMRect(500, 40, 288, 320));
    document.body.append(overlay);
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(calls.at(-1)).toMatchObject({ visible: true });
    vi.spyOn(overlay, "getBoundingClientRect").mockReturnValue(new DOMRect(600, 40, 288, 320));
    overlay.setAttribute("data-state", "open");
    await waitFor(() => expect(calls.at(-1)).toMatchObject({ visible: false }));
  });

  it("hides native content while a modal outside its bounds blocks the workspace", async () => {
    const calls: Record<string, unknown>[] = [];
    window.mewQuery = ({ request, onSuccess }) => { calls.push(JSON.parse(request)); onSuccess("{}"); };
    const { getByLabelText } = render(<NativeBrowserPage client={{ on: vi.fn(), off: vi.fn(), hasConfirmedSession: () => false } as unknown as MewClient} connected={false} sessionId="modal-overlay" tabId="tab" url="https://example.com" active onChange={() => {}} onError={() => {}} />);
    vi.spyOn(getByLabelText("Browser page"), "getBoundingClientRect").mockReturnValue(new DOMRect(800, 80, 400, 600));
    window.dispatchEvent(new Event("resize"));
    const overlay = document.createElement("div");
    overlay.role = "dialog";
    overlay.setAttribute("aria-modal", "true");
    vi.spyOn(overlay, "getBoundingClientRect").mockReturnValue(new DOMRect(400, 40, 288, 320));
    document.body.append(overlay);
    await waitFor(() => expect(calls.at(-1)).toMatchObject({ visible: false }));
  });

  it.each(["menu", "listbox"])("hides beneath an open %s and restores the page after dismissal", async (role) => {
    vi.spyOn(Element.prototype, "getBoundingClientRect").mockReturnValue({ x: 400, y: 80, width: 500, height: 600, top: 80, right: 900, bottom: 680, left: 400, toJSON: () => ({}) });
    const calls: Record<string, unknown>[] = [];
    window.mewQuery = ({ request, onSuccess }) => { calls.push(JSON.parse(request)); onSuccess("{}"); };
    render(<NativeBrowserPage client={{ on: vi.fn(), off: vi.fn(), hasConfirmedSession: () => false } as unknown as MewClient} connected={false} sessionId={`overlay-${role}`} tabId="tab" url="https://example.com" active onChange={() => {}} onError={() => {}} />);
    expect(calls.at(-1)).toMatchObject({ visible: true });
    const overlay = document.createElement("div");
    overlay.role = role;
    overlay.dataset.testOverlay = "";
    document.body.append(overlay);
    await waitFor(() => expect(calls.at(-1)).toMatchObject({ visible: false }));
    overlay.remove();
    await waitFor(() => expect(calls.at(-1)).toMatchObject({ visible: true }));
  });
  it("registers only its own ready page and hides the native view beneath dialogs", async () => {
    vi.spyOn(Element.prototype, "getBoundingClientRect").mockReturnValue({ x: 400, y: 80, width: 500, height: 600, top: 80, right: 900, bottom: 680, left: 400, toJSON: () => ({}) });
    const calls: Record<string, unknown>[] = [];
    window.mewQuery = ({ request, onSuccess }) => { calls.push(JSON.parse(request)); onSuccess("{}"); };
    let revoke!: () => void;
    const revocation = new Promise<void>((resolve) => { revoke = resolve; });
    const client = { on: vi.fn(), off: vi.fn(), hasConfirmedSession: () => true, registerBrowserPage: vi.fn().mockResolvedValue(undefined), unregisterBrowserPage: vi.fn(() => revocation), isConnected: () => true } as unknown as MewClient;
    const changed = vi.fn();
    const { rerender, unmount } = render(<NativeBrowserPage client={client} connected sessionId="chat" tabId="tab" url="https://example.com" active onChange={changed} onError={vi.fn()} />);
    window.dispatchEvent(new CustomEvent("mew:browser-event", { detail: { sessionId: "other", tabId: "tab", pageInstance: "old", kind: "ready", url: "wrong", title: "wrong", cdpEndpoint: "http://127.0.0.1:9222", targetId: "wrong" } }));
    expect(client.registerBrowserPage).not.toHaveBeenCalled();
    window.dispatchEvent(new CustomEvent("mew:browser-event", { detail: { sessionId: "chat", tabId: "tab", pageInstance: "page-1", kind: "ready", url: "https://example.com", title: "Example", cdpEndpoint: "http://127.0.0.1:9222", targetId: "target" } }));
    await waitFor(() => expect(client.registerBrowserPage).toHaveBeenCalledWith("chat", "page-1", "http://127.0.0.1:9222", "target"));
    expect(calls.at(-1)).toMatchObject({ visible: true, rect: { x: 400, y: 80, width: 500, height: 600 } });
    const dialog = document.createElement("div"); dialog.role = "dialog"; document.body.append(dialog);
    await waitFor(() => expect(calls.at(-1)).toMatchObject({ type: "browser_update", visible: false }));
    dialog.remove();
    rerender(<NativeBrowserPage client={client} connected sessionId="chat" tabId="tab" url="https://example.com" active={false} onChange={changed} onError={vi.fn()} />);
    await waitFor(() => expect(client.unregisterBrowserPage).toHaveBeenCalledWith("chat", "page-1"));
    unmount();
    expect(calls.some((call) => call.type === "browser_close")).toBe(false);
    revoke();
    await waitFor(() => expect(calls.at(-1)).toMatchObject({ type: "browser_close", sessionId: "chat", tabId: "tab" }));
  });
  it("re-registers a retained ready generation after an immediate remount", async () => {
    const calls: Record<string, unknown>[] = [];
    window.mewQuery = ({ request, onSuccess }) => { calls.push(JSON.parse(request)); onSuccess("{}"); };
    let revoke!: () => void;
    const revocation = new Promise<void>((resolve) => { revoke = resolve; });
    const client = { on: vi.fn(), off: vi.fn(), hasConfirmedSession: () => true, registerBrowserPage: vi.fn().mockResolvedValue(undefined), unregisterBrowserPage: vi.fn(() => revocation), isConnected: () => true } as unknown as MewClient;
    const props = { client, connected: true, sessionId: "retained-chat", tabId: "retained-tab", url: "https://example.com", active: true, onChange: vi.fn(), onError: vi.fn() };
    const first = render(<NativeBrowserPage {...props} />);
    window.dispatchEvent(new CustomEvent("mew:browser-event", { detail: { sessionId: props.sessionId, tabId: props.tabId, pageInstance: "retained-page", kind: "ready", url: props.url, title: "Example", cdpEndpoint: "http://127.0.0.1:9222", targetId: "target" } }));
    await waitFor(() => expect(client.registerBrowserPage).toHaveBeenCalledTimes(1));
    first.unmount();
    render(<NativeBrowserPage {...props} />);
    revoke();
    await waitFor(() => expect(client.registerBrowserPage).toHaveBeenCalledTimes(2));
    expect(calls.some((call) => call.type === "browser_close")).toBe(false);
  });

});
