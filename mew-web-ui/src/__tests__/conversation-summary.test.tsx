import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { ConversationSummary } from "../components/conversation-summary";
import { useSessionStore } from "../stores/session";

beforeEach(() => {
  useSessionStore.setState({
    sessionId: "chat-a",
    sessionCwd: "/code/mew",
    availableSessions: [],
    gitStatus: [{ path: "src/main.rs", status: "modified" }],
    subagents: new Map([
      ["build", { parentCallId: "build", name: "builder", childSessionId: "child-a", displayName: "Build the UI", status: "running", lastProgress: null, outcome: null, manifests: [] }],
      ["review", { parentCallId: "review", name: "reviewer", childSessionId: "child-b", displayName: "Review", status: "completed", lastProgress: null, outcome: null, manifests: [] }],
    ]),
    jobs: new Map([["job-a", { jobId: "job-a", command: "pnpm build", state: "running" }]]),
    flaggedFiles: [{ path: "notes/brief.md" }],
  });
});

afterEach(() => { cleanup(); });

describe("conversation summary", () => {
  it("shows confirmed change totals for the selected chat", () => {
    useSessionStore.setState({ availableSessions: [{ session_id: "chat-a", state: "idle", created_at: 1000, client_count: 0, change_stats: { added: 23, removed: 7, files: ["src/main.rs"] } }] });
    render(<ConversationSummary onOpenSurface={() => {}} />);
    fireEvent.click(screen.getByRole("button", { name: "Open summary" }));
    expect(screen.getByText("+23")).toBeTruthy();
    expect(screen.getByText("-7")).toBeTruthy();
  });

  it("opens current-chat activity from the corner control without opening the workbench", () => {
    const onOpenSurface = vi.fn();
    render(<ConversationSummary onOpenSurface={onOpenSurface} />);
    expect(screen.queryByRole("dialog", { name: "Conversation summary" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Open summary" }));
    const summary = screen.getByRole("dialog", { name: "Conversation summary" });
    expect(summary.hasAttribute("data-mew-overlay")).toBe(true);
    expect(within(summary).getByText("mew")).toBeTruthy();
    expect(within(summary).getByRole("button", { name: /Changes.*1 file/ })).toBeTruthy();
    expect(within(summary).getByRole("button", { name: /Subagents.*1 working.*1 done/ })).toBeTruthy();
    expect(within(summary).getByRole("button", { name: /pnpm build.*running/ })).toBeTruthy();
    expect(within(summary).getByText("brief.md")).toBeTruthy();
    expect(onOpenSurface).not.toHaveBeenCalled();
  });

  it.each([
    [/Changes.*1 file/, "changes"],
    [/Subagents.*1 working/, "agents"],
    [/pnpm build.*running/, "jobs"],
    [/View all sources/, "file"],
  ] as const)("opens the selected detail surface and dismisses the summary", (name, kind) => {
    const onOpenSurface = vi.fn();
    render(<ConversationSummary onOpenSurface={onOpenSurface} />);
    fireEvent.click(screen.getByRole("button", { name: "Open summary" }));
    fireEvent.click(screen.getByRole("button", { name }));
    expect(onOpenSurface).toHaveBeenCalledWith(kind);
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("updates for the selected chat and represents empty activity without stale counts", () => {
    render(<ConversationSummary onOpenSurface={() => {}} />);
    fireEvent.click(screen.getByRole("button", { name: "Open summary" }));
    act(() => {
      useSessionStore.getState().reset();
      useSessionStore.setState({ sessionId: "chat-b", sessionCwd: "/code/other" });
    });
    expect(screen.getByText("other")).toBeTruthy();
    expect(screen.queryByRole("region", { name: "Todos" })).toBeNull();
    expect(screen.queryByRole("region", { name: "Subagents" })).toBeNull();
    expect(screen.queryByRole("region", { name: "Background processes" })).toBeNull();
    expect(screen.queryByRole("region", { name: "Sources" })).toBeNull();
    expect(screen.queryByText("brief.md")).toBeNull();
    expect(screen.queryByText("pnpm build")).toBeNull();
  });

describe("conversation summary todos", () => {
  it("lists the agent's todos with progress and status", () => {
    useSessionStore.setState({
      todos: [
        { id: 1, content: "Read the scanner config", status: "done", dependsOn: [] },
        { id: 2, content: "Add the fade gradient", status: "in_progress", dependsOn: [1] },
        { id: 3, content: "Verify in the app", status: "pending", dependsOn: [2] },
        { id: 4, content: "Ship it", status: "blocked", dependsOn: [3] },
      ],
    });
    render(<ConversationSummary onOpenSurface={() => {}} />);
    fireEvent.click(screen.getByRole("button", { name: "Open summary" }));
    const summary = screen.getByRole("dialog", { name: "Conversation summary" });
    const section = within(summary).getByRole("region", { name: "Todos" });
    expect(within(section).getByText("3 open · 1 done")).toBeTruthy();
    expect(within(section).getByText("Read the scanner config")).toBeTruthy();
    expect(within(section).getByText("Add the fade gradient")).toBeTruthy();
    expect(within(section).getByText("Verify in the app")).toBeTruthy();
    expect(within(section).getByText("Ship it")).toBeTruthy();
  });

  it("omits the todos section when the agent has not written a plan", () => {
    useSessionStore.setState({ todos: [] });
    render(<ConversationSummary onOpenSurface={() => {}} />);
    fireEvent.click(screen.getByRole("button", { name: "Open summary" }));
    const summary = screen.getByRole("dialog", { name: "Conversation summary" });
    expect(within(summary).queryByRole("region", { name: "Todos" })).toBeNull();
    expect(within(summary).getByRole("button", { name: /Changes.*1 file/ })).toBeTruthy();
  });

  it("caps a long list so the panel stays scannable", () => {
    useSessionStore.setState({
      todos: Array.from({ length: 9 }, (_, index) => ({ id: index, content: `Step ${index}`, status: "pending" as const, dependsOn: [] })),
    });
    render(<ConversationSummary onOpenSurface={() => {}} />);
    fireEvent.click(screen.getByRole("button", { name: "Open summary" }));
    const section = within(screen.getByRole("dialog", { name: "Conversation summary" })).getByRole("region", { name: "Todos" });
    expect(within(section).getByText("+4 earlier")).toBeTruthy();
    expect(section.textContent).not.toContain("Step 8");
  });
});

  it("dismisses with Escape and returns focus to its trigger", async () => {
    const user = userEvent.setup();
    render(<ConversationSummary onOpenSurface={() => {}} />);
    const trigger = screen.getByRole("button", { name: "Open summary" });
    await user.click(trigger);
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(document.activeElement).toBe(trigger);
  });

  it("shows a hover tooltip with the current action", async () => {
    const user = userEvent.setup();
    render(<ConversationSummary onOpenSurface={() => {}} />);
    await user.hover(screen.getByRole("button", { name: "Open summary" }));
    expect((await screen.findByRole("tooltip")).textContent).toContain("Open summary");
  });
});
