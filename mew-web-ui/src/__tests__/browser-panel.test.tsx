import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { BrowserPanel } from "../components/browser-panel";
import type { MewClient } from "@mew/web-client";
import { useState } from "react";
import type { WorkbenchTab } from "../lib/workbench-tabs";
import { initializeHost, resetHost } from "../lib/host";

afterEach(() => { cleanup(); resetHost(); delete window.mewQuery; vi.restoreAllMocks(); });

describe("browser address editing", () => {
  it.each([false, true])("returns only a closed native guest to New tab while preserving an unfinished address: %s", async (dirty) => {
    vi.spyOn(Element.prototype, "getBoundingClientRect").mockReturnValue(new DOMRect(800, 80, 400, 600));
    window.mewQuery = ({ request, onSuccess }) => {
      const value = JSON.parse(request);
      onSuccess(JSON.stringify(value.type === "bootstrap"
        ? { browserMode: "electron", daemonUrl: "ws://127.0.0.1:25566", themeMode: "dark", lightTheme: "light", darkTheme: "dark", themes: [] }
        : value.type === "browser_update" && value.visible ? { partition: value.sessionId, pageInstance: `${value.sessionId}-page` } : {}));
    };
    await initializeHost();
    const client = { on: vi.fn(), off: vi.fn(), hasConfirmedSession: () => true, registerBrowserPage: vi.fn().mockResolvedValue(undefined), unregisterBrowserPage: vi.fn().mockResolvedValue(undefined), isConnected: () => true } as unknown as MewClient;
    function OwnedBrowser({ owner, active }: { owner: string; active: boolean }) {
      const [tab, setTab] = useState<WorkbenchTab>({ id: "same-tab", kind: "browser", title: owner, closable: true, payload: { url: `https://${owner}.example` } });
      return <section aria-label={owner}><BrowserPanel client={client} connected active={active} sessionId={owner} tab={tab} onTabChange={(patch) => setTab((current) => ({ ...current, ...patch }))} launcher={<p>New tab launcher</p>} /></section>;
    }
    const view = render(<><OwnedBrowser owner="chat-a" active /><OwnedBrowser owner="chat-b" active={false} /></>);
    await waitFor(() => expect(view.container.querySelector("webview")).not.toBeNull());
    const a = within(screen.getByRole("region", { name: "chat-a" }));
    const b = within(screen.getByRole("region", { name: "chat-b" }));
    const address = a.getByRole("textbox", { name: "Browser URL" }) as HTMLInputElement;
    if (dirty) fireEvent.change(address, { target: { value: "https://unfinished.example/path" } });
    const send = (sessionId: string, pageInstance: string, kind: string) => fireEvent(window, new CustomEvent("mew:browser-event", { detail: { sessionId, tabId: "same-tab", pageInstance, kind, url: "https://chat-a.example", title: "A", cdpEndpoint: "http://127.0.0.1:9222", targetId: "a-target" } }));
    send("chat-a", "chat-a-page", "ready");
    await waitFor(() => expect(client.registerBrowserPage).toHaveBeenCalledOnce());
    send("chat-b", "chat-a-page", "closed");
    send("chat-a", "retired-page", "closed");
    expect(a.queryByText("New tab launcher")).toBeNull();
    send("chat-a", "chat-a-page", "closed");
    expect(a.getByText("New tab launcher")).toBeTruthy();
    expect(b.queryByText("New tab launcher")).toBeNull();
    expect((b.getByRole("textbox", { name: "Browser URL" }) as HTMLInputElement).value).toBe("https://chat-b.example");
    expect(address.value).toBe(dirty ? "https://unfinished.example/path" : "");
    expect(view.container.querySelector("webview")).toBeNull();
    fireEvent.click(a.getByRole("button", { name: "Open browser tools" }));
    expect((a.getByRole("button", { name: "Text snapshot" }) as HTMLButtonElement).disabled).toBe(true);
    await waitFor(() => expect(client.unregisterBrowserPage).toHaveBeenCalledWith("chat-a", "chat-a-page"));
  });

  it("dismisses browser tools with Escape or an outside click", () => {
    render(<BrowserPanel client={null} connected={false} tab={{ id: "new", kind: "browser", title: "New tab", closable: true }} onTabChange={() => {}} />);
    const trigger = screen.getByRole("button", { name: "Open browser tools" });
    fireEvent.click(trigger);
    expect(screen.getByRole("button", { name: "Text snapshot" })).toBeTruthy();
    fireEvent.keyDown(window, { key: "Escape" });
    expect(screen.queryByRole("button", { name: "Text snapshot" })).toBeNull();
    fireEvent.click(trigger);
    fireEvent.mouseDown(document.body);
    expect(screen.queryByRole("button", { name: "Text snapshot" })).toBeNull();
  });
  it("keeps an unfinished address while background page metadata changes", () => {
    window.mewQuery = ({ onSuccess }) => onSuccess("{}");
    const client = { on: vi.fn(), off: vi.fn(), hasConfirmedSession: () => true } as unknown as MewClient;
    const patch = vi.fn();
    render(<BrowserPanel client={client} connected sessionId="chat" tab={{ id: "page", kind: "browser", title: "Example", closable: true, payload: { url: "https://example.com" } }} onTabChange={patch} />);
    const address = screen.getByRole("textbox", { name: "Browser URL" }) as HTMLInputElement;
    fireEvent.change(address, { target: { value: "https://unfinished.dev/path" } });
    fireEvent(window, new CustomEvent("mew:browser-event", { detail: { sessionId: "chat", tabId: "page", pageInstance: "generation", url: "https://example.com/redirected", title: "New title", kind: "changed" } }));
    expect(address.value).toBe("https://unfinished.dev/path");
    expect(patch).toHaveBeenCalledWith({ title: "New title", payload: { url: "https://example.com/redirected" } });
  });
  it.each([true, false])("scopes browser operations and replies to the %s native generation", async (native) => {
    if (native) window.mewQuery = ({ onSuccess }) => onSuccess("{}");
    let confirm!: () => void;
    const registered = new Promise<void>((resolve) => { confirm = resolve; });
    const listeners = new Map<string, (data: unknown) => void>();
    const client = {
      on: vi.fn((name, listener) => listeners.set(name, listener)), off: vi.fn(),
      browserOpen: vi.fn(), browserSnapshot: vi.fn(), browserScreenshot: vi.fn(), browserFill: vi.fn(), browserClick: vi.fn(),
      registerBrowserPage: vi.fn(() => registered), unregisterBrowserPage: vi.fn(() => registered), isConnected: () => true, hasConfirmedSession: () => true,
    };
    render(<BrowserPanel client={client as unknown as MewClient} connected sessionId="chat" tab={{ id: "ui-tab", kind: "browser", title: "Example", closable: true, payload: { url: "https://example.com" } }} onTabChange={vi.fn()} />);
    fireEvent.click(screen.getByRole("button", { name: "Open browser tools" }));
    if (native) {
      expect((screen.getByRole("button", { name: "Text snapshot" }) as HTMLButtonElement).disabled).toBe(true);
      fireEvent(window, new CustomEvent("mew:browser-event", { detail: { sessionId: "chat", tabId: "ui-tab", pageInstance: "ready-generation", url: "https://example.com", title: "Example", kind: "error", message: "Update the daemon to enable browser input." } }));
      expect(screen.getByRole("alert").textContent).toBe("Update the daemon to enable browser input.");
      fireEvent(window, new CustomEvent("mew:browser-event", { detail: { sessionId: "chat", tabId: "ui-tab", pageInstance: "ready-generation", url: "https://example.com", title: "Example", kind: "ready", cdpEndpoint: "http://127.0.0.1:9222", targetId: "target" } }));
    }
    if (native) {
      await waitFor(() => expect(client.registerBrowserPage).toHaveBeenCalled());
      expect((screen.getByRole("button", { name: "Text snapshot" }) as HTMLButtonElement).disabled).toBe(true);
      confirm();
    }
    await waitFor(() => expect((screen.getByRole("button", { name: "Text snapshot" }) as HTMLButtonElement).disabled).toBe(false));
    if (native) expect(screen.queryByRole("alert")).toBeNull();
    const identity = native ? "ready-generation" : "ui-tab";
    fireEvent.click(screen.getByRole("button", { name: "Text snapshot" }));
    expect(client.browserSnapshot).toHaveBeenCalledWith(identity);
    fireEvent.click(screen.getByRole("button", { name: "Screenshot" }));
    expect(client.browserScreenshot).toHaveBeenCalledWith(false, identity);
    fireEvent.click(screen.getByRole("button", { name: /Inspect and interact/ }));
    fireEvent.change(screen.getByRole("textbox", { name: "Browser element selector" }), { target: { value: "@e1" } });
    fireEvent.change(screen.getByRole("textbox", { name: "Browser fill text" }), { target: { value: "hello" } });
    fireEvent.click(screen.getByRole("button", { name: "Fill" }));
    fireEvent.click(screen.getByRole("button", { name: "Click" }));
    expect(client.browserFill).toHaveBeenCalledWith("@e1", "hello", identity);
    expect(client.browserClick).toHaveBeenCalledWith("@e1", identity);
    listeners.get("browser-snapshot")?.({ tabId: "retired-generation", snapshot: "stale text", url: "https://example.com", title: "Old" });
    expect(screen.queryByText("stale text")).toBeNull();
    listeners.get("browser-snapshot")?.({ tabId: identity, snapshot: "current text", url: "https://example.com", title: "Example" });
    await waitFor(() => expect(screen.getByText("current text")).toBeTruthy());
    listeners.get("browser-error")?.({ tabId: "retired-generation", message: "stale error" });
    expect(screen.queryByRole("alert")).toBeNull();
    listeners.get("browser-error")?.({ tabId: identity, message: "current error" });
    await waitFor(() => expect(screen.getByRole("alert").textContent).toBe("current error"));
  });

});
