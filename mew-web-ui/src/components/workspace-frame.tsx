import { createContext, useContext, useEffect, useMemo, useReducer, type CSSProperties, type ReactNode } from "react";
import { useSessionStore } from "../stores/session";
import { createWorkbenchTab } from "../lib/workbench-tabs";
import { isDesktopHost } from "../lib/host";
import { getClient } from "../lib/client";
import { useIsMobile } from "../hooks/use-mobile";
import {
  DEFAULT_WORKSPACE_SURFACES,
  WORKSPACE_SURFACES_STORAGE_KEY,
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

interface WorkspaceFrameContextValue {
  surfaces: WorkspaceSurfacesState;
  dispatch: (action: WorkspaceSurfacesAction) => void;
  toggleSidebar: () => void;
  toggleWorkbench: () => void;
}

const WorkspaceFrameContext = createContext<WorkspaceFrameContextValue | null>(null);

export function WorkspaceFrame({ children }: { children: ReactNode }) {
  const isMobile = useIsMobile();
  const sessionId = useSessionStore((state) => state.sessionId);
  const workbenchPanelRef = usePanelRef();
  const [surfaces, dispatch] = useReducer(
    workspaceSurfacesReducer,
    DEFAULT_WORKSPACE_SURFACES,
    () => loadWorkspaceSurfaces(typeof window === "undefined" ? undefined : window.localStorage),
  );

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

  useEffect(() => {
    if (isMobile) return;
    if (surfaces.workbenchOpen) workbenchPanelRef.current?.expand();
    else workbenchPanelRef.current?.collapse();
  }, [isMobile, surfaces.workbenchOpen, workbenchPanelRef]);

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
    }),
    [surfaces, dispatch],
  );

  return (
    <WorkspaceFrameContext.Provider value={contextValue}>
      <SidebarProvider
        open={surfaces.sidebarOpen}
        onOpenChange={(open) => dispatch({ type: "set-sidebar-open", open })}
        style={
          {
            "--sidebar-width": "14rem",
            "--header-height": "calc(var(--spacing) * 12)",
          } as CSSProperties
        }
      >
        <div className="flex h-full min-h-0 flex-1">
          <SessionRail client={getClient()} />
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
                onWorkbenchTabsAction={(action) => dispatch({ type: "workbench-tabs-action", action })}
              />
            </>
          ) : (
            <ResizablePanelGroup
              orientation="horizontal"
              className="min-w-0 flex-1"
              id="mew-workspace-panels"
              resizeTargetMinimumSize={{ coarse: 28, fine: 20 }}
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
                defaultSize={`${100 - surfaces.workbenchSize}%`}
                minSize="24rem"
                className="motion-panel-size min-w-0"
              >
                <SidebarInset className="max-h-screen h-full">
                  {children}
                </SidebarInset>
              </ResizablePanel>
              <ResizableHandle withHandle aria-label="Resize workbench" />
              <ResizablePanel
                id="workbench"
                panelRef={workbenchPanelRef}
                defaultSize={`${surfaces.workbenchSize}%`}
                minSize="22rem"
                maxSize="65%"
                collapsible
                collapsedSize="0%"
                className="motion-panel-size min-w-0"
              >
                <RightRail
                  mode="dock"
                  open={surfaces.workbenchOpen}
                  onOpenChange={(open) => dispatch({ type: "set-workbench-open", open })}
                  workbenchTabs={surfaces.workbenchTabs}
                  onWorkbenchTabsAction={(action) => dispatch({ type: "workbench-tabs-action", action })}
                />
              </ResizablePanel>
            </ResizablePanelGroup>
          )}
        </div>
      </SidebarProvider>
    </WorkspaceFrameContext.Provider>
  );
}

export function useWorkspaceFrame() {
  const context = useContext(WorkspaceFrameContext);
  if (!context) throw new Error("useWorkspaceFrame must be used within WorkspaceFrame");
  return context;
}
