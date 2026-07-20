import { describe, expect, it } from "vitest";
import {
  DEFAULT_WORKSPACE_SURFACES,
  loadWorkspaceSurfaces,
  workspaceSurfacesReducer,
  type WorkspaceSurfacesState,
} from "@/lib/workspace-surfaces";

describe("workspace surfaces", () => {
  it("starts with the pinned summary visible and workbench hidden", () => {
    expect(DEFAULT_WORKSPACE_SURFACES).toEqual({
      summaryOpen: true,
      workbenchOpen: false,
      workbenchTabs: {
        tabs: [],
        activeTabId: "",
      },
      workbenchSize: 28,
    });
  });

  it("toggles the summary and workbench independently", () => {
    let state: WorkspaceSurfacesState = DEFAULT_WORKSPACE_SURFACES;

    state = workspaceSurfacesReducer(state, { type: "toggle-summary" });
    expect(state).toMatchObject({ summaryOpen: false, workbenchOpen: false });

    state = workspaceSurfacesReducer(state, { type: "toggle-workbench" });
    expect(state).toMatchObject({ summaryOpen: false, workbenchOpen: true });

    state = workspaceSurfacesReducer(state, { type: "toggle-summary" });
    expect(state).toMatchObject({ summaryOpen: true, workbenchOpen: true });
  });

  it("accepts an explicit summary visibility update from the sidebar provider", () => {
    const state = workspaceSurfacesReducer(DEFAULT_WORKSPACE_SURFACES, {
      type: "set-summary-open",
      open: false,
    });

    expect(state.summaryOpen).toBe(false);
    expect(state.workbenchOpen).toBe(false);
  });

  it("keeps the selected workbench tab when either surface toggles", () => {
    let state: WorkspaceSurfacesState = {
      ...DEFAULT_WORKSPACE_SURFACES,
      workbenchOpen: true,
      workbenchTabs: {
        tabs: [
          ...DEFAULT_WORKSPACE_SURFACES.workbenchTabs.tabs,
          { id: "browser-1", kind: "browser", title: "New tab", closable: true },
        ],
        activeTabId: "browser-1",
      },
    };

    state = workspaceSurfacesReducer(state, { type: "toggle-summary" });
    state = workspaceSurfacesReducer(state, { type: "toggle-workbench" });

    expect(state.workbenchTabs.activeTabId).toBe("browser-1");
  });

  it("changes only the workbench tabs", () => {
    const tabs = {
      tabs: [
        ...DEFAULT_WORKSPACE_SURFACES.workbenchTabs.tabs,
        { id: "review-1", kind: "review" as const, title: "Review", closable: true },
      ],
      activeTabId: "review-1",
    };
    const state = workspaceSurfacesReducer(DEFAULT_WORKSPACE_SURFACES, {
      type: "set-workbench-tabs",
      tabs,
    });

    expect(state).toEqual({
      summaryOpen: true,
      workbenchOpen: false,
      workbenchTabs: tabs,
      workbenchSize: 28,
    });
  });

  it("applies sequential workbench actions against the latest tab state", () => {
    let state = workspaceSurfacesReducer(DEFAULT_WORKSPACE_SURFACES, {
      type: "workbench-tabs-action",
      action: {
        type: "add",
        tab: { id: "browser-1", kind: "browser", title: "New tab", closable: true },
      },
    });
    state = workspaceSurfacesReducer(state, {
      type: "workbench-tabs-action",
      action: {
        type: "update",
        id: "browser-1",
        patch: { payload: { url: "https://example.com" } },
      },
    });

    expect(state.workbenchTabs.activeTabId).toBe("browser-1");
    expect(state.workbenchTabs.tabs.find((tab) => tab.id === "browser-1")?.payload?.url)
      .toBe("https://example.com");
  });

  it("loads valid surface preferences and ignores malformed values", () => {
    expect(loadWorkspaceSurfaces({
      getItem: () => JSON.stringify({
        summaryOpen: false,
        workbenchOpen: true,
        workbenchTab: "changes",
        workbenchSize: 36,
      }),
    })).toEqual({
      summaryOpen: false,
      workbenchOpen: true,
      workbenchTabs: {
        tabs: [
          { id: "changes-1", kind: "changes", title: "Changes", closable: true },
        ],
        activeTabId: "changes-1",
      },
      workbenchSize: 36,
    });

    expect(loadWorkspaceSurfaces({ getItem: () => "not json" })).toEqual(
      DEFAULT_WORKSPACE_SURFACES,
    );
  });

  it("clamps the persisted workbench width", () => {
    let state = workspaceSurfacesReducer(DEFAULT_WORKSPACE_SURFACES, {
      type: "set-workbench-size",
      size: 62,
    });
    expect(state.workbenchSize).toBe(50);

    state = workspaceSurfacesReducer(state, {
      type: "set-workbench-size",
      size: 12,
    });
    expect(state.workbenchSize).toBe(18);
  });

  it("synchronizes visibility without changing the workbench width", () => {
    const state = workspaceSurfacesReducer(
      { ...DEFAULT_WORKSPACE_SURFACES, workbenchSize: 34 },
      { type: "set-workbench-open", open: true },
    );

    expect(state).toMatchObject({ workbenchOpen: true, workbenchSize: 34 });
  });
});
