import { useEffect, useState, useMemo } from "react";
import { useRouter } from "@tanstack/react-router";
import type { MewClient, SessionInfo, GroupInfo, ProjectInfo } from "@mew/web-client";
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
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  DropdownMenu,
  DropdownMenuTrigger,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuLabel,
} from "@/components/ui/dropdown-menu";
import {
  Plus,
  RotateCcw,
  Settings,
  Folder,
  FolderPlus,
  Archive,
  Pin,
  MoreHorizontal,
  Trash2,
  Check,
  FolderInput,
  ArrowUpDown,
} from "lucide-react";

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
  const projects = useSessionStore((s) => s.projects);
  const { setOpenMobile } = useSidebar();
  const router = useRouter();
  const [view, setView] = useState<ViewMode>("timeline");
  const [showArchived, setShowArchived] = useState(false);
  const [showProjectPicker, setShowProjectPicker] = useState(false);

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
    router.navigate({ to: "/session/$sessionId", params: { sessionId: newId } });
    setOpenMobile(false);
  };

  const handleNewSessionFromCwd = async (cwd: string) => {
    if (!client) return;
    useSessionStore.getState().reset();
    const newId = await client.newSession(cwd);
    localStorage.setItem("mew.sessionId", newId);
    router.navigate({ to: "/session/$sessionId", params: { sessionId: newId } });
    setShowProjectPicker(false);
    setOpenMobile(false);
  };

  const handleAttach = (sessionId: string) => {
    localStorage.setItem("mew.sessionId", sessionId);
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

  type GroupedSection = { label: string; items: SessionInfo[]; groupId?: string };

  const grouped = useMemo<GroupedSection[]>(() => {
    if (view === "workspace") {
      const map = new Map<string, SessionInfo[]>();
      for (const s of sorted) {
        const key = deriveWorkspaceName(s.cwd);
        const list = map.get(key) ?? [];
        list.push(s);
        map.set(key, list);
      }
      return Array.from(map.entries()).map(([label, items]) => ({ label, items }));
    }
    if (view === "grouped") {
      // Preserve group order from the store; ungrouped sessions land last.
      const map = new Map<string, { items: SessionInfo[]; groupId: string }>();
      const ungrouped: SessionInfo[] = [];
      for (const s of sorted) {
        if (s.group_id) {
          const group = groups.find((g) => g.id === s.group_id);
          const key = group?.id ?? s.group_id;
          const entry = map.get(key);
          if (entry) {
            entry.items.push(s);
          } else {
            map.set(key, { items: [s], groupId: key });
          }
        } else {
          ungrouped.push(s);
        }
      }
      const result: GroupedSection[] = Array.from(map.entries()).map(
        ([, { items, groupId }]) => {
          const g = groups.find((x) => x.id === groupId);
          return { label: g?.name ?? "Unknown", items, groupId };
        },
      );
      if (ungrouped.length > 0) result.push({ label: "Ungrouped", items: ungrouped });
      return result;
    }
    return [{ label: "All Sessions", items: sorted }];
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
            <div className="flex gap-1.5">
              <Button onClick={handleNewSession} disabled={!client} variant="default" size="sm" className="flex-1">
                <Plus className="h-3.5 w-3.5" />
                New session
              </Button>
              <Button
                variant="outline"
                size="sm"
                className="px-2"
                disabled={!client}
                title="New session from a project directory"
                onClick={() => {
                  if (client) {
                    client.listProjects();
                    useSessionStore.getState().setProjectsLoading(true);
                  }
                  setShowProjectPicker(true);
                }}
              >
                <FolderPlus className="h-3.5 w-3.5" />
              </Button>
            </div>
            {continueTarget && (
              <Button onClick={() => handleAttach(continueTarget.session_id)} variant="outline" size="sm" className="w-full" title={`Resume ${deriveTitle(continueTarget)}`}>
                <RotateCcw className="h-3.5 w-3.5" />
                Continue latest
              </Button>
            )}
          </div>
        </SidebarGroup>
        {view === "grouped" && (
          <SidebarGroup>
            <div className="px-2 pb-1">
              <NewGroupButton client={client} />
            </div>
          </SidebarGroup>
        )}
        {grouped.map((section) => (
          <SidebarGroup key={section.groupId ?? section.label}>
            {view !== "timeline" && (
              <SidebarGroupLabel className="flex items-center gap-1">
                {view === "workspace" && <Folder className="h-3 w-3" />}
                {view === "grouped" && section.groupId && (
                  <GroupColorSwatch groupId={section.groupId} groups={groups} />
                )}
                {section.label}
                <span className="text-muted-foreground">({section.items.length})</span>
                {view === "grouped" && section.groupId && (
                  <GroupActions groupId={section.groupId} groups={groups} client={client} />
                )}
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
                {section.items.map((s) => (
                  <SidebarMenuItem key={s.session_id}>
                    <SessionRow
                      session={s}
                      isActive={s.session_id === currentSessionId}
                      title={sessionTitles.get(s.session_id) ?? deriveTitle(s)}
                      onClick={() => handleAttach(s.session_id)}
                      onArchive={(archived) => handleArchive(s.session_id, archived)}
                      onPin={(pinned) => handlePin(s.session_id, pinned)}
                      groups={groups}
                      client={client}
                    />
                  </SidebarMenuItem>
                ))}
              </SidebarMenu>
            </SidebarGroupContent>
          </SidebarGroup>
        ))}
      </SidebarContent>
      <ProjectPickerModal
        open={showProjectPicker}
        onOpenChange={setShowProjectPicker}
        projects={projects}
        onSelect={handleNewSessionFromCwd}
      />
    </Sidebar>
  );
}

function SessionRow({ session: s, isActive, title, onClick, onArchive, onPin, groups, client }: {
  session: SessionInfo; isActive: boolean; title: string;
  onClick: () => void; onArchive: (a: boolean) => void; onPin: (p: boolean) => void;
  groups: GroupInfo[]; client: MewClient | null;
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
        <MoveToGroupDropdown sessionId={s.session_id} currentGroupId={s.group_id} groups={groups} client={client} />
      </div>
    </div>
  );
}

/** "New group" affordance: prompts for a name, then calls createGroup. */
function NewGroupButton({ client }: { client: MewClient | null }) {
  const handleCreate = () => {
    if (!client) return;
    const name = window.prompt("New group name");
    if (!name?.trim()) return;
    client.createGroup(name.trim());
    // The groups-changed broadcast will refresh the store.
  };
  return (
    <Button onClick={handleCreate} disabled={!client} variant="outline" size="sm" className="w-full">
      <FolderPlus className="h-3.5 w-3.5" />
      New group
    </Button>
  );
}

/** A small color swatch that reflects the group color, used in the label. */
function GroupColorSwatch({ groupId, groups }: { groupId: string; groups: GroupInfo[] }) {
  const group = groups.find((g) => g.id === groupId);
  const color = group?.color ?? "gray";
  return (
    <span
      className={cn("h-2.5 w-2.5 shrink-0 rounded-full", swatchClass(color))}
      title={group?.color ?? "default"}
    />
  );
}

/** Inline rename, color picker, reorder, and delete actions for a group. */
function GroupActions({
  groupId,
  groups,
  client,
}: {
  groupId: string;
  groups: GroupInfo[];
  client: MewClient | null;
}) {
  const group = groups.find((g) => g.id === groupId);
  if (!group || !client) return null;

  const handleRename = () => {
    const name = window.prompt("Rename group", group.name);
    if (name && name.trim() && name.trim() !== group.name) {
      client.updateGroup(groupId, { name: name.trim() });
    }
  };

  const handleColor = (color: string) => {
    client.updateGroup(groupId, { color });
  };

  const handleClearColor = () => {
    client.updateGroup(groupId, { color: null });
  };

  const handleDelete = () => {
    if (!window.confirm(`Delete group "${group.name}"? Sessions will be ungrouped.`)) return;
    client.deleteGroup(groupId);
  };

  const handleReorder = (delta: number) => {
    client.updateGroup(groupId, { order: group.order + delta });
  };

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <button
          className="ml-1 rounded p-0.5 text-muted-foreground hover:bg-accent hover:text-foreground"
          onClick={(e) => e.stopPropagation()}
          title="Group actions"
        >
          <MoreHorizontal className="h-3 w-3" />
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" className="w-44" onClick={(e) => e.stopPropagation()}>
        <DropdownMenuLabel className="text-[10px]">Group</DropdownMenuLabel>
        <DropdownMenuItem onClick={handleRename}>
          Rename…
        </DropdownMenuItem>
        <DropdownMenuSeparator />
        <DropdownMenuLabel className="text-[10px]">Color</DropdownMenuLabel>
        <div className="grid grid-cols-6 gap-1 p-1">
          {GROUP_COLORS.map((c) => (
            <button
              key={c}
              className={cn(
                "flex h-4 w-4 items-center justify-center rounded-full",
                swatchClass(c),
              )}
              onClick={() => handleColor(c)}
              title={c}
            >
              {group.color === c && <Check className="h-2.5 w-2.5 text-white" />}
            </button>
          ))}
        </div>
        <DropdownMenuItem onClick={handleClearColor}>
          Clear color
        </DropdownMenuItem>
        <DropdownMenuSeparator />
        <DropdownMenuLabel className="text-[10px]">Reorder</DropdownMenuLabel>
        <div className="flex gap-1 p-1">
          <Button variant="outline" size="sm" className="h-6 flex-1" onClick={() => handleReorder(-1)}>
            <ArrowUpDown className="h-3 w-3" /> Up
          </Button>
          <Button variant="outline" size="sm" className="h-6 flex-1" onClick={() => handleReorder(1)}>
            <ArrowUpDown className="h-3 w-3 scale-y-[-1]" /> Down
          </Button>
        </div>
        <DropdownMenuSeparator />
        <DropdownMenuItem onClick={handleDelete} className="text-destructive focus:text-destructive">
          <Trash2 className="h-3 w-3" />
          Delete group
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

/** Per-session "Move to group" dropdown, added to the hover actions. */
function MoveToGroupDropdown({
  sessionId,
  currentGroupId,
  groups,
  client,
}: {
  sessionId: string;
  currentGroupId?: string;
  groups: GroupInfo[];
  client: MewClient | null;
}) {
  if (!client) return null;
  const sortedGroups = [...groups].sort((a, b) => a.order - b.order);

  const handleAssign = (groupId: string | null) => {
    client.assignSessionGroup(sessionId, groupId);
  };

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button
          variant="ghost"
          size="icon"
          className="h-5 w-5"
          onClick={(e) => e.stopPropagation()}
          title="Move to group"
        >
          <FolderInput className="h-3 w-3" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-48" onClick={(e) => e.stopPropagation()}>
        <DropdownMenuLabel className="text-[10px]">Move to group</DropdownMenuLabel>
        <DropdownMenuSeparator />
        <DropdownMenuItem onClick={() => handleAssign(null)}>
          {!currentGroupId && <Check className="h-3 w-3" />}
          Ungrouped
        </DropdownMenuItem>
        {sortedGroups.map((g) => (
          <DropdownMenuItem key={g.id} onClick={() => handleAssign(g.id)}>
            <span className={cn("h-2.5 w-2.5 rounded-full", swatchClass(g.color ?? "gray"))} />
            <span className="truncate">{g.name}</span>
            {currentGroupId === g.id && <Check className="ml-auto h-3 w-3" />}
          </DropdownMenuItem>
        ))}
        {sortedGroups.length === 0 && (
          <div className="px-2 py-1.5 text-[10px] text-muted-foreground">
            No groups yet
          </div>
        )}
      </DropdownMenuContent>
    </DropdownMenu>
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

/** The palette offered in the group color picker. */
const GROUP_COLORS = [
  "red",
  "orange",
  "amber",
  "green",
  "teal",
  "blue",
  "indigo",
  "purple",
  "pink",
  "gray",
] as const;

/** Map a color name to a tailwind background class for a swatch. */
function swatchClass(color: string): string {
  const map: Record<string, string> = {
    red: "bg-red-500",
    orange: "bg-orange-500",
    amber: "bg-amber-500",
    green: "bg-green-500",
    teal: "bg-teal-500",
    blue: "bg-blue-500",
    indigo: "bg-indigo-500",
    purple: "bg-purple-500",
    pink: "bg-pink-500",
    gray: "bg-muted-foreground",
  };
  return map[color] ?? "bg-muted-foreground";
}

function ProjectPickerModal({
  open,
  onOpenChange,
  projects,
  onSelect,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  projects: ProjectInfo[];
  onSelect: (cwd: string) => void;
}) {
  const [manualPath, setManualPath] = useState("");

  const sortedProjects = useMemo(
    () =>
      [...projects].sort(
        (a, b) => (b.last_used_at ?? 0) - (a.last_used_at ?? 0),
      ),
    [projects],
  );

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>New session from project</DialogTitle>
        </DialogHeader>
        <div className="space-y-3">
          {sortedProjects.length > 0 && (
            <div className="space-y-1">
              <p className="text-xs font-medium text-muted-foreground">Recent projects</p>
              <div className="max-h-64 space-y-1 overflow-y-auto">
                {sortedProjects.map((p) => (
                  <button
                    key={p.path}
                    onClick={() => onSelect(p.path)}
                    className="flex w-full items-center gap-2 rounded-md border border-border px-3 py-2 text-left text-sm hover:bg-muted/50 transition-colors"
                  >
                    <Folder className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
                    <div className="min-w-0 flex-1">
                      <div className="truncate font-medium">{p.display_name}</div>
                      <div className="truncate text-xs text-muted-foreground">{p.path}</div>
                    </div>
                    <span className="shrink-0 text-xs text-muted-foreground">
                      {p.session_count} session{p.session_count === 1 ? "" : "s"}
                    </span>
                  </button>
                ))}
              </div>
            </div>
          )}
          {sortedProjects.length === 0 && (
            <p className="text-sm text-muted-foreground">
              No recent projects found. Enter a path below to start a session in a specific directory.
            </p>
          )}
          <div className="space-y-2 border-t border-border pt-3">
            <p className="text-xs font-medium text-muted-foreground">Or enter a path:</p>
            <div className="flex gap-2">
              <input
                type="text"
                value={manualPath}
                onChange={(e) => setManualPath(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter" && manualPath.trim()) {
                    onSelect(manualPath.trim());
                  }
                }}
                placeholder="/path/to/project"
                className="flex-1 rounded-md border border-border bg-background px-3 py-1.5 text-sm outline-none focus:ring-1 focus:ring-ring"
              />
              <Button
                size="sm"
                disabled={!manualPath.trim()}
                onClick={() => onSelect(manualPath.trim())}
              >
                Open
              </Button>
            </div>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}
