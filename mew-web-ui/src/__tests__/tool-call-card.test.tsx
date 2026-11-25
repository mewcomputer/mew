import { act, cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ToolCallCard } from "../components/tool-call-card";
import { useSessionStore, type MessagePart } from "../stores/session";

type ToolPart = Extract<MessagePart, { type: "tool-call" }>;

function toolPart(overrides: Partial<ToolPart> = {}): ToolPart {
  return { type: "tool-call", callId: "call-1", toolName: "read", input: { path: "/projects/mew/README.md" }, state: "pending", ...overrides };
}

beforeEach(() => useSessionStore.setState({ toolStates: new Map(), toolOutputs: new Map(), sessionCwd: null }));
afterEach(() => { cleanup(); vi.restoreAllMocks(); });

describe("ToolCallCard", () => {
  it("states what the call did and keeps the complete command reachable from the row", async () => {
    const command = "cargo test --package mew-agent --test agent_flow -- --nocapture --test-threads=1";
    const user = userEvent.setup();
    render(<ToolCallCard part={toolPart({ toolName: "bash", input: { command } })} />);
    const details = screen.getByRole("button", { name: "Show bash details" });
    expect(details.getAttribute("aria-expanded")).toBe("false");
    expect(details.textContent).toContain("Ran");
    expect(screen.getByTitle(command).textContent).toBe(command);
    details.focus();
    await user.keyboard("{Enter}");
    expect(screen.getByRole("button", { name: "Hide bash details" }).getAttribute("aria-expanded")).toBe("true");
    expect(screen.getByText(JSON.stringify({ command }, null, 2), { normalizer: (text) => text })).toBeTruthy();
  });

  it("folds a finished call into one line", async () => {
    const user = userEvent.setup();
    render(<ToolCallCard part={toolPart({ state: "completed", output: "three\nlines\nof output" })} />);
    expect(screen.getByRole("button", { name: "Show read details" }).textContent).toContain("Read");
    expect(screen.queryByText(/three/)).toBeNull();
    await user.click(screen.getByRole("button", { name: "Show read details" }));
    expect(screen.getByText("Output")).toBeTruthy();
    expect(screen.getByText(/three/)).toBeTruthy();
  });

  it("drops the workspace prefix from paths", () => {
    useSessionStore.setState({ sessionCwd: "/Users/natalie/code/mew" });
    render(<ToolCallCard part={toolPart({ input: { path: "/Users/natalie/code/mew/src/app.tsx" } })} />);
    const row = screen.getByRole("button", { name: "Show read details" });
    expect(row.textContent).toContain("src/app.tsx");
    expect(screen.getByTitle("/Users/natalie/code/mew/src/app.tsx")).toBeTruthy();
  });

  it("shortens paths outside the workspace and keeps short ones whole", () => {
    render(<ToolCallCard part={toolPart({ input: { path: "/etc/hosts" } })} />);
    expect(screen.getByRole("button", { name: "Show read details" }).textContent).toContain("/etc/hosts");
    cleanup();
    render(<ToolCallCard part={toolPart({ input: { path: "/Users/natalie/code/mew/CURRENT.md" } })} />);
    const row = screen.getByRole("button", { name: "Show read details" });
    expect(row.textContent).toContain("…/mew/CURRENT.md");
    expect(screen.getByTitle("/Users/natalie/code/mew/CURRENT.md")).toBeTruthy();
  });

  it("hides the duration of a historical call that never recorded an end time", () => {
    render(<ToolCallCard part={toolPart({ state: "completed", time: { start: 1, end: null }, output: "done" })} />);
    const row = screen.getByRole("button", { name: "Show read details" });
    expect(row.textContent).not.toMatch(/\ds/);
    cleanup();
    render(<ToolCallCard part={toolPart({ state: "completed", time: { start: 1000, end: 2500 }, output: "done" })} />);
    expect(screen.getByRole("button", { name: "Show read details" }).textContent).toContain("1.5s");
  });

  it("summarizes an edit with its line delta", () => {
    render(<ToolCallCard part={toolPart({
      toolName: "edit",
      input: { path: "/projects/mew/README.md", old_string: "alpha\nbeta", new_string: "alpha\ngamma" },
      state: "completed",
    })} />);
    const row = screen.getByRole("button", { name: "Show edit details" });
    expect(row.textContent).toContain("Edited");
    expect(row.textContent).toContain("README.md");
    expect(row.textContent).toContain("+1");
    expect(row.textContent).toContain("-1");
  });

  it("reads the target file from a hash-anchored patch and shows no line delta", () => {
    render(<ToolCallCard part={toolPart({
      toolName: "edit_hashline",
      input: { patch: "[src/app.tsx#A1B2]\nSWAP 5.=5:\n+fn updated() {}" },
      state: "completed",
    })} />);
    const row = screen.getByRole("button", { name: "Show edit_hashline details" });
    expect(row.textContent).toContain("Edited");
    expect(row.textContent).toContain("src/app.tsx");
    expect(row.textContent).not.toContain("+0");
    expect(row.textContent).not.toContain("-0");
  });

  it("keeps running output collapsed by default and reveals it when expanded", async () => {
    const user = userEvent.setup();
    render(<ToolCallCard part={toolPart()} />);
    expect(screen.getByText("pending")).toBeTruthy();
    act(() => useSessionStore.setState({ toolStates: new Map([["call-1", "running"]]), toolOutputs: new Map([["call-1", "first line\nsecond line"]]) }));
    expect(screen.getByText("running")).toBeTruthy();
    expect(screen.queryByText(/first line/)).toBeNull();
    await user.click(screen.getByRole("button", { name: "Show read details" }));
    expect(screen.getByText(/first line/).textContent).toBe("first line\nsecond line");
    act(() => useSessionStore.setState({ toolStates: new Map([["call-1", "completed"]]) }));
    expect(screen.getByText("completed")).toBeTruthy();
  });

  it("keeps failed output collapsed until manually expanded", async () => {
    const user = userEvent.setup();
    render(<ToolCallCard part={toolPart({ state: "error", output: "The file could not be read." })} />);
    expect(screen.getByText("error")).toBeTruthy();
    expect(screen.queryByText("The file could not be read.")).toBeNull();
    await user.click(screen.getByRole("button", { name: "Show read details" }));
    expect(screen.getByText("Output")).toBeTruthy();
    expect(screen.getByText("The file could not be read.")).toBeTruthy();
  });

  it("copies the full output and announces success", async () => {
    const user = userEvent.setup();
    const output = "a long output line\nwith another line";
    render(<ToolCallCard part={toolPart({ state: "completed", output })} />);
    await user.click(screen.getByRole("button", { name: "Show read details" }));
    await user.click(screen.getByRole("button", { name: "Copy output" }));
    expect(await navigator.clipboard.readText()).toBe(output);
    expect(screen.getByRole("status").textContent).toBe("Output copied");
  });

  it("reports a clipboard failure and allows copying again", async () => {
    const user = userEvent.setup();
    const write = vi.spyOn(navigator.clipboard, "writeText").mockRejectedValueOnce(new Error("Clipboard unavailable"));
    render(<ToolCallCard part={toolPart({ output: "tool output" })} />);
    await user.click(screen.getByRole("button", { name: "Show read details" }));
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

it("keeps a running Python cell folded and names what it will call", () => {
  render(<ToolCallCard part={toolPart({
    toolName: "python",
    input: { source: "r = tools.grep({'pattern': 'desktop-electron'})\nprint(r)" },
    state: "running",
    nestedCalls: [],
  })} />);
  const row = screen.getByRole("button", { name: "Show python details" });
  expect(row.textContent).toContain("grep desktop-electron");
  expect(screen.queryByLabelText("Python code")).toBeNull();
  expect(screen.queryByLabelText("Nested tools")).toBeNull();
});

it("shows a Python cell and its nested call status and details", async () => {
  const user = userEvent.setup();
  render(<ToolCallCard part={toolPart({
    toolName: "python", input: { source: "text(tools.read({'path': 'notes.txt'}))" }, state: "completed",
    nestedCalls: [{ parent_call_id: "call-1", call_id: "nested-1", tool_name: "read", input: { path: "notes.txt" }, status: "error", error: "permission denied" }],
  })} />);
  expect(screen.getByRole("button", { name: "Show python details" }).textContent).toContain("read");
  await user.click(screen.getByRole("button", { name: "Show python details" }));
  expect(screen.getByLabelText("Python code").textContent).toContain("text(tools.read({'path': 'notes.txt'}))");
  expect(screen.getByLabelText("Nested tools").textContent).toContain("read");
  expect(screen.getByText("notes.txt")).toBeTruthy();
  expect(screen.getByText("permission denied")).toBeTruthy();
  // The nested call's input is already visible in the Python source above it.
  expect(screen.getByText("error").className).toContain("text-destructive");
});

it("names multiple executed tools on one row and retains Python source for inspection", async () => {
  const user = userEvent.setup();
  const source = "value = tools.read({'path': 'a'})\nprint(value)";
  render(<ToolCallCard part={toolPart({
    toolName: "python", input: { source }, state: "completed",
    nestedCalls: [
      { parent_call_id: "call-1", call_id: "n1", tool_name: "read", input: {}, status: "completed" },
      { parent_call_id: "call-1", call_id: "n2", tool_name: "write", input: {}, status: "completed" },
    ],
  })} />);
  const row = screen.getByRole("button", { name: "Show python details" });
  expect(row.textContent).toContain("Python");
  expect(row.textContent).toContain("read, write");
  expect(screen.queryByText("Mutating")).toBeNull();
  await user.click(row);
  expect(screen.getByLabelText("Python code").textContent).toContain(source);
});

it("keeps the original Python available when display indentation changes", async () => {
  const user = userEvent.setup();
  const source = "if ready:\n    print('hello')";
  render(<ToolCallCard part={toolPart({ toolName: "python", input: { source } })} />);
  await user.click(screen.getByRole("button", { name: "Show python details" }));
  expect(screen.getByLabelText("Python code").textContent).toContain("if ready:\n  print('hello')");
  expect(screen.getByText("Original Python").nextElementSibling?.textContent).toBe(source);
});

it("shows description on the collapsed row when provided and preserves underlying details", async () => {
  const user = userEvent.setup();
  const command = "cargo test -p mew-tools";
  render(<ToolCallCard part={toolPart({
    toolName: "bash",
    input: { command, description: "Run mew-tools tests" },
    state: "completed",
  })} />);
  const row = screen.getByRole("button", { name: "Show bash details" });
  expect(row.textContent).toContain("Run mew-tools tests");
  expect(row.textContent).not.toContain("Ran");
  expect(screen.getByTitle(command)).toBeTruthy();
  await user.click(row);
  expect(screen.getByText(JSON.stringify({ command, description: "Run mew-tools tests" }, null, 2), { normalizer: (text) => text })).toBeTruthy();
  cleanup();

  render(<ToolCallCard part={toolPart({
    toolName: "web_fetch",
    input: { url: "https://example.com/docs", description: "Fetch API docs" },
    state: "completed",
  })} />);
  const fetchRow = screen.getByRole("button", { name: "Show web_fetch details" });
  expect(fetchRow.textContent).toContain("Fetch API docs");
  expect(fetchRow.textContent).not.toContain("Fetched");
  expect(screen.getByTitle("https://example.com/docs")).toBeTruthy();
  cleanup();

  render(<ToolCallCard part={toolPart({
    toolName: "python",
    input: { source: "text(tools.read({'path': 'a'}))", description: "Inspect file header" },
    state: "completed",
  })} />);
  const pyRow = screen.getByRole("button", { name: "Show python details" });
  expect(pyRow.textContent).toContain("Inspect file header");
  expect(pyRow.textContent).not.toContain("Python");
  expect(screen.getByTitle("read a")).toBeTruthy();
});

it("falls back to reason or verb/detail when description is omitted", () => {
  render(<ToolCallCard part={toolPart({
    toolName: "bash",
    input: { command: "pwd", reason: "Check working directory" },
  })} />);
  expect(screen.getByRole("button", { name: "Show bash details" }).textContent).toContain("Check working directory");
  cleanup();

  render(<ToolCallCard part={toolPart({
    toolName: "bash",
    input: { command: "pwd" },
  })} />);
  const row = screen.getByRole("button", { name: "Show bash details" });
  expect(row.textContent).toContain("Ran");
  expect(row.textContent).toContain("pwd");
});
