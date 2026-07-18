import { describe, expect, it } from "vitest";
import {
  browserWebSocketUrl,
  getWebSocketUrl,
  initializeHost,
  listenCefBrowserEvents,
  resetHost,
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

  it("keeps the native browser event listener a no-op in the web host", async () => {
    const cleanup = await listenCefBrowserEvents(() => undefined);

    expect(cleanup()).toBeUndefined();
  });
});
