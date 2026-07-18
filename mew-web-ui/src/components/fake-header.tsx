import { useRouter } from "@tanstack/react-router";
import { useSessionStore } from "../stores/session";
import { getSessionAttention } from "../lib/attention";
import { Button } from "@/components/ui/button";
import { displaySessionTitle } from "./session-rail";
import { useWorkspaceFrame } from "./workspace-frame";
import { PanelLeft, PanelRight, Activity, Settings, Folder } from "lucide-react";
import type { AlertKind } from "@mew/web-client";

/** Fake header — borderless, natural extension of the chat surface. */
export function FakeHeader() {
  const { surfaces, toggleSummary, toggleWorkbench } = useWorkspaceFrame();
  const sessionId = useSessionStore((s) => s.sessionId);
  const titles = useSessionStore((s) => s.sessionTitles);
  const sessions = useSessionStore((s) => s.availableSessions);
  const alerts = useSessionStore((s) => s.alerts);
  const router = useRouter();

  const session = sessionId ? sessions.find((item) => item.session_id === sessionId) : undefined;
  const title = sessionId ? displaySessionTitle(titles.get(sessionId), session) : "mew";
  const workspace = session?.cwd ? workspaceName(session.cwd) : null;
  const alertKindsBySession = new Map<string, AlertKind[]>();
  for (const alert of alerts) {
    const kinds = alertKindsBySession.get(alert.sessionId) ?? [];
    kinds.push(alert.kind);
    alertKindsBySession.set(alert.sessionId, kinds);
  }
  const attentionCount = sessions.filter((item) =>
    getSessionAttention(item, alertKindsBySession.get(item.session_id)).length > 0,
  ).length;

  return (
    <>
      <div className="mx-auto flex w-full shrink-0 items-center gap-2 px-3 py-1.5">
        <Button
          variant="ghost"
          size="icon"
          className="relative h-7 w-7"
          onClick={toggleSummary}
          title={surfaces.summaryOpen ? "Collapse summary" : "Expand summary"}
          aria-label={surfaces.summaryOpen ? "Collapse summary" : "Expand summary"}
          aria-pressed={surfaces.summaryOpen}
        >
          {surfaces.summaryOpen ? (
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

        <Button
          variant="ghost"
          size="icon"
          className={surfaces.workbenchOpen ? "relative h-7 w-7 bg-accent text-accent-foreground" : "relative h-7 w-7"}
          onClick={toggleWorkbench}
          title={surfaces.workbenchOpen ? "Close workbench" : "Open workbench"}
          aria-label={surfaces.workbenchOpen ? "Close workbench" : "Open workbench"}
          aria-pressed={surfaces.workbenchOpen}
        >
          <Activity className="h-3.5 w-3.5" />
          {attentionCount > 0 && (
            <span className="absolute right-0.5 top-0.5 flex h-2 w-2 rounded-full bg-amber-500 ring-2 ring-background" aria-label={`${attentionCount} sessions need attention`} />
          )}
        </Button>
        <Button
          variant="ghost"
          size="icon"
          className="h-7 w-7"
          onClick={() => router.navigate({ to: "/settings" })}
          title="Settings"
          aria-label="Open settings"
        >
          <Settings className="h-3.5 w-3.5" />
        </Button>
      </div>
    </>
  );
}

function workspaceName(cwd: string): string {
  const parts = cwd.replace(/\/+$/, "").split("/");
  return parts[parts.length - 1] || cwd;
}
