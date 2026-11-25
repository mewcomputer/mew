import { afterEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { WorkspaceFrame } from "../components/workspace-frame";
import { useSessionStore } from "../stores/session";
import { StrictMode, type ReactNode, type Ref } from "react";
import type { WorkbenchTabsState } from "../lib/workbench-tabs";
import { ConversationLayout } from "../components/conversation-layout";

vi.mock("../lib/client", () => ({ getClient: () => ({}) }));
vi.mock("../components/session-rail", async (importOriginal) => ({
  ...await importOriginal<typeof import("../components/session-rail")>(),
  SessionRail: () => <nav aria-label="Chats" />,
}));
vi.mock("../components/right-rail", () => ({ RightRail: ({ open, present = open, workbenchTabs }: { open: boolean; present?: boolean; workbenchTabs: WorkbenchTabsState }) => <aside aria-label="Workbench" data-open={open} data-present={present}>{workbenchTabs.tabs.map((tab) => <output key={tab.id}>{`${tab.title}:${tab.payload?.url}:${tab.sessionId}`}</output>)}</aside> }));
vi.mock("../components/ui/resizable", () => ({
  ResizablePanelGroup: ({ children, elementRef, id }: { children: ReactNode; elementRef?: Ref<HTMLDivElement>; id?: string }) => <div id={id} ref={elementRef}>{children}</div>,
  ResizablePanel: ({ children, id, inert }: { children: ReactNode; id: string; inert?: boolean }) => <section id={id} data-panel inert={inert}>{children}</section>,
  ResizableHandle: () => null,
}));

afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals(); delete window.mewQuery; localStorage.clear(); });

describe("native workspace navigation", () => {
  it("renders the conversation title in the window titlebar outside the content card", () => {
    useSessionStore.setState({ sessionId: "titled-chat", sessionTitles: new Map([["titled-chat", "Frame Redesign"]]), availableSessions: [{ session_id: "titled-chat", cwd: "/Users/me/code/mew" } as never] });
    render(<WorkspaceFrame><ConversationLayout><article>Conversation</article></ConversationLayout></WorkspaceFrame>);
    const titlebar = document.querySelector(".mew-titlebar")!;
    expect(within(titlebar as HTMLElement).getByText("Frame Redesign")).toBeTruthy();
    expect(within(titlebar as HTMLElement).getByText("mew")).toBeTruthy();
    expect(document.getElementById("mew-workspace-panels")!.contains(titlebar)).toBe(false);
  });

  it("resizes the sidebar by dragging or keyboard and persists the width", () => {
    render(<WorkspaceFrame><article>Conversation</article></WorkspaceFrame>);
    const handle = screen.getByRole("separator", { name: "Resize sidebar" });
    expect(handle.getAttribute("aria-valuenow")).toBe("256");
    fireEvent.mouseDown(handle, { button: 0, clientX: 256 });
    fireEvent.mouseMove(window, { clientX: 310 });
    fireEvent.mouseUp(window);
    expect(handle.getAttribute("aria-valuenow")).toBe("310");
    expect(JSON.parse(localStorage.getItem("mew.workspaceSurfaces")!).sidebarWidth).toBe(310);
    fireEvent.keyDown(handle, { key: "ArrowLeft" });
    expect(handle.getAttribute("aria-valuenow")).toBe("294");
    fireEvent.doubleClick(handle);
    expect(handle.getAttribute("aria-valuenow")).toBe("256");
  });

  it("restores the initial pane without motion during strict effect replay", () => {
    localStorage.setItem("mew.workspaceSurfaces", JSON.stringify({ workbenchOpen: true }));
    const frame = vi.spyOn(window, "requestAnimationFrame").mockReturnValue(1);
    render(<StrictMode><WorkspaceFrame><article>Conversation</article></WorkspaceFrame></StrictMode>);
    expect(document.getElementById("mew-workspace-panels")!.getAttribute("data-workbench-motion")).toBe("false");
    expect(frame).not.toHaveBeenCalled();
    expect(screen.getByLabelText("Workbench").getAttribute("data-open")).toBe("true");
  });

  it("keeps workbench contents present until both panels finish closing", async () => {
    localStorage.setItem("mew.workspaceSurfaces", JSON.stringify({ workbenchOpen: true }));
    let nextFrame!: FrameRequestCallback;
    vi.spyOn(window, "requestAnimationFrame").mockImplementation((callback) => { nextFrame = callback; return 1; });
    useSessionStore.setState({ sessionId: "closing-chat" });
    render(<WorkspaceFrame><ConversationLayout><article>Conversation</article></ConversationLayout></WorkspaceFrame>);
    let finishChat!: () => void;
    let finishWorkbench!: () => void;
    document.getElementById("conversation")!.getAnimations = () => [{ finished: new Promise<void>((resolve) => { finishChat = resolve; }) } as unknown as Animation];
    document.getElementById("workbench")!.getAnimations = () => [{ finished: new Promise<void>((resolve) => { finishWorkbench = resolve; }) } as unknown as Animation];
    fireEvent.click(screen.getByRole("button", { name: "Close workbench" }));
    expect(screen.getByLabelText("Workbench").getAttribute("data-open")).toBe("false");
    expect(screen.getByLabelText("Workbench").getAttribute("data-present")).toBe("true");
    expect(document.getElementById("workbench")!.hasAttribute("inert")).toBe(true);
    act(() => nextFrame(0));
    await act(async () => finishChat());
    expect(screen.getByLabelText("Workbench").getAttribute("data-present")).toBe("true");
    await act(async () => finishWorkbench());
    expect(screen.getByLabelText("Workbench").getAttribute("data-present")).toBe("false");
  });

  it("ignores completion of a close transition after reopening the pane", async () => {
    localStorage.setItem("mew.workspaceSurfaces", JSON.stringify({ workbenchOpen: true }));
    let nextFrame!: FrameRequestCallback;
    vi.spyOn(window, "requestAnimationFrame").mockImplementation((callback) => { nextFrame = callback; return 1; });
    useSessionStore.setState({ sessionId: "reopened-chat" });
    render(<WorkspaceFrame><ConversationLayout><article>Conversation</article></ConversationLayout></WorkspaceFrame>);
    let finishClose!: () => void;
    document.getElementById("workbench")!.getAnimations = () => [{ finished: new Promise<void>((resolve) => { finishClose = resolve; }) } as unknown as Animation];
    fireEvent.click(screen.getByRole("button", { name: "Close workbench" }));
    act(() => nextFrame(0));
    fireEvent.click(screen.getByRole("button", { name: "Open workbench" }));
    await act(async () => finishClose());
    expect(screen.getByLabelText("Workbench").getAttribute("data-open")).toBe("true");
    expect(screen.getByLabelText("Workbench").getAttribute("data-present")).toBe("true");
    expect(document.getElementById("workbench")!.hasAttribute("inert")).toBe(false);
  });

  it("finishes a closing pane immediately when reduced motion becomes enabled", () => {
    localStorage.setItem("mew.workspaceSurfaces", JSON.stringify({ workbenchOpen: true }));
    let reduced = false;
    const media = new EventTarget();
    Object.defineProperty(media, "matches", { get: () => reduced });
    const matchMedia = window.matchMedia;
    vi.spyOn(window, "matchMedia").mockImplementation((query) => query === "(prefers-reduced-motion: reduce)" ? media as MediaQueryList : matchMedia(query));
    vi.spyOn(window, "requestAnimationFrame").mockReturnValue(1);
    useSessionStore.setState({ sessionId: "reduced-pane-chat" });
    render(<WorkspaceFrame><ConversationLayout><article>Conversation</article></ConversationLayout></WorkspaceFrame>);
    fireEvent.click(screen.getByRole("button", { name: "Close workbench" }));
    expect(screen.getByLabelText("Workbench").getAttribute("data-present")).toBe("true");
    act(() => { reduced = true; media.dispatchEvent(new Event("change")); });
    expect(screen.getByLabelText("Workbench").getAttribute("data-present")).toBe("false");
    expect(document.getElementById("mew-workspace-panels")!.getAttribute("data-workbench-motion")).toBe("false");
    fireEvent.click(screen.getByRole("button", { name: "Open workbench" }));
    fireEvent.click(screen.getByRole("button", { name: "Close workbench" }));
    expect(screen.getByLabelText("Workbench").getAttribute("data-present")).toBe("false");
  });

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
    useSessionStore.setState({ sessionId: "wide-chat", sessionCwd: "/code/mew", subagents: new Map(), jobs: new Map(), flaggedFiles: [{ path: "notes/brief.md" }], gitStatus: [] });
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
    const trigger = within(document.querySelector<HTMLElement>(".mew-titlebar")!).getByRole("button", { name: "Open summary" });
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

  it("only insets the chat column while the summary panel is in its way", () => {
    let resize: ResizeObserverCallback;
    vi.stubGlobal("ResizeObserver", class {
      constructor(private readonly callback: ResizeObserverCallback) {}
      observe(element: Element) { if (element.getAttribute("aria-label") === "Chat") resize = this.callback; }
      disconnect() {}
    });
    useSessionStore.setState({ sessionId: "inset-chat", sessionCwd: "/code/mew", subagents: new Map(), jobs: new Map(), flaggedFiles: [], gitStatus: [] });
    render(<WorkspaceFrame><ConversationLayout><article className="mew-conversation-width">Conversation</article></ConversationLayout></WorkspaceFrame>);
    const pane = screen.getByRole("region", { name: "Chat" });
    const inset = () => screen.getByRole("complementary", { name: "Conversation summary" }).parentElement!.style.getPropertyValue("--summary-inset");
    act(() => resize!([{ contentRect: { width: 1100 } }] as ResizeObserverEntry[], {} as ResizeObserver));
    expect(inset()).toBe("16rem");
    // Wide enough for the column and the panel side by side, so the chat stays centred.
    act(() => resize!([{ contentRect: { width: 1400 } }] as ResizeObserverEntry[], {} as ResizeObserver));
    expect(inset()).toBe("0px");
    expect(pane.contains(screen.getByRole("complementary", { name: "Conversation summary" }))).toBe(true);
    act(() => resize!([{ contentRect: { width: 1040 } }] as ResizeObserverEntry[], {} as ResizeObserver));
    expect(inset()).toBe("16rem");
  });

  it("opens the corner summary independently and enters subagents through it", () => {
    useSessionStore.setState({ sessionId: "source-chat", sessionCwd: "/code/mew", subagents: new Map([["build", { parentCallId: "build", name: "builder", childSessionId: "child-a", displayName: "Build", status: "running", lastProgress: null, outcome: null, manifests: [] }]]), jobs: new Map(), flaggedFiles: [], gitStatus: [] });
    render(<WorkspaceFrame><ConversationLayout><article>Conversation</article></ConversationLayout></WorkspaceFrame>);
    fireEvent.click(screen.getByRole("button", { name: "Open summary" }));
    expect(screen.getByRole("dialog", { name: "Conversation summary" })).toBeTruthy();
    expect(screen.getByLabelText("Workbench").getAttribute("data-open")).toBe("false");
    expect(screen.getByRole("button", { name: "Collapse sidebar" })).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: /Subagents.*1 working/ }));
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
