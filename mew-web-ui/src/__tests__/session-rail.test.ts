import { describe, it, expect } from "vitest";
import type { SessionInfo } from "@mew/web-client";
import { groupByWorkspace, deriveTitle } from "../components/session-rail";

function makeSession(overrides: Partial<SessionInfo> = {}): SessionInfo {
  return {
    session_id: "sess_001",
    state: "idle",
    created_at: 1000,
    client_count: 0,
    ...overrides,
  };
}

describe("groupByWorkspace", () => {
  it("groups by full cwd path", () => {
    const sessions = [
      makeSession({ session_id: "a", cwd: "/home/alice/myproject" }),
      makeSession({ session_id: "b", cwd: "/home/alice/myproject" }),
      makeSession({ session_id: "c", cwd: "/home/bob/other" }),
    ];
    const result = groupByWorkspace(sessions);
    expect(result).toHaveLength(2);
    expect(result.map((s) => s.items.length).sort()).toEqual([1, 2]);
  });

  it("sorts folders by most recent activity (newest first)", () => {
    const sessions = [
      makeSession({
        session_id: "old",
        cwd: "/projects/old",
        created_at: 1000,
        last_message_at: 2000,
      }),
      makeSession({
        session_id: "new",
        cwd: "/projects/new",
        created_at: 1000,
        last_message_at: 5000,
      }),
      makeSession({
        session_id: "mid",
        cwd: "/projects/mid",
        created_at: 1000,
        last_message_at: 3000,
      }),
    ];
    const result = groupByWorkspace(sessions);
    expect(result).toHaveLength(3);
    // /projects/new (5000) > /projects/mid (3000) > /projects/old (2000)
    expect(result[0]!.cwd).toBe("/projects/new");
    expect(result[1]!.cwd).toBe("/projects/mid");
    expect(result[2]!.cwd).toBe("/projects/old");
  });

  it("uses created_at as fallback when last_message_at is missing", () => {
    const sessions = [
      makeSession({
        session_id: "a",
        cwd: "/dir/a",
        created_at: 100,
      }),
      makeSession({
        session_id: "b",
        cwd: "/dir/b",
        created_at: 500,
      }),
    ];
    const result = groupByWorkspace(sessions);
    expect(result[0]!.cwd).toBe("/dir/b");
    expect(result[1]!.cwd).toBe("/dir/a");
  });

  it("displays last path component as label", () => {
    const sessions = [
      makeSession({ session_id: "a", cwd: "/home/alice/myproject" }),
    ];
    const result = groupByWorkspace(sessions);
    expect(result[0]!.label).toBe("myproject");
  });

  it("groups sessions with no cwd under ~", () => {
    const sessions = [
      makeSession({ session_id: "a", cwd: undefined }),
      makeSession({ session_id: "b", cwd: undefined }),
    ];
    const result = groupByWorkspace(sessions);
    expect(result).toHaveLength(1);
    expect(result[0]!.cwd).toBe("~");
    expect(result[0]!.label).toBe("~");
    expect(result[0]!.items).toHaveLength(2);
  });

  it("separates sessions with same last component but different full paths", () => {
    const sessions = [
      makeSession({ session_id: "a", cwd: "/home/alice/myproject" }),
      makeSession({ session_id: "b", cwd: "/home/bob/myproject" }),
    ];
    const result = groupByWorkspace(sessions);
    expect(result).toHaveLength(2);
    expect(result[0]!.label).toBe("myproject");
    expect(result[1]!.label).toBe("myproject");
    expect(result[0]!.cwd).not.toBe(result[1]!.cwd);
  });

  it("preserves input order of sessions within each folder", () => {
    const sessions = [
      makeSession({ session_id: "first", cwd: "/proj", created_at: 5000 }),
      makeSession({ session_id: "second", cwd: "/proj", created_at: 1000 }),
    ];
    const result = groupByWorkspace(sessions);
    expect(result[0]!.items[0]!.session_id).toBe("first");
    expect(result[0]!.items[1]!.session_id).toBe("second");
  });
});

describe("deriveTitle", () => {
  it("returns summary when available", () => {
    const s = makeSession({ summary: "My Session Summary" });
    expect(deriveTitle(s)).toBe("My Session Summary");
  });

  it("returns first_message when no summary", () => {
    const s = makeSession({ first_message: "How do I fix a bug?" });
    expect(deriveTitle(s)).toBe("How do I fix a bug?");
  });

  it("returns model name when no summary or first_message", () => {
    const s = makeSession({ model: "deepseek/deepseek-v4-flash" });
    expect(deriveTitle(s)).toBe("deepseek-v4-flash");
  });

  it("returns 'Untitled' when nothing is available", () => {
    const s = makeSession();
    expect(deriveTitle(s)).toBe("Untitled");
  });

  it("prioritizes summary over first_message", () => {
    const s = makeSession({
      summary: "Summary wins",
      first_message: "first message text",
    });
    expect(deriveTitle(s)).toBe("Summary wins");
  });

  it("never returns raw session ID", () => {
    const s = makeSession({ session_id: "sess_abc123def456" });
    const title = deriveTitle(s);
    expect(title).not.toContain("sess_");
    expect(title).toBe("Untitled");
  });
});
