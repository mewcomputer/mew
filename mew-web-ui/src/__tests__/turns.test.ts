import { describe, expect, it } from "vitest";
import { buildTurns, formatDuration, splitTurnParts } from "../lib/turns";
import type { ChatMessage } from "../stores/session";

const user = (text: string): ChatMessage => ({ id: `u-${text}`, role: "user", timestamp: 0, parts: [{ type: "text", text }] });
const tools = (): ChatMessage => ({ id: "tools", role: "user", timestamp: 1, parts: [] });
const assistant = (text: string): ChatMessage => ({ id: `a-${text}`, role: "assistant", timestamp: 2, parts: [{ type: "text", text }] });

describe("buildTurns", () => {
  it("keeps tool-result messages with the turn they belong to", () => {
    const turns = buildTurns([user("hi"), assistant("one"), tools(), assistant("two"), user("again"), assistant("three")]);
    expect(turns.map((turn) => turn.messages.length)).toEqual([4, 2]);
    expect(turns.map((turn) => turn.messageIndex)).toEqual([0, 4]);
  });

  it("does not start a turn on a message with no visible text", () => {
    const turns = buildTurns([tools(), assistant("leading reply")]);
    expect(turns).toHaveLength(1);
    expect(turns[0]?.messages).toHaveLength(2);
  });
});

describe("splitTurnParts", () => {
  it("folds every earlier round and keeps the trailing text as the response", () => {
    const { work, response } = splitTurnParts([
      { id: "a1", role: "assistant", timestamp: 0, parts: [{ type: "reasoning", text: "plan" }, { type: "tool-call", toolName: "read", callId: "c1", input: {}, state: "completed" }] },
      { id: "a2", role: "assistant", timestamp: 1, parts: [{ type: "tool-call", toolName: "read", callId: "c2", input: {}, state: "completed" }, { type: "text", text: "answer" }] },
    ]);
    expect(work).toHaveLength(3);
    expect(response).toEqual([{ type: "text", text: "answer" }]);
  });

  it("treats a round that ended on tool calls as work with no response", () => {
    const { work, response } = splitTurnParts([
      { id: "a1", role: "assistant", timestamp: 0, parts: [{ type: "tool-call", toolName: "read", callId: "c1", input: {}, state: "completed" }] },
    ]);
    expect(work).toHaveLength(1);
    expect(response).toEqual([]);
  });

  it("excludes compaction parts from folded work and response slices", () => {
    const { work, response } = splitTurnParts([
      {
        id: "a1",
        role: "assistant",
        timestamp: 0,
        parts: [
          { type: "tool-call", toolName: "read", callId: "c1", input: {}, state: "completed" },
          { type: "compaction", summary: "summary", removed: 4 },
        ],
      },
      { id: "a2", role: "assistant", timestamp: 1, parts: [{ type: "text", text: "done" }] },
    ]);
    expect(work).toHaveLength(1);
    expect(work[0]?.type).toBe("tool-call");
    expect(response).toEqual([{ type: "text", text: "done" }]);
  });
});

describe("formatDuration", () => {
  it.each([
    [0, "0s"],
    [42_000, "42s"],
    [65_000, "1m 5s"],
    [3_600_000, "1h 0m"],
    [7_800_000, "2h 10m"],
  ])("formats %ims as %s", (ms, expected) => {
    expect(formatDuration(ms)).toBe(expected);
  });
});
