import { afterEach, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { useReducer } from "react";
import { RightRail } from "../components/right-rail";
import { DEFAULT_WORKSPACE_SURFACES, workspaceSurfacesReducer } from "../lib/workspace-surfaces";
import { useSessionStore } from "../stores/session";

const { client } = vi.hoisted(() => ({ client: { watchWorkspace: vi.fn(), gitStatus: vi.fn(), listDir() {}, readFilePreview: vi.fn(), readSessionDiff: vi.fn(), on() {}, off() {} } }));
vi.mock("../lib/client-ref", () => ({ getClient: () => client }));
vi.mock("@pierre/diffs/react", () => ({
  File: ({ file }: { file: { contents: string } }) => <pre>{file.contents}</pre>,
}));
vi.mock("@pierre/trees/react", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@pierre/trees/react")>();
  return { ...actual, FileTree: ({ model }: { model: ReturnType<typeof actual.useFileTree>["model"] }) => (
    <div>{model.getVisibleRows(0, model.getVisibleCount()).map((row) => (
      <button key={row.path} onClick={() => model.getItem(row.path)?.select()}>{row.name}</button>
    ))}</div>
  ) };
});

function Workbench({ existingFile = false, review = false }: { existingFile?: boolean; review?: boolean }) {
  const [state, dispatch] = useReducer(workspaceSurfacesReducer, {
    ...DEFAULT_WORKSPACE_SURFACES,
    sessionId: "chat-a",
    workbenchOpen: true,
    workbenchTabs: {
      tabs: [
        ...(existingFile ? [{ id: "files", kind: "file" as const, title: "Files", closable: true, payload: { files: { openPaths: ["README.md"], activePath: "README.md" } } }] : []),
        { id: "changes", kind: "changes", title: "Changes", closable: true },
        { id: "review", kind: "review", title: "Review", closable: true },
      ],
      activeTabId: review ? "review" : "changes",
    },
  });
  return <RightRail mode="dock" open onOpenChange={() => {}} workbenchSessionId={state.sessionId}
    workbenchTabs={state.workbenchTabs} sessionTabs={state.sessionTabs}
    onWorkbenchTabsAction={(action, sessionId) => dispatch({ type: "workbench-tabs-action", action, sessionId })} />;
}

afterEach(() => {
  cleanup();
  vi.useRealTimers();
  client.readFilePreview.mockClear();
  useSessionStore.setState({ sessionId: null, sessionCwd: null, gitStatus: [], filePreview: null, dirListing: null, dirListingPath: null });
});

it("shows a retryable error when refreshing an open file gets no response", () => {
  vi.useFakeTimers();
  useSessionStore.setState({ sessionId: "chat-a", sessionCwd: "/work/mew", connectionState: "connected", gitStatus: [{ path: "README.md", status: "modified" }] });
  render(<Workbench existingFile />);
  act(() => useSessionStore.getState().onGitStatus([{ path: "README.md", status: "modified" }]));
  act(() => useSessionStore.getState().onFilePreview("README.md", "old contents", false));
  fireEvent.click(screen.getByRole("button", { name: /README.md/ }));
  fireEvent.click(screen.getByRole("button", { name: "Open file" }));
  act(() => vi.advanceTimersByTime(15_000));
  expect(screen.getByText(/file preview timed out/i)).toBeTruthy();
  expect(screen.getByRole("button", { name: "Retry" })).toBeTruthy();
});

it("refreshes an already-open file when opened from Changes", async () => {
  useSessionStore.setState({ sessionId: "chat-a", sessionCwd: "/work/mew", connectionState: "connected", gitStatus: [{ path: "README.md", status: "modified" }] });
  render(<Workbench existingFile />);
  act(() => useSessionStore.getState().onGitStatus([{ path: "README.md", status: "modified" }]));
  useSessionStore.getState().onFilePreview("README.md", "old contents", false);
  client.readFilePreview.mockClear();
  fireEvent.click(screen.getByRole("button", { name: /README.md/ }));
  fireEvent.click(screen.getByRole("button", { name: "Open file" }));
  expect(client.readFilePreview).toHaveBeenCalledWith("chat-a", "README.md");
  useSessionStore.getState().onFilePreview("README.md", "new contents", false);
  expect(await screen.findByText("new contents")).toBeTruthy();
});

it("opens a changed file in the files workbench on request", async () => {
  useSessionStore.setState({
    sessionId: "chat-a", sessionCwd: "/work/mew", connectionState: "connected",
    gitStatus: [{ path: "README.md", status: "modified" }],
  });
  render(<Workbench />);
  act(() => useSessionStore.getState().onGitStatus([{ path: "README.md", status: "modified" }]));
  fireEvent.click(screen.getByRole("button", { name: /README.md/ }));
  fireEvent.click(screen.getByRole("button", { name: "Open file" }));
  expect(screen.getByRole("tab", { name: "Files" }).getAttribute("aria-selected")).toBe("true");
  expect(screen.getByRole("button", { name: "Open README.md" })).toBeTruthy();
  useSessionStore.getState().onFilePreview("README.md", "project readme", false);
  expect(await screen.findByText("project readme")).toBeTruthy();
});

it("resumes watching changes after reconnecting to the same session", () => {
  useSessionStore.setState({ sessionId: "chat-a", sessionCwd: "/work/mew", connectionState: "connected", confirmedSessionId: "chat-a" });
  render(<Workbench />);
  client.gitStatus.mockClear();
  client.watchWorkspace.mockClear();
  act(() => useSessionStore.setState({ connectionState: "disconnected", confirmedSessionId: null }));
  act(() => useSessionStore.setState({ connectionState: "connected", confirmedSessionId: "chat-a" }));
  expect(client.watchWorkspace).toHaveBeenCalledWith("chat-a", true);
  expect(client.gitStatus).toHaveBeenCalledWith("chat-a");
});

it("requests status and displays errors when Review opens", () => {
  useSessionStore.setState({ sessionId: "chat-a", sessionCwd: "/work/mew", connectionState: "connected", confirmedSessionId: "chat-a", gitStatus: [], gitStatusError: null, gitStatusLoaded: false });
  client.gitStatus.mockClear();
  render(<Workbench review />);
  expect(client.gitStatus).toHaveBeenCalledWith("chat-a");
  expect(screen.getAllByText("Loading changes…").length).toBeGreaterThan(0);
  expect(screen.queryByText("Nothing to review")).toBeNull();
  act(() => useSessionStore.setState({ gitStatusError: "Could not read status" }));
  expect(screen.getAllByText("Could not read status").length).toBeGreaterThan(0);
  expect(screen.queryByText("Nothing to review")).toBeNull();
});

it("does not reuse a clean status when Review is reopened", () => {
  vi.useFakeTimers();
  useSessionStore.setState({ sessionId: "chat-a", sessionCwd: "/work/mew", connectionState: "connected", confirmedSessionId: "chat-a", gitStatus: [], gitStatusLoaded: true, gitStatusError: null });
  render(<Workbench review />);
  act(() => useSessionStore.getState().onGitStatus([]));
  fireEvent.click(screen.getByRole("tab", { name: "Changes" }));
  act(() => useSessionStore.getState().onGitStatus([]));
  fireEvent.click(screen.getByRole("tab", { name: "Review" }));
  expect(screen.getAllByText("Loading changes…").length).toBeGreaterThan(0);
  expect(screen.queryByText("Nothing to review")).toBeNull();
  act(() => vi.advanceTimersByTime(15_000));
  expect(screen.getAllByText(/loading changes timed out/i).length).toBeGreaterThan(0);
});

it("does not throw when the connection closes while watching the workspace", () => {
  useSessionStore.setState({ sessionId: "chat-a", sessionCwd: "/work/mew", connectionState: "connected", confirmedSessionId: "chat-a" });
  client.watchWorkspace.mockImplementation((_session, enabled) => { if (!enabled) throw new Error("not connected"); });
  render(<Workbench />);
  expect(() => act(() => useSessionStore.setState({ connectionState: "disconnected", confirmedSessionId: null }))).not.toThrow();
  client.watchWorkspace.mockReset();
});
