import { act, cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { CompactingIndicator, TurnItem } from "../components/turn-item";
import { useSessionStore, type ChatMessage } from "../stores/session";
import type { Turn } from "../lib/turns";

function turn(messages: ChatMessage[]): Turn {
  return { key: messages[0]!.id, messageIndex: 0, messages };
}

const user = (text: string, timestamp = 0): ChatMessage => ({
  id: `user-${timestamp}`, role: "user", timestamp, parts: [{ type: "text", text }],
});

beforeEach(() => useSessionStore.setState({
  toolStates: new Map(),
  toolOutputs: new Map(),
  sessionCwd: null,
  confirmedSessionId: null,
  sessionActivity: null,
  availableSessions: [],
  compacting: false,
  compactingStartedAt: null,
  retryWait: null,
  pendingPermissions: [],
  pendingAskUser: [],
  subagents: new Map(),
}));
afterEach(() => cleanup());

describe("TurnItem", () => {
  it("keeps a completed turn collapsed while a different turn runs a tool", () => {
    useSessionStore.setState({ toolStates: new Map([["other", "running"]]) });
    render(<TurnItem turn={turn([
      user("previous question", 0),
      { id: "previous-work", role: "assistant", timestamp: 1000, completedAt: 2000, parts: [
        { type: "tool-call", toolName: "read", callId: "previous", input: {}, state: "completed" },
      ] },
      { id: "previous-answer", role: "assistant", timestamp: 3000, completedAt: 4000, parts: [
        { type: "text", text: "Previous answer" },
      ] },
    ])} />);

    expect(screen.getByRole("button", { name: "Show work" })).toBeTruthy();
    expect(screen.queryByText(/Working for/)).toBeNull();
    expect(screen.queryByRole("button", { name: "Show read details" })).toBeNull();
  });

  it("folds the work of a settled turn behind one duration line", async () => {
    const session = user("does it build?", 0);
    const work: ChatMessage = {
      id: "round-1", role: "assistant", timestamp: 1000, completedAt: 4000,
      parts: [
        { type: "reasoning", text: "check the justfile" },
        { type: "text", text: "Let me look at the build recipes." },
        { type: "tool-call", toolName: "read", callId: "c1", input: { path: "/repo/AGENTS.md" }, state: "completed" },
      ],
    };
    const answer: ChatMessage = {
      id: "round-2", role: "assistant", timestamp: 5000, completedAt: 65000,
      parts: [{ type: "text", text: "It builds once you run the UI build first." }],
    };

    render(<TurnItem turn={turn([session, work, answer])} />);

    const header = screen.getByRole("button", { name: "Show work" });
    expect(header.textContent).toBe("Worked for 1m 5s");
    expect(screen.getByText("It builds once you run the UI build first.")).toBeTruthy();
    expect(screen.queryByText("Let me look at the build recipes.")).toBeNull();
    expect(screen.queryByText("check the justfile")).toBeNull();

    await userEvent.setup().click(header);
    expect(screen.getByRole("button", { name: "Hide work" })).toBeTruthy();
    expect(screen.getByText("Let me look at the build recipes.")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Show thinking" }).textContent).toContain("check the justfile");
    await userEvent.setup().click(screen.getByRole("button", { name: "Show thinking" }));
    expect(screen.getByText("check the justfile")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Show read details" })).toBeTruthy();
  });

  it("keeps a turn that is still running collapsed by step group, labelled as in progress", async () => {
    const userClient = userEvent.setup();
    useSessionStore.setState({ streamingReasoningText: "check the justfile" });
    render(<TurnItem turn={turn([
      user("does it build?", 0),
      {
        id: "round-1", role: "assistant", timestamp: 1000,
        parts: [
          { type: "reasoning", text: "check the justfile", streaming: true },
          { type: "tool-call", toolName: "read", callId: "c1", input: { path: "/repo/AGENTS.md" }, state: "running" },
        ],
      },
    ])} />);

    // In-flight turns have nothing to fold yet, so the line is not a disclosure.
    expect(screen.queryByRole("button", { name: /work/i })).toBeNull();
    expect(document.body.textContent).toContain("Working for");
    const groupButton = screen.getByRole("button", { name: "Show 1 thought, 1 tool call" });
    expect(screen.queryByRole("button", { name: "Show thinking" })).toBeNull();
    await userClient.click(groupButton);
    expect(screen.getByRole("button", { name: "Show thinking" }).textContent).toContain("check the justfile");
    expect(screen.getByRole("button", { name: "Show read details" })).toBeTruthy();
  });

  it("keeps expanded thinking visible as streaming completes", async () => {
    useSessionStore.setState({ streamingReasoningText: "First line\nMore detail" });
    const message: ChatMessage = {
      id: "round-1", role: "assistant", timestamp: 1000,
      parts: [{ type: "reasoning", text: "", streaming: true }],
    };
    const { rerender } = render(<TurnItem turn={turn([user("question"), message])} />);
    await userEvent.setup().click(screen.getByRole("button", { name: "Show 1 thought" }));
    expect(screen.getByRole("button", { name: "Show thinking" }).textContent).toContain("First line");
    expect(screen.queryByText(/More detail/)).toBeNull();
    await userEvent.setup().click(screen.getByRole("button", { name: "Show thinking" }));
    expect(screen.getByText(/More detail/)).toBeTruthy();
    expect(screen.getByRole("button", { name: "Hide thinking" }).textContent).not.toContain("First line");

    rerender(<TurnItem turn={turn([user("question"), {
      ...message, parts: [{ type: "reasoning", text: "First line\nMore detail" }],
    }])} />);
    expect(screen.getByRole("button", { name: "Hide thinking" }).textContent).toContain("Thought");
    expect(screen.getByText(/More detail/)).toBeTruthy();
    await userEvent.setup().click(screen.getByRole("button", { name: "Hide thinking" }));
    expect(screen.getByRole("button", { name: "Show thinking" }).textContent).toContain("First line");
    expect(screen.queryByText(/More detail/)).toBeNull();
  });

  it("keeps finished thinking accessible while the turn is still running", async () => {
    render(<TurnItem turn={turn([
      user("does it build?", 0),
      {
        id: "round-1", role: "assistant", timestamp: 1000,
        parts: [
          { type: "reasoning", text: "older thinking\nthat already finished" },
          { type: "tool-call", toolName: "read", callId: "c1", input: { path: "/repo/AGENTS.md" }, state: "running" },
        ],
      },
    ])} />);

    expect(document.body.textContent).toContain("Working for");
    await userEvent.setup().click(screen.getByRole("button", { name: "Show 1 thought, 1 tool call" }));
    const disclosure = screen.getByRole("button", { name: "Show thinking" });
    expect(disclosure.textContent).toContain("older thinking");
    expect(screen.queryByText(/that already finished/)).toBeNull();
    await userEvent.setup().click(disclosure);
    expect(screen.getByText(/that already finished/)).toBeTruthy();
    expect(screen.getByRole("button", { name: "Show read details" })).toBeTruthy();
  });

  it("stays open when the turn produced no answer to read", async () => {
    render(<TurnItem turn={turn([
      user("does it build?", 0),
      {
        id: "round-1", role: "assistant", timestamp: 1000, completedAt: 3000,
        parts: [{ type: "tool-call", toolName: "read", callId: "c1", input: { path: "/repo/AGENTS.md" }, state: "completed" }],
      },
    ])} />);

    expect(screen.queryByRole("button", { name: /work/i })).toBeNull();
    expect(document.body.textContent).toContain("Worked for 3s");
    expect(screen.getByRole("button", { name: "Show read details" })).toBeTruthy();
  });

  it("reports failed tools on the collapsed line instead of unfolding the turn", () => {
    render(<TurnItem turn={turn([
      user("does it build?", 0),
      {
        id: "round-1", role: "assistant", timestamp: 1000, completedAt: 3000,
        parts: [{ type: "tool-call", toolName: "bash", callId: "c1", input: { command: "just build-ui" }, state: "error" }],
      },
      { id: "round-2", role: "assistant", timestamp: 4000, completedAt: 5000, parts: [{ type: "text", text: "The build failed." }] },
    ])} />);

    const header = screen.getByRole("button", { name: "Show work" });
    expect(header.textContent).toContain("Worked for");
    expect(header.textContent).toContain("1 failed");
    expect(screen.getByText("The build failed.")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Show bash details" })).toBeNull();
  });

  it("renders a plain answer without a work line", () => {
    render(<TurnItem turn={turn([
      user("hey", 0),
      { id: "round-1", role: "assistant", timestamp: 1000, completedAt: 2000, parts: [{ type: "text", text: "hey!" }] },
    ])} />);

    expect(screen.getByText("hey!")).toBeTruthy();
    expect(screen.queryByRole("button", { name: /work/i })).toBeNull();
  });

  it("keeps the latest turn working between rounds while the session is running", () => {
    useSessionStore.setState({
      confirmedSessionId: "sess-1",
      availableSessions: [{ session_id: "sess-1", state: "running" } as never],
    });
    const completedRoundTurn = turn([
      user("check the repo", 0),
      {
        id: "round-1",
        role: "assistant",
        timestamp: 1000,
        completedAt: 3000,
        parts: [{ type: "tool-call", toolName: "read", callId: "c1", input: { path: "README.md" }, state: "completed" }],
      },
      { id: "round-2", role: "assistant", timestamp: 3500, completedAt: 4000, parts: [{ type: "text", text: "Intermediate note" }] },
    ]);

    const { rerender } = render(<TurnItem turn={completedRoundTurn} isLast />);
    expect(document.body.textContent).toContain("Working for");
    expect(screen.queryByRole("button", { name: "Show work" })).toBeNull();

    rerender(<TurnItem turn={completedRoundTurn} isLast={false} />);
    expect(screen.getByRole("button", { name: "Show work" }).textContent).toContain("Worked for");
  });

  it("keeps intermediate prose visible while folding step groups live, then collapses all under Worked for when settled", async () => {
    const workMsg: ChatMessage = {
      id: "round-1",
      role: "assistant",
      timestamp: 1000,
      completedAt: 4000,
      parts: [
        { type: "reasoning", text: "check paths" },
        { type: "text", text: "Checking where absolute paths are rejected." },
        { type: "tool-call", toolName: "read", callId: "c1", input: { path: "files.rs" }, state: "completed" },
      ],
    };

    const { rerender } = render(<TurnItem turn={turn([
      user("allow absolute paths", 0),
      workMsg,
      {
        id: "round-2",
        role: "assistant",
        timestamp: 5000,
        parts: [{
          type: "tool-call",
          toolName: "bash",
          callId: "c2",
          input: { command: "cargo test", description: "Run daemon tests" },
          state: "running",
        }],
      },
    ])} isLast />);

    expect(document.body.textContent).toContain("Working for");
    expect(document.body.textContent).toContain("Run daemon tests");
    expect(screen.getByText("Checking where absolute paths are rejected.")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Show 1 thought" })).toBeNull();
    expect(screen.getByRole("button", { name: "Show 2 tool calls" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Show read details" })).toBeNull();

    rerender(<TurnItem turn={turn([
      user("allow absolute paths", 0),
      workMsg,
      {
        id: "round-2",
        role: "assistant",
        timestamp: 5000,
        completedAt: 8000,
        parts: [{ type: "tool-call", toolName: "bash", callId: "c2", input: { command: "cargo test" }, state: "completed" }],
      },
      {
        id: "round-3",
        role: "assistant",
        timestamp: 9000,
        completedAt: 10000,
        parts: [{ type: "text", text: "Allowed scoped absolute paths." }],
      },
    ])} isLast />);

    expect(screen.getByRole("button", { name: "Show work" }).textContent).toContain("Worked for");
    expect(screen.queryByText("Checking where absolute paths are rejected.")).toBeNull();
    expect(screen.getByText("Allowed scoped absolute paths.")).toBeTruthy();
  });

  it("transitions cleanly from a user-only turn to a live turn with tool calls", () => {
    const prompt = user("check this", 0);
    const { rerender } = render(<TurnItem turn={turn([prompt])} isLast />);
    expect(screen.getByText("check this")).toBeTruthy();

    rerender(<TurnItem turn={turn([
      prompt,
      {
        id: "round-1",
        role: "assistant",
        timestamp: 1000,
        parts: [{ type: "tool-call", toolName: "read", callId: "c1", input: { path: "README.md" }, state: "running" }],
      },
    ])} isLast />);

    expect(document.body.textContent).toContain("Working for");
    expect(screen.getByRole("button", { name: "Show 1 tool call" })).toBeTruthy();
  });

  it("never renders Working for on a non-last turn even if an assistant message lacks completedAt", () => {
    render(<TurnItem turn={turn([
      user("first prompt", 0),
      {
        id: "work-1",
        role: "assistant",
        timestamp: 1000,
        completedAt: 2000,
        parts: [{ type: "reasoning", text: "checking" }],
      },
      {
        id: "err-1",
        role: "assistant",
        timestamp: 3000,
        parts: [{ type: "error", message: "SSE stream ended before the response was complete" }],
      },
    ])} isLast={false} />);

    expect(screen.queryByText(/Working for/)).toBeNull();
    expect(document.body.textContent).toContain("Worked for");
    expect(screen.getByText("SSE stream ended before the response was complete")).toBeTruthy();
  });

  it("shows Working for on the latest user prompt while session is running before any assistant reply arrives", () => {
    useSessionStore.setState({
      confirmedSessionId: "sess-1",
      availableSessions: [{ session_id: "sess-1", state: "running" } as never],
    });
    render(<TurnItem turn={turn([user("uhh test test", 0)])} isLast />);

    expect(screen.getByText("uhh test test")).toBeTruthy();
    expect(document.body.textContent).toContain("Working for");
  });

  it("stays live between tool rounds via sessionActivity even when availableSessions is empty", () => {
    useSessionStore.setState({
      sessionId: "sess-new",
      confirmedSessionId: "sess-new",
      sessionActivity: "running",
      availableSessions: [],
    });
    render(
      <TurnItem
        turn={turn([
          user("inspect workspace", 0),
          {
            id: "round-1",
            role: "assistant",
            timestamp: 1000,
            completedAt: 2000,
            parts: [
              { type: "tool-call", toolName: "read", callId: "c1", input: { path: "README.md" }, state: "completed" },
            ],
          },
        ])}
        isLast
      />,
    );

    expect(document.body.textContent).toContain("Working for");
  });

  it("prioritizes compacting, retryWait, approval, ask-user, and subagent status on the live row", () => {
    useSessionStore.setState({
      confirmedSessionId: "sess-1",
      availableSessions: [{ session_id: "sess-1", state: "running" } as never],
      compacting: true,
      retryWait: { attempt: 1, maxAttempts: 3, delaySecs: 4, reason: "overloaded" },
      pendingPermissions: [{ requestId: "perm-1", toolName: "bash", input: {} }],
      pendingAskUser: [{ requestId: "ask-1", callId: "call-1", questions: [] }],
      subagents: new Map([
        [
          "call-sub",
          {
            parentCallId: "call-sub",
            name: "research",
            childSessionId: "child-1",
            displayName: "Researcher",
            status: "running",
            lastProgress: "Scanning crate docs",
            outcome: null,
            manifests: [],
          },
        ],
      ]),
    });

    render(<TurnItem turn={turn([user("check status", 0)])} isLast />);
    expect(document.body.textContent).toContain("Compacting context…");

    act(() => useSessionStore.setState({ compacting: false }));
    expect(document.body.textContent).toContain("Retrying (1/3) in 4s — overloaded");

    act(() => useSessionStore.setState({ retryWait: null }));
    expect(document.body.textContent).toContain("Waiting for approval");

    act(() => useSessionStore.setState({ pendingPermissions: [] }));
    expect(document.body.textContent).toContain("Waiting for answer");

    act(() => useSessionStore.setState({ pendingAskUser: [] }));
    expect(document.body.textContent).toContain("Researcher: Scanning crate docs");
  });

  it("keeps a settled turn collapsed when manual /compact runs while idle and renders CompactingIndicator cleanly", () => {
    const oldStart = Date.now() - 35 * 60 * 60 * 1000;
    useSessionStore.setState({
      confirmedSessionId: "sess-1",
      availableSessions: [{ session_id: "sess-1", state: "idle" } as never],
      compacting: true,
      compactingStartedAt: Date.now(),
    });

    render(
      <>
        <TurnItem
          turn={turn([
            user("old task", oldStart),
            {
              id: "old-work",
              role: "assistant",
              timestamp: oldStart + 1000,
              completedAt: oldStart + 2000,
              parts: [
                { type: "tool-call", toolName: "bash", callId: "c-fail-1", input: {}, state: "error" },
              ],
            },
            {
              id: "old-answer",
              role: "assistant",
              timestamp: oldStart + 3000,
              completedAt: oldStart + 4000,
              parts: [{ type: "text", text: "Done earlier" }],
            },
          ])}
          isLast
        />
        <CompactingIndicator startedAt={Date.now()} />
      </>,
    );

    expect(screen.getByRole("button", { name: "Show work" })).toBeTruthy();
    expect(screen.queryByText(/Working for/)).toBeNull();
    expect(document.body.textContent).toContain("Compacting context… 0s");
  });

  it("renders steered user tags, cancelled turn state, and expandable compaction dividers", async () => {
    const userAct = userEvent.setup();
    render(
      <TurnItem
        turn={turn([
          { ...user("redirect focus", 0), steered: true },
          {
            id: "work-comp",
            role: "assistant",
            timestamp: 1000,
            completedAt: 2000,
            parts: [{ type: "tool-call", toolName: "read", callId: "c-1", input: {}, state: "completed" }],
          },
          {
            id: "comp-1",
            role: "system",
            timestamp: 2500,
            completedAt: 2500,
            parts: [{ type: "compaction", summary: "Dropped earlier messages cleanly", removed: 12 }],
          },
          {
            id: "ans-comp",
            role: "assistant",
            timestamp: 3000,
            completedAt: 4000,
            cancelled: true,
            parts: [{ type: "text", text: "Partial response" }],
          },
        ])}
        isLast
      />,
    );

    expect(screen.getByText("Steered")).toBeTruthy();
    expect(screen.getByText("Cancelled")).toBeTruthy();
    expect(screen.getByText("Context compacted · 12 messages")).toBeTruthy();
    expect(screen.queryByText("Dropped earlier messages cleanly")).toBeNull();

    await userAct.click(screen.getByRole("button", { name: "Show compaction summary" }));
    expect(screen.getByText("Dropped earlier messages cleanly")).toBeTruthy();
  });

  it("renders Cancelled even when stopped before an assistant response arrived", () => {
    render(
      <TurnItem
        turn={turn([
          user("stop before reply", 0),
          { id: "cancelled-empty", role: "assistant", timestamp: 1000, completedAt: 2000, cancelled: true, parts: [] },
        ])}
        isLast
      />,
    );

    expect(screen.getByText("stop before reply")).toBeTruthy();
    expect(screen.getByText("Cancelled")).toBeTruthy();
  });

  it("collapses stopped or cancelled reasoning-only turns by default and expands on click", async () => {
    const userAct = userEvent.setup();
    render(
      <TurnItem
        turn={turn([
          user("try reasoning then cancel", 0),
          {
            id: "cancelled-thinking",
            role: "assistant",
            timestamp: 1000,
            completedAt: 4000,
            cancelled: true,
            parts: [{ type: "reasoning", text: "Checking half-finished plan..." }],
          },
        ])}
        isLast
      />,
    );

    const toggle = screen.getByRole("button", { name: "Show work" });
    expect(toggle.textContent).toContain("Worked for");
    expect(toggle.textContent).toContain("Cancelled");
    expect(screen.queryByRole("button", { name: "Show thinking" })).toBeNull();
    expect(screen.queryByText("Checking half-finished plan...")).toBeNull();

    await userAct.click(toggle);
    expect(screen.getByRole("button", { name: "Show thinking" }).textContent).toContain(
      "Checking half-finished plan...",
    );
  });
});
