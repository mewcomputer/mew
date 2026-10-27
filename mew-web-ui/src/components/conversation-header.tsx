import { useSessionStore } from "../stores/session";
import { Button } from "@/components/ui/button";
import { displaySessionTitle } from "./session-rail";
import { useWorkspaceFrame } from "./workspace-frame";
import { ConversationSummary } from "./conversation-summary";
import { createWorkbenchTab, type WorkbenchTabKind } from "../lib/workbench-tabs";
import { PanelLeft, PanelRight, Folder } from "lucide-react";

/** Conversation identity and workspace navigation. */
export function ConversationHeader() {
  const { surfaces, dispatch, toggleSidebar, toggleWorkbench } = useWorkspaceFrame();
  const sessionId = useSessionStore((s) => s.sessionId);
  const titles = useSessionStore((s) => s.sessionTitles);
  const sessions = useSessionStore((s) => s.availableSessions);

  const session = sessionId ? sessions.find((item) => item.session_id === sessionId) : undefined;
  const title = sessionId ? displaySessionTitle(titles.get(sessionId), session) : "mew";
  const workspace = session?.cwd ? workspaceName(session.cwd) : null;
  const openSummarySurface = (kind: WorkbenchTabKind) => {
    const existing = surfaces.workbenchTabs.tabs.find((tab) => tab.kind === kind);
    dispatch({ type: "workbench-tabs-action", action: existing
      ? { type: "select", id: existing.id }
      : { type: "add", tab: createWorkbenchTab(kind, { sessionId: sessionId ?? undefined, cwd: session?.cwd }) } });
    dispatch({ type: "set-workbench-open", open: true });
  };

  return (
    <>
      <div className="mx-auto flex h-9 w-full shrink-0 items-center gap-2 px-3">
        <Button
          variant="ghost"
          size="icon"
          className="relative h-7 w-7"
          onClick={toggleSidebar}
          title={surfaces.sidebarOpen ? "Collapse sidebar" : "Expand sidebar"}
          aria-label={surfaces.sidebarOpen ? "Collapse sidebar" : "Expand sidebar"}
          aria-pressed={surfaces.sidebarOpen}
        >
          {surfaces.sidebarOpen ? (
            <PanelLeft className="h-3.5 w-3.5" />
          ) : (
            <PanelRight className="h-3.5 w-3.5" />
          )}
        </Button>

        <div className="flex min-w-0 flex-col leading-tight">
          <span className="truncate text-xs font-medium text-muted-foreground" title={title}>
            {title}
          </span>
          {workspace && (
            <span className="flex items-center gap-1 truncate text-[10px] text-muted-foreground/70" title={session?.cwd}>
              <Folder className="h-2.5 w-2.5 shrink-0" />
              {workspace}
            </span>
          )}
        </div>

        <div className="flex-1" />

        <ConversationSummary onOpenSurface={openSummarySurface} />

        <Button
          variant="ghost"
          size="icon"
          className={surfaces.workbenchOpen ? "relative h-7 w-7 bg-accent text-accent-foreground" : "relative h-7 w-7"}
          onClick={toggleWorkbench}
          title={surfaces.workbenchOpen ? "Close workbench" : "Open workbench"}
          aria-label={surfaces.workbenchOpen ? "Close workbench" : "Open workbench"}
          aria-pressed={surfaces.workbenchOpen}
        >
          <PanelRight className="h-3.5 w-3.5" />
        </Button>
      </div>
    </>
  );
}

function workspaceName(cwd: string): string {
  const parts = cwd.replace(/\/+$/, "").split("/");
  return parts[parts.length - 1] || cwd;
}
