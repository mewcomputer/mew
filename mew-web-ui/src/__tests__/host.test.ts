import { afterEach, describe, expect, it, vi } from "vitest";
import {
  browserWebSocketUrl,
  getWebSocketUrl,
  initializeHost,
  resetHost,
  getHostBootstrap,
  hostRequest,
  isDesktopHost,
} from "@/lib/host";

describe("host runtime", () => {
  it("builds the browser websocket URL from the current origin", () => {
    expect(
      browserWebSocketUrl({ protocol: "https:", host: "mew.example.test" }),
    ).toBe("wss://mew.example.test/ws");
  });

  it("initializes the browser host without native commands", async () => {
    await initializeHost();

    expect(getWebSocketUrl()).toBe(browserWebSocketUrl(window.location));
  });

  it("can retry host initialization after clearing cached state", async () => {
    await initializeHost();
    resetHost();
    await initializeHost();

    expect(getWebSocketUrl()).toBe(browserWebSocketUrl(window.location));
  });
});


afterEach(() => {
  resetHost();
  delete window.mewQuery;
});

describe("native host", () => {
  it("bootstraps the native daemon and installed theme inventory", async () => {
    const bootstrap = { daemonUrl: "ws://127.0.0.1:25566", themeMode: "system", lightTheme: "light", darkTheme: "custom", themes: [{ id: "custom", mode: "dark", css: '[data-theme="custom"] { --background: #123456; }' }] };
    window.mewQuery = vi.fn(({ request, onSuccess }) => {
      expect(JSON.parse(request)).toEqual({ type: "bootstrap" });
      onSuccess(JSON.stringify(bootstrap));
    });
    await initializeHost();
    expect(getWebSocketUrl()).toBe(bootstrap.daemonUrl);
    expect(getHostBootstrap()).toEqual(bootstrap);
    expect(isDesktopHost()).toBe(true);
  });

  it("leaves a failed native bootstrap retryable", async () => {
    window.mewQuery = ({ onFailure }) => onFailure(7, "daemon unavailable");
    await expect(initializeHost()).rejects.toThrow("daemon unavailable");
    resetHost();
    window.mewQuery = ({ onSuccess }) => onSuccess(JSON.stringify({ daemonUrl: "ws://127.0.0.1:25566", themeMode: "dark", lightTheme: "light", darkTheme: "dark", themes: [] }));
    await initializeHost();
    expect(getWebSocketUrl()).toBe("ws://127.0.0.1:25566");
  });

  it("passes structured browser bounds and visibility to the native host", async () => {
    const calls: unknown[] = [];
    window.mewQuery = ({ request, onSuccess }) => { calls.push(JSON.parse(request)); onSuccess("{}"); };
    await hostRequest({ type: "browser_update", sessionId: "chat", tabId: "tab", url: "https://example.com", rect: { x: 400, y: 70, width: 500, height: 650 }, visible: false });
    expect(calls).toEqual([{ type: "browser_update", sessionId: "chat", tabId: "tab", url: "https://example.com", rect: { x: 400, y: 70, width: 500, height: 650 }, visible: false }]);
  });
});
