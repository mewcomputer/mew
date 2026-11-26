import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { WorkspaceFileWorkbench } from "../components/file-workbench";
import { useSessionStore } from "../stores/session";

const { clientMock } = vi.hoisted(() => ({
  clientMock: {
    listDir: vi.fn(),
    readFilePreview: vi.fn(),
    openPath: vi.fn(),
  },
}));

vi.mock("../lib/client-ref", () => ({
  getClient: () => clientMock,
}));

vi.mock("@pierre/diffs/react", () => ({
  File: ({ file }: { file: { contents: string } }) => <pre>{file.contents}</pre>,
}));

describe("WorkspaceFileWorkbench", () => {
  beforeEach(() => useSessionStore.setState({ connectionState: "connected" }));
  afterEach(() => {
    vi.useRealTimers();
    cleanup();
    clientMock.listDir.mockClear();
    clientMock.readFilePreview.mockClear();
    clientMock.openPath.mockClear();
    useSessionStore.setState({
      sessionId: null,
      sessionCwd: null,
      dirListing: null,
      dirListingError: null,
      dirListingPath: null,
      filePreview: null,
    });
  });

  it("loads the root, expands directories lazily, and opens files in tabs", async () => {
    useSessionStore.setState({ sessionId: "sess-1", sessionCwd: "/work/mew" });
    render(<WorkspaceFileWorkbench hasWorkspace />);

    expect(clientMock.listDir).toHaveBeenCalledWith("sess-1");

    useSessionStore.getState().onDirListing("", [
      { name: "src", is_dir: true },
      { name: "README.md", is_dir: false, size: 12 },
    ]);
    expect(await screen.findByRole("button", { name: "src" })).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "src" }));
    expect(clientMock.listDir).toHaveBeenCalledWith("sess-1", "src");

    useSessionStore.getState().onDirListing("src", [
      { name: "main.rs", is_dir: false, size: 42 },
    ]);
    expect(await screen.findByRole("button", { name: "main.rs" })).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "main.rs" }));
    expect(clientMock.readFilePreview).toHaveBeenCalledWith("sess-1", "src/main.rs");

    useSessionStore.getState().onFilePreview("src/main.rs", "fn main() {}", false, "rust");
    expect(await screen.findByText("fn main() {}"))
      .toBeTruthy();
    expect(screen.getAllByText("main.rs").length).toBeGreaterThan(0);

    const wrapButton = screen.getByRole("button", { name: "Wrap lines" });
    expect(wrapButton.getAttribute("aria-pressed")).toBe("false");
    fireEvent.click(wrapButton);
    expect(wrapButton.getAttribute("aria-pressed")).toBe("true");
  });

  it("opens markdown as a rendered document with a reopenable explorer", async () => {
    useSessionStore.setState({ sessionId: "sess-1", sessionCwd: "/work/mew" });
    render(<WorkspaceFileWorkbench hasWorkspace />);
    act(() => useSessionStore.getState().onDirListing("", [{ name: "README.MD", is_dir: false }]));
    fireEvent.click(await screen.findByRole("button", { name: "README.MD" }));
    expect(screen.getByRole("button", { name: "Hide file explorer" })).toBeTruthy();
    act(() => useSessionStore.getState().onFilePreview("README.MD", "# Project\n\nA **useful** guide.\n\n```rust\nfn main() {}\n```", true));
    expect(await screen.findByRole("heading", { name: "Project" })).toBeTruthy();
    expect(screen.getByText("useful").tagName).toBe("STRONG");
    expect(screen.getByRole("button", { name: "Copy code" })).toBeTruthy();
    expect(screen.getByText("Preview truncated")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Hide file explorer" }));
    expect(screen.getByRole("button", { name: "Show file explorer" })).toBeTruthy();
  });

  it("restores the explorer preference with the files tab", () => {
    useSessionStore.setState({ sessionId: "sess-1", sessionCwd: "/work/mew" });
    let saved: import("../lib/workbench-tabs").FileWorkbenchState = { openPaths: ["guide.md"], activePath: "guide.md", explorerVisible: false };
    const view = render(<WorkspaceFileWorkbench hasWorkspace fileState={saved} onFileStateChange={(state) => { saved = state; }} />);
    expect(screen.getByRole("button", { name: "Show file explorer" })).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Show file explorer" }));
    expect(saved.explorerVisible).toBe(true);
    view.unmount();
    render(<WorkspaceFileWorkbench hasWorkspace fileState={saved} />);
    expect(screen.getByRole("button", { name: "Hide file explorer" })).toBeTruthy();
  });

  it("keeps replies for multiple directories delivered in one batch", async () => {
    useSessionStore.setState({ sessionId: "sess-1", sessionCwd: "/work/mew" });
    render(<WorkspaceFileWorkbench hasWorkspace />);
    act(() => useSessionStore.getState().onDirListing("", [{ name: "a", is_dir: true }, { name: "b", is_dir: true }]));
    fireEvent.click(await screen.findByRole("button", { name: "a" }));
    fireEvent.click(screen.getByRole("button", { name: "b" }));
    act(() => {
      useSessionStore.getState().onDirListing("a", [{ name: "one.txt", is_dir: false }]);
      useSessionStore.getState().onDirListing("b", [{ name: "two.txt", is_dir: false }]);
    });
    expect(await screen.findByRole("button", { name: "one.txt" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "two.txt" })).toBeTruthy();
  });

  it("shows a retryable error for a failed directory and recovers on retry", async () => {
    useSessionStore.setState({ sessionId: "sess-1", sessionCwd: "/work/mew" });
    render(<WorkspaceFileWorkbench hasWorkspace />);
    act(() => useSessionStore.getState().onDirListing("", [{ name: "src", is_dir: true }]));
    fireEvent.click(await screen.findByRole("button", { name: "src" }));
    act(() => useSessionStore.getState().onDirError("src", "Permission denied"));
    expect(await screen.findByText("Permission denied")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Retry src" }));
    expect(clientMock.listDir).toHaveBeenLastCalledWith("sess-1", "src");
    act(() => useSessionStore.getState().onDirError("src", "Permission denied"));
    expect(await screen.findByText("Permission denied")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Retry src" }));
    act(() => useSessionStore.getState().onDirListing("src", [{ name: "main.rs", is_dir: false }]));
    expect(await screen.findByRole("button", { name: "main.rs" })).toBeTruthy();
    expect(screen.queryByText("Permission denied")).toBeNull();
  });

  it("shows a root refresh failure after an earlier successful listing", async () => {
    useSessionStore.setState({ sessionId: "sess-1", sessionCwd: "/work/mew" });
    render(<WorkspaceFileWorkbench hasWorkspace />);
    act(() => useSessionStore.getState().onDirListing("", [{ name: "old.txt", is_dir: false }]));
    expect(await screen.findByRole("button", { name: "old.txt" })).toBeTruthy();
    act(() => useSessionStore.getState().onDirError("", "Workspace unavailable"));
    expect(await screen.findByText("Workspace unavailable")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Retry workspace" }));
    act(() => useSessionStore.getState().onDirListing("", [{ name: "new.txt", is_dir: false }]));
    expect(await screen.findByRole("button", { name: "new.txt" })).toBeTruthy();
  });

  it("times out again when a retry receives no response", () => {
    vi.useFakeTimers();
    useSessionStore.setState({ sessionId: "sess-1", sessionCwd: "/work/mew" });
    render(<WorkspaceFileWorkbench hasWorkspace />);
    act(() => useSessionStore.getState().onDirError("", "Unavailable"));
    fireEvent.click(screen.getByRole("button", { name: "Retry workspace" }));
    act(() => vi.advanceTimersByTime(15_000));
    expect(screen.getByText(/directory listing timed out/i)).toBeTruthy();
  });

  it("does not reuse an old root response when Files remounts", () => {
    vi.useFakeTimers();
    useSessionStore.setState({ sessionId: "sess-1", sessionCwd: "/work/mew" });
    const view = render(<WorkspaceFileWorkbench hasWorkspace />);
    act(() => useSessionStore.getState().onDirListing("", [{ name: "old.txt", is_dir: false }]));
    view.unmount();
    render(<WorkspaceFileWorkbench hasWorkspace />);
    act(() => vi.advanceTimersByTime(15_000));
    expect(screen.getByText(/directory listing timed out/i)).toBeTruthy();
  });

  it("allows retrying a workspace listing that never responds", async () => {
    vi.useFakeTimers();
    useSessionStore.setState({ sessionId: "sess-1", sessionCwd: "/work/mew" });
    render(<WorkspaceFileWorkbench hasWorkspace />);
    act(() => vi.advanceTimersByTime(15_000));
    expect(screen.getByText(/directory listing timed out/i)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Retry workspace" }));
    expect(clientMock.listDir).toHaveBeenCalledTimes(2);
  });

  it("filters explorer entries without changing their relative paths", async () => {
    useSessionStore.setState({ sessionId: "sess-1", sessionCwd: "/work/mew" });
    render(<WorkspaceFileWorkbench hasWorkspace />);
    useSessionStore.getState().onDirListing("", [
      { name: "src", is_dir: true },
      { name: "README.md", is_dir: false },
    ]);
    await screen.findByRole("button", { name: "src" });

    fireEvent.change(screen.getByRole("textbox", { name: "Filter files" }), { target: { value: "read" } });
    expect(screen.getByRole("button", { name: "README.md" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "src" })).toBeNull();
  });

  it("shows a focused empty state when no file is open", async () => {
    useSessionStore.setState({ sessionId: "sess-1", sessionCwd: "/work/mew" });
    render(<WorkspaceFileWorkbench hasWorkspace />);
    expect(await screen.findByText("Open a file")).toBeTruthy();
    await waitFor(() => expect(clientMock.listDir).toHaveBeenCalledWith("sess-1"));
  });

  it("does not reopen a file closed while its preview is loading", async () => {
    useSessionStore.setState({ sessionId: "sess-1", sessionCwd: "/work/mew" });
    render(<WorkspaceFileWorkbench hasWorkspace />);
    useSessionStore.getState().onDirListing("", [{ name: "README.md", is_dir: false }]);
    fireEvent.click(await screen.findByRole("button", { name: "README.md" }));
    fireEvent.click(screen.getByRole("button", { name: "Close README.md" }));
    useSessionStore.getState().onFilePreview("README.md", "late response", false);
    await waitFor(() => expect(screen.getByText("Open a file")).toBeTruthy());
    expect(screen.queryByText("late response")).toBeNull();
    expect(screen.queryByRole("button", { name: "Open README.md" })).toBeNull();
  });

  it("shows a useful error when a saved file can no longer be read", async () => {
    useSessionStore.setState({ sessionId: "sess-1", sessionCwd: "/work/mew" });
    render(<WorkspaceFileWorkbench hasWorkspace fileState={{ openPaths: ["gone.md"], activePath: "gone.md" }} />);
    useSessionStore.getState().onFilePreview("gone.md", "", false, undefined, "file not found");
    expect(await screen.findByText("file not found")).toBeTruthy();
    expect(screen.queryByText("Loading file")).toBeNull();
  });

  it("opens a file whose name is also an object property", async () => {
    useSessionStore.setState({ sessionId: "sess-1", sessionCwd: "/work/mew" });
    render(<WorkspaceFileWorkbench hasWorkspace />);
    useSessionStore.getState().onDirListing("", [{ name: "constructor", is_dir: false }]);
    fireEvent.click(await screen.findByRole("button", { name: "constructor" }));
    expect(clientMock.readFilePreview).toHaveBeenLastCalledWith("sess-1", "constructor");
    useSessionStore.getState().onFilePreview("constructor", "actual content", false);
    expect(await screen.findByText("actual content")).toBeTruthy();
  });

  it("expands a directory whose name is also an object property", async () => {
    useSessionStore.setState({ sessionId: "sess-1", sessionCwd: "/work/mew" });
    render(<WorkspaceFileWorkbench hasWorkspace />);
    useSessionStore.getState().onDirListing("", [{ name: "constructor", is_dir: true }]);
    fireEvent.click(await screen.findByRole("button", { name: "constructor" }));
    expect(clientMock.listDir).toHaveBeenLastCalledWith("sess-1", "constructor");
    useSessionStore.getState().onDirListing("constructor", [{ name: "index.txt", is_dir: false }]);
    expect(await screen.findByRole("button", { name: "index.txt" })).toBeTruthy();
  });

  it("recovers a pending preview after disconnection and reconnection", async () => {
    useSessionStore.setState({ sessionId: "sess-1", sessionCwd: "/work/mew" });
    render(<WorkspaceFileWorkbench hasWorkspace fileState={{ openPaths: ["README.md"], activePath: "README.md" }} />);
    act(() => useSessionStore.setState({ connectionState: "disconnected" }));
    expect(await screen.findByText(/disconnected/i)).toBeTruthy();
    clientMock.readFilePreview.mockClear();
    act(() => useSessionStore.setState({ connectionState: "connected" }));
    expect(clientMock.readFilePreview).toHaveBeenCalledWith("sess-1", "README.md");
    useSessionStore.getState().onFilePreview("README.md", "reconnected contents", false);
    expect(await screen.findByText("reconnected contents")).toBeTruthy();
  });

  it("does not leave a file loading indefinitely if a daemon never replies", () => {
    vi.useFakeTimers();
    useSessionStore.setState({ sessionId: "sess-1", sessionCwd: "/work/mew" });
    render(<WorkspaceFileWorkbench hasWorkspace fileState={{ openPaths: ["gone.md"], activePath: "gone.md" }} />);
    act(() => vi.advanceTimersByTime(15_000));
    expect(screen.getByText(/file preview timed out/i)).toBeTruthy();
    expect(screen.getByRole("button", { name: "Retry" })).toBeTruthy();
  });

  it("restores open files and the selected document after leaving a conversation", async () => {
    useSessionStore.setState({ sessionId: "sess-1", sessionCwd: "/work/mew", filePreview: null });
    let saved: { openPaths: string[]; activePath: string | null } = { openPaths: [], activePath: null };
    const first = render(<WorkspaceFileWorkbench hasWorkspace fileState={saved} onFileStateChange={(state) => { saved = state; }} />);
    useSessionStore.getState().onDirListing("", [
      { name: "README.md", is_dir: false },
      { name: "PLAN.md", is_dir: false },
    ]);
    fireEvent.click(await screen.findByRole("button", { name: "README.md" }));
    useSessionStore.getState().onFilePreview("README.md", "readme contents", false);
    expect(await screen.findByText("readme contents")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "PLAN.md" }));
    useSessionStore.getState().onFilePreview("PLAN.md", "plan contents", false);
    expect(await screen.findByText("plan contents")).toBeTruthy();
    first.unmount();

    useSessionStore.setState({ sessionId: "sess-2", sessionCwd: "/other", filePreview: null, dirListing: null, dirListingPath: null });
    const second = render(<WorkspaceFileWorkbench hasWorkspace />);
    expect(await screen.findByText("Open a file")).toBeTruthy();
    second.unmount();

    useSessionStore.setState({ sessionId: "sess-1", sessionCwd: "/work/mew", filePreview: null, dirListing: null, dirListingPath: null });
    render(<WorkspaceFileWorkbench hasWorkspace fileState={saved} onFileStateChange={(state) => { saved = state; }} />);
    expect(screen.getByRole("button", { name: "Open README.md" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Open PLAN.md" })).toBeTruthy();
    expect(clientMock.readFilePreview).toHaveBeenLastCalledWith("sess-1", "PLAN.md");
    useSessionStore.getState().onFilePreview("PLAN.md", "updated plan", false);
    expect(await screen.findByText("updated plan")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Close PLAN.md" }));
    expect(screen.queryByRole("button", { name: "Open PLAN.md" })).toBeNull();
    expect(saved).toEqual({ openPaths: ["README.md"], activePath: "README.md", explorerVisible: true });
    expect(clientMock.readFilePreview).toHaveBeenLastCalledWith("sess-1", "README.md");
    useSessionStore.getState().onFilePreview("README.md", "updated readme", false);
    expect(await screen.findByText("updated readme")).toBeTruthy();
  });
});
