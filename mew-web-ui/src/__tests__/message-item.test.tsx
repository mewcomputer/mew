import type { ReactNode } from "react";
import { act, cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { MessageItem } from "../components/message-item";
import { useSessionStore } from "../stores/session";

const markdownRenderCount = vi.hoisted(() => ({ value: 0 }));

vi.mock("../components/markdown-body", () => ({
  MarkdownBody: ({ children }: { children: ReactNode }) => {
    markdownRenderCount.value += 1;
    return <div data-testid="markdown">{children}</div>;
  },
}));

describe("MessageItem streaming subscriptions", () => {
  afterEach(() => {
    cleanup();
    markdownRenderCount.value = 0;
    useSessionStore.setState({
      messages: [],
      streamingPartId: null,
      streamingText: "",
      streamingReasoningId: null,
      streamingReasoningText: "",
    });
  });

  it("does not rerender completed messages for streaming deltas", () => {
    const message = {
      id: "message-1",
      role: "user" as const,
      parts: [{ type: "text" as const, text: "already complete" }],
      timestamp: 1,
    };

    render(<MessageItem message={message} />);
    expect(markdownRenderCount.value).toBe(1);

    act(() => {
      useSessionStore.setState({ streamingText: "new assistant text" });
    });

    expect(markdownRenderCount.value).toBe(1);
    expect(screen.getByTestId("markdown").textContent).toContain("already complete");
  });

  it.each(["handoff_plan", "switch_persona"])("renders historical %s as an ordinary completed tool", (toolName) => {
    render(<MessageItem message={{
      id: "historical-message",
      role: "assistant",
      timestamp: 1,
      parts: [{
        type: "tool-call",
        toolName,
        callId: "historical-call",
        input: { name: "builder" },
        state: "completed",
        output: "historical result",
      }],
    }} />);

    expect(screen.getByText(toolName).textContent).toBe(toolName);
    expect(screen.getByText("completed").textContent).toBe("completed");
    expect(screen.getByText("historical result").textContent).toBe("historical result");
    expect(screen.queryByRole("button", { name: /approve|request changes/i })).toBeNull();
  });

  it("still updates the active streaming message", () => {
    const message = {
      id: "message-2",
      role: "assistant" as const,
      parts: [{ type: "text" as const, text: "seed", streaming: true }],
      timestamp: 1,
    };

    render(<MessageItem message={message} />);
    expect(screen.getByTestId("markdown").textContent).toContain("…");

    act(() => {
      useSessionStore.setState({ streamingText: "live response" });
    });

    expect(markdownRenderCount.value).toBe(2);
    expect(screen.getByTestId("markdown").textContent).toContain("live response");
  });

  it("updates when a text part is appended after reasoning", () => {
    useSessionStore.setState({
      messages: [{
        id: "message-3",
        role: "assistant",
        parts: [{ type: "reasoning", text: "thinking", streaming: true }],
        timestamp: 1,
      }],
      streamingReasoningId: "reasoning-1",
      streamingReasoningText: "thinking",
      streamingPartId: null,
      streamingText: "",
    });

    function StoreMessage() {
      const message = useSessionStore((s) => s.messages[0]);
      return message ? <MessageItem message={message} /> : null;
    }

    render(<StoreMessage />);

    act(() => {
      useSessionStore.getState().onProviderEvent({
        type: "part_start",
        part: {
          type: "text",
          base: { id: "text-1", message_id: "message-3", session_id: "session-1" },
          text: "",
          synthetic: false,
        },
      });
      useSessionStore.getState().onProviderEvent({
        type: "part_delta",
        part_id: "text-1",
        field: "text",
        delta: "live response",
      });
    });

    expect(screen.getByTestId("markdown").textContent).toContain("live response");
  });
});
