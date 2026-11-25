import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { RightRail } from "../components/right-rail";
import { useSessionStore } from "../stores/session";
import { initializeHost, resetHost } from "../lib/host";

const {
  browserListeners,
  watchWorkspaceMock,
  gitStatusMock,
  browserOpenMock,
  clientMock,
} = vi.hoisted(() => {
  type BrowserEventData = { message: string; tabId?: string };
  const listeners = new Map<string, (data: BrowserEventData) => void>();
  const mock = {
    on: vi.fn((event: string, handler: (data: BrowserEventData) => void) => {
      listeners.set(event, handler);
    }),
    off: vi.fn((event: string) => {
      listeners.delete(event);
    }),
    hasConfirmedSession: () => true,
    watchWorkspace: vi.fn(),
    gitStatus: vi.fn(),
    browserOpen: vi.fn(),
    browserClose: vi.fn(),
    browserSnapshot: vi.fn(),
    browserScreenshot: vi.fn(),
    browserFill: vi.fn(),
    browserClick: vi.fn(),
  };
  return {
    browserListeners: listeners,
    watchWorkspaceMock: mock.watchWorkspace,
    gitStatusMock: mock.gitStatus,
    browserOpenMock: mock.browserOpen,
    clientMock: mock,
  };
});

vi.mock("../lib/client-ref", () => ({
  getClient: () => clientMock,
}));

vi.mock("../components/file-tree", () => ({
  FileTreePanel: ({ hasWorkspace, headerActions }: { hasWorkspace: boolean; headerActions?: React.ReactNode }) => (
    <div>{headerActions}{hasWorkspace ? "Files panel" : "Choose a workspace to browse files."}</div>
  ),
  ChangesPanel: ({ hasWorkspace }: { hasWorkspace: boolean }) => (
    <div>{hasWorkspace ? "Changes panel" : "Choose a workspace to see changes."}</div>
  ),
}));

describe("RightRail", () => {
  function addWorkbenchTab(label: string) {
    const trigger = screen.getByRole("button", { name: "Add workbench tab" });
    fireEvent.click(trigger);
    if (label !== "Browser") fireEvent.click(screen.getByRole("button", { name: new RegExp(`^${label}\\b`) }));
  }

  afterEach(() => {
    cleanup();
    delete window.mewQuery;
    resetHost();
    vi.restoreAllMocks();
    watchWorkspaceMock.mockClear();
    gitStatusMock.mockClear();
    browserOpenMock.mockClear();
    browserListeners.clear();
    useSessionStore.setState({
      sessionId: null,
      sessionCwd: null,
      pendingAskUser: [],
      todos: [],
      subagents: new Map(),
      jobs: new Map(),
      gitStatus: [],
      flaggedFiles: [],
      availableSessions: [],
      alerts: [],
    });
  });

  beforeEach(() => {
    useSessionStore.setState({ connectionState: "connected" });
  });

  it("opens and reveals the workbench when a message file chip asks for it", () => {
    useSessionStore.setState({ sessionId: "chat", sessionCwd: "/work" });
    const onAction = vi.fn();
    const onOpenChange = vi.fn();
    render(<RightRail mode="dock" open={false} onOpenChange={onOpenChange} workbenchSessionId="chat" workbenchTabs={{
      tabs: [], activeTabId: "",
    }} onWorkbenchTabsAction={onAction} />);
    fireEvent(window, new CustomEvent("mew:open-file", { detail: { sessionId: "other", path: "ignored.ts" } }));
    expect(onAction).not.toHaveBeenCalled();
    expect(onOpenChange).not.toHaveBeenCalled();
    fireEvent(window, new CustomEvent("mew:open-file", { detail: { sessionId: "chat", path: "mascot.tsx" } }));
    expect(onOpenChange).toHaveBeenCalledWith(true);
    expect(onAction).toHaveBeenCalledWith(expect.objectContaining({
      type: "add",
      tab: expect.objectContaining({ kind: "file", payload: { files: { openPaths: ["mascot.tsx"], activePath: "mascot.tsx" } } }),
    }), undefined);
  });

  it("keeps pinned context compact and lets users inspect it", () => {
    useSessionStore.setState({ sessionId: "chat", sessionCwd: "/work", flaggedFiles: [{ path: "/work/PLAN.md" }] });
    render(<RightRail mode="dock" open onOpenChange={vi.fn()} workbenchSessionId="chat" workbenchTabs={{
      tabs: [{ id: "files", kind: "file", title: "Files", closable: true }], activeTabId: "files",
    }} onWorkbenchTabsAction={vi.fn()} />);
    const button = screen.getByRole("button", { name: /Pinned context/ });
    expect(button.getAttribute("aria-expanded")).toBe("false");
    expect(screen.queryByText("/work/PLAN.md")).toBeNull();
    fireEvent.click(button);
    expect(screen.getByText("/work/PLAN.md")).toBeTruthy();
  });

  it("retains visited Electron guests through pane, tab and chat switches and closes only the removed owner", async () => {
    vi.spyOn(Element.prototype, "getBoundingClientRect").mockReturnValue(new DOMRect(800, 80, 400, 600));
    const requests: Array<{ type: string; sessionId?: string; tabId?: string }> = [];
    window.mewQuery = ({ request, onSuccess }) => {
      const value = JSON.parse(request);
      if (value.type === "bootstrap") { onSuccess(JSON.stringify({ browserMode: "electron", daemonUrl: "ws://127.0.0.1:25566", themeMode: "dark", lightTheme: "light", darkTheme: "dark", themes: [] })); return; }
      requests.push(value);
      onSuccess(JSON.stringify(value.type === "browser_update" && value.visible ? { partition: `${value.sessionId}/${value.tabId}`, pageInstance: `${value.sessionId}/${value.tabId}` } : {}));
    };
    await initializeHost();
    useSessionStore.setState({ sessionId: "chat-a" });
    const a1 = { id: "shared-id", kind: "browser" as const, title: "A one", closable: true, sessionId: "chat-a", payload: { url: "https://a.example" } };
    const a2 = { ...a1, id: "a-two", title: "A two", payload: { url: "https://a.example/two" } };
    const b = { ...a1, title: "B", sessionId: "chat-b", payload: { url: "https://b.example" } };
    const tabsA = { tabs: [a1, a2], activeTabId: a1.id };
    const tabsB = { tabs: [b], activeTabId: b.id };
    const changed = vi.fn();
    const props = { mode: "dock" as const, open: true, onOpenChange: vi.fn(), workbenchSessionId: "chat-a", workbenchTabs: tabsA, sessionTabs: { "chat-b": tabsB }, onWorkbenchTabsAction: changed };
    const view = render(<RightRail {...props} />);
    await waitFor(() => expect(view.container.querySelectorAll("webview")).toHaveLength(1));
    const first = view.container.querySelector("webview")!;
    const firstPanel = first.closest('[role="tabpanel"]')!;
    const removedNodes: Node[] = [];
    const moves = new MutationObserver((records) => records.forEach((record) => removedNodes.push(...record.removedNodes)));
    moves.observe(firstPanel.parentElement!, { childList: true });
    view.rerender(<RightRail {...props} open={false} />);
    expect(view.container.querySelector("webview")).toBe(first);
    view.rerender(<RightRail {...props} workbenchTabs={{ ...tabsA, activeTabId: a2.id }} />);
    await waitFor(() => expect(view.container.querySelectorAll("webview")).toHaveLength(2));
    expect(first.isConnected).toBe(true);
    useSessionStore.setState({ sessionId: "chat-b" });
    view.rerender(<RightRail {...props} workbenchSessionId="chat-b" workbenchTabs={tabsB} sessionTabs={{ "chat-a": tabsA }} />);
    await waitFor(() => expect(view.container.querySelectorAll("webview")).toHaveLength(3));
    expect(first.isConnected).toBe(true);
    expect(removedNodes).not.toContain(firstPanel);
    moves.disconnect();
    const secondChatGuest = view.container.querySelector('webview[partition="chat-b/shared-id"]')!;
    fireEvent(window, new CustomEvent("mew:browser-event", { detail: { sessionId: "chat-a", tabId: a1.id, pageInstance: "chat-a/shared-id", kind: "ready", url: a1.payload.url, title: "A" } }));
    fireEvent(window, new CustomEvent("mew:browser-event", { detail: { sessionId: "chat-a", tabId: a1.id, pageInstance: "chat-a/shared-id", kind: "changed", url: "https://a.example/changed", title: "A changed" } }));
    expect(changed).toHaveBeenCalledWith({ type: "update", id: a1.id, patch: { title: "A changed", payload: { url: "https://a.example/changed" } } }, "chat-a");
    expect(screen.getByRole("tab", { name: "B" }).getAttribute("aria-selected")).toBe("true");
    expect(requests.some((request) => request.type === "browser_close")).toBe(false);
    view.rerender(<RightRail {...props} workbenchSessionId="chat-b" workbenchTabs={{ tabs: [], activeTabId: "" }} sessionTabs={{ "chat-a": tabsA }} />);
    expect(secondChatGuest.isConnected).toBe(false);
    expect(first.isConnected).toBe(true);
    await waitFor(() => expect(requests).toContainEqual({ type: "browser_close", sessionId: "chat-b", tabId: b.id }));
    expect(requests.some((request) => request.type === "browser_close" && request.sessionId === "chat-a")).toBe(false);
  });

  it("keeps questions in the main interface instead of duplicating them in the rail", () => {
    useSessionStore.setState({
      sessionId: "sess-1",
      pendingAskUser: [{
        requestId: "request-1",
        callId: "call-1",
        questions: [{ prompt: "Which file should I edit?", options: [] }],
      }],
    });

    render(<RightRail open onOpenChange={vi.fn()} />);
    expect(screen.queryByRole("tab", { name: /questions/i })).toBeNull();
    expect(screen.queryByText("Which file should I edit?")).toBeNull();
  });

  it("uses the workbench toolbar as the single sheet close control", () => {
    render(<RightRail mode="sheet" open onOpenChange={vi.fn()} />);
    expect(screen.getByRole("dialog").getAttribute("aria-labelledby")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Close workbench" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: /^Close$/ })).toBeNull();
  });

  it("exposes every section as a keyboard-navigable tab", () => {
    render(<RightRail open onOpenChange={vi.fn()} />);

    expect(screen.getByRole("tablist", { name: "Workbench tabs" })).toBeTruthy();
    expect(screen.queryByRole("tablist", { name: "Activity sections" })).toBeNull();
    expect(screen.queryAllByRole("tab")).toHaveLength(0);
    expect(screen.getByText("Tools")).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "Add workbench tab" }));
    expect(screen.getByRole("button", { name: /Files Browse the active project files/ })).toBeTruthy();
    expect(screen.getByRole("textbox", { name: "Browser URL" })).toBeTruthy();
  });

  it.each([false, true])("keeps activity out of the launcher (explicit new tab: %s)", (newTab) => {
    render(<RightRail mode="dock" open onOpenChange={vi.fn()} />);
    if (newTab) addWorkbenchTab("Browser");

    expect(screen.queryByRole("button", { name: /^Agents\b/ })).toBeNull();
    expect(screen.queryByRole("button", { name: /^Jobs\b/ })).toBeNull();
    for (const label of ["Changes", "Terminal", "Files", "Review"]) {
      expect(screen.getByRole("button", { name: new RegExp(`^${label}\\b`) })).toBeTruthy();
    }
  });

  it("switches between workbench surfaces without closing the rail", () => {
    render(<RightRail mode="dock" open onOpenChange={vi.fn()} />);

    addWorkbenchTab("Browser");
    expect(screen.getByRole("tab", { name: "New tab" }).getAttribute("aria-selected")).toBe("true");
    expect(screen.getByRole("textbox", { name: "Browser URL" })).toBeTruthy();
    expect(screen.queryByRole("heading", { name: "Workbench" })).toBeNull();
    expect(screen.queryByRole("textbox", { name: "Browser element selector" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Open browser tools" }));
    expect(screen.getByRole("button", { name: "Text snapshot" })).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: /Inspect and interact/ }));
    expect(screen.getByRole("textbox", { name: "Browser element selector" })).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Close browser tools" }));

    addWorkbenchTab("Review");
    expect(screen.getByRole("tab", { name: "Review" }).getAttribute("aria-selected")).toBe("true");
    expect(screen.getByText("Choose a workspace first")).toBeTruthy();
  });

  it("focuses a new tab URL and handles the native terminal menu action", () => {
    render(<RightRail mode="dock" open onOpenChange={vi.fn()} />);
    fireEvent.click(screen.getByRole("button", { name: "Add workbench tab" }));
    expect(document.activeElement).toBe(screen.getByRole("textbox", { name: "Browser URL" }));
    fireEvent(window, new CustomEvent("mew:menu-action", { detail: "terminal" }));
    expect(screen.getByRole("tab", { name: "Terminal" }).getAttribute("aria-selected")).toBe("true");
  });

  it("supports multiple browser workbench tabs", () => {
    render(<RightRail mode="dock" open onOpenChange={vi.fn()} />);
    addWorkbenchTab("Browser");

    expect(screen.getAllByRole("tab", { name: "New tab" })).toHaveLength(1);
    addWorkbenchTab("Browser");
    expect(screen.getAllByRole("tab", { name: "New tab" })).toHaveLength(2);

    fireEvent.change(screen.getByRole("textbox", { name: "Browser URL" }), {
      target: { value: "https://example.com" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Open URL" }));

    expect(screen.getByRole("tab", { name: "example.com" }).getAttribute("aria-selected")).toBe("true");
    expect(browserOpenMock).toHaveBeenCalledTimes(1);

    const newTabs = screen.getAllByRole("tab", { name: "New tab" });
    fireEvent.click(newTabs[0]!);
    expect(screen.getAllByRole("tab", { name: "New tab" })[0]!.getAttribute("aria-selected")).toBe("true");
  });

  it("does not send a restored browser tab while the daemon is disconnected", () => {
    useSessionStore.setState({ connectionState: "disconnected" });
    const restoredBrowserTab = {
      id: "browser-restored",
      kind: "browser" as const,
      title: "example.com",
      closable: true,
      payload: { url: "https://example.com" },
    };

    render(
      <RightRail
        mode="dock"
        open
        onOpenChange={vi.fn()}
        workbenchTabs={{
          tabs: [restoredBrowserTab],
          activeTabId: restoredBrowserTab.id,
        }}
        onWorkbenchTabsChange={vi.fn()}
      />,
    );

    expect(browserOpenMock).not.toHaveBeenCalled();
    expect((screen.getByRole("button", { name: "Open URL" }) as HTMLButtonElement).disabled).toBe(true);
  });

  it("uses Codex-style tab shortcuts for browser tabs and tab selection", () => {
    render(<RightRail mode="dock" open onOpenChange={vi.fn()} />);
    addWorkbenchTab("Browser");

    fireEvent.keyDown(window, { key: "t", metaKey: true });
    expect(screen.getAllByRole("tab", { name: "New tab" })).toHaveLength(2);

    fireEvent.keyDown(window, { key: "1", metaKey: true });
    expect(screen.getAllByRole("tab", { name: "New tab" })[0]!.getAttribute("aria-selected")).toBe("true");
  });

  it("leaves chat shortcuts alone while the closed pane finishes its exit", () => {
    const onTabsChange = vi.fn();
    const tabs = { tabs: [{ id: "closing-browser", kind: "browser" as const, title: "New tab", closable: true }], activeTabId: "closing-browser" };
    render(<RightRail mode="dock" open={false} present onOpenChange={vi.fn()} workbenchTabs={tabs} onWorkbenchTabsChange={onTabsChange} />);
    for (const key of ["t", "w", "1"]) {
      const event = new KeyboardEvent("keydown", { key, metaKey: true, cancelable: true });
      fireEvent(window, event);
      expect(event.defaultPrevented).toBe(false);
    }
    fireEvent(window, new CustomEvent("mew:menu-action", { detail: "close-tab" }));
    expect(onTabsChange).not.toHaveBeenCalled();
  });

  it("retains the selected native browser through exit without leaving it interactive", () => {
    const calls: Record<string, unknown>[] = [];
    window.mewQuery = ({ request, onSuccess }) => { calls.push(JSON.parse(request)); onSuccess("{}"); };
    vi.spyOn(Element.prototype, "getBoundingClientRect").mockReturnValue(new DOMRect(800, 80, 400, 600));
    const tabs = { tabs: [{ id: "native-closing-browser", sessionId: "native-closing-chat", kind: "browser" as const, title: "Example", closable: true, payload: { url: "https://example.com" } }], activeTabId: "native-closing-browser" };
    const props = { mode: "dock" as const, onOpenChange: vi.fn(), workbenchTabs: tabs, onWorkbenchTabsChange: vi.fn() };
    const { rerender } = render(<RightRail {...props} open present />);
    expect(calls.at(-1)).toMatchObject({ visible: true, interactive: true });
    rerender(<RightRail {...props} open={false} present />);
    expect(calls.at(-1)).toMatchObject({ visible: true, interactive: false });
    rerender(<RightRail {...props} open={false} present={false} />);
    expect(calls.at(-1)).toMatchObject({ visible: false, interactive: false });
  });

  it("surfaces browser protocol errors and stops the loading state", async () => {
    render(<RightRail mode="dock" open onOpenChange={vi.fn()} />);
    addWorkbenchTab("Browser");
    fireEvent.change(screen.getByRole("textbox", { name: "Browser URL" }), {
      target: { value: "https://example.com" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Open URL" }));

    expect((screen.getByRole("button", { name: "Open URL" }) as HTMLButtonElement).disabled).toBe(true);
    browserListeners.get("errorMessage")?.({ message: "unknown variant `browser_open`" });

    await waitFor(() => {
      expect(screen.getByRole("alert").textContent).toContain("unknown variant `browser_open`");
      expect((screen.getByRole("button", { name: "Open URL" }) as HTMLButtonElement).disabled).toBe(false);
    });
  });

  it("ignores browser errors addressed to another tab", async () => {
    render(<RightRail mode="dock" open onOpenChange={vi.fn()} />);
    addWorkbenchTab("Browser");
    fireEvent.change(screen.getByRole("textbox", { name: "Browser URL" }), {
      target: { value: "https://example.com" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Open URL" }));
    const activeTabId = browserOpenMock.mock.calls.at(-1)?.[1] as string;

    browserListeners.get("browser-error")?.({ message: "stale error", tabId: "browser-other" });
    expect(screen.queryByRole("alert")).toBeNull();
    expect((screen.getByRole("button", { name: "Open URL" }) as HTMLButtonElement).disabled).toBe(true);

    browserListeners.get("browser-error")?.({ message: "current tab failed", tabId: activeTabId });
    await waitFor(() => {
      expect(screen.getByRole("alert").textContent).toContain("current tab failed");
      expect((screen.getByRole("button", { name: "Open URL" }) as HTMLButtonElement).disabled).toBe(false);
    });
  });

  it("explains when the session has no workspace instead of requesting files", () => {
    useSessionStore.setState({ sessionId: "sess-1", sessionCwd: null });

    render(<RightRail open onOpenChange={vi.fn()} />);
    addWorkbenchTab("Files");

    expect(screen.getByText("Choose a workspace to browse files.")).toBeTruthy();
    expect(watchWorkspaceMock).not.toHaveBeenCalled();
    expect(gitStatusMock).not.toHaveBeenCalled();
  });
});
