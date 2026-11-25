import type { Message, ProviderEventWire } from "@mew/web-client";
import type { ReactNode } from "react";
import { act, cleanup, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { MessageItem } from "../components/message-item";
import { setClient } from "../lib/client-ref";
import { useSessionStore, wireMessageToChatMessage } from "../stores/session";

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

  it("keeps copy and message details in one compact action row", () => {
    render(<MessageItem message={{
      id: "with-details", role: "assistant", timestamp: 1,
      parts: [{ type: "text", text: "A reply" }],
      assistantMeta: {
        provider_id: "test", model_id: "test", cost: 0,
        tokens: { input: 10, output: 2, reasoning: 0, cache_read: 0, cache_write: 0 },
        manifest: { model: "test", context_window: 1000, input_tokens: 10, output_tokens: 2, cache_read_tokens: 0, cache_write_tokens: 0, reasoning_tokens: 0, segments: [] },
      },
    }} />);
    const actions = screen.getByRole("group", { name: "Message actions" });
    expect(within(actions).getByRole("button", { name: "Copy" })).toBeTruthy();
    expect(within(actions).getByRole("button", { name: "Message details" })).toBeTruthy();
  });

  it("renders the canonical flattened daemon text stream through completion", () => {
    function Conversation() {
      const messages = useSessionStore((state) => state.messages);
      return messages.map((message) => <MessageItem key={message.id} message={message} />);
    }
    render(<Conversation />);
    const events: ProviderEventWire[] = JSON.parse(`[
      {"type":"part_start","part":{"type":"text","id":"part-1","message_id":"message-1","session_id":"session-1","text":""}},
      {"type":"part_delta","part_id":"part-1","field":"text","delta":"hello from fake provider"},
      {"type":"part_end","part_id":"part-1"},
      {"type":"message_end","finish":"stop","usage":{"input":0,"output":0,"reasoning":0,"cache_read":0,"cache_write":0},"cost":0}
    ]`);
    act(() => events.forEach((event) => useSessionStore.getState().onProviderEvent(event)));
    expect(screen.getByText("hello from fake provider")).toBeTruthy();
    expect(useSessionStore.getState().streamingPartId).toBeNull();
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

  it.each(["handoff_plan", "switch_persona"])("renders historical %s as an ordinary completed tool", async (toolName) => {
    const user = userEvent.setup();
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

    const row = screen.getByRole("button", { name: `Show ${toolName} details` });
    expect(row.textContent).toContain(toolName);
    expect(screen.getByText("completed").textContent).toBe("completed");
    expect(screen.queryByText("historical result")).toBeNull();
    expect(screen.queryByRole("button", { name: /approve|request changes/i })).toBeNull();
    await user.click(row);
    expect(screen.getByText("historical result").textContent).toBe("historical result");
  });

  it("renders consecutive tool calls as separate rows instead of a nested fold", () => {
    render(<MessageItem message={{
      id: "tool-run",
      role: "assistant",
      timestamp: 1,
      parts: [
        { type: "tool-call", toolName: "read", callId: "c1", input: { path: "/repo/AGENTS.md" }, state: "completed" },
        { type: "tool-call", toolName: "bash", callId: "c2", input: { command: "cargo test" }, state: "completed" },
      ],
    }} />);

    expect(screen.getByRole("button", { name: "Show read details" }).textContent).toContain("Read");
    expect(screen.getByRole("button", { name: "Show bash details" }).textContent).toContain("Ran");
    expect(screen.queryByText(/tool calls/)).toBeNull();
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
          id: "text-1", message_id: "message-3", session_id: "session-1",
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
describe("file parts", () => {
  afterEach(() => { cleanup(); });

  it("shows a raster attachment as an image instead of its base64 url", () => {
    const url = `data:image/png;base64,${"iVBORw0KGgoAAAANSUhEUg==".repeat(20)}`;
    render(<MessageItem message={{
      id: "with-image", role: "user", timestamp: 1,
      parts: [{ type: "file", mime: "image/png", filename: "shot.png", url }],
    }} />);
    expect(screen.getByRole("img", { name: "shot.png" }).getAttribute("src")).toBe(url);
    expect(document.body.textContent).not.toContain("base64");
  });

  it("labels other attachments without printing their url", () => {
    render(<MessageItem message={{
      id: "with-file", role: "assistant", timestamp: 1,
      parts: [{ type: "file", mime: "application/pdf", filename: "report.pdf", url: "file:///tmp/report.pdf" }],
    }} />);
    expect(screen.getByText("report.pdf")).toBeTruthy();
    expect(document.body.textContent).not.toContain("file:///tmp");
  });

  it("keeps file parts from session history structured", () => {
    const url = "data:image/png;base64,iVBORw0KGgo=";
    const message: Message = {
      id: "history", session_id: "session", role: "user", time: { created: 1 },
      parts: [{ type: "file", id: "part", message_id: "history", session_id: "session", mime: "image/png", filename: "shot.png", url }],
    };
    expect(wireMessageToChatMessage(message).parts).toEqual([{ type: "file", mime: "image/png", filename: "shot.png", url }]);
  });

  it("fetches mew-blob images over the wire on demand and opens lightbox preview", async () => {
    const user = userEvent.setup();
    const id = "a".repeat(64);
    const getAttachment = vi.fn().mockResolvedValue({ id, data_base64: "iVBORw0KGgo=", size: 8 });
    setClient({ getAttachment } as never);
    const originalCreateObjectUrl = URL.createObjectURL;
    URL.createObjectURL = vi.fn(() => "blob:http://localhost/cat-shot");

    try {
      render(<MessageItem message={{
        id: "with-blob-image",
        role: "user",
        timestamp: 1,
        parts: [{ type: "file", mime: "image/png", filename: "cat.png", url: `mew-blob://${id}` }],
      }} />);

      const img = await screen.findByRole("img", { name: "cat.png" });
      expect(img.getAttribute("src")).toBe("blob:http://localhost/cat-shot");
      expect(getAttachment).toHaveBeenCalledWith(id);

      await user.click(screen.getByRole("button", { name: "Preview cat.png" }));
      expect(screen.getByRole("dialog")).toBeTruthy();
    } finally {
      URL.createObjectURL = originalCreateObjectUrl;
      setClient(null);
    }
  });
});
