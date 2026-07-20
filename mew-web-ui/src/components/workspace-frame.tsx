import { createContext, useContext, useEffect, useMemo, useReducer, type CSSProperties, type ReactNode } from "react";
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
  toggleSummary: () => void;
  toggleWorkbench: () => void;
}

const WorkspaceFrameContext = createContext<WorkspaceFrameContextValue | null>(null);

export function WorkspaceFrame({ children }: { children: ReactNode }) {
  const isMobile = useIsMobile();
  const workbenchPanelRef = usePanelRef();
  const [surfaces, dispatch] = useReducer(
    workspaceSurfacesReducer,
    DEFAULT_WORKSPACE_SURFACES,
    () => loadWorkspaceSurfaces(typeof window === "undefined" ? undefined : window.localStorage),
  );

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

    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, []);

  const contextValue = useMemo<WorkspaceFrameContextValue>(
    () => ({
      surfaces,
      dispatch,
      toggleSummary: () => dispatch({ type: "toggle-summary" }),
      toggleWorkbench: () => dispatch({ type: "toggle-workbench" }),
    }),
    [surfaces, dispatch],
  );

  return (
    <WorkspaceFrameContext.Provider value={contextValue}>
      <SidebarProvider
        open={surfaces.summaryOpen}
        onOpenChange={(open) => dispatch({ type: "set-summary-open", open })}
        style={
          {
            "--sidebar-width": "calc(var(--spacing) * 72)",
            "--header-height": "calc(var(--spacing) * 12)",
          } as CSSProperties
        }
      >
        <div className="flex h-full min-h-0 flex-1">
          <SessionRail client={getClient()} />
          {isMobile ? (
            <>
              <SidebarInset className="flex h-full min-w-0 flex-1 flex-col overflow-hidden md:m-2 md:ml-0 md:rounded-[var(--radius-shell)] md:shadow-sm">
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
                minSize="30%"
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
                minSize="18rem"
                maxSize="50%"
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
