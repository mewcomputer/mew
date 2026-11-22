import { createContext, useContext, useEffect, useLayoutEffect, useMemo, useReducer, useRef, useState, type CSSProperties, type ReactNode } from "react";
import { useSessionStore } from "../stores/session";
import { createWorkbenchTab, type WorkbenchTabKind } from "../lib/workbench-tabs";
import { isDesktopHost } from "../lib/host";
import { getClient } from "../lib/client";
import { useIsMobile } from "../hooks/use-mobile";
import {
  DEFAULT_SIDEBAR_WIDTH,
  DEFAULT_WORKSPACE_SURFACES,
  MAX_SIDEBAR_WIDTH,
  MIN_SIDEBAR_WIDTH,
  WORKSPACE_SURFACES_STORAGE_KEY,
  clampSidebarWidth,
  loadWorkspaceSurfaces,
  workspaceSurfacesReducer,
  type WorkspaceSurfacesAction,
  type WorkspaceSurfacesState,
} from "../lib/workspace-surfaces";
import { RightRail } from "./right-rail";
import { SessionRail } from "./session-rail";
import { SidebarInset, SidebarProvider } from "./ui/sidebar";
import {
  ResizableHandle,
  ResizablePanel,
  ResizablePanelGroup,
} from "./ui/resizable";
import { usePanelRef } from "react-resizable-panels";
import { createPortal } from "react-dom";

interface WorkspaceFrameContextValue {
  surfaces: WorkspaceSurfacesState;
  dispatch: (action: WorkspaceSurfacesAction) => void;
  toggleSidebar: () => void;
  toggleWorkbench: () => void;
  openWorkbenchSurface: (kind: WorkbenchTabKind) => void;
  titlebar: HTMLElement | null;
  workbenchTitlebar: HTMLElement | null;
  setWorkbenchTitlebar: (element: HTMLElement | null) => void;
}

const WorkspaceFrameContext = createContext<WorkspaceFrameContextValue | null>(null);

export function WorkspaceFrame({ children }: { children: ReactNode }) {
  const isMobile = useIsMobile();
  const sessionId = useSessionStore((state) => state.sessionId);
  const workbenchPanelRef = usePanelRef();
  const panelGroup = useRef<HTMLDivElement>(null);
  const previousWorkbenchOpen = useRef<boolean | null>(null);
  const [surfaces, dispatch] = useReducer(
    workspaceSurfacesReducer,
    DEFAULT_WORKSPACE_SURFACES,
    () => loadWorkspaceSurfaces(typeof window === "undefined" ? undefined : window.localStorage),
  );
  const [initialWorkbenchSize] = useState(surfaces.workbenchSize);
  const [workbenchPresent, setWorkbenchPresent] = useState(surfaces.workbenchOpen);
  const [titlebar, setTitlebar] = useState<HTMLElement | null>(null);
  const [workbenchTitlebar, setWorkbenchTitlebar] = useState<HTMLElement | null>(null);
  const [sidebarResizing, setSidebarResizing] = useState(false);
  const sidebarWidth = surfaces.sidebarWidth ?? DEFAULT_SIDEBAR_WIDTH;

  useEffect(() => {
    dispatch({ type: "select-session", sessionId: sessionId ?? "new" });
  }, [sessionId]);

  useEffect(() => {
    try {
      localStorage.setItem(WORKSPACE_SURFACES_STORAGE_KEY, JSON.stringify(surfaces));
    } catch {
      // A restricted browser context should not prevent the workspace from opening.
    }
  }, [surfaces]);

  useLayoutEffect(() => {
    if (isMobile) {
      previousWorkbenchOpen.current = null;
      setWorkbenchPresent(surfaces.workbenchOpen);
      return;
    }
    const group = panelGroup.current;
    if (!group) return;
    const reducedMotion = window.matchMedia("(prefers-reduced-motion: reduce)");
    const changed = previousWorkbenchOpen.current !== null && previousWorkbenchOpen.current !== surfaces.workbenchOpen;
    const alreadyAnimating = group.dataset.workbenchMotion === "true";
    const animate = (changed || alreadyAnimating) && !reducedMotion.matches;
    let cancelled = false;
    const finish = () => {
      if (cancelled) return;
      group.dataset.workbenchMotion = "false";
      setWorkbenchPresent(surfaces.workbenchOpen);
    };
    if (changed && !surfaces.workbenchOpen
      && (document.activeElement === document.body || group.querySelector("#workbench")?.contains(document.activeElement))) {
      document.querySelector<HTMLButtonElement>('[aria-label="Open workbench"]')?.focus();
    }
    group.dataset.workbenchMotion = String(animate);
    if (surfaces.workbenchOpen) setWorkbenchPresent(true);
    if (surfaces.workbenchOpen) {
      if (workbenchPanelRef.current?.isCollapsed?.() ?? true) workbenchPanelRef.current?.expand();
    } else {
      if (!(workbenchPanelRef.current?.isCollapsed?.() ?? false)) workbenchPanelRef.current?.collapse();
    }
    previousWorkbenchOpen.current = surfaces.workbenchOpen;
    if (!animate) { finish(); return; }

    const frame = requestAnimationFrame(() => {
      const animations = Array.from(group.querySelectorAll<HTMLElement>(":scope > [data-panel]"))
        .flatMap((panel) => panel.getAnimations?.().map((animation) => animation.finished) ?? []);
      void Promise.allSettled(animations).then(finish);
    });
    const motionChanged = () => { if (reducedMotion.matches) finish(); };
    reducedMotion.addEventListener("change", motionChanged);
    return () => {
      cancelled = true;
      cancelAnimationFrame(frame);
      reducedMotion.removeEventListener("change", motionChanged);
    };
  }, [isMobile, surfaces.workbenchOpen, workbenchPanelRef]);

  useLayoutEffect(() => {
    if (isMobile || !workbenchTitlebar) return;
    const panel = panelGroup.current?.querySelector<HTMLElement>("#workbench");
    if (!panel || typeof ResizeObserver === "undefined") return;
    const syncWidth = () => {
      const rect = panel.getBoundingClientRect();
      const parentRect = workbenchTitlebar.parentElement?.getBoundingClientRect();
      if (!parentRect) return;
      if (rect.width <= 1) {
        workbenchTitlebar.style.width = "0px";
        return;
      }
      // Align the left edge of the workbench tab strip directly above the workbench panel's left border.
      const availableRight = Math.max(0, Math.round(parentRect.right - 36 - rect.left));
      workbenchTitlebar.style.width = `${availableRight}px`;
    };
    syncWidth();
    const observer = new ResizeObserver(syncWidth);
    observer.observe(panel);
    return () => observer.disconnect();
  }, [isMobile, workbenchTitlebar, surfaces.workbenchOpen, workbenchPresent]);

  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if (!(event.metaKey || event.ctrlKey) || !event.shiftKey || event.key.toLowerCase() !== "b") {
        return;
      }
      event.preventDefault();
      dispatch({ type: "toggle-workbench" });
    };

    const menu = (event: Event) => {
      const action = (event as CustomEvent<string>).detail;
      if (action === "toggle-sidebar") dispatch({ type: "toggle-sidebar" });
      if (action === "toggle-workbench") dispatch({ type: "toggle-workbench" });
      if (action === "new-tab" || action === "terminal") dispatch({ type: "set-workbench-open", open: true });
    };
    window.addEventListener("keydown", handleKeyDown);
    window.addEventListener("mew:menu-action", menu);
    return () => { window.removeEventListener("keydown", handleKeyDown); window.removeEventListener("mew:menu-action", menu); };
  }, []);

  useEffect(() => {
    if (!isDesktopHost()) return;
    const openLink = (event: MouseEvent) => {
      if (event.defaultPrevented || event.button !== 0 || !(event.target instanceof Element)) return;
      const anchor = event.target.closest("a[href]");
      if (!(anchor instanceof HTMLAnchorElement) || anchor.hasAttribute("download")) return;
      const destination = new URL(anchor.href, window.location.href);
      if (!/^https?:$/.test(destination.protocol) || destination.origin === window.location.origin) return;
      event.preventDefault();
      dispatch({ type: "workbench-tabs-action", action: { type: "add", tab: createWorkbenchTab("browser", {
        title: destination.hostname,
        sessionId: useSessionStore.getState().sessionId ?? undefined,
        payload: { url: destination.href },
      }) } });
      dispatch({ type: "set-workbench-open", open: true });
    };
    window.addEventListener("click", openLink);
    return () => window.removeEventListener("click", openLink);
  }, []);

  const contextValue = useMemo<WorkspaceFrameContextValue>(
    () => ({
      surfaces,
      dispatch,
      toggleSidebar: () => dispatch({ type: "toggle-sidebar" }),
      toggleWorkbench: () => dispatch({ type: "toggle-workbench" }),
      openWorkbenchSurface: (kind) => {
        const existing = surfaces.workbenchTabs.tabs.find((tab) => tab.kind === kind);
        const { sessionId: selectedId, sessionCwd } = useSessionStore.getState();
        dispatch({ type: "workbench-tabs-action", action: existing
          ? { type: "select", id: existing.id }
          : { type: "add", tab: createWorkbenchTab(kind, { sessionId: selectedId ?? undefined, cwd: sessionCwd ?? undefined }) } });
        dispatch({ type: "set-workbench-open", open: true });
      },
      titlebar,
      workbenchTitlebar,
      setWorkbenchTitlebar,
    }),
    [surfaces, dispatch, titlebar, workbenchTitlebar],
  );

  return (
    <WorkspaceFrameContext.Provider value={contextValue}>
      <SidebarProvider
        data-desktop-host={isDesktopHost() ? "" : undefined}
        data-sidebar-open={surfaces.sidebarOpen}
        data-sidebar-resizing={sidebarResizing ? "true" : undefined}
        open={surfaces.sidebarOpen}
        onOpenChange={(open) => dispatch({ type: "set-sidebar-open", open })}
        style={
          {
            "--sidebar-width": `${sidebarWidth}px`,
            "--header-height": "calc(var(--spacing) * 12)",
          } as CSSProperties
        }
      >
        <div className="flex h-full min-h-0 flex-1">
          <SessionRail client={getClient()} />
          {!isMobile && surfaces.sidebarOpen && (
            <div
              role="separator"
              aria-label="Resize sidebar"
              aria-orientation="vertical"
              aria-valuemin={MIN_SIDEBAR_WIDTH}
              aria-valuemax={MAX_SIDEBAR_WIDTH}
              aria-valuenow={sidebarWidth}
              tabIndex={0}
              onDoubleClick={() => dispatch({ type: "set-sidebar-width", width: DEFAULT_SIDEBAR_WIDTH })}
              onKeyDown={(event) => {
                if (event.key === "ArrowLeft") {
                  event.preventDefault();
                  dispatch({ type: "set-sidebar-width", width: clampSidebarWidth(sidebarWidth - 16) });
                } else if (event.key === "ArrowRight") {
                  event.preventDefault();
                  dispatch({ type: "set-sidebar-width", width: clampSidebarWidth(sidebarWidth + 16) });
                } else if (event.key === "Home") {
                  event.preventDefault();
                  dispatch({ type: "set-sidebar-width", width: MIN_SIDEBAR_WIDTH });
                } else if (event.key === "End") {
                  event.preventDefault();
                  dispatch({ type: "set-sidebar-width", width: MAX_SIDEBAR_WIDTH });
                }
              }}
              onMouseDown={(event) => {
                if (event.button !== 0) return;
                event.preventDefault();
                const startX = event.clientX;
                const startWidth = sidebarWidth;
                let collapsed = false;
                setSidebarResizing(true);
                const onMove = (moveEvent: MouseEvent) => {
                  const next = moveEvent.clientX - startX + startWidth;
                  if (next < MIN_SIDEBAR_WIDTH - 48) {
                    if (!collapsed) {
                      collapsed = true;
                      dispatch({ type: "set-sidebar-open", open: false });
                    }
                  } else {
                    if (collapsed) {
                      collapsed = false;
                      dispatch({ type: "set-sidebar-open", open: true });
                    }
                    dispatch({ type: "set-sidebar-width", width: clampSidebarWidth(next) });
                  }
                };
                const onUp = () => {
                  setSidebarResizing(false);
                  window.removeEventListener("mousemove", onMove);
                  window.removeEventListener("mouseup", onUp);
                };
                window.addEventListener("mousemove", onMove);
                window.addEventListener("mouseup", onUp);
              }}
              className="group/sidebar-resize relative z-20 -ml-1.5 -mr-1.5 hidden w-3 shrink-0 cursor-col-resize select-none items-center justify-center outline-none md:flex"
            >
              <div className="pointer-events-none h-9 w-1 rounded-full bg-sidebar-foreground/30 opacity-0 transition-all duration-150 group-hover/sidebar-resize:opacity-100 group-focus-visible/sidebar-resize:bg-sidebar-ring group-focus-visible/sidebar-resize:opacity-100 group-active/sidebar-resize:h-11 group-active/sidebar-resize:bg-sidebar-foreground/50 group-active/sidebar-resize:opacity-100" />
            </div>
          )}
          <div className="flex min-h-0 min-w-0 flex-1 flex-col">
          {/* Route headers portal here so the title sits in the window chrome above the content card. */}
          <div ref={setTitlebar} className="mew-titlebar flex min-w-0 shrink-0 flex-col" />
          {isMobile ? (
            <>
              <SidebarInset className="flex h-full min-w-0 flex-1 flex-col overflow-hidden">
                {children}
              </SidebarInset>
              <RightRail
                mode="dock"
                open={surfaces.workbenchOpen}
                onOpenChange={(open) => dispatch({ type: "set-workbench-open", open })}
                workbenchTabs={surfaces.workbenchTabs}
                workbenchSessionId={surfaces.sessionId}
                sessionTabs={surfaces.sessionTabs}
                onWorkbenchTabsAction={(action, sessionId) => dispatch({ type: "workbench-tabs-action", action, sessionId })}
              />
            </>
          ) : (
            <div className={surfaces.sidebarOpen ? "flex min-h-0 min-w-0 flex-1 pr-2 pb-2" : "flex min-h-0 min-w-0 flex-1 pl-2 pr-2 pb-2"}>
              <ResizablePanelGroup
                elementRef={panelGroup}
                orientation="horizontal"
                className="mew-workspace-panels min-h-0 min-w-0 flex-1 overflow-hidden rounded-xl border bg-background shadow-sm"
                id="mew-workspace-panels"
              resizeTargetMinimumSize={{ coarse: 28, fine: 20 }}
              onLayoutChange={(layout) => {
                const group = panelGroup.current;
                if (!group || window.matchMedia("(prefers-reduced-motion: reduce)").matches) return;
                const workbenchSize = layout.workbench;
                if (typeof workbenchSize !== "number") return;
                const dragActive = group.querySelector('[data-separator="active"], [data-separator="focus"]') !== null;
                if (!dragActive) return;
                const nowOpen = workbenchSize > 0;
                if (nowOpen !== previousWorkbenchOpen.current) {
                  previousWorkbenchOpen.current = nowOpen;
                  group.dataset.workbenchMotion = "true";
                  if (nowOpen) setWorkbenchPresent(true);
                  dispatch({ type: "set-workbench-open", open: nowOpen });
                }
              }}
              onLayoutChanged={(layout, meta) => {
                const workbenchSize = layout.workbench;
                if (typeof workbenchSize !== "number") return;
                if (workbenchSize > 0) {
                  dispatch({ type: "set-workbench-size", size: workbenchSize });
                }
                if (meta.isUserInteraction) {
                  dispatch({ type: "set-workbench-open", open: workbenchSize > 0 });
                }
              }}
            >
              <ResizablePanel
                id="conversation"
                defaultSize={`${100 - initialWorkbenchSize}%`}
                minSize="40%"
                className="motion-panel-size min-w-0"
              >
                <SidebarInset className="h-full">
                  {children}
                </SidebarInset>
              </ResizablePanel>
              <ResizableHandle withHandle aria-label="Resize workbench" />
              <ResizablePanel
                id="workbench"
                inert={!surfaces.workbenchOpen}
                aria-hidden={!surfaces.workbenchOpen}
                panelRef={workbenchPanelRef}
                defaultSize={`${initialWorkbenchSize}%`}
                minSize="18rem"
                maxSize="65%"
                collapsible
                collapsedSize="0%"
                className="motion-panel-size min-w-0"
              >
                <RightRail
                  mode="dock"
                  open={surfaces.workbenchOpen}
                  present={workbenchPresent}
                  onOpenChange={(open) => dispatch({ type: "set-workbench-open", open })}
                  workbenchTabs={surfaces.workbenchTabs}
                  workbenchSessionId={surfaces.sessionId}
                  sessionTabs={surfaces.sessionTabs}
                  onWorkbenchTabsAction={(action, sessionId) => dispatch({ type: "workbench-tabs-action", action, sessionId })}
                />
              </ResizablePanel>
              </ResizablePanelGroup>
            </div>
          )}
          </div>
        </div>
      </SidebarProvider>
    </WorkspaceFrameContext.Provider>
  );
}

/** Renders window-chrome content (title, frame toggles) in the frame's titlebar row. */
export function FrameTitlebar({ children }: { children: ReactNode }) {
  const context = useContext(WorkspaceFrameContext);
  if (!context) return <>{children}</>;
  return context.titlebar ? createPortal(children, context.titlebar) : null;
}

/** Renders the workbench tab strip into the window titlebar row when mounted inside WorkspaceFrame. */
export function FrameWorkbenchTitlebar({ children }: { children: ReactNode }) {
  const context = useContext(WorkspaceFrameContext);
  if (!context || !context.workbenchTitlebar) return <>{children}</>;
  return createPortal(children, context.workbenchTitlebar);
}

export function useOptionalWorkspaceFrame() {
  return useContext(WorkspaceFrameContext);
}

export function useWorkspaceFrame() {
  const context = useContext(WorkspaceFrameContext);
  if (!context) throw new Error("useWorkspaceFrame must be used within WorkspaceFrame");
  return context;
}
