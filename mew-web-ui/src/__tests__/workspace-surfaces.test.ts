import { describe, expect, it } from "vitest";
import {
  DEFAULT_WORKSPACE_SURFACES,
  loadWorkspaceSurfaces,
  workspaceSurfacesReducer,
  type WorkspaceSurfacesState,
} from "@/lib/workspace-surfaces";

describe("workspace surfaces", () => {
  it("starts with the pinned sidebar visible and workbench hidden", () => {
    expect(DEFAULT_WORKSPACE_SURFACES).toEqual({
      sidebarOpen: true,
      workbenchOpen: false,
      workbenchTabs: {
        tabs: [],
        activeTabId: "",
      },
      workbenchSize: 42,
    });
  });

  it("toggles the sidebar and workbench independently", () => {
    let state: WorkspaceSurfacesState = DEFAULT_WORKSPACE_SURFACES;

    state = workspaceSurfacesReducer(state, { type: "toggle-sidebar" });
    expect(state).toMatchObject({ sidebarOpen: false, workbenchOpen: false });

    state = workspaceSurfacesReducer(state, { type: "toggle-workbench" });
    expect(state).toMatchObject({ sidebarOpen: false, workbenchOpen: true });

    state = workspaceSurfacesReducer(state, { type: "toggle-sidebar" });
    expect(state).toMatchObject({ sidebarOpen: true, workbenchOpen: true });
  });

  it("accepts an explicit sidebar visibility update from the sidebar provider", () => {
    const state = workspaceSurfacesReducer(DEFAULT_WORKSPACE_SURFACES, {
      type: "set-sidebar-open",
      open: false,
    });

    expect(state.sidebarOpen).toBe(false);
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

    state = workspaceSurfacesReducer(state, { type: "toggle-sidebar" });
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
      sidebarOpen: true,
      workbenchOpen: false,
      workbenchTabs: tabs,
      workbenchSize: 42,
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
        sidebarOpen: false,
        workbenchOpen: true,
        workbenchTab: "changes",
        workbenchSize: 36,
      }),
    })).toEqual({
      sidebarOpen: false,
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

  it("restores workbench tabs for each chat without moving a page to another chat", () => {
    let state = workspaceSurfacesReducer(DEFAULT_WORKSPACE_SURFACES, { type: "select-session", sessionId: "chat-a" });
    state = workspaceSurfacesReducer(state, { type: "workbench-tabs-action", action: { type: "add", tab: { id: "page-a", sessionId: "chat-a", kind: "browser", title: "A", closable: true } } });
    state = workspaceSurfacesReducer(state, { type: "select-session", sessionId: "chat-b" });
    expect(state.workbenchTabs.tabs).toEqual([]);
    state = workspaceSurfacesReducer(state, { type: "workbench-tabs-action", action: { type: "add", tab: { id: "file-b", sessionId: "chat-b", kind: "file", title: "B", closable: true } } });
    state = workspaceSurfacesReducer(state, { type: "select-session", sessionId: "chat-a" });
    expect(state.workbenchTabs.activeTabId).toBe("page-a");
    expect(loadWorkspaceSurfaces({ getItem: () => JSON.stringify(state) }).sessionTabs?.["chat-b"]?.activeTabId).toBe("file-b");
  });

  it("keeps browser metadata with its background chat and ignores owners that have no saved tabs", () => {
    const state: WorkspaceSurfacesState = {
      ...DEFAULT_WORKSPACE_SURFACES,
      sessionId: "chat-b",
      workbenchTabs: { tabs: [{ id: "same-id", kind: "browser", title: "B", closable: true }], activeTabId: "same-id" },
      sessionTabs: { "chat-a": { tabs: [{ id: "same-id", kind: "browser", title: "A", closable: true }], activeTabId: "same-id" } },
    };
    const next = workspaceSurfacesReducer(state, { type: "workbench-tabs-action", sessionId: "chat-a", action: { type: "update", id: "same-id", patch: { title: "A changed", payload: { url: "https://a.example" } } } });
    expect(next.workbenchTabs).toBe(state.workbenchTabs);
    expect(next.sessionTabs?.["chat-a"]?.tabs[0]).toMatchObject({ title: "A changed", payload: { url: "https://a.example" } });
    expect(workspaceSurfacesReducer(next, { type: "workbench-tabs-action", sessionId: "missing-chat", action: { type: "update", id: "same-id", patch: { title: "Wrong" } } })).toBe(next);
  });

  it("imports the previous sidebar visibility without treating it as the activity summary", () => {
    expect(loadWorkspaceSurfaces({ getItem: () => JSON.stringify({ summaryOpen: false }) }).sidebarOpen).toBe(false);
    expect(loadWorkspaceSurfaces({ getItem: () => JSON.stringify({ summaryOpen: false, sidebarOpen: true }) }).sidebarOpen).toBe(true);
  });

  it("clamps the persisted workbench width", () => {
    let state = workspaceSurfacesReducer(DEFAULT_WORKSPACE_SURFACES, {
      type: "set-workbench-size",
      size: 82,
    });
    expect(state.workbenchSize).toBe(65);

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
