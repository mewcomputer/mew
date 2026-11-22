import { useSessionStore } from "../stores/session";
import { Button } from "@/components/ui/button";
import { displaySessionTitle } from "./session-rail";
import { FrameTitlebar, useWorkspaceFrame } from "./workspace-frame";
import { ConversationSummary } from "./conversation-summary";
import { PanelLeft, PanelRight } from "lucide-react";

/** Conversation identity and workspace navigation. */
export function ConversationHeader({ summaryPinned }: { summaryPinned: boolean }) {
  const { surfaces, toggleSidebar, toggleWorkbench, openWorkbenchSurface, setWorkbenchTitlebar } = useWorkspaceFrame();
  const sessionId = useSessionStore((s) => s.sessionId);
  const titles = useSessionStore((s) => s.sessionTitles);
  const sessions = useSessionStore((s) => s.availableSessions);

  const session = sessionId ? sessions.find((item) => item.session_id === sessionId) : undefined;
  const title = sessionId ? displaySessionTitle(titles.get(sessionId), session) : "mew";
  const workspace = session?.cwd ? workspaceName(session.cwd) : null;

  return (
    <FrameTitlebar>
      <div className="mew-conversation-header mew-drag-region mx-auto flex h-9 w-full min-w-0 shrink-0 items-center gap-1.5 px-3">
        <Button
          variant="ghost"
          size="icon"
          className={surfaces.sidebarOpen ? "relative h-7 w-7 shrink-0 text-muted-foreground md:hidden" : "relative h-7 w-7 shrink-0 text-muted-foreground"}
          onClick={toggleSidebar}
          title={surfaces.sidebarOpen ? "Collapse sidebar" : "Expand sidebar"}
          aria-label={surfaces.sidebarOpen ? "Collapse sidebar" : "Expand sidebar"}
          aria-pressed={surfaces.sidebarOpen}
        >
          <PanelLeft className="h-3.5 w-3.5" />
        </Button>

        <div className="mew-titlebar-main flex min-w-0 flex-1 items-center gap-1.5 pr-2">
          <div className="flex min-w-0 flex-1 items-center gap-1.5 text-[13px]">
            {workspace && (
              <>
                <span className="shrink-0 text-foreground/85" title={session?.cwd}>{workspace}</span>
                <span className="shrink-0 text-muted-foreground" aria-hidden>/</span>
              </>
            )}
            <span className="truncate text-foreground/85" title={title}>
              {title}
            </span>
          </div>

          {!summaryPinned && <ConversationSummary onOpenSurface={openWorkbenchSurface} />}
        </div>

        <div
          ref={setWorkbenchTitlebar}
          data-open={surfaces.workbenchOpen}
          inert={!surfaces.workbenchOpen}
          aria-hidden={!surfaces.workbenchOpen}
          className="mew-workbench-titlebar hidden min-w-0 shrink-0 items-center overflow-hidden transition-opacity duration-180 ease-out data-[open=false]:pointer-events-none data-[open=false]:opacity-0 md:flex"
        />

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
    </FrameTitlebar>
  );
}

function workspaceName(cwd: string): string {
  const parts = cwd.replace(/\/+$/, "").split("/");
  return parts[parts.length - 1] || cwd;
}
