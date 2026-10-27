import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { WorkspaceFrame } from "../components/workspace-frame";
import { useSessionStore } from "../stores/session";
import type { ReactNode } from "react";
import type { WorkbenchTabsState } from "../lib/workbench-tabs";
import { ConversationHeader } from "../components/conversation-header";

vi.mock("../lib/client", () => ({ getClient: () => ({}) }));
vi.mock("../components/session-rail", async (importOriginal) => ({
  ...await importOriginal<typeof import("../components/session-rail")>(),
  SessionRail: () => <nav aria-label="Chats" />,
}));
vi.mock("../components/right-rail", () => ({ RightRail: ({ open, workbenchTabs }: { open: boolean; workbenchTabs: WorkbenchTabsState }) => <aside aria-label="Workbench" data-open={open}>{workbenchTabs.tabs.map((tab) => <output key={tab.id}>{`${tab.title}:${tab.payload?.url}:${tab.sessionId}`}</output>)}</aside> }));
vi.mock("../components/ui/resizable", () => ({ ResizablePanelGroup: ({ children }: { children: ReactNode }) => <div>{children}</div>, ResizablePanel: ({ children }: { children: ReactNode }) => <section>{children}</section>, ResizableHandle: () => null }));

afterEach(() => { cleanup(); delete window.mewQuery; localStorage.clear(); });

describe("native workspace navigation", () => {
  it("opens the corner summary independently and enters subagents through it", () => {
    useSessionStore.setState({ sessionId: "source-chat", sessionCwd: "/code/mew", subagents: new Map(), jobs: new Map(), flaggedFiles: [], gitStatus: [] });
    render(<WorkspaceFrame><ConversationHeader /><article>Conversation</article></WorkspaceFrame>);
    fireEvent.click(screen.getByRole("button", { name: "Open summary" }));
    expect(screen.getByRole("dialog", { name: "Conversation summary" })).toBeTruthy();
    expect(screen.getByLabelText("Workbench").getAttribute("data-open")).toBe("false");
    expect(screen.getByRole("button", { name: "Collapse sidebar" })).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Subagents: No subagents" }));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(screen.getByLabelText("Workbench").getAttribute("data-open")).toBe("true");
    expect(screen.getByText("Agents:undefined:source-chat")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Collapse sidebar" })).toBeTruthy();
  });

  it("opens external chat links in a browser tab beside the conversation", () => {
    window.mewQuery = vi.fn();
    useSessionStore.setState({ sessionId: "source-chat" });
    render(<WorkspaceFrame><article>Conversation <a href="https://example.com/docs" target="_blank" rel="noreferrer">Read docs</a></article></WorkspaceFrame>);
    const allowed = fireEvent.click(screen.getByText("Read docs"));
    expect(allowed).toBe(false);
    expect(screen.getByLabelText("Workbench").getAttribute("data-open")).toBe("true");
    expect(screen.getByText("example.com:https://example.com/docs:source-chat")).toBeTruthy();
    expect(screen.getByText("Conversation")).toBeTruthy();
  });
});
