import { afterEach, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { ChangesPanel } from "../components/file-tree";

const { client, handlers } = vi.hoisted(() => ({
  handlers: new Map<string, Set<(value: any) => void>>(),
  client: {
    readSessionDiff: vi.fn(),
    on(event: string, handler: (value: any) => void) {
      if (!handlers.has(event)) handlers.set(event, new Set());
      handlers.get(event)!.add(handler);
    },
    off(event: string, handler: (value: any) => void) { handlers.get(event)?.delete(handler); },
  },
}));
vi.mock("../lib/client-ref", () => ({ getClient: () => client }));
vi.mock("@pierre/diffs/react", () => ({ FileDiff: () => <div>Rendered diff</div> }));
vi.mock("@pierre/trees/react", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@pierre/trees/react")>();
  return { ...actual, FileTree: ({ model }: { model: ReturnType<typeof actual.useFileTree>["model"] }) => (
    <div>{model.getVisibleRows(0, model.getVisibleCount()).map((row) => (
      <button key={row.path} aria-pressed={row.isSelected} onClick={() => model.getItem(row.path)?.select()}>{row.name}</button>
    ))}</div>
  ) };
});

afterEach(() => { cleanup(); handlers.clear(); client.readSessionDiff.mockClear(); vi.useRealTimers(); });

it("keeps the changes tree open while showing the selected file diff", () => {
  vi.useFakeTimers();
  const openFile = vi.fn();
  render(<ChangesPanel sessionId="session-a" connected gitStatus={[{ path: "one.ts", status: "modified" }]} hasWorkspace statusLoaded onOpenFile={openFile} />);
  fireEvent.click(screen.getByRole("button", { name: "one.ts" }));
  expect(screen.getByText("Loading diff…")).toBeTruthy();
  const [sessionId, requestId, path] = client.readSessionDiff.mock.lastCall!;
  expect([sessionId, path]).toEqual(["session-a", "one.ts"]);
  act(() => handlers.get("session-diff")?.forEach((handler) => handler({ session_id: sessionId, request_id: requestId, files: [{
    path, old_path: null, status: "Modified", added: 1, removed: 1,
    hunks: [{ old_start: 1, old_count: 1, new_start: 1, new_count: 1, lines: [
      { kind: "Deletion", old_line: 1, new_line: null, text: "before" },
      { kind: "Addition", old_line: null, new_line: 1, text: "after" },
    ] }],
  }] })));
  expect(screen.getByText("Rendered diff")).toBeTruthy();
  act(() => vi.advanceTimersByTime(15_000));
  expect(screen.getByText("Rendered diff")).toBeTruthy();
  expect(screen.getByRole("button", { name: "one.ts" }).getAttribute("aria-pressed")).toBe("true");
  fireEvent.click(screen.getByRole("button", { name: "Open file" }));
  expect(openFile).toHaveBeenCalledWith("one.ts");
});

it("ignores replies to an earlier file selection", () => {
  render(<ChangesPanel sessionId="session-a" connected gitStatus={[{ path: "one.ts", status: "modified" }, { path: "two.ts", status: "modified" }]} hasWorkspace statusLoaded onOpenFile={() => {}} />);
  fireEvent.click(screen.getByRole("button", { name: "one.ts" }));
  const first = client.readSessionDiff.mock.lastCall![1];
  fireEvent.click(screen.getByRole("button", { name: "two.ts" }));
  act(() => handlers.get("session-diff-error")?.forEach((handler) => handler({ session_id: "session-a", request_id: first, message: "stale failure" })));
  expect(screen.queryByText("stale failure")).toBeNull();
  expect(screen.getByText("Loading diff…")).toBeTruthy();
});

it("explains when an older daemon cannot load selected diffs", () => {
  render(<ChangesPanel sessionId="session-a" connected gitStatus={[{ path: "one.ts", status: "modified" }]} hasWorkspace statusLoaded onOpenFile={() => {}} />);
  fireEvent.click(screen.getByRole("button", { name: "one.ts" }));
  act(() => handlers.get("errorMessage")?.forEach((handler) => handler({ message: "unknown variant `read_session_file_diff`" })));
  expect(screen.getByText(/update the daemon/i)).toBeTruthy();
});

it("shows staged and unstaged sections and calls out line ending changes", () => {
  render(<ChangesPanel sessionId="session-a" connected gitStatus={[{ path: "one.ts", status: "modified" }]} hasWorkspace statusLoaded onOpenFile={() => {}} />);
  fireEvent.click(screen.getByRole("button", { name: "one.ts" }));
  const requestId = client.readSessionDiff.mock.lastCall![1];
  const file = { path: "one.ts", old_path: null, status: "Modified", added: 1, removed: 1, hunks: [{ old_start: 1, old_count: 1, new_start: 1, new_count: 1, lines: [
    { kind: "Deletion", old_line: 1, new_line: null, text: "old", ending: "CrLf" },
    { kind: "Addition", old_line: null, new_line: 1, text: "new", ending: "None" },
  ] }] };
  act(() => handlers.get("session-diff")?.forEach((handler) => handler({ session_id: "session-a", request_id: requestId, files: [{ ...file, layer: "Staged" }, { ...file, layer: "Unstaged" }] })));
  expect(screen.getByText("Staged")).toBeTruthy();
  expect(screen.getByText("Unstaged")).toBeTruthy();
  expect(screen.getAllByText(/line ending/i).length).toBeGreaterThan(0);
  expect(screen.getAllByText("Rendered diff")).toHaveLength(2);
});

it("renders native Git patches and mode-only changes", () => {
  render(<ChangesPanel sessionId="session-a" connected gitStatus={[{ path: "one.ts", status: "modified" }]} hasWorkspace statusLoaded onOpenFile={() => {}} />);
  fireEvent.click(screen.getByRole("button", { name: "one.ts" }));
  const requestId = client.readSessionDiff.mock.lastCall![1];
  const file = { path: "one.ts", old_path: null, status: "Modified", added: 0, removed: 0, hunks: [] };
  const textPatch = "diff --git a/one.ts b/one.ts\n--- a/one.ts\n+++ b/one.ts\n@@ -1,1 +1,1 @@\n-old\n+new\n";
  const modePatch = "diff --git a/one.ts b/one.ts\nold mode 100644\nnew mode 100755\n";
  act(() => handlers.get("session-diff")?.forEach((handler) => handler({ session_id: "session-a", request_id: requestId, files: [{ ...file, layer: "Staged", patch: textPatch }, { ...file, layer: "Unstaged", patch: modePatch }] })));
  expect(screen.getByText("Rendered diff")).toBeTruthy();
  expect(screen.getByText(/file mode changed/i)).toBeTruthy();
});

it("coalesces changes while a diff is still loading", () => {
  render(<ChangesPanel sessionId="session-a" connected gitStatus={[{ path: "one.ts", status: "modified" }]} hasWorkspace statusLoaded onOpenFile={() => {}} />);
  fireEvent.click(screen.getByRole("button", { name: "one.ts" }));
  const requestId = client.readSessionDiff.mock.lastCall![1];
  act(() => { for (let i = 0; i < 3; i++) handlers.get("fs-changed")?.forEach((handler) => handler({ session_id: "session-a", paths: ["one.ts"] })); });
  expect(client.readSessionDiff).toHaveBeenCalledTimes(1);
  const file = { path: "one.ts", old_path: null, status: "Modified", added: 1, removed: 0, hunks: [] };
  act(() => handlers.get("session-diff")?.forEach((handler) => handler({ session_id: "session-a", request_id: requestId, files: [file] })));
  expect(client.readSessionDiff).toHaveBeenCalledTimes(2);
});

it("refreshes on a file change after a diff request times out", () => {
  vi.useFakeTimers();
  render(<ChangesPanel sessionId="session-a" connected gitStatus={[{ path: "one.ts", status: "modified" }]} hasWorkspace statusLoaded onOpenFile={() => {}} />);
  fireEvent.click(screen.getByRole("button", { name: "one.ts" }));
  act(() => vi.advanceTimersByTime(15_000));
  expect(screen.getByText(/loading the diff timed out/i)).toBeTruthy();
  act(() => handlers.get("fs-changed")?.forEach((handler) => handler({ session_id: "session-a", paths: ["one.ts"] })));
  expect(client.readSessionDiff).toHaveBeenCalledTimes(2);
});

it("does not mistake text containing Git metadata words for a binary or mode change", () => {
  render(<ChangesPanel sessionId="session-a" connected gitStatus={[{ path: "one.ts", status: "modified" }]} hasWorkspace statusLoaded onOpenFile={() => {}} />);
  fireEvent.click(screen.getByRole("button", { name: "one.ts" }));
  const requestId = client.readSessionDiff.mock.lastCall![1];
  const patch = "diff --git a/one.ts b/one.ts\n--- a/one.ts\n+++ b/one.ts\n@@ -1,1 +1,3 @@\n old\n+Binary files are interesting\n+old mode and new mode matter\n";
  act(() => handlers.get("session-diff")?.forEach((handler) => handler({ session_id: "session-a", request_id: requestId, files: [{ path: "one.ts", old_path: null, status: "Modified", layer: "Unstaged", added: 2, removed: 0, hunks: [], patch }] })));
  expect(screen.getByText("Rendered diff")).toBeTruthy();
  expect(screen.queryByText(/binary file changes cannot/i)).toBeNull();
  expect(screen.queryByText(/file mode changed/i)).toBeNull();
});

it("identifies the old and new paths of a pure rename", () => {
  render(<ChangesPanel sessionId="session-a" connected gitStatus={[{ path: "new.ts", status: "renamed" }]} hasWorkspace statusLoaded onOpenFile={() => {}} />);
  fireEvent.click(screen.getByRole("button", { name: "new.ts" }));
  const requestId = client.readSessionDiff.mock.lastCall![1];
  const patch = "diff --git a/old.ts b/new.ts\nsimilarity index 100%\nrename from old.ts\nrename to new.ts\n";
  act(() => handlers.get("session-diff")?.forEach((handler) => handler({ session_id: "session-a", request_id: requestId, files: [{ path: "new.ts", old_path: "old.ts", status: "Renamed", layer: "Staged", added: 0, removed: 0, hunks: [], patch }] })));
  expect(screen.getByText(/renamed from old.ts/i)).toBeTruthy();
});
