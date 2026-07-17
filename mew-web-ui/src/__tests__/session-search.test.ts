import { describe, expect, it } from "vitest";
import type { ProjectInfo, SessionInfo } from "@mew/web-client";
import {
  filterProjects,
  filterSessions,
  getSessionSearchText,
} from "../lib/session-search";

const sessions: SessionInfo[] = [
  {
    session_id: "alpha",
    state: "idle",
    created_at: 1719792000,
    last_message_at: 1720051200,
    cwd: "/Users/natalie/code/mew",
    first_message: "Fix the daemon reconnect flow",
    summary: "daemon lifecycle and reconnects",
    client_count: 0,
  },
  {
    session_id: "beta",
    state: "idle",
    created_at: 1722470400,
    last_message_at: 1722556800,
    cwd: "/Users/natalie/code/notes",
    first_message: "Draft the release notes",
    client_count: 0,
  },
];

const projects: ProjectInfo[] = [
  { path: "/Users/natalie/code/mew", display_name: "mew", session_count: 4, last_used_at: 1720051200 },
  { path: "/Users/natalie/code/notes", display_name: "notes", session_count: 1, last_used_at: 1722556800 },
];

describe("session search", () => {
  it("indexes title, content, project path, and dates", () => {
    const text = getSessionSearchText(sessions[0]!, "Reconnect bug");

    expect(text).toContain("reconnect bug");
    expect(text).toContain("daemon lifecycle and reconnects");
    expect(text).toContain("/users/natalie/code/mew");
    expect(text).toContain("2024-07-04");
  });

  it("matches content and project filters", () => {
    expect(filterSessions(sessions, "reconnect").map((s) => s.session_id)).toEqual(["alpha"]);
    expect(filterSessions(sessions, "project:notes").map((s) => s.session_id)).toEqual(["beta"]);
    expect(filterSessions(sessions, "folder:mew").map((s) => s.session_id)).toEqual(["alpha"]);
  });

  it("matches inclusive date filters and combines terms", () => {
    expect(filterSessions(sessions, "after:2024-07-01").map((s) => s.session_id)).toEqual(["alpha", "beta"]);
    expect(filterSessions(sessions, "before:2024-07-10").map((s) => s.session_id)).toEqual(["alpha"]);
    expect(filterSessions(sessions, "daemon after:2024-07-01").map((s) => s.session_id)).toEqual(["alpha"]);
  });

  it("searches project names and paths", () => {
    expect(filterProjects(projects, "code/mew").map((p) => p.display_name)).toEqual(["mew"]);
  });
});
