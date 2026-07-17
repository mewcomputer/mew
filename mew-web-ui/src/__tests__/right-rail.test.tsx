import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { RightRail } from "../components/right-rail";
import { useSessionStore } from "../stores/session";

const respondToAskUserMock = vi.fn();
const watchWorkspaceMock = vi.fn();
const gitStatusMock = vi.fn();

vi.mock("../lib/client-ref", () => ({
  getClient: () => ({
    respondToAskUser: respondToAskUserMock,
    watchWorkspace: watchWorkspaceMock,
    gitStatus: gitStatusMock,
  }),
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
  afterEach(() => {
    cleanup();
    respondToAskUserMock.mockClear();
    watchWorkspaceMock.mockClear();
    gitStatusMock.mockClear();
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

  it("opens on the first actionable section", () => {
    useSessionStore.setState({
      sessionId: "sess-1",
      pendingAskUser: [{
        requestId: "request-1",
        callId: "call-1",
        questions: [{ prompt: "Which file should I edit?", options: [] }],
      }],
    });

    render(<RightRail open onOpenChange={vi.fn()} />);

    expect(screen.getByRole("tab", { name: /questions/i }).getAttribute("aria-selected")).toBe("true");
    expect(screen.getByText("Which file should I edit?")).toBeTruthy();
  });

  it("shows explicit cross-session attention at the top of the panel", () => {
    useSessionStore.setState({
      availableSessions: [{
        session_id: "sess-2",
        state: "idle",
        created_at: 1,
        client_count: 0,
        summary: "api refactor",
        pending_permissions: 1,
      }],
    });

    render(<RightRail open onOpenChange={vi.fn()} />);

    expect(screen.getByRole("heading", { name: "Needs attention" })).toBeTruthy();
    expect(screen.getByText("Permissions needed")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Permissions needed in api refactor" })).toBeTruthy();
  });

  it("keeps the panel open after resolving a question", () => {
    const onOpenChange = vi.fn();
    useSessionStore.setState({
      sessionId: "sess-1",
      pendingAskUser: [{
        requestId: "request-1",
        callId: "call-1",
        questions: [{
          prompt: "Which file should I edit?",
          options: [{ label: "src/app.ts", description: "The app entrypoint" }],
        }],
      }],
    });

    render(<RightRail open onOpenChange={onOpenChange} />);
    fireEvent.click(screen.getByRole("button", { name: /src\/app\.ts/i }));
    fireEvent.click(screen.getByRole("button", { name: "Submit" }));

    expect(respondToAskUserMock).toHaveBeenCalledWith("request-1", ["src/app.ts"]);
    expect(onOpenChange).not.toHaveBeenCalled();
  });

  it("exposes every section as a keyboard-navigable tab", () => {
    render(<RightRail open onOpenChange={vi.fn()} />);

    expect(screen.getAllByRole("tab")).toHaveLength(7);
    expect(screen.getByRole("tablist", { name: "Activity sections" })).toBeTruthy();
  });

  it("explains when the session has no workspace instead of requesting files", () => {
    useSessionStore.setState({ sessionId: "sess-1", sessionCwd: null });

    render(<RightRail open onOpenChange={vi.fn()} />);
    fireEvent.click(screen.getByRole("tab", { name: /files/i }));

    expect(screen.getByText("Choose a workspace to browse files.")).toBeTruthy();
    expect(watchWorkspaceMock).not.toHaveBeenCalled();
    expect(gitStatusMock).not.toHaveBeenCalled();
  });
});
