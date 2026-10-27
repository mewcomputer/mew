import { useSessionStore } from "../stores/session";
import { Button } from "@/components/ui/button";
import { displaySessionTitle } from "./session-rail";
import { useWorkspaceFrame } from "./workspace-frame";
import { ConversationSummary } from "./conversation-summary";
import { PanelLeft, PanelRight, Folder } from "lucide-react";

/** Conversation identity and workspace navigation. */
export function ConversationHeader({ summaryPinned }: { summaryPinned: boolean }) {
  const { surfaces, toggleSidebar, toggleWorkbench, openWorkbenchSurface } = useWorkspaceFrame();
  const sessionId = useSessionStore((s) => s.sessionId);
  const titles = useSessionStore((s) => s.sessionTitles);
  const sessions = useSessionStore((s) => s.availableSessions);

  const session = sessionId ? sessions.find((item) => item.session_id === sessionId) : undefined;
  const title = sessionId ? displaySessionTitle(titles.get(sessionId), session) : "mew";
  const workspace = session?.cwd ? workspaceName(session.cwd) : null;

  return (
    <>
      <div className="mew-conversation-header mew-drag-region mx-auto flex h-9 w-full min-w-0 shrink-0 items-center gap-1.5 px-3">
        <Button
          variant="ghost"
          size="icon"
          className="relative h-7 w-7 shrink-0 text-muted-foreground"
          onClick={toggleSidebar}
          title={surfaces.sidebarOpen ? "Collapse sidebar" : "Expand sidebar"}
          aria-label={surfaces.sidebarOpen ? "Collapse sidebar" : "Expand sidebar"}
          aria-pressed={surfaces.sidebarOpen}
        >
          <PanelLeft className="h-3.5 w-3.5" />
        </Button>

        <div className="flex min-w-0 flex-1 flex-col gap-0.5 leading-tight">
          <span className="truncate text-xs font-medium text-foreground/85" title={title}>
            {title}
          </span>
          {workspace && (
            <span className="flex min-w-0 items-center gap-1 text-[10px] text-muted-foreground" title={session?.cwd}>
              <Folder className="h-2.5 w-2.5 shrink-0" />
              <span className="truncate">{workspace}</span>
            </span>
          )}
        </div>

        {!summaryPinned && <ConversationSummary onOpenSurface={openWorkbenchSurface} />}

        <Button
          variant="ghost"
          size="icon"
          className={surfaces.workbenchOpen ? "relative h-7 w-7 shrink-0 bg-accent text-accent-foreground" : "relative h-7 w-7 shrink-0 text-muted-foreground"}
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
