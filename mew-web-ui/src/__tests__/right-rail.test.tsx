import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { RightRail } from "../components/right-rail";
import { useSessionStore } from "../stores/session";

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
  FileTreePanel: ({ hasWorkspace }: { hasWorkspace: boolean }) => (
    <div>{hasWorkspace ? "Files panel" : "Choose a workspace to browse files."}</div>
  ),
  ChangesPanel: ({ hasWorkspace }: { hasWorkspace: boolean }) => (
    <div>{hasWorkspace ? "Changes panel" : "Choose a workspace to see changes."}</div>
  ),
}));

describe("RightRail", () => {
  function addWorkbenchTab(label: string) {
    const trigger = screen.getByRole("button", { name: "Add workbench tab" });
    fireEvent.click(trigger);
    fireEvent.click(screen.getByRole("option", { name: new RegExp(`^${label}\\b`) }));
  }

  afterEach(() => {
    cleanup();
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

  it("exposes every section as a keyboard-navigable tab", () => {
    render(<RightRail open onOpenChange={vi.fn()} />);

    expect(screen.getByRole("tablist", { name: "Workbench tabs" })).toBeTruthy();
    expect(screen.queryByRole("tablist", { name: "Activity sections" })).toBeNull();
    expect(screen.queryAllByRole("tab")).toHaveLength(0);
    expect(screen.getByText("No workbench tabs")).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "Add workbench tab" }));
    expect(screen.getAllByRole("option")).toHaveLength(7);
    expect(screen.getByRole("option", { name: /Browser Open a browser tab/ })).toBeTruthy();
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
