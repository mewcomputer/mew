import { act, cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ToolCallCard } from "../components/tool-call-card";
import { useSessionStore, type MessagePart } from "../stores/session";

type ToolPart = Extract<MessagePart, { type: "tool-call" }>;

function toolPart(overrides: Partial<ToolPart> = {}): ToolPart {
  return { type: "tool-call", callId: "call-1", toolName: "read", input: { path: "/projects/mew/README.md" }, state: "pending", ...overrides };
}

beforeEach(() => useSessionStore.setState({ toolStates: new Map(), toolOutputs: new Map() }));
afterEach(() => { cleanup(); vi.restoreAllMocks(); });

describe("ToolCallCard", () => {
  it("names the disclosure and preserves the complete input summary for long commands", async () => {
    const command = "cargo test --package mew-agent --test agent_flow -- --nocapture --test-threads=1";
    const user = userEvent.setup();
    render(<ToolCallCard part={toolPart({ toolName: "shell", input: { command } })} />);
    const details = screen.getByRole("button", { name: "Show shell details" });
    expect(details.getAttribute("aria-expanded")).toBe("false");
    expect(screen.getByTitle(command).textContent).toBe(command);
    details.focus();
    await user.keyboard("{Enter}");
    expect(screen.getByRole("button", { name: "Hide shell details" }).getAttribute("aria-expanded")).toBe("true");
    expect(screen.getByText(JSON.stringify({ command }, null, 2), { normalizer: (text) => text })).toBeTruthy();
  });

  it("updates the visible tool state and output from the session store", () => {
    render(<ToolCallCard part={toolPart()} />);
    expect(screen.getByText("pending")).toBeTruthy();
    act(() => useSessionStore.setState({ toolStates: new Map([["call-1", "running"]]), toolOutputs: new Map([["call-1", "first line\nsecond line"]]) }));
    expect(screen.getByText("running")).toBeTruthy();
    expect(screen.getByText(/first line/).textContent).toBe("first line\nsecond line");
    act(() => useSessionStore.setState({ toolStates: new Map([["call-1", "completed"]]) }));
    expect(screen.getByText("completed")).toBeTruthy();
  });

  it("keeps failed output available in both collapsed and expanded views", async () => {
    const user = userEvent.setup();
    render(<ToolCallCard part={toolPart({ state: "error", output: "The file could not be read." })} />);
    expect(screen.getByText("error")).toBeTruthy();
    expect(screen.getByText("The file could not be read.")).toBeTruthy();
    await user.click(screen.getByRole("button", { name: "Show read details" }));
    expect(screen.getByText("Output")).toBeTruthy();
    expect(screen.getByText("The file could not be read.")).toBeTruthy();
  });

  it("copies the full output and announces success", async () => {
    const user = userEvent.setup();
    const output = "a long output line\nwith another line";
    render(<ToolCallCard part={toolPart({ state: "completed", output })} />);
    await user.click(screen.getByRole("button", { name: "Copy output" }));
    expect(await navigator.clipboard.readText()).toBe(output);
    expect(screen.getByRole("status").textContent).toBe("Output copied");
  });

  it("reports a clipboard failure and allows copying again", async () => {
    const user = userEvent.setup();
    const write = vi.spyOn(navigator.clipboard, "writeText").mockRejectedValueOnce(new Error("Clipboard unavailable"));
    render(<ToolCallCard part={toolPart({ output: "tool output" })} />);
    await user.click(screen.getByRole("button", { name: "Copy output" }));
    expect(screen.getByRole("alert").textContent).toContain("Could not copy output");
    write.mockRestore();
    await user.click(screen.getByRole("button", { name: "Copy output" }));
    expect(await navigator.clipboard.readText()).toBe("tool output");
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it.each([
    { toolName: "edit", input: { path: "/a/very/long/file/path.ts", old_string: "before", new_string: "after" }, text: "after" },
    { toolName: "write", input: { path: "/a/very/long/file/path.ts", text: "new contents" }, text: "new contents" },
  ])("preserves the $toolName input view", async ({ toolName, input, text }) => {
    const user = userEvent.setup();
    render(<ToolCallCard part={toolPart({ toolName, input, state: "completed" })} />);
    await user.click(screen.getByRole("button", { name: `Show ${toolName} details` }));
    expect(screen.getAllByText("/a/very/long/file/path.ts").length).toBeGreaterThan(0);
    expect(screen.getByText(text)).toBeTruthy();
  });
});

it("shows a Python cell and its nested call status and details", async () => {
  const user = userEvent.setup();
  render(<ToolCallCard part={toolPart({
    toolName: "python", input: { source: "text(tools.read({'path': 'notes.txt'}))" }, state: "completed",
    nestedCalls: [{ parent_call_id: "call-1", call_id: "nested-1", tool_name: "read", input: { path: "notes.txt" }, status: "error", error: "permission denied" }],
  })} />);
  expect(screen.getByText("text(tools.read({'path': 'notes.txt'}))")).toBeTruthy();
  expect(screen.getByText("read")).toBeTruthy();
  expect(screen.getByText("notes.txt")).toBeTruthy();
  expect(screen.getByText("permission denied")).toBeTruthy();
  await user.click(screen.getByRole("button", { name: "Show python details" }));
  expect(screen.getByText(JSON.stringify({ path: "notes.txt" }, null, 2), { normalizer: (text) => text })).toBeTruthy();
});
