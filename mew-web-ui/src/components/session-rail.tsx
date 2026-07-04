import { useEffect, useState, useMemo } from "react";
import { useRouter } from "@tanstack/react-router";
import type { MewClient, SessionInfo } from "@mew/web-client";
import { useSessionStore } from "../stores/session";
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
import { Plus, RotateCcw, Settings, Folder, Archive, Pin } from "lucide-react";

interface SessionRailProps {
  client: MewClient | null;
}

type ViewMode = "timeline" | "workspace" | "grouped";

export function SessionRail({ client }: SessionRailProps) {
  const sessions = useSessionStore((s) => s.availableSessions);
  const loading = useSessionStore((s) => s.sessionsLoading);
  const currentSessionId = useSessionStore((s) => s.sessionId);
  const sessionTitles = useSessionStore((s) => s.sessionTitles);
  const connectionState = useSessionStore((s) => s.connectionState);
  const groups = useSessionStore((s) => s.groups);
  const { setOpenMobile } = useSidebar();
  const router = useRouter();
  const [view, setView] = useState<ViewMode>("timeline");
  const [showArchived, setShowArchived] = useState(false);

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
    try {
      localStorage.setItem("mew.sessionId", newId);
    } catch {
      /* localStorage may be unavailable (e.g. private mode); ignore */
    }
    router.navigate({ to: "/session/$sessionId", params: { sessionId: newId } });
    setOpenMobile(false);
  };

  const handleAttach = (sessionId: string) => {
    try {
      localStorage.setItem("mew.sessionId", sessionId);
    } catch {
      /* localStorage may be unavailable (e.g. private mode); ignore */
    }
    router.navigate({ to: "/session/$sessionId", params: { sessionId } });
    setOpenMobile(false);
  };

  const handleArchive = (sessionId: string, archived: boolean) => {
    if (!client) return;
    client.archiveSession(sessionId, archived);
    const updated = useSessionStore.getState().availableSessions.map((s) =>
      s.session_id === sessionId ? { ...s, archived } : s,
    );
    useSessionStore.getState().setAvailableSessions(updated);
  };

  const handlePin = (sessionId: string, pinned: boolean) => {
    if (!client) return;
    client.pinSession(sessionId, pinned);
    const updated = useSessionStore.getState().availableSessions.map((s) =>
      s.session_id === sessionId ? { ...s, pinned } : s,
    );
    useSessionStore.getState().setAvailableSessions(updated);
  };

  const visibleSessions = useMemo(
    () => sessions.filter((s) => showArchived || !s.archived),
    [sessions, showArchived],
  );

  const sorted = useMemo(
    () =>
      [...visibleSessions].sort((a, b) => {
        const sp = statePriority(a, currentSessionId ?? undefined) - statePriority(b, currentSessionId ?? undefined);
        if (sp !== 0) return sp;
        return (b.last_message_at ?? b.created_at) - (a.last_message_at ?? a.created_at);
      }),
    [visibleSessions, currentSessionId],
  );

  const continueTarget = sorted.find(
    (s) => s.session_id !== currentSessionId && s.state === "idle" && s.model && !s.archived,
  );

  const grouped = useMemo(() => {
    if (view === "workspace") {
      const map = new Map<string, SessionInfo[]>();
      for (const s of sorted) {
        const key = deriveWorkspaceName(s.cwd);
        const list = map.get(key) ?? [];
        list.push(s);
        map.set(key, list);
      }
      return Array.from(map.entries());
    }
    if (view === "grouped") {
      const map = new Map<string, SessionInfo[]>();
      const ungrouped: SessionInfo[] = [];
      for (const s of sorted) {
        if (s.group_id) {
          const group = groups.find((g) => g.id === s.group_id);
          const key = group?.name ?? "Unknown";
          const list = map.get(key) ?? [];
          list.push(s);
          map.set(key, list);
        } else {
          ungrouped.push(s);
        }
      }
      const result = Array.from(map.entries());
      if (ungrouped.length > 0) result.push(["Ungrouped", ungrouped]);
      return result;
    }
    return [["All Sessions", sorted] as [string, SessionInfo[]]];
  }, [sorted, view, groups]);

  return (
    <Sidebar side="left" variant="floating" collapsible="icon">
      <SidebarHeader className="px-3 py-2">
        <div className="flex items-center justify-between">
          <div className="flex min-w-0 flex-col">
            <span className="text-xs font-semibold text-sidebar-foreground">mew</span>
            <SessionLabel />
          </div>
          <div className="flex items-center gap-0.5">
            <ViewSwitcher view={view} onChange={setView} />
            <Button variant="ghost" size="icon" className="h-7 w-7" onClick={() => setShowArchived((v) => !v)} title={showArchived ? "Hide archived" : "Show archived"}>
              <Archive className="h-3.5 w-3.5" />
            </Button>
            <Button variant="ghost" size="icon" className="h-7 w-7" onClick={() => router.navigate({ to: "/settings" })} title="Settings">
              <Settings className="h-3.5 w-3.5" />
            </Button>
          </div>
        </div>
      </SidebarHeader>
      <SidebarContent>
        <SidebarGroup>
          <div className="space-y-1.5 p-2">
            <Button onClick={handleNewSession} disabled={!client} variant="default" size="sm" className="w-full">
              <Plus className="h-3.5 w-3.5" />
              New session
            </Button>
            {continueTarget && (
              <Button onClick={() => handleAttach(continueTarget.session_id)} variant="outline" size="sm" className="w-full" title={`Resume ${deriveTitle(continueTarget)}`}>
                <RotateCcw className="h-3.5 w-3.5" />
                Continue latest
              </Button>
            )}
          </div>
        </SidebarGroup>
        {grouped.map(([label, items]) => (
          <SidebarGroup key={label}>
            {view !== "timeline" && (
              <SidebarGroupLabel className="flex items-center gap-1">
                {view === "workspace" && <Folder className="h-3 w-3" />}
                {label}
                <span className="text-muted-foreground">({items.length})</span>
              </SidebarGroupLabel>
            )}
            <SidebarGroupContent>
              {loading && sorted.length === 0 && (
                <div className="px-3 py-4 text-center text-xs text-muted-foreground">Loading sessions…</div>
              )}
              {!loading && sorted.length === 0 && (
                <div className="px-3 py-4 text-center text-xs text-muted-foreground">No sessions yet</div>
              )}
              <SidebarMenu className="gap-2">
                {items.map((s) => (
                  <SidebarMenuItem key={s.session_id}>
                    <SessionRow
                      session={s}
                      isActive={s.session_id === currentSessionId}
                      title={sessionTitles.get(s.session_id) ?? deriveTitle(s)}
                      onClick={() => handleAttach(s.session_id)}
                      onArchive={(archived) => handleArchive(s.session_id, archived)}
                      onPin={(pinned) => handlePin(s.session_id, pinned)}
                    />
                  </SidebarMenuItem>
                ))}
              </SidebarMenu>
            </SidebarGroupContent>
          </SidebarGroup>
        ))}
      </SidebarContent>
    </Sidebar>
  );
}

function SessionRow({ session: s, isActive, title, onClick, onArchive, onPin }: {
  session: SessionInfo; isActive: boolean; title: string;
  onClick: () => void; onArchive: (a: boolean) => void; onPin: (p: boolean) => void;
}) {
  return (
    <div className="group relative">
      <SidebarMenuButton isActive={isActive} onClick={onClick} className="flex-col items-start gap-0.5 pb-10">
        <div className="flex w-full items-center justify-between gap-1.5">
          <div className="flex items-center gap-1.5 min-w-0">
            <StatusDot state={s.state} failed={s.last_turn_failed} needsAttention={!!((s.pending_permissions ?? 0) + (s.pending_questions ?? 0))} />
            <span className="truncate text-xs font-medium">{title}</span>
          </div>
          {s.pinned && <Pin className="h-3 w-3 shrink-0 text-yellow-500" />}
        </div>
        <div className="flex items-center gap-1.5 text-[10px] text-muted-foreground">
          {s.cwd && (
            <span className="flex items-center gap-0.5 truncate">
              <Folder className="h-2.5 w-2.5" />
              {deriveWorkspaceName(s.cwd)}
            </span>
          )}
          {s.model && <span className="truncate font-mono">{shortModel(s.model)}</span>}
          <span>· {formatRelativeAge(s.last_message_at ?? s.created_at)}</span>
        </div>
        {(s.change_stats || s.usage) && (
          <div className="flex items-center gap-2 text-[9px] font-mono">
            {s.change_stats && (s.change_stats.added > 0 || s.change_stats.removed > 0) && (
              <>
                <span className="text-green-600 dark:text-green-400">+{s.change_stats.added}</span>
                <span className="text-red-500 dark:text-red-400">−{s.change_stats.removed}</span>
              </>
            )}
            {s.usage && s.usage.cost > 0 && (
              <span className="text-muted-foreground">{formatCost(s.usage.cost)}</span>
            )}
          </div>
        )}
      </SidebarMenuButton>
      <div className="absolute right-1 top-1 hidden gap-0.5 group-hover:flex">
        <Button variant="ghost" size="icon" className="h-5 w-5" onClick={(e) => { e.stopPropagation(); onPin(!s.pinned); }} title={s.pinned ? "Unpin" : "Pin"}>
          <Pin className={cn("h-3 w-3", s.pinned && "fill-yellow-500 text-yellow-500")} />
        </Button>
        <Button variant="ghost" size="icon" className="h-5 w-5" onClick={(e) => { e.stopPropagation(); onArchive(!s.archived); }} title={s.archived ? "Restore" : "Archive"}>
          <Archive className="h-3 w-3" />
        </Button>
      </div>
    </div>
  );
}

function statePriority(s: SessionInfo, currentSessionId?: string): number {
  // Exclude current session from the attention tier — its toast is already visible.
  if (s.session_id !== currentSessionId) {
    const pending = (s.pending_permissions ?? 0) + (s.pending_questions ?? 0);
    if (pending > 0) return 0; // needs attention
  }
  if (s.state === "running") return 1;
  if (s.state === "active") return 2;
  return 3;
}

function StatusDot({ state, failed, needsAttention }: { state: string; failed?: boolean; needsAttention?: boolean }) {
  if (needsAttention) {
    return (
      <span className="relative h-2 w-2 shrink-0" title="Needs attention">
        <span className="absolute inline-flex h-full w-full animate-ping rounded-full bg-amber-400 opacity-75" />
        <span className="relative inline-flex h-2 w-2 rounded-full bg-amber-500" />
      </span>
    );
  }
  if (failed) return <span className="h-2 w-2 shrink-0 rounded-full bg-red-500" title="Failed" />;
  if (state === "running") return (
    <span className="relative h-2 w-2 shrink-0" title="Running">
      <span className="absolute inline-flex h-full w-full animate-ping rounded-full bg-blue-400 opacity-75" />
      <span className="relative inline-flex h-2 w-2 rounded-full bg-blue-500" />
    </span>
  );
  if (state === "active") return <span className="h-2 w-2 shrink-0 rounded-full bg-green-500" title="Active" />;
  return <span className="h-2 w-2 shrink-0 rounded-full bg-muted-foreground/40" title="Idle" />;
}

function ViewSwitcher({ view, onChange }: { view: ViewMode; onChange: (v: ViewMode) => void }) {
  return (
    <div className="flex items-center gap-0.5 rounded-md bg-muted/50 p-0.5">
      {(["timeline", "workspace", "grouped"] as const).map((v) => (
        <Button key={v} variant={view === v ? "secondary" : "ghost"} size="sm" className="h-5 px-1.5 text-[9px] capitalize" onClick={() => onChange(v)}>
          {v.charAt(0).toUpperCase()}
        </Button>
      ))}
    </div>
  );
}

function SessionLabel() {
  const sessionId = useSessionStore((s) => s.sessionId);
  const titles = useSessionStore((s) => s.sessionTitles);
  const label = sessionId ? (titles.get(sessionId) ?? sessionId.slice(0, 10) + "…") : "no session";
  return <span className="truncate text-[10px] text-muted-foreground">{label}</span>;
}

function deriveTitle(s: SessionInfo): string {
  if (s.summary) return s.summary;
  if (s.model) return s.model.split("/").pop() ?? s.model;
  return s.session_id.slice(0, 8);
}

function deriveWorkspaceName(cwd?: string): string {
  if (!cwd) return "~";
  const parts = cwd.replace(/\/+$/, "").split("/");
  return parts[parts.length - 1] ?? cwd;
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

function formatCost(cost: number): string {
  if (cost < 0.01) return "<1¢";
  return `$${cost.toFixed(2)}`;
}
