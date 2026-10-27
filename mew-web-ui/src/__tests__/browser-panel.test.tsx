import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { BrowserPanel } from "../components/browser-panel";
import type { MewClient } from "@mew/web-client";

afterEach(() => { cleanup(); delete window.mewQuery; });

describe("browser address editing", () => {
  it("keeps an unfinished address while background page metadata changes", () => {
    window.mewQuery = ({ onSuccess }) => onSuccess("{}");
    const client = { on: vi.fn(), off: vi.fn() } as unknown as MewClient;
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
      registerBrowserPage: vi.fn(() => registered), unregisterBrowserPage: vi.fn(() => registered), isConnected: () => true,
    };
    render(<BrowserPanel client={client as unknown as MewClient} connected sessionId="chat" tab={{ id: "ui-tab", kind: "browser", title: "Example", closable: true, payload: { url: "https://example.com" } }} onTabChange={vi.fn()} />);
    fireEvent.click(screen.getByRole("button", { name: "Open browser tools" }));
    if (native) {
      expect((screen.getByRole("button", { name: "Text snapshot" }) as HTMLButtonElement).disabled).toBe(true);
      fireEvent(window, new CustomEvent("mew:browser-event", { detail: { sessionId: "chat", tabId: "ui-tab", pageInstance: "ready-generation", url: "https://example.com", title: "Example", kind: "ready", cdpEndpoint: "http://127.0.0.1:9222", targetId: "target" } }));
    }
    if (native) {
      await waitFor(() => expect(client.registerBrowserPage).toHaveBeenCalled());
      expect((screen.getByRole("button", { name: "Text snapshot" }) as HTMLButtonElement).disabled).toBe(true);
      confirm();
    }
    await waitFor(() => expect((screen.getByRole("button", { name: "Text snapshot" }) as HTMLButtonElement).disabled).toBe(false));
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
