import {
  DEFAULT_WORKBENCH_TABS,
  normalizeWorkbenchTabs,
  workbenchTabsFromLegacyKind,
  workbenchTabsReducer,
  type WorkbenchTabsAction,
  type WorkbenchTabsState,
} from "./workbench-tabs";

export interface WorkspaceSurfacesState {
  sessionId?: string;
  sessionTabs?: Record<string, WorkbenchTabsState>;
  sidebarOpen: boolean;
  workbenchOpen: boolean;
  workbenchTabs: WorkbenchTabsState;
  workbenchSize: number;
}

export type WorkspaceSurfacesAction =
  | { type: "select-session"; sessionId: string }
  | { type: "set-sidebar-open"; open: boolean }
  | { type: "toggle-sidebar" }
  | { type: "toggle-workbench" }
  | { type: "set-workbench-open"; open: boolean }
  | { type: "set-workbench-size"; size: number }
  | { type: "set-workbench-tabs"; tabs: WorkbenchTabsState }
  | { type: "workbench-tabs-action"; action: WorkbenchTabsAction };

export const WORKSPACE_SURFACES_STORAGE_KEY = "mew.workspaceSurfaces";

export const DEFAULT_WORKSPACE_SURFACES: WorkspaceSurfacesState = {
  sidebarOpen: true,
  workbenchOpen: false,
  workbenchTabs: DEFAULT_WORKBENCH_TABS,
  workbenchSize: 42,
};

const MIN_WORKBENCH_SIZE = 18;
const MAX_WORKBENCH_SIZE = 65;

export function workspaceSurfacesReducer(
  state: WorkspaceSurfacesState,
  action: WorkspaceSurfacesAction,
): WorkspaceSurfacesState {
  switch (action.type) {
    case "select-session": {
      if (state.sessionId === action.sessionId) return state;
      const sessionTabs = { ...state.sessionTabs };
      if (state.sessionId) sessionTabs[state.sessionId] = state.workbenchTabs;
      return { ...state, sessionId: action.sessionId, sessionTabs, workbenchTabs: sessionTabs[action.sessionId] ?? DEFAULT_WORKBENCH_TABS };
    }
    case "set-sidebar-open":
      return { ...state, sidebarOpen: action.open };
    case "toggle-sidebar":
      return { ...state, sidebarOpen: !state.sidebarOpen };
    case "toggle-workbench":
      return { ...state, workbenchOpen: !state.workbenchOpen };
    case "set-workbench-open":
      return { ...state, workbenchOpen: action.open };
    case "set-workbench-size":
      return { ...state, workbenchSize: clampWorkbenchSize(action.size) };
    case "set-workbench-tabs":
      return { ...state, workbenchTabs: action.tabs };
    case "workbench-tabs-action":
      return {
        ...state,
        workbenchTabs: workbenchTabsReducer(state.workbenchTabs, action.action),
      };
  }
}

export function loadWorkspaceSurfaces(
  storage: Pick<Storage, "getItem"> | undefined,
): WorkspaceSurfacesState {
  if (!storage) return DEFAULT_WORKSPACE_SURFACES;

  try {
    const raw = storage.getItem(WORKSPACE_SURFACES_STORAGE_KEY);
    if (!raw) return DEFAULT_WORKSPACE_SURFACES;
    const value: unknown = JSON.parse(raw);
    if (!isRecord(value)) return DEFAULT_WORKSPACE_SURFACES;

    const workbenchTabs = "workbenchTabs" in value
      ? normalizeWorkbenchTabs(value.workbenchTabs)
      : workbenchTabsFromLegacyKind(value.workbenchTab);

    const sessionTabs = isRecord(value.sessionTabs) ? Object.fromEntries(Object.entries(value.sessionTabs).map(([id, tabs]) => [id, normalizeWorkbenchTabs(tabs)])) : undefined;
    return {
      ...(typeof value.sessionId === "string" ? { sessionId: value.sessionId } : {}),
      ...(sessionTabs ? { sessionTabs } : {}),
      sidebarOpen: typeof value.sidebarOpen === "boolean"
        ? value.sidebarOpen
        : typeof value.summaryOpen === "boolean"
          ? value.summaryOpen
          : DEFAULT_WORKSPACE_SURFACES.sidebarOpen,
      workbenchOpen: typeof value.workbenchOpen === "boolean"
        ? value.workbenchOpen
        : DEFAULT_WORKSPACE_SURFACES.workbenchOpen,
      workbenchTabs,
      workbenchSize: typeof value.workbenchSize === "number"
        ? clampWorkbenchSize(value.workbenchSize)
        : DEFAULT_WORKSPACE_SURFACES.workbenchSize,
    };
  } catch {
    return DEFAULT_WORKSPACE_SURFACES;
  }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function clampWorkbenchSize(value: number): number {
  return Math.min(MAX_WORKBENCH_SIZE, Math.max(MIN_WORKBENCH_SIZE, value));
}
