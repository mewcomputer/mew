export type WorkbenchTabKind =
  | "agents"
  | "jobs"
  | "browser"
  | "terminal"
  | "file"
  | "changes"
  | "review";

export type WorkbenchTabStatus = "idle" | "running" | "attention" | "error";

export interface WorkbenchTab {
  id: string;
  kind: WorkbenchTabKind;
  title: string;
  closable: boolean;
  status?: WorkbenchTabStatus;
  sessionId?: string;
  cwd?: string;
  payload?: {
    url?: string;
    path?: string;
    jobId?: string;
  };
}

export interface WorkbenchTabsState {
  tabs: WorkbenchTab[];
  activeTabId: string;
}

export type WorkbenchTabsAction =
  | { type: "add"; tab: WorkbenchTab }
  | { type: "select"; id: string }
  | { type: "close"; id: string }
  | { type: "update"; id: string; patch: Partial<WorkbenchTab> };

export const OPTIONAL_AGENTS_TAB: WorkbenchTab = {
  id: "agents-1",
  kind: "agents",
  title: "Agents",
  closable: true,
};

export const OPTIONAL_JOBS_TAB: WorkbenchTab = {
  id: "jobs-1",
  kind: "jobs",
  title: "Jobs",
  closable: true,
};

export const DEFAULT_WORKBENCH_TABS: WorkbenchTabsState = {
  tabs: [],
  activeTabId: "",
};

export function createWorkbenchTab(
  kind: WorkbenchTabKind,
  overrides: Partial<WorkbenchTab> = {},
): WorkbenchTab {
  const defaults = defaultTabForKind(kind);
  return {
    ...defaults,
    ...overrides,
    id: overrides.id ?? `${kind}-${Date.now()}-${Math.random().toString(36).slice(2, 7)}`,
  };
}

export function workbenchTabsReducer(
  state: WorkbenchTabsState,
  action: WorkbenchTabsAction,
): WorkbenchTabsState {
  switch (action.type) {
    case "add":
      return {
        tabs: [...state.tabs, action.tab],
        activeTabId: action.tab.id,
      };
    case "select":
      return state.tabs.some((tab) => tab.id === action.id)
        ? { ...state, activeTabId: action.id }
        : state;
    case "update":
      return {
        ...state,
        tabs: state.tabs.map((tab) => (
          tab.id === action.id ? { ...tab, ...action.patch, id: tab.id } : tab
        )),
      };
    case "close": {
      const index = state.tabs.findIndex((tab) => tab.id === action.id);
      const tab = state.tabs[index];
      if (!tab || !tab.closable) return state;

      const tabs = state.tabs.filter((candidate) => candidate.id !== action.id);
      if (tabs.length === 0) return DEFAULT_WORKBENCH_TABS;
      if (state.activeTabId !== action.id) return { ...state, tabs };

      const nextIndex = Math.min(index, tabs.length - 1);
      return { tabs, activeTabId: tabs[nextIndex]!.id };
    }
  }
}

export function getActiveWorkbenchTab(state: WorkbenchTabsState): WorkbenchTab | undefined {
  return state.tabs.find((tab) => tab.id === state.activeTabId) ?? state.tabs[0];
}

export function isWorkbenchTabKind(value: unknown): value is WorkbenchTabKind {
  return value === "agents"
    || value === "jobs"
    || value === "browser"
    || value === "terminal"
    || value === "file"
    || value === "changes"
    || value === "review";
}

export function normalizeWorkbenchTabs(value: unknown): WorkbenchTabsState {
  if (!isRecord(value) || !Array.isArray(value.tabs)) return DEFAULT_WORKBENCH_TABS;

  const tabs = value.tabs
    .filter(isPersistedWorkbenchTab)
    .filter((tab): tab is WorkbenchTab => isWorkbenchTabKind(tab.kind));
  const withCore = tabs.filter((tab) => !isLegacyDefaultActivityTab(tab));
  const activeTabId = typeof value.activeTabId === "string"
    && withCore.some((tab) => tab.id === value.activeTabId)
    ? value.activeTabId
    : withCore[0]?.id ?? "";

  return { tabs: withCore, activeTabId };
}

export function workbenchTabsFromLegacyKind(value: unknown): WorkbenchTabsState {
  if (!isWorkbenchTabKind(value)) return DEFAULT_WORKBENCH_TABS;
  const tab = createWorkbenchTab(value, { id: `${value}-1` });
  return {
    tabs: [...DEFAULT_WORKBENCH_TABS.tabs, tab],
    activeTabId: tab.id,
  };
}

function defaultTabForKind(kind: WorkbenchTabKind): WorkbenchTab {
  switch (kind) {
    case "agents":
      return { id: "agents-1", kind, title: "Agents", closable: true };
    case "jobs":
      return { id: "jobs-1", kind, title: "Jobs", closable: true };
    case "browser":
      return {
        id: "browser-1",
        kind,
        title: "New tab",
        closable: true,
        payload: { url: "" },
      };
    case "terminal":
      return { id: "terminal-1", kind, title: "Terminal", closable: true };
    case "file":
      return { id: "file-1", kind, title: "Files", closable: true };
    case "changes":
      return { id: "changes-1", kind, title: "Changes", closable: true };
    case "review":
      return { id: "review-1", kind, title: "Review", closable: true };
  }
}

function isPersistedWorkbenchTab(value: unknown): value is PersistedWorkbenchTab {
  if (!isRecord(value)) return false;
  return typeof value.id === "string"
    && isPersistedWorkbenchTabKind(value.kind)
    && typeof value.title === "string"
    && typeof value.closable === "boolean";
}

type PersistedWorkbenchTabKind = WorkbenchTabKind | "activity" | "plan" | "questions";

type PersistedWorkbenchTab = Omit<WorkbenchTab, "kind"> & {
  kind: PersistedWorkbenchTabKind;
};

function isPersistedWorkbenchTabKind(value: unknown): value is PersistedWorkbenchTabKind {
  return isWorkbenchTabKind(value) || value === "activity" || value === "plan" || value === "questions";
}

function isLegacyDefaultActivityTab(tab: WorkbenchTab): boolean {
  return (tab.id === OPTIONAL_AGENTS_TAB.id || tab.id === OPTIONAL_JOBS_TAB.id) && !tab.closable;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}
