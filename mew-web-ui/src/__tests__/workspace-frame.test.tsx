import { afterEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { WorkspaceFrame } from "../components/workspace-frame";
import { useSessionStore } from "../stores/session";
import type { ReactNode } from "react";
import type { WorkbenchTabsState } from "../lib/workbench-tabs";
import { ConversationLayout } from "../components/conversation-layout";

vi.mock("../lib/client", () => ({ getClient: () => ({}) }));
vi.mock("../components/session-rail", async (importOriginal) => ({
  ...await importOriginal<typeof import("../components/session-rail")>(),
  SessionRail: () => <nav aria-label="Chats" />,
}));
vi.mock("../components/right-rail", () => ({ RightRail: ({ open, workbenchTabs }: { open: boolean; workbenchTabs: WorkbenchTabsState }) => <aside aria-label="Workbench" data-open={open}>{workbenchTabs.tabs.map((tab) => <output key={tab.id}>{`${tab.title}:${tab.payload?.url}:${tab.sessionId}`}</output>)}</aside> }));
vi.mock("../components/ui/resizable", () => ({ ResizablePanelGroup: ({ children }: { children: ReactNode }) => <div>{children}</div>, ResizablePanel: ({ children }: { children: ReactNode }) => <section>{children}</section>, ResizableHandle: () => null }));

afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals(); delete window.mewQuery; localStorage.clear(); });

describe("native workspace navigation", () => {
  it("stops in-flight movement when reduced motion becomes enabled", () => {
    let resize: ResizeObserverCallback;
    vi.stubGlobal("ResizeObserver", class {
      constructor(private readonly callback: ResizeObserverCallback) {}
      observe(element: Element) { if (element.getAttribute("aria-label") === "Chat") resize = this.callback; }
      disconnect() {}
    });
    let reduced = false;
    const media = new EventTarget();
    Object.defineProperty(media, "matches", { get: () => reduced });
    const matchMedia = window.matchMedia;
    vi.spyOn(window, "matchMedia").mockImplementation((query) => query === "(prefers-reduced-motion: reduce)" ? media as MediaQueryList : matchMedia(query));
    useSessionStore.setState({ sessionId: "motion-chat" });
    render(<WorkspaceFrame><ConversationLayout><article className="mew-conversation-width">Conversation</article></ConversationLayout></WorkspaceFrame>);
    const column = screen.getByText("Conversation");
    vi.spyOn(column, "getBoundingClientRect").mockReturnValueOnce(new DOMRect(100, 0, 600, 800)).mockReturnValue(new DOMRect(50, 0, 600, 800));
    const animation = { playState: "running", cancel() { this.playState = "idle"; } };
    column.animate = () => animation as Animation;
    act(() => resize!([{ contentRect: { width: 700 } }] as ResizeObserverEntry[], {} as ResizeObserver));
    act(() => resize!([{ contentRect: { width: 1200 } }] as ResizeObserverEntry[], {} as ResizeObserver));
    const summary = screen.getByRole("complementary", { name: "Conversation summary" });
    expect(animation.playState).toBe("running");
    reduced = true;
    media.dispatchEvent(new Event("change"));
    expect(animation.playState).toBe("idle");
    expect(screen.getByRole("complementary", { name: "Conversation summary" })).toBe(summary);
    expect(column.isConnected).toBe(true);
  });

  it("automatically mounts the summary inside a wide chat pane and follows pane resizing", () => {
    let resize: ResizeObserverCallback;
    vi.stubGlobal("ResizeObserver", class {
      constructor(private readonly callback: ResizeObserverCallback) {}
      observe(element: Element) { if (element.getAttribute("aria-label") === "Chat") resize = this.callback; }
      disconnect() {}
    });
    useSessionStore.setState({ sessionId: "wide-chat", sessionCwd: "/code/mew", subagents: new Map(), jobs: new Map(), flaggedFiles: [], gitStatus: [] });
    render(<WorkspaceFrame><ConversationLayout><article>Conversation</article></ConversationLayout></WorkspaceFrame>);
    act(() => resize!([{ contentRect: { width: 1200 } }] as ResizeObserverEntry[], {} as ResizeObserver));
    const summary = screen.getByRole("complementary", { name: "Conversation summary" });
    act(() => resize!([], {} as ResizeObserver));
    expect(screen.getByRole("complementary", { name: "Conversation summary" })).toBe(summary);
    expect(summary.closest('[aria-label="Chat"]')).toBe(screen.getByRole("region", { name: "Chat" }));
    expect(summary.closest('[aria-label="Workbench"]')).toBeNull();
    expect(within(summary).getByText("Sources")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Open summary" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Open workbench" }));
    expect(screen.getByRole("complementary", { name: "Conversation summary" })).toBe(summary);
    within(summary).getByRole("button", { name: /Changes/ }).focus();
    act(() => resize!([{ contentRect: { width: 700 } }] as ResizeObserverEntry[], {} as ResizeObserver));
    expect(screen.queryByRole("complementary", { name: "Conversation summary" })).toBeNull();
    const trigger = within(screen.getByRole("region", { name: "Chat" })).getByRole("button", { name: "Open summary" });
    expect(document.activeElement).toBe(trigger);
    expect(summary.isConnected).toBe(true);
    expect(summary.getAttribute("aria-hidden")).toBe("true");
    expect(summary.hasAttribute("inert")).toBe(true);
    expect(within(screen.getByLabelText("Workbench")).queryByRole("button", { name: "Open summary" })).toBeNull();
    act(() => resize!([{ contentRect: { width: 1200 } }] as ResizeObserverEntry[], {} as ResizeObserver));
    expect(screen.getByRole("complementary", { name: "Conversation summary" })).toBe(summary);
    expect(summary.hasAttribute("inert")).toBe(false);
    expect(document.activeElement).toBe(within(summary).getByRole("button", { name: /Changes/ }));
    act(() => resize!([{ contentRect: { width: 700 } }] as ResizeObserverEntry[], {} as ResizeObserver));
    fireEvent.click(screen.getByRole("button", { name: "Open summary" }));
    within(screen.getByRole("dialog", { name: "Conversation summary" })).getByRole("button", { name: "View all sources" }).focus();
    act(() => resize!([{ contentRect: { width: 1200 } }] as ResizeObserverEntry[], {} as ResizeObserver));
    expect(screen.queryByRole("dialog", { name: "Conversation summary" })).toBeNull();
    expect(document.activeElement).toBe(within(summary).getByRole("button", { name: /Changes/ }));
  });

  it("opens the corner summary independently and enters subagents through it", () => {
    useSessionStore.setState({ sessionId: "source-chat", sessionCwd: "/code/mew", subagents: new Map(), jobs: new Map(), flaggedFiles: [], gitStatus: [] });
    render(<WorkspaceFrame><ConversationLayout><article>Conversation</article></ConversationLayout></WorkspaceFrame>);
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
