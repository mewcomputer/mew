import { describe, expect, it } from "vitest";
import { acceptsBrowserEvent } from "@/lib/browser-lifecycle";

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

});
