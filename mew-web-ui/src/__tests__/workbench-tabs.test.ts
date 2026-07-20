import { describe, expect, it } from "vitest";
import {
  DEFAULT_WORKBENCH_TABS,
  normalizeWorkbenchTabs,
  workbenchTabsFromLegacyKind,
  workbenchTabsReducer,
} from "@/lib/workbench-tabs";

describe("workbench tabs", () => {
  it("starts empty and adds a selected surface", () => {
    const next = workbenchTabsReducer(DEFAULT_WORKBENCH_TABS, {
      type: "add",
      tab: { id: "browser-1", kind: "browser", title: "New tab", closable: true },
    });

    expect(next.activeTabId).toBe("browser-1");
    expect(next.tabs.map((tab) => tab.kind)).toEqual([
      "browser",
    ]);
  });

  it("closes the active tab and selects its nearest neighbor", () => {
    const state = {
      tabs: [
        ...DEFAULT_WORKBENCH_TABS.tabs,
        { id: "browser-1", kind: "browser" as const, title: "GitHub", closable: true },
        { id: "review-1", kind: "review" as const, title: "Review", closable: true },
      ],
      activeTabId: "browser-1",
    };

    const next = workbenchTabsReducer(state, { type: "close", id: "browser-1" });
    expect(next.activeTabId).toBe("review-1");
    expect(next.tabs.some((tab) => tab.id === "browser-1")).toBe(false);
    expect(next.tabs[next.tabs.length - 1]?.id).toBe("review-1");
  });

  it("does not allow core tabs to be closed", () => {
    const state = {
      tabs: [{ id: "agents-1", kind: "agents" as const, title: "Agents", closable: false }],
      activeTabId: "agents-1",
    };
    expect(workbenchTabsReducer(state, { type: "close", id: "agents-1" })).toBe(state);
  });

  it("migrates the previous single-tab preference into a real tab", () => {
    expect(workbenchTabsFromLegacyKind("changes")).toEqual({
      tabs: [
        { id: "changes-1", kind: "changes", title: "Changes", closable: true },
      ],
      activeTabId: "changes-1",
    });
  });

  it("normalizes persisted tabs and restores core tabs when missing", () => {
    expect(normalizeWorkbenchTabs({
      tabs: [{ id: "terminal-1", kind: "terminal", title: "Terminal", closable: true }],
      activeTabId: "missing",
    })).toEqual({
      tabs: [
        { id: "terminal-1", kind: "terminal", title: "Terminal", closable: true },
      ],
      activeTabId: "terminal-1",
    });
  });

  it("drops legacy activity, plan, and questions tabs", () => {
    expect(normalizeWorkbenchTabs({
      tabs: [
        { id: "activity-1", kind: "activity", title: "Activity", closable: false },
        { id: "plan-1", kind: "plan", title: "Plan", closable: false },
        { id: "questions-1", kind: "questions", title: "Questions", closable: false },
      ],
      activeTabId: "questions-1",
    })).toEqual(DEFAULT_WORKBENCH_TABS);
  });
});
