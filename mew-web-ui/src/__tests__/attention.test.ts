import { describe, expect, it } from "vitest";
import type { AlertKind, SessionInfo } from "@mew/web-client";
import { compareSessionsByAttention, getSessionAttention } from "../lib/attention";

function session(id: string, overrides: Partial<SessionInfo> = {}): SessionInfo {
  return {
    session_id: id,
    state: "idle",
    created_at: 1_000,
    client_count: 0,
    ...overrides,
  };
}

describe("session attention", () => {
  it("labels blocking states explicitly and orders them by urgency", () => {
    const item = session("a", { pending_permissions: 2, pending_questions: 1 });
    expect(getSessionAttention(item)).toEqual([
      { kind: "permission", label: "Permissions needed", rank: 0, count: 2 },
      { kind: "question", label: "Question · needs input", rank: 1, count: 1 },
    ]);
  });

  it("uses daemon alert kinds for failures", () => {
    const alerts: AlertKind[] = ["turn_complete", "turn_failed"];
    expect(getSessionAttention(session("a"), alerts)[0]?.label).toBe("Turn failed");
  });

  it("puts attention first even when the session is current or less recent", () => {
    const attention = session("attention", { pending_questions: 1, last_message_at: 1_000 });
    const recent = session("recent", { last_message_at: 9_000 });
    const current = session("current", { pending_permissions: 1, last_message_at: 500 });
    const alerts = new Map([["attention", ["input_needed"] as AlertKind[]]]);

    expect(compareSessionsByAttention(attention, recent, alerts)).toBeLessThan(0);
    expect(compareSessionsByAttention(current, attention, alerts)).toBeLessThan(0);
  });
});
