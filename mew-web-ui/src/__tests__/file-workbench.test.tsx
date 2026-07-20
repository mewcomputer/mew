import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
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

vi.mock("../components/code-block", () => ({
  CodeBlock: ({ code }: { code: string }) => <pre>{code}</pre>,
}));

describe("WorkspaceFileWorkbench", () => {
  afterEach(() => {
    cleanup();
    clientMock.listDir.mockClear();
    clientMock.readFilePreview.mockClear();
    clientMock.openPath.mockClear();
    useSessionStore.setState({
      sessionId: null,
      sessionCwd: null,
      dirListing: null,
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

    const wrapButton = screen.getByRole("button", { name: "Stop wrapping lines" });
    expect(wrapButton.getAttribute("aria-pressed")).toBe("true");
    fireEvent.click(wrapButton);
    expect(wrapButton.getAttribute("aria-pressed")).toBe("false");
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
});
