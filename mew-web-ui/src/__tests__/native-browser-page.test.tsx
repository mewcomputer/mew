import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, waitFor } from "@testing-library/react";
import { NativeBrowserPage } from "../components/native-browser-page";
import type { MewClient } from "@mew/web-client";

afterEach(() => { cleanup(); delete window.mewQuery; vi.restoreAllMocks(); document.querySelectorAll('[data-test-overlay], [role="dialog"]').forEach((node) => node.remove()); });

describe("native browser surface", () => {
  it.each(["menu", "dialog"])("keeps the browser visible beside a non-overlapping %s", async (role) => {
    const calls: Record<string, unknown>[] = [];
    window.mewQuery = ({ request, onSuccess }) => { calls.push(JSON.parse(request)); onSuccess("{}"); };
    const { getByLabelText } = render(<NativeBrowserPage client={{} as MewClient} connected={false} sessionId={`chat-overlay-${role}`} tabId="tab" url="https://example.com" active onChange={() => {}} onError={() => {}} />);
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
    const { getByLabelText } = render(<NativeBrowserPage client={{} as MewClient} connected={false} sessionId="modal-overlay" tabId="tab" url="https://example.com" active onChange={() => {}} onError={() => {}} />);
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
    render(<NativeBrowserPage client={{} as MewClient} connected={false} sessionId={`overlay-${role}`} tabId="tab" url="https://example.com" active onChange={() => {}} onError={() => {}} />);
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
    const client = { registerBrowserPage: vi.fn().mockResolvedValue(undefined), unregisterBrowserPage: vi.fn(() => revocation), isConnected: () => true } as unknown as MewClient;
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
    const client = { registerBrowserPage: vi.fn().mockResolvedValue(undefined), unregisterBrowserPage: vi.fn(() => revocation), isConnected: () => true } as unknown as MewClient;
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
