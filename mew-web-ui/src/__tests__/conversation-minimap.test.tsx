import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  ConversationMinimap,
  buildConversationTurns,
} from "../components/conversation-minimap";
import type { ChatMessage } from "../stores/session";

const SAMPLE_MESSAGES: ChatMessage[] = [
  {
    id: "user-1",
    role: "user",
    timestamp: 1000,
    parts: [{ type: "text", text: "probably fair, i feel like glm5.3's harness does more" }],
  },
  {
    id: "asst-1",
    role: "assistant",
    timestamp: 1005,
    parts: [
      {
        type: "text",
        text: "your hunch is interesting: **DeepSeek's own harness does have a code mode.** It calls it programmatic tool calling, or PTC.",
      },
    ],
  },
  {
    id: "user-2",
    role: "user",
    timestamp: 2000,
    parts: [{ type: "text", text: "can we wire up the timeline scrubber?" }],
  },
  {
    id: "asst-2",
    role: "assistant",
    timestamp: 2005,
    parts: [{ type: "text", text: "added the vertical turn tick bar on the left edge." }],
  },
];

describe("ConversationMinimap", () => {
  beforeEach(() => {
    window.localStorage.clear();
  });

  afterEach(() => {
    cleanup();
  });

  it("pairs user prompts with subsequent assistant replies into turns", () => {
    const turns = buildConversationTurns(SAMPLE_MESSAGES);
    expect(turns).toHaveLength(2);
    expect(turns[0]).toMatchObject({
      id: "user-1",
      messageIndex: 0,
      promptText: "probably fair, i feel like glm5.3's harness does more",
    });
    expect(turns[0]?.replyText).toContain("DeepSeek's own harness does have a code mode.");
    expect(turns[1]).toMatchObject({
      id: "user-2",
      messageIndex: 2,
      promptText: "can we wire up the timeline scrubber?",
      replyText: "added the vertical turn tick bar on the left edge.",
    });
  });

  it("shows the turn preview card on focus/hover and jumps to the message index on click", () => {
    const onSelect = vi.fn();
    render(
      <ConversationMinimap
        messages={SAMPLE_MESSAGES}
        sessionId="sess-1"
        activeMessageIndex={2}
        onSelectMessageIndex={onSelect}
      />,
    );

    expect(screen.getByLabelText("Conversation timeline")).toBeTruthy();

    const tickButtons = screen.getAllByRole("button", { name: /^Jump to turn/i });
    expect(tickButtons).toHaveLength(2);

    const firstTurnBtn = screen.getByRole("button", {
      name: /Jump to turn 1: probably fair/i,
    });
    fireEvent.focus(firstTurnBtn);

    const dialog = screen.getByRole("dialog", { name: "Turn preview" });
    expect(dialog.textContent).toContain("probably fair, i feel like glm5.3's harness does more");
    expect(dialog.textContent).toContain("DeepSeek's own harness does have a code mode.");

    // Toggle bookmark inside the preview card
    const bookmarkBtn = screen.getByRole("button", { name: "Bookmark turn" });
    fireEvent.click(bookmarkBtn);
    expect(screen.getByRole("button", { name: "Remove turn bookmark" })).toBeTruthy();

    // Clicking the preview card jumps to the turn's message index
    fireEvent.click(dialog);
    expect(onSelect).toHaveBeenCalledWith(0);
  });
});
describe("session index merge", () => {
  afterEach(cleanup);

  it("keeps local turns the daemon index has not seen yet", () => {
    const sessionIndex = [
      {
        id: "user-1",
        index: 0,
        message_count: 2,
        prompt: "older prompt",
        reply: "older reply",
      },
    ];
    render(
      <ConversationMinimap
        messages={SAMPLE_MESSAGES}
        sessionId="sess-1"
        activeMessageIndex={2}
        onSelectMessageIndex={() => {}}
        sessionIndex={sessionIndex}
      />,
    );
    expect(document.querySelectorAll("[data-turn-tick]")).toHaveLength(2);
    expect(document.querySelector('[data-turn-tick="0"]')?.getAttribute("aria-label")).toContain("older prompt");
    expect(document.querySelector('[data-turn-tick="1"]')?.getAttribute("aria-label")).toContain("can we wire up the timeline scrubber?");
  });
});
