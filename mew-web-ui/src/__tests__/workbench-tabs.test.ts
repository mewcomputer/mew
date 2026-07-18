import { describe, expect, it } from "vitest";
import {
  DEFAULT_WORKBENCH_TABS,
  normalizeWorkbenchTabs,
  workbenchTabsFromLegacyKind,
  workbenchTabsReducer,
} from "@/lib/workbench-tabs";

describe("workbench tabs", () => {
  it("keeps activity pinned while adding and selecting a surface", () => {
    const next = workbenchTabsReducer(DEFAULT_WORKBENCH_TABS, {
      type: "add",
      tab: { id: "browser-1", kind: "browser", title: "New tab", closable: true },
    });

    expect(next.activeTabId).toBe("browser-1");
    expect(next.tabs.map((tab) => tab.kind)).toEqual(["activity", "browser"]);
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

    expect(workbenchTabsReducer(state, { type: "close", id: "browser-1" })).toEqual({
      tabs: [state.tabs[0], state.tabs[2]],
      activeTabId: "review-1",
    });
  });

  it("does not allow the activity tab to be closed", () => {
    expect(workbenchTabsReducer(DEFAULT_WORKBENCH_TABS, { type: "close", id: "activity-1" }))
      .toBe(DEFAULT_WORKBENCH_TABS);
  });

  it("migrates the previous single-tab preference into a real tab", () => {
    expect(workbenchTabsFromLegacyKind("changes")).toEqual({
      tabs: [
        { id: "activity-1", kind: "activity", title: "Activity", closable: false },
        { id: "changes-1", kind: "changes", title: "Changes", closable: true },
      ],
      activeTabId: "changes-1",
    });
  });

  it("normalizes persisted tabs and restores activity when missing", () => {
    expect(normalizeWorkbenchTabs({
      tabs: [{ id: "terminal-1", kind: "terminal", title: "Terminal", closable: true }],
      activeTabId: "missing",
    })).toEqual({
      tabs: [
        { id: "activity-1", kind: "activity", title: "Activity", closable: false },
        { id: "terminal-1", kind: "terminal", title: "Terminal", closable: true },
      ],
      activeTabId: "activity-1",
    });
  });
});
