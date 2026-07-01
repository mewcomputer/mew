import { useEffect } from "react";
import { useRouter } from "@tanstack/react-router";
import type { MewClient } from "@mew/web-client";
import { useSessionStore } from "../stores/session";
import type { SessionInfo } from "@mew/web-client";
import { cn } from "../lib/utils";
import {
  Sidebar,
  SidebarContent,
  SidebarGroup,
  SidebarGroupContent,
  SidebarGroupLabel,
  SidebarHeader,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
  useSidebar,
} from "@/components/ui/sidebar";
import { Button } from "@/components/ui/button";
import { Plus, RotateCcw, Settings } from "lucide-react";

interface SessionRailProps {
  client: MewClient | null;
}

export function SessionRail({ client }: SessionRailProps) {
  const sessions = useSessionStore((s) => s.availableSessions);
  const loading = useSessionStore((s) => s.sessionsLoading);
  const currentSessionId = useSessionStore((s) => s.sessionId);
  const sessionTitles = useSessionStore((s) => s.sessionTitles);
  const connectionState = useSessionStore((s) => s.connectionState);
  const { setOpenMobile } = useSidebar();
  const router = useRouter();

  // Fetch sessions once the client is connected.
  useEffect(() => {
    if (client && connectionState === "connected") {
      useSessionStore.getState().setSessionsLoading(true);
      client.listSessions().then((list) => {
        useSessionStore.getState().setAvailableSessions(list);
        useSessionStore.getState().setSessionsLoading(false);
      });
    }
  }, [client, connectionState]);

  const handleNewSession = async () => {
    if (!client) return;
    useSessionStore.getState().reset();
    const newId = await client.newSession();
    localStorage.setItem("mew.sessionId", newId);
    router.navigate({
      to: "/session/$sessionId",
      params: { sessionId: newId },
    });
    setOpenMobile(false);
  };

  const handleAttach = (sessionId: string) => {
    localStorage.setItem("mew.sessionId", sessionId);
    router.navigate({ to: "/session/$sessionId", params: { sessionId } });
    setOpenMobile(false);
  };

  const sorted = [...sessions].sort((a, b) => {
    if (a.state !== b.state) return a.state === "active" ? -1 : 1;
    const aT = a.last_message_at ?? a.created_at;
    const bT = b.last_message_at ?? b.created_at;
    return bT - aT;
  });

  const continueTarget = sorted.find(
    (s) => s.session_id !== currentSessionId && s.state === "idle" && s.model,
  );

  return (
    <Sidebar side="left" variant="floating" collapsible="icon">
      <SidebarHeader className="px-3 py-2">
        <div className="flex items-center justify-between">
          <div className="flex min-w-0 flex-col">
            <span className="text-xs font-semibold text-sidebar-foreground">
              mew
            </span>
            <SessionLabel />
          </div>
          <Button
            variant="ghost"
            size="icon"
            className="h-7 w-7"
            onClick={() => router.navigate({ to: "/settings" })}
            title="Settings"
          >
            <Settings className="h-3.5 w-3.5" />
          </Button>
        </div>
      </SidebarHeader>

      <SidebarContent>
        <SidebarGroup>
          <div className="space-y-1.5 p-2">
            <Button
              onClick={handleNewSession}
              disabled={!client}
              variant="default"
              size="sm"
              className="w-full"
            >
              <Plus className="h-3.5 w-3.5" />
              New session
            </Button>
            {continueTarget && (
              <Button
                onClick={() => handleAttach(continueTarget.session_id)}
                variant="outline"
                size="sm"
                className="w-full"
                title={`Resume ${deriveTitle(continueTarget)}`}
              >
                <RotateCcw className="h-3.5 w-3.5" />
                Continue latest
              </Button>
            )}
          </div>
        </SidebarGroup>

        <SidebarGroup>
          <SidebarGroupLabel>Sessions</SidebarGroupLabel>
          <SidebarGroupContent>
            {loading && sorted.length === 0 && (
              <div className="px-3 py-4 text-center text-xs text-muted-foreground">
                Loading sessions…
              </div>
            )}
            {!loading && sorted.length === 0 && (
              <div className="px-3 py-4 text-center text-xs text-muted-foreground">
                No sessions yet
              </div>
            )}
            <SidebarMenu className="gap-2">
              {sorted.map((s) => (
                <SidebarMenuItem key={s.session_id}>
                  <SidebarMenuButton
                    isActive={s.session_id === currentSessionId}
                    onClick={() => handleAttach(s.session_id)}
                    className="flex-col items-start gap-0.5 pb-10"
                  >
                    <div className="flex w-full items-center justify-between gap-1.5">
                      <span className="truncate text-xs font-medium">
                        {sessionTitles.get(s.session_id) ?? deriveTitle(s)}
                      </span>
                      <span
                        className={cn(
                          "shrink-0 rounded-full px-1.5 py-0.5 text-[9px] font-medium uppercase",
                          s.state === "active"
                            ? "bg-green-500/15 text-green-600 dark:text-green-400"
                            : "bg-muted text-muted-foreground",
                        )}
                      >
                        {s.state}
                      </span>
                    </div>
                    <div className="flex items-center gap-1.5 text-[10px] text-muted-foreground">
                      {s.model && (
                        <span className="truncate font-mono">
                          {shortModel(s.model)}
                        </span>
                      )}
                      <span>
                        · {formatRelativeAge(s.last_message_at ?? s.created_at)}
                      </span>
                    </div>
                  </SidebarMenuButton>
                </SidebarMenuItem>
              ))}
            </SidebarMenu>
          </SidebarGroupContent>
        </SidebarGroup>
      </SidebarContent>
    </Sidebar>
  );
}

function SessionLabel() {
  const sessionId = useSessionStore((s) => s.sessionId);
  const titles = useSessionStore((s) => s.sessionTitles);
  const label = sessionId
    ? (titles.get(sessionId) ?? sessionId.slice(0, 10) + "…")
    : "no session";
  return (
    <span className="truncate text-[10px] text-muted-foreground">{label}</span>
  );
}

function deriveTitle(s: SessionInfo): string {
  if (s.model) {
    const short = s.model.split("/").pop() ?? s.model;
    return short;
  }
  return s.session_id.slice(0, 8);
}

function shortModel(model: string): string {
  return model.split("/").pop() ?? model;
}

function formatRelativeAge(timestampMs: number): string {
  const diffMs = Date.now() - timestampMs;
  const sec = Math.floor(diffMs / 1000);
  if (sec < 60) return "now";
  const min = Math.floor(sec / 60);
  if (min < 60) return `${min}m`;
  const hr = Math.floor(min / 60);
  if (hr < 24) return `${hr}h`;
  const day = Math.floor(hr / 24);
  if (day < 7) return `${day}d`;
  return new Date(timestampMs).toLocaleDateString();
}
