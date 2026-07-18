import { useState, useEffect, type ReactNode } from "react";
import {
  AlertCircle,
  Activity,
  CheckCircle2,
  CircleAlert,
  Folder,
  GitBranch,
  GitPullRequest,
  HelpCircle,
  Loader2,
  MessageCircleQuestion,
  PlayCircle,
  Pin,
  ShieldAlert,
  Terminal,
  Plus,
  X,
  XCircle,
  ArrowUpRight,
  Globe,
  type LucideIcon,
} from "lucide-react";
import { useSessionStore, type TodoItem, type SubagentInfo, type PendingAskUser, type JobInfo } from "../stores/session";
import { cn } from "../lib/utils";
import { compareSessionsByAttention, getSessionAttention, type SessionAttention } from "../lib/attention";
import { navigateToSession } from "../lib/router-ref";
import type { AlertKind, GitEntry, SessionInfo } from "@mew/web-client";
import { useIsMobile } from "../hooks/use-mobile";
import {
  createWorkbenchTab,
  DEFAULT_WORKBENCH_TABS,
  getActiveWorkbenchTab,
  workbenchTabsReducer,
  type WorkbenchTab,
  type WorkbenchTabsAction,
  type WorkbenchTabKind,
  type WorkbenchTabsState,
} from "../lib/workbench-tabs";
import { AskUserForm } from "./ask-user-card";
import { getClient } from "../lib/client-ref";
import { FileTreePanel, ChangesPanel } from "./file-tree";
import { BrowserPanel } from "./browser-panel";
import {
  Sheet,
  SheetContent,
} from "@/components/ui/sheet";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";

type ActivityTabKey = "todos" | "subagents" | "questions" | "jobs";

const ACTIVITY_TABS: ReadonlyArray<{
  key: ActivityTabKey;
  label: string;
  icon: LucideIcon;
}> = [
  { key: "todos", label: "Plan", icon: CheckCircle2 },
  { key: "subagents", label: "Agents", icon: Loader2 },
  { key: "questions", label: "Questions", icon: HelpCircle },
  { key: "jobs", label: "Jobs", icon: Terminal },
];

interface RightRailProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  mode?: "sheet" | "dock";
  workbenchTabs?: WorkbenchTabsState;
  onWorkbenchTabsChange?: (tabs: WorkbenchTabsState) => void;
  onWorkbenchTabsAction?: (action: WorkbenchTabsAction) => void;
}

export function RightRail({
  open,
  onOpenChange,
  mode = "sheet",
  workbenchTabs = DEFAULT_WORKBENCH_TABS,
  onWorkbenchTabsChange,
  onWorkbenchTabsAction,
}: RightRailProps) {
  const isMobile = useIsMobile();
  const [activeTab, setActiveTab] = useState<ActivityTabKey>("todos");
  const [addMenuOpen, setAddMenuOpen] = useState(false);
  const [uncontrolledWorkbenchTabs, setUncontrolledWorkbenchTabs] = useState(DEFAULT_WORKBENCH_TABS);
  const selectedWorkbenchTabs = onWorkbenchTabsChange || onWorkbenchTabsAction
    ? workbenchTabs
    : uncontrolledWorkbenchTabs;
  const selectedWorkbenchTab = getActiveWorkbenchTab(selectedWorkbenchTabs);
  const setSelectedWorkbenchTabs = (next: WorkbenchTabsState) => {
    if (onWorkbenchTabsChange) onWorkbenchTabsChange(next);
    else setUncontrolledWorkbenchTabs(next);
  };
  const applyWorkbenchAction = (action: WorkbenchTabsAction) => {
    if (onWorkbenchTabsAction) onWorkbenchTabsAction(action);
    else setSelectedWorkbenchTabs(workbenchTabsReducer(selectedWorkbenchTabs, action));
  };
  const todos = useSessionStore((s) => s.todos);
  const subagents = useSessionStore((s) => s.subagents);
  const questions = useSessionStore((s) => s.pendingAskUser);
  const flaggedFiles = useSessionStore((s) => s.flaggedFiles);
  const jobs = useSessionStore((s) => s.jobs);
  const gitStatus = useSessionStore((s) => s.gitStatus);
  const sessionId = useSessionStore((s) => s.sessionId);
  const sessionCwd = useSessionStore((s) => s.sessionCwd);
  const availableSessions = useSessionStore((s) => s.availableSessions);
  const alerts = useSessionStore((s) => s.alerts);
  const sessionHasWorkspace = Boolean(
    sessionCwd || availableSessions.find((s) => s.session_id === sessionId)?.cwd,
  );

  const addWorkbenchTab = (kind: WorkbenchTabKind) => {
    const tab = createWorkbenchTab(kind, {
      sessionId: sessionId ?? undefined,
      cwd: sessionCwd ?? undefined,
    });
    applyWorkbenchAction({ type: "add", tab });
  };

  const alertKindsBySession = new Map<string, AlertKind[]>();
  for (const alert of alerts) {
    const kinds = alertKindsBySession.get(alert.sessionId) ?? [];
    kinds.push(alert.kind);
    alertKindsBySession.set(alert.sessionId, kinds);
  }
  const attentionSessions = [...availableSessions]
    .filter((session) => !session.archived)
    .filter((session) => getSessionAttention(session, alertKindsBySession.get(session.session_id)).length > 0)
    .sort((a, b) => compareSessionsByAttention(a, b, alertKindsBySession));
  const attentionCount = attentionSessions.length;

  // When the Files or Changes tab is opened, enable workspace watching so
  // fs_changed events keep the listings/git status live. Stop watching
  // when those tabs are closed to avoid unnecessary traffic.
  useEffect(() => {
    if (!open || !sessionId || !sessionHasWorkspace) return;
    if (selectedWorkbenchTab.kind !== "changes" && selectedWorkbenchTab.kind !== "file") return;
    const client = getClient();
    if (!client) return;
    client.watchWorkspace(sessionId, true);
    // Also seed the git status when the Changes tab opens.
    if (selectedWorkbenchTab.kind === "changes") client.gitStatus(sessionId);
    return () => {
      client.watchWorkspace(sessionId, false);
    };
  }, [open, activeTab, selectedWorkbenchTab, sessionId, sessionHasWorkspace]);

  const activeCounts = {
    todos: todos.filter((t) => t.status === "in_progress" || t.status === "pending").length,
    subagents: [...subagents.values()].filter((s) => s.status === "running").length,
    questions: questions.length,
    jobs: [...jobs.values()].filter(
      (j) => j.state !== "done" && j.state !== "failed" && j.state !== "cancelled",
    ).length,
  };
  const totalActive = activeCounts.todos + activeCounts.subagents + activeCounts.questions + activeCounts.jobs;

  useEffect(() => {
    if (!open) return;
    const currentCount = activeCounts[activeTab];
    if (currentCount > 0) return;
    const preferred = ACTIVITY_TABS.map((tab) => tab.key).find(
      (key) => activeCounts[key] > 0,
    );
    setActiveTab(preferred ?? "todos");
  }, [open]);

  useEffect(() => {
    if (!open) return;
    const handleKeyDown = (event: KeyboardEvent) => {
      if (!(event.metaKey || event.ctrlKey)) return;
      const key = event.key.toLowerCase();
      if (key === "w" && selectedWorkbenchTab.closable) {
        event.preventDefault();
        applyWorkbenchAction({ type: "close", id: selectedWorkbenchTab.id });
      } else if (key === "t") {
        event.preventDefault();
        if (selectedWorkbenchTab.kind === "browser") addWorkbenchTab("browser");
        else setAddMenuOpen(true);
      } else if (/^[1-9]$/.test(key)) {
        const tab = selectedWorkbenchTabs.tabs[Number(key) - 1];
        if (!tab) return;
        event.preventDefault();
        applyWorkbenchAction({ type: "select", id: tab.id });
      }
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [open, selectedWorkbenchTab, selectedWorkbenchTabs]);

  const activityContent = (
    <div className="flex min-h-0 flex-1 flex-col">
      {attentionSessions.length > 0 && (
        <AttentionQueue
          sessions={attentionSessions}
          alertKindsBySession={alertKindsBySession}
          onOpenSession={(sessionId) => {
            navigateToSession(sessionId);
            onOpenChange(false);
          }}
        />
      )}

      {flaggedFiles.length > 0 && <PinnedContext files={flaggedFiles} />}

      <div
        role="tablist"
        aria-label="Activity sections"
        className="flex min-w-0 flex-wrap gap-1 border-b border-border px-3 py-2"
      >
        {ACTIVITY_TABS.map(({ key, label, icon: Icon }) => (
          <TabButton
            key={key}
            id={`activity-tab-${key}`}
            label={label}
            icon={Icon}
            count={activeCounts[key]}
            active={activeTab === key}
            onClick={() => setActiveTab(key)}
          />
        ))}
      </div>

      <div
        id={`activity-panel-${activeTab}`}
        role="tabpanel"
        aria-labelledby={`activity-tab-${activeTab}`}
        className="min-h-0 flex-1 overflow-y-auto p-3"
      >
        {activeTab === "todos" && <TodoRailPanel todos={todos} />}
        {activeTab === "subagents" && <SubagentRailPanel subagents={subagents} />}
        {activeTab === "questions" && <QuestionsRailPanel questions={questions} />}
        {activeTab === "jobs" && <JobsRailPanel jobs={jobs} />}
      </div>
    </div>
  );

  const workbenchContent = (
    <div className="flex min-h-0 flex-1 flex-col">
      <WorkbenchHeader
        totalActive={totalActive}
        attentionCount={attentionCount}
        onClose={() => onOpenChange(false)}
      />
      <Tabs
        value={selectedWorkbenchTab.id}
        onValueChange={(id) => applyWorkbenchAction({ type: "select", id })}
        className="flex min-h-0 flex-1 flex-col"
      >
        <div className="flex min-w-0 shrink-0 items-center border-b border-border">
          <TabsList aria-label="Workbench sections" className="h-11 min-w-0 flex-1 justify-start gap-0 overflow-x-auto rounded-none border-0 bg-transparent p-0">
            {selectedWorkbenchTabs.tabs.map((tab) => {
              const meta = workbenchTabMeta(tab.kind);
              const Icon = meta.icon;
              const count = tab.kind === "activity" ? totalActive : tab.kind === "changes" ? gitStatus.length : 0;
              return (
                <div key={tab.id} className={cn("group flex h-full shrink-0 items-center border-b-2", tab.id === selectedWorkbenchTab.id ? "border-primary" : "border-transparent")}>
                  <TabsTrigger
                    value={tab.id}
                    aria-label={tab.title}
                    onClick={() => applyWorkbenchAction({ type: "select", id: tab.id })}
                    className="h-full gap-1.5 rounded-none border-0 border-transparent px-2.5 text-[11px] font-medium text-muted-foreground shadow-none transition-colors duration-150 data-[state=active]:bg-transparent data-[state=active]:text-foreground data-[state=active]:shadow-none"
                  >
                    <Icon className="h-3.5 w-3.5" aria-hidden="true" />
                    <span className="max-w-32 truncate">{tab.title}</span>
                    {count > 0 && <span className="flex h-4 min-w-4 items-center justify-center rounded-full bg-primary/10 px-1 text-[9px] text-primary">{count}</span>}
                  </TabsTrigger>
                  {tab.closable && (
                    <button
                      type="button"
                      onClick={() => applyWorkbenchAction({ type: "close", id: tab.id })}
                      className="motion-pressable mr-1 rounded p-1 text-muted-foreground opacity-0 transition-opacity hover:bg-accent hover:text-foreground focus-visible:opacity-100 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring group-hover:opacity-100"
                      aria-label={`Close ${tab.title}`}
                      title={`Close ${tab.title}`}
                    >
                      <X className="h-3 w-3" />
                    </button>
                  )}
                </div>
              );
            })}
          </TabsList>
          <div className="relative shrink-0">
            <button
              type="button"
              onClick={() => setAddMenuOpen((value) => !value)}
              className="motion-pressable ml-1 rounded p-1.5 text-muted-foreground hover:bg-accent hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
              aria-label="Add workbench tab"
              title="Add workbench tab"
              aria-expanded={addMenuOpen}
              aria-haspopup="menu"
            >
              <Plus className="h-3.5 w-3.5" />
            </button>
            {addMenuOpen && <div role="menu" aria-label="Add workbench tab" className="motion-enter absolute right-0 top-full z-50 mt-1 min-w-36 rounded-md border border-border bg-popover p-1 text-popover-foreground shadow-lg">
              {(["browser", "terminal", "file", "changes", "review"] as WorkbenchTabKind[]).map((kind) => {
                const meta = workbenchTabMeta(kind);
                const Icon = meta.icon;
                return (
                  <button
                    type="button"
                    role="menuitem"
                    key={kind}
                    onClick={() => {
                      addWorkbenchTab(kind);
                      setAddMenuOpen(false);
                    }}
                    className="motion-pressable flex w-full items-center gap-2 rounded-sm px-2 py-1.5 text-left text-xs outline-none hover:bg-accent hover:text-accent-foreground focus-visible:bg-accent focus-visible:text-accent-foreground"
                  >
                    <Icon className="h-4 w-4" />
                    {meta.label}
                  </button>
                );
              })}
            </div>}
          </div>
        </div>
        <div className="min-h-0 flex-1 overflow-hidden">
          {selectedWorkbenchTabs.tabs.map((tab) => (
            <TabsContent
              key={tab.id}
              value={tab.id}
              forceMount
              hidden={tab.id !== selectedWorkbenchTab.id}
              className={cn(
                "mt-0 h-full min-h-0 overflow-hidden focus-visible:outline-none",
                tab.id !== selectedWorkbenchTab.id && "hidden",
              )}
            >
              {renderWorkbenchSurface(tab, tab.id === selectedWorkbenchTab.id)}
            </TabsContent>
          ))}
        </div>
      </Tabs>
    </div>
  );

  function renderWorkbenchSurface(tab: WorkbenchTab, active: boolean) {
    switch (tab.kind) {
      case "activity":
        return <div className="flex h-full min-h-0 flex-col">{activityContent}</div>;
      case "browser":
        return (
          <div className="h-full min-h-0 p-3">
            <BrowserPanel
              client={getClient()}
              active={open && active}
              tab={tab}
              onTabChange={(patch) => applyWorkbenchAction({ type: "update", id: tab.id, patch })}
            />
          </div>
        );
      case "file":
        return <div className="h-full min-h-0 overflow-y-auto p-3"><FileTreePanel hasWorkspace={sessionHasWorkspace} /></div>;
      case "changes":
        return <div className="h-full min-h-0 overflow-y-auto p-3"><ChangesPanel gitStatus={gitStatus} hasWorkspace={sessionHasWorkspace} /></div>;
      case "review":
        return <div className="h-full min-h-0 overflow-y-auto p-3"><ReviewPanel gitStatus={gitStatus} hasWorkspace={sessionHasWorkspace} /></div>;
      case "terminal":
        return <TerminalWorkbenchPanel jobs={jobs} />;
    }
  }

  if (mode === "dock" && !isMobile) {
    return (
      <aside
        aria-label="Workspace workbench"
        data-open={open}
        aria-hidden={!open}
        className="motion-panel-size flex h-full min-w-0 overflow-hidden border-l border-border bg-panel-background md:my-2 md:mr-2 md:rounded-[var(--radius-shell)] md:shadow-sm"
      >
        <div className="flex min-w-0 flex-1 flex-col">{workbenchContent}</div>
      </aside>
    );
  }

  return (
    <Sheet open={open} onOpenChange={onOpenChange}>
      <SheetContent
        side="right"
        className="w-full gap-0 p-0 sm:inset-y-2 sm:right-2 sm:h-auto sm:max-h-[calc(100vh-1rem)] sm:w-[min(24rem,calc(100vw-2rem))] sm:max-w-md sm:rounded-xl"
      >
        {workbenchContent}
      </SheetContent>
    </Sheet>
  );
}

function WorkbenchHeader({
  totalActive,
  attentionCount,
  onClose,
}: {
  totalActive: number;
  attentionCount: number;
  onClose: () => void;
}) {
  return (
    <div className="flex shrink-0 items-start gap-3 border-b border-border px-4 py-3">
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2">
          <Activity className="h-3.5 w-3.5 text-primary" />
          <h2 className="text-sm font-semibold text-foreground">Workbench</h2>
        </div>
        <p className="mt-1 text-[11px] text-muted-foreground">
          {totalActive > 0
            ? `${totalActive} active ${totalActive === 1 ? "item" : "items"}`
            : "Your coding workspace, in context"}
        </p>
      </div>
      <div className="flex shrink-0 items-center gap-1.5">
        {totalActive > 0 && (
          <span className="flex h-5 min-w-5 items-center justify-center rounded-full bg-primary px-1.5 text-[10px] font-semibold text-primary-foreground">
            {totalActive}
          </span>
        )}
        {attentionCount > 0 && (
          <span className="flex h-5 min-w-5 items-center justify-center rounded-full bg-amber-500/15 px-1.5 text-[10px] font-semibold text-amber-700 dark:text-amber-300" aria-label={`${attentionCount} sessions need attention`}>
            {attentionCount}
          </span>
        )}
        <button
          type="button"
          onClick={onClose}
          className="motion-pressable rounded-md p-1.5 text-muted-foreground hover:bg-accent hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          aria-label="Close workbench"
          title="Close workbench"
        >
          <X className="h-3.5 w-3.5" />
        </button>
      </div>
    </div>
  );
}

function PinnedContext({ files }: { files: { path: string; reason?: string }[] }) {
  return (
    <div className="motion-enter border-b border-border px-3 py-2.5">
      <div className="rounded-lg bg-muted/35 px-2.5 py-2">
        <div className="flex items-center gap-1.5 text-[10px] font-semibold text-foreground">
          <Pin className="h-3 w-3 text-primary" />
          Pinned context
          <span className="text-muted-foreground">{files.length}</span>
        </div>
        <div className="mt-1.5 space-y-1">
          {files.map((file) => (
            <div key={file.path} className="group flex min-w-0 items-center gap-1.5">
              <span className="min-w-0 truncate font-mono text-[10px] text-muted-foreground">{file.path}</span>
              {file.reason && <span className="shrink-0 text-[9px] text-muted-foreground/60">{file.reason}</span>}
              <button
                type="button"
                className="ml-auto shrink-0 rounded p-1 text-muted-foreground opacity-0 transition-opacity hover:bg-background hover:text-foreground focus-visible:opacity-100 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring group-hover:opacity-100"
                onClick={() => {
                  const client = getClient();
                  const sessionId = useSessionStore.getState().sessionId;
                  if (sessionId) client?.unflagFile(sessionId, file.path);
                }}
                title={`Remove ${file.path} from pinned context`}
                aria-label={`Remove ${file.path} from pinned context`}
              >
                <X className="h-3 w-3" />
              </button>
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}

function ReviewPanel({
  gitStatus,
  hasWorkspace,
}: {
  gitStatus: GitEntry[];
  hasWorkspace: boolean;
}) {
  if (!hasWorkspace) {
    return (
      <div className="flex h-full min-h-48 flex-col items-center justify-center px-6 text-center">
        <GitPullRequest className="h-5 w-5 text-muted-foreground" />
        <h3 className="mt-3 text-xs font-semibold text-foreground">Choose a workspace first</h3>
        <p className="mt-1 max-w-[18rem] text-[11px] leading-relaxed text-muted-foreground">
          Review will use the active project&apos;s working tree and selected files.
        </p>
      </div>
    );
  }

  if (gitStatus.length === 0) {
    return (
      <div className="flex h-full min-h-48 flex-col items-center justify-center px-6 text-center">
        <GitPullRequest className="h-5 w-5 text-muted-foreground" />
        <h3 className="mt-3 text-xs font-semibold text-foreground">Nothing to review</h3>
        <p className="mt-1 max-w-[18rem] text-[11px] leading-relaxed text-muted-foreground">
          Changes to the working tree will appear here when they are ready for review.
        </p>
      </div>
    );
  }

  return (
    <div className="space-y-3">
      <div>
        <p className="text-[10px] font-semibold uppercase tracking-[0.12em] text-muted-foreground">Review queue</p>
        <h3 className="mt-1 text-sm font-semibold text-foreground">{gitStatus.length} changed files</h3>
        <p className="mt-1 text-[11px] leading-relaxed text-muted-foreground">
          Select a file from Changes to inspect the working tree before asking mew for a review.
        </p>
      </div>
      <div className="space-y-1">
        {gitStatus.slice(0, 8).map((entry) => (
          <div key={entry.path} className="flex min-w-0 items-center gap-2 rounded-md bg-muted/35 px-2 py-1.5 text-[11px]">
            <span className="shrink-0 font-mono text-muted-foreground">{entry.status.slice(0, 1).toUpperCase()}</span>
            <span className="min-w-0 truncate font-mono text-foreground">{entry.path}</span>
          </div>
        ))}
      </div>
    </div>
  );
}

function workbenchTabMeta(kind: WorkbenchTabKind): {
  label: string;
  icon: LucideIcon;
} {
  switch (kind) {
    case "activity":
      return { label: "Activity", icon: Activity };
    case "browser":
      return { label: "Browser", icon: Globe };
    case "terminal":
      return { label: "Terminal", icon: Terminal };
    case "file":
      return { label: "Files", icon: Folder };
    case "changes":
      return { label: "Changes", icon: GitBranch };
    case "review":
      return { label: "Review", icon: GitPullRequest };
  }
}

function TerminalWorkbenchPanel({ jobs }: { jobs: Map<string, JobInfo> }) {
  return (
    <div className="flex h-full min-h-0 flex-col gap-3 p-3">
      <div className="rounded-lg border border-border bg-background/60 p-3">
        <div className="flex items-center gap-2 text-xs font-medium text-foreground">
          <Terminal className="h-3.5 w-3.5 text-primary" />
          Terminal
        </div>
        <p className="mt-1 text-[11px] leading-relaxed text-muted-foreground">
          interactive terminal sessions will attach here. background command output is available below for now.
        </p>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto">
        <JobsRailPanel jobs={jobs} />
      </div>
    </div>
  );
}

function TabButton({
  id,
  label,
  icon: Icon,
  count,
  active,
  onClick,
}: {
  id: string;
  label: string;
  icon: LucideIcon;
  count: number;
  active: boolean;
  onClick: () => void;
}) {
  return (
    <button
      id={id}
      type="button"
      role="tab"
      aria-selected={active}
      aria-controls={id.replace("tab", "panel")}
      onClick={onClick}
      className={cn(
        "relative flex shrink-0 items-center gap-1.5 rounded-md px-2.5 py-1.5 text-[11px] font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
        active
          ? "bg-accent text-accent-foreground"
          : "text-muted-foreground hover:bg-accent hover:text-accent-foreground",
      )}
    >
      <Icon className="h-3.5 w-3.5" aria-hidden="true" />
      <span>{label}</span>
      {count > 0 && (
        <span className="flex h-4 min-w-4 items-center justify-center rounded-full bg-primary/10 px-1 text-[9px] text-primary" aria-label={`${count} active`}>
          {count}
        </span>
      )}
    </button>
  );
}

function AttentionQueue({
  sessions,
  alertKindsBySession,
  onOpenSession,
}: {
  sessions: SessionInfo[];
  alertKindsBySession: ReadonlyMap<string, readonly AlertKind[]>;
  onOpenSession: (sessionId: string) => void;
}) {
  return (
    <section className="motion-enter border-b border-amber-500/20 bg-amber-500/[0.06] px-3 py-3" aria-labelledby="needs-attention-heading">
      <div className="mb-2 flex items-center justify-between gap-2">
        <div>
          <h2 id="needs-attention-heading" className="text-[11px] font-semibold text-foreground">Needs attention</h2>
          <p className="mt-0.5 text-[10px] text-muted-foreground">Resolve these before they can continue.</p>
        </div>
        <span className="rounded-full bg-amber-500/15 px-1.5 py-0.5 text-[10px] font-semibold text-amber-700 dark:text-amber-300">
          {sessions.length}
        </span>
      </div>
      <div className="space-y-1.5">
        {sessions.map((session) => {
          const attention = getSessionAttention(session, alertKindsBySession.get(session.session_id));
          const primary = attention[0];
          if (!primary) return null;
          return (
            <button
              key={session.session_id}
              type="button"
              onClick={() => onOpenSession(session.session_id)}
              className="motion-pressable group flex w-full items-start gap-2 rounded-lg border border-amber-500/20 bg-background/70 px-2.5 py-2 text-left hover:border-amber-500/40 hover:bg-background focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
              aria-label={`${primary.label} in ${sessionTitle(session)}`}
            >
              <AttentionIcon kind={primary.kind} />
              <span className="min-w-0 flex-1">
                <span className="block truncate text-xs font-medium text-foreground">{sessionTitle(session)}</span>
                <span className="mt-0.5 flex items-center gap-1 text-[10px] font-medium text-amber-700 dark:text-amber-300">
                  {primary.label}{primary.count > 1 ? ` · ${primary.count}` : ""}
                  {attention.length > 1 && <span className="text-muted-foreground">+{attention.length - 1} more</span>}
                </span>
              </span>
              <ArrowUpRight className="mt-0.5 h-3.5 w-3.5 shrink-0 text-muted-foreground/60 transition-transform group-hover:-translate-y-0.5 group-hover:translate-x-0.5" aria-hidden="true" />
            </button>
          );
        })}
      </div>
    </section>
  );
}

function AttentionIcon({ kind }: { kind: SessionAttention["kind"] }) {
  const Icon = kind === "permission" ? ShieldAlert : kind === "question" ? MessageCircleQuestion : CircleAlert;
  return <Icon className="mt-0.5 h-4 w-4 shrink-0 text-amber-600 dark:text-amber-400" aria-hidden="true" />;
}

function sessionTitle(session: SessionInfo): string {
  return session.summary ?? session.first_message ?? session.model?.split("/").pop() ?? session.session_id.slice(0, 8);
}

function TodoRailPanel({ todos }: { todos: TodoItem[] }) {
  const [expandedId, setExpandedId] = useState<number | null>(null);

  if (todos.length === 0) {
    return (
      <EmptyState
        icon={<CheckCircle2 className="h-4 w-4" />}
        title="No plan yet"
        description="Plans and task progress will appear here."
      />
    );
  }

  return (
    <div className="space-y-0.5">
      <div className="mb-1.5 flex items-center justify-between text-[10px] uppercase tracking-wide text-muted-foreground">
        <span>Plan</span>
        <span>
          {todos.filter((t) => t.status === "done").length}/{todos.length} done
        </span>
      </div>
      {todos.map((todo) => (
        <TodoRailRow
          key={todo.id}
          todo={todo}
          todos={todos}
          expanded={expandedId === todo.id}
          onToggle={() => setExpandedId(expandedId === todo.id ? null : todo.id)}
        />
      ))}
    </div>
  );
}

function TodoRailRow({
  todo,
  todos,
  expanded,
  onToggle,
}: {
  todo: TodoItem;
  todos: TodoItem[];
  expanded: boolean;
  onToggle: () => void;
}) {
  const { icon, color } = todoStatusMeta(todo.status);
  const deps = todo.dependsOn
    .map((id) => todos.find((t) => t.id === id))
    .filter(Boolean) as TodoItem[];

  return (
    <div className="rounded-md border border-transparent hover:border-border hover:bg-muted/30">
      <button
        onClick={onToggle}
        className="flex w-full items-start gap-2 px-2 py-1.5 text-left"
      >
        <span className={cn("mt-0.5 shrink-0", color)}>{icon}</span>
        <span
          className={cn(
            "flex-1 text-xs",
            todo.status === "done" && "text-muted-foreground line-through",
            todo.status === "blocked" && "text-destructive",
            todo.status === "in_progress" && "font-medium text-foreground",
          )}
        >
          {todo.content}
        </span>
      </button>
      {expanded && deps.length > 0 && (
        <div className="pb-2 pl-7 pr-2">
          <div className="relative border-l-2 border-border pl-3 text-[10px] text-muted-foreground">
            <span className="mb-1 block">Depends on:</span>
            {deps.map((d) => (
              <div key={d.id} className="truncate">· {d.content}</div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}

function todoStatusMeta(status: TodoItem["status"]) {
  switch (status) {
    case "done":
      return { icon: <CheckCircle2 className="h-3.5 w-3.5" />, color: "text-green-500" };
    case "in_progress":
      return { icon: <Loader2 className="h-3.5 w-3.5 animate-spin" />, color: "text-blue-500" };
    case "blocked":
      return { icon: <AlertCircle className="h-3.5 w-3.5" />, color: "text-destructive" };
    default:
      return { icon: <PlayCircle className="h-3.5 w-3.5" />, color: "text-muted-foreground" };
  }
}

function SubagentRailPanel({ subagents }: { subagents: Map<string, SubagentInfo> }) {
  const subs = [...subagents.values()];
  if (subs.length === 0) {
    return (
      <EmptyState
        icon={<Loader2 className="h-4 w-4" />}
        title="No active agents"
        description="Delegated work will appear here while it runs."
      />
    );
  }
  return (
    <div className="space-y-1.5">
      {subs.map((sub) => (
        <SubagentRailRow key={sub.parentCallId} sub={sub} />
      ))}
    </div>
  );
}

function SubagentRailRow({ sub }: { sub: SubagentInfo }) {
  const dotColor = {
    running: "bg-blue-500 animate-pulse",
    completed: "bg-green-500",
    cancelled: "bg-muted-foreground",
    failed: "bg-destructive",
  }[sub.status] ?? "bg-muted-foreground";

  return (
    <div className="rounded-md border border-border bg-card/50 px-2 py-1.5">
      <div className="flex items-center gap-2">
        <span className={cn("h-1.5 w-1.5 shrink-0 rounded-full", dotColor)} />
        <span className="truncate text-xs font-medium text-foreground">
          {sub.displayName ?? sub.name}
        </span>
        {sub.status !== "running" && (
          <span className="ml-auto text-[10px] capitalize text-muted-foreground">{sub.status}</span>
        )}
      </div>
      <div className="mt-0.5 flex items-center gap-1 text-[10px] text-muted-foreground">
        <span className="font-mono">{sub.name}</span>
        {sub.childSessionId && <span>· {sub.childSessionId.slice(0, 8)}</span>}
      </div>
      {sub.lastProgress && (
        <div className="mt-1 truncate text-[11px] text-muted-foreground">
          ↳ {sub.lastProgress}
        </div>
      )}
      {sub.outcome?.type === "failed" && (
        <div className="mt-1 flex items-start gap-1 text-[11px] text-destructive">
          <XCircle className="mt-0.5 h-3 w-3 shrink-0" />
          {sub.outcome.reason}
        </div>
      )}
    </div>
  );
}

function QuestionsRailPanel({ questions }: { questions: PendingAskUser[] }) {
  const resolveAskUser = useSessionStore((s) => s.resolveAskUser);

  if (questions.length === 0) {
    return (
      <EmptyState
        icon={<HelpCircle className="h-4 w-4" />}
        title="No pending questions"
        description="When mew needs a decision, it will appear here."
      />
    );
  }

  return (
    <div className="space-y-3">
      {questions.map((req) => (
        <AskUserForm
          key={req.requestId}
          req={req}
          onSubmit={(answers) => {
            const client = getClient();
            if (client) client.respondToAskUser(req.requestId, answers);
            resolveAskUser(req.requestId);
          }}
        />
      ))}
    </div>
  );
}

function JobsRailPanel({ jobs }: { jobs: Map<string, JobInfo> }) {
  const list = [...jobs.values()];
  if (list.length === 0) {
    return (
      <EmptyState
        icon={<Terminal className="h-4 w-4" />}
        title="No background jobs"
        description="Long-running commands will be tracked here."
      />
    );
  }
  return (
    <div className="space-y-1.5">
      {list.map((job) => (
        <JobRailRow key={job.jobId} job={job} />
      ))}
    </div>
  );
}

function JobRailRow({ job }: { job: JobInfo }) {
  const isTerminal = job.state === "done" || job.state === "failed" || job.state === "cancelled";
  const dotColor = isTerminal
    ? job.state === "done"
      ? "bg-green-500"
      : job.state === "failed"
        ? "bg-destructive"
        : "bg-muted-foreground"
    : "bg-blue-500 animate-pulse";

  return (
    <div className="rounded-md border border-border bg-card/50 px-2 py-1.5">
      <div className="flex items-center gap-2">
        <span className={cn("h-1.5 w-1.5 shrink-0 rounded-full", dotColor)} />
        <span className="truncate font-mono text-[11px] text-foreground">
          {job.command}
        </span>
        {isTerminal && (
          <span className="ml-auto text-[10px] capitalize text-muted-foreground">{job.state}</span>
        )}
      </div>
      <div className="mt-0.5 truncate text-[10px] text-muted-foreground">
        {job.jobId.slice(0, 8)}
      </div>
    </div>
  );
}

function EmptyState({
  icon,
  title,
  description,
}: {
  icon: ReactNode;
  title: string;
  description: string;
}) {
  return (
    <div className="flex flex-col items-center px-4 py-12 text-center">
      <div className="flex h-9 w-9 items-center justify-center rounded-full bg-muted text-muted-foreground">
        {icon}
      </div>
      <span className="mt-3 text-xs font-medium text-foreground">{title}</span>
      <span className="mt-1 max-w-[14rem] text-[11px] leading-relaxed text-muted-foreground">
        {description}
      </span>
    </div>
  );
}
