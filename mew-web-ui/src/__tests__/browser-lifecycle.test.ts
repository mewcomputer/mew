import { describe, expect, it } from "vitest";
import { acceptsBrowserEvent, acceptsNativeBrowserEvent } from "@/lib/browser-lifecycle";

describe("browser lifecycle event routing", () => {
  it("accepts an event tagged for the active tab even after navigation", () => {
    expect(acceptsBrowserEvent({ tabId: "browser-2", url: "https://example.org" }, {
      tabId: "browser-2",
      url: "https://example.com",
    })).toBe(true);
  });

  it("rejects a late event from a different browser tab", () => {
    expect(acceptsBrowserEvent({ tabId: "browser-1", url: "https://example.com" }, {
      tabId: "browser-2",
      url: "https://example.org",
    })).toBe(false);
  });

  it("accepts legacy untagged events only when their URL matches", () => {
    expect(acceptsBrowserEvent({ url: "https://example.org" }, {
      tabId: "browser-2",
      url: "https://example.org",
    })).toBe(true);
    expect(acceptsBrowserEvent({ url: "https://example.com" }, {
      tabId: "browser-2",
      url: "https://example.org",
    })).toBe(false);
  });

  it("rejects untagged events for a blank tab", () => {
    expect(acceptsBrowserEvent({ url: "https://example.org" }, {
      tabId: "browser-2",
      url: "",
    })).toBe(false);
  });

  it("rejects legacy untagged events for inactive tabs", () => {
    expect(acceptsBrowserEvent({ url: "https://example.com" }, {
      tabId: "browser-2",
      url: "https://example.com",
      active: false,
    })).toBe(false);
  });

  it("accepts native events only for the visible owner", () => {
    expect(acceptsNativeBrowserEvent({
      kind: "address_changed",
      owner: "browser-2",
      url: "https://example.org",
    }, {
      tabId: "browser-2",
      url: "https://example.com",
      active: true,
      visible: true,
      loading: false,
    })).toBe(true);

    expect(acceptsNativeBrowserEvent({
      kind: "address_changed",
      owner: "browser-1",
      url: "https://example.com",
    }, {
      tabId: "browser-2",
      url: "https://example.com",
      active: true,
      visible: true,
      loading: false,
    })).toBe(false);
  });

  it("rejects a stale native navigation while the active tab is loading another URL", () => {
    expect(acceptsNativeBrowserEvent({
      kind: "address_changed",
      owner: "browser-2",
      url: "https://old.example",
    }, {
      tabId: "browser-2",
      url: "https://new.example",
      active: true,
      visible: true,
      loading: true,
    })).toBe(false);
  });

  it("rejects native events for a hidden blank tab", () => {
    expect(acceptsNativeBrowserEvent({
      kind: "title_changed",
      owner: "browser-2",
      title: "Old page",
      url: "https://old.example",
    }, {
      tabId: "browser-2",
      url: "",
      active: true,
      visible: false,
      loading: false,
    })).toBe(false);
  });
});
