import { useState, type ReactNode } from "react";
import {
  CheckCircle2,
  HelpCircle,
  Loader2,
  PlayCircle,
  XCircle,
  AlertCircle,
  Pin,
  X,
  Bell,
  Gauge,
  Activity as ActivityIcon,
  FileDiff,
} from "lucide-react";
import { useSessionStore, type TodoItem, type SubagentInfo, type PendingAskUser } from "../stores/session";
import { cn } from "../lib/utils";
import { formatTokens } from "../lib/format";
import { AskUserForm } from "./ask-user-card";
import { getClient } from "../lib/client-ref";
import { routerRef } from "../lib/router-ref";
import {
  Sheet,
  SheetContent,
  SheetHeader,
  SheetTitle,
  SheetDescription,
} from "@/components/ui/sheet";

type TabKey = "activity" | "todos" | "subagents" | "questions" | "changes";

interface RightRailProps {
  /** Desktop: null (docked). Mobile: controls sheet visibility. */
  mobileOpen?: boolean;
  onMobileOpenChange?: (open: boolean) => void;
}

export function RightRail({ mobileOpen = false, onMobileOpenChange }: RightRailProps = {}) {
  // On mobile, render as a Sheet slide-over
  if (mobileOpen || onMobileOpenChange) {
    return <MobileRightRail open={mobileOpen} onOpenChange={onMobileOpenChange!} />;
  }

  // On desktop, render as a docked panel
  return <DesktopRightRail />;
}

// ---------------------------------------------------------------------------
// Desktop: docked panel matching sidebar aesthetics
// ---------------------------------------------------------------------------

function DesktopRightRail() {
  return (
    <aside className="hidden w-72 shrink-0 flex-col border-l border-sidebar-border bg-sidebar text-sidebar-foreground md:flex">
      <RightRailContent />
    </aside>
  );
}

// ---------------------------------------------------------------------------
// Mobile: Sheet slide-over
// ---------------------------------------------------------------------------

function MobileRightRail({ open, onOpenChange }: { open: boolean; onOpenChange: (v: boolean) => void }) {
  return (
    <Sheet open={open} onOpenChange={onOpenChange}>
      <SheetContent side="right" className="w-80 p-0 sm:max-w-sm">
        <SheetHeader className="border-b border-border px-3 py-2">
          <SheetTitle className="text-xs">Activity</SheetTitle>
          <SheetDescription className="sr-only">
            Todos, subagents, pending questions, and alerts.
          </SheetDescription>
        </SheetHeader>
        <RightRailContent inSheet />
      </SheetContent>
    </Sheet>
  );
}

// ---------------------------------------------------------------------------
// Shared content (used by both desktop and mobile)
// ---------------------------------------------------------------------------

function RightRailContent({ inSheet = false }: { inSheet?: boolean }) {
  const [activeTab, setActiveTab] = useState<TabKey>("subagents");
  const todos = useSessionStore((s) => s.todos);
  const subagents = useSessionStore((s) => s.subagents);
  const questions = useSessionStore((s) => s.pendingAskUser);
  const flaggedFiles = useSessionStore((s) => s.flaggedFiles);
  const alerts = useSessionStore((s) => s.alerts);
  const lastInputTokens = useSessionStore((s) => s.lastInputTokens);
  const availableModels = useSessionStore((s) => s.availableModels);
  const currentModel = useSessionStore((s) => s.currentModel);
  const messages = useSessionStore((s) => s.messages);
  const availableSessions = useSessionStore((s) => s.availableSessions);
  const sessionId = useSessionStore((s) => s.sessionId);

  // Find the current model's context window
  const modelInfo = availableModels.find((m) => m.id === currentModel);
  const contextWindow = modelInfo?.context_window;

  // Active counts for tabs
  const activeCounts = {
    todos: todos.filter((t) => t.status === "in_progress" || t.status === "pending").length,
    subagents: [...subagents.values()].filter((s) => s.status === "running").length,
    questions: questions.length,
  };
  const totalActive = activeCounts.todos + activeCounts.subagents + activeCounts.questions;

  // Determine unread alerts (not for current session)
  const unreadAlerts = alerts.filter((a) => a.sessionId !== sessionId);

  return (
    <>
      {/* Alert Banner */}
      {unreadAlerts.length > 0 && (
        <AlertBanner alerts={unreadAlerts} />
      )}

      {/* Context Gauge */}
      <ContextGauge
        used={lastInputTokens}
        limit={contextWindow}
      />

      {/* Pinned Context (flagged files) */}
      {flaggedFiles.length > 0 && (
        <div className="border-b border-sidebar-border px-3 py-2">
          <div className="flex items-center gap-1 text-[10px] font-semibold text-sidebar-foreground/70">
            <Pin className="h-2.5 w-2.5" />
            Pinned Context ({flaggedFiles.length})
          </div>
          <div className="mt-1 space-y-0.5">
            {flaggedFiles.map((f) => (
              <div key={f.path} className="flex items-center gap-1 group">
                <span className="truncate text-[10px] text-sidebar-foreground/60">
                  {f.path.split("/").pop() ?? f.path}
                </span>
                {f.reason && (
                  <span className="text-[8px] text-sidebar-foreground/40">
                    ({f.reason})
                  </span>
                )}
                <button
                  className="ml-auto hidden text-sidebar-foreground/60 hover:text-sidebar-foreground group-hover:block"
                  onClick={() => {
                    const client = getClient();
                    const sid = useSessionStore.getState().sessionId;
                    if (sid) client?.unflagFile(sid, f.path);
                  }}
                  title="Unflag"
                >
                  <X className="h-2.5 w-2.5" />
                </button>
              </div>
            ))}
          </div>
        </div>
      )}

      {/* Tabs */}
      <div className="flex items-center gap-1.5 border-b border-sidebar-border px-2 py-1.5">
        <TabButton
          label="Activity"
          active={activeTab === "activity"}
          onClick={() => setActiveTab("activity")}
        />
        <TabButton
          label="Todos"
          count={activeCounts.todos}
          active={activeTab === "todos"}
          onClick={() => setActiveTab("todos")}
        />
        <TabButton
          label="Subs"
          count={activeCounts.subagents}
          active={activeTab === "subagents"}
          onClick={() => setActiveTab("subagents")}
        />
        <TabButton
          label="Questions"
          count={activeCounts.questions}
          active={activeTab === "questions"}
          onClick={() => setActiveTab("questions")}
        />
        <TabButton
          label="Changes"
          active={activeTab === "changes"}
          onClick={() => setActiveTab("changes")}
        />
      </div>

      {/* Content */}
      <div className="flex-1 overflow-y-auto p-2">
        {activeTab === "activity" && <ActivityTimeline messages={messages} subagents={subagents} />}
        {activeTab === "todos" && <TodoRailPanel todos={todos} />}
        {activeTab === "subagents" && <SubagentRailPanel subagents={subagents} />}
        {activeTab === "questions" && (
          <QuestionsRailPanel
            questions={questions}
            onResolved={() => { /* no-op for docked */ }}
          />
        )}
        {activeTab === "changes" && (
          <ChangesPanel sessionId={sessionId} availableSessions={availableSessions} />
        )}
      </div>

      {inSheet && totalActive > 0 && (
        <div className="border-t border-border px-3 py-1 text-[9px] text-muted-foreground">
          {totalActive} active item{totalActive !== 1 ? "s" : ""}
        </div>
      )}
    </>
  );
}

// ---------------------------------------------------------------------------
// Alert Banner
// ---------------------------------------------------------------------------

function AlertBanner({ alerts }: { alerts: { sessionId: string; title: string; kind: string; detail?: string; timestamp: number }[] }) {
  const dismissAlert = useSessionStore((s) => s.dismissAlert);
  const top = alerts[alerts.length - 1];
  if (!top) return null;

  const icon = alertIcon(top.kind);
  const color = alertColor(top.kind);

  return (
    <div className={cn("border-b px-3 py-2", color)}>
      <div className="flex items-start gap-2">
        <span className="mt-0.5 shrink-0">{icon}</span>
        <div className="min-w-0 flex-1">
          <button
            className="block w-full text-left text-[11px] font-medium"
            onClick={() => {
              routerRef.navigate?.(top.sessionId);
              const store = useSessionStore.getState();
              store.clearAlertsForSession(top.sessionId);
            }}
          >
            {top.title}
          </button>
          {top.detail && (
            <p className="mt-0.5 truncate text-[10px] opacity-70">{top.detail}</p>
          )}
          {alerts.length > 1 && (
            <span className="mt-0.5 inline-block text-[9px] opacity-60">
              +{alerts.length - 1} more
            </span>
          )}
        </div>
        <button
          className="shrink-0 opacity-60 hover:opacity-100"
          onClick={() => dismissAlert(top.sessionId, top.timestamp)}
        >
          <X className="h-3 w-3" />
        </button>
      </div>
    </div>
  );
}

function alertIcon(kind: string) {
  switch (kind) {
    case "permission_needed":
    case "input_needed":
      return <Bell className="h-3 w-3" />;
    case "turn_failed":
      return <XCircle className="h-3 w-3" />;
    default:
      return <CheckCircle2 className="h-3 w-3" />;
  }
}

function alertColor(kind: string) {
  switch (kind) {
    case "permission_needed":
    case "input_needed":
      return "bg-yellow-500/10 text-yellow-700 dark:text-yellow-400 border-yellow-500/20";
    case "turn_failed":
      return "bg-destructive/10 text-destructive border-destructive/20";
    default:
      return "bg-green-500/10 text-green-700 dark:text-green-400 border-green-500/20";
  }
}

// ---------------------------------------------------------------------------
// Context Gauge
// ---------------------------------------------------------------------------

function ContextGauge({ used, limit }: { used: number; limit?: number }) {
  if (!limit || limit <= 0) {
    // No context window info available
    return null;
  }

  const pct = Math.min(100, (used / limit) * 100);
  const barColor =
    pct > 80 ? "bg-red-500" : pct > 50 ? "bg-yellow-500" : "bg-green-500";

  return (
    <div className="border-b border-sidebar-border px-3 py-2">
      <div className="mb-1 flex items-center justify-between text-[10px]">
        <span className="flex items-center gap-1 font-semibold text-sidebar-foreground/70">
          <Gauge className="h-2.5 w-2.5" />
          Context
        </span>
        <span className="tabular-nums text-sidebar-foreground/50">
          {formatTokens(used)} / {formatTokens(limit)}
        </span>
      </div>
      <div className="h-1.5 overflow-hidden rounded-full bg-sidebar-border/50">
        <div
          className={cn("h-full rounded-full transition-all duration-300", barColor)}
          style={{ width: `${pct}%` }}
        />
      </div>
      {pct > 80 && (
        <p className="mt-0.5 text-[9px] text-red-500">
          Context nearly full — consider starting a new session
        </p>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Activity Timeline
// ---------------------------------------------------------------------------

interface TimelineEntry {
  id: string;
  type: "text" | "tool" | "subagent" | "error";
  label: string;
  detail?: string;
  timestamp: number;
  icon: ReactNode;
}

function ActivityTimeline({
  messages,
  subagents,
}: {
  messages: { id: string; role: string; parts: { type: string; text?: string; name?: string; state?: string; error?: { message: string } }[]; timestamp: number }[];
  subagents: Map<string, SubagentInfo>;
}) {
  const entries: TimelineEntry[] = [];

  // Flatten recent messages into timeline entries
  for (const msg of messages) {
    for (const part of msg.parts) {
      if (part.type === "text" && part.text) {
        entries.push({
          id: `${msg.id}-text`,
          type: "text",
          label: msg.role === "user" ? "You" : "Assistant",
          detail: part.text.slice(0, 80) + (part.text.length > 80 ? "…" : ""),
          timestamp: msg.timestamp,
          icon: <ActivityIcon className="h-3 w-3 text-blue-400" />,
        });
      } else if (part.type === "tool-call") {
        entries.push({
          id: `${msg.id}-${part.name}`,
          type: "tool",
          label: part.name ?? "tool",
          detail: part.state,
          timestamp: msg.timestamp,
          icon: part.state === "error" ? (
            <XCircle className="h-3 w-3 text-destructive" />
          ) : part.state === "completed" ? (
            <CheckCircle2 className="h-3 w-3 text-green-500" />
          ) : (
            <Loader2 className="h-3 w-3 animate-spin text-blue-400" />
          ),
        });
      } else if (part.type === "error") {
        entries.push({
          id: `${msg.id}-err`,
          type: "error",
          label: "Error",
          detail: part.error?.message?.slice(0, 80),
          timestamp: msg.timestamp,
          icon: <AlertCircle className="h-3 w-3 text-destructive" />,
        });
      }
    }
  }

  // Add subagent entries
  for (const sub of subagents.values()) {
    entries.push({
      id: `sub-${sub.parentCallId}`,
      type: "subagent",
      label: sub.displayName ?? sub.name,
      detail: sub.lastProgress ?? undefined,
      timestamp: Date.now(),
      icon: sub.status === "running" ? (
        <Loader2 className="h-3 w-3 animate-spin text-blue-400" />
      ) : sub.status === "failed" ? (
        <XCircle className="h-3 w-3 text-destructive" />
      ) : (
        <CheckCircle2 className="h-3 w-3 text-green-500" />
      ),
    });
  }

  // Sort newest first
  entries.sort((a, b) => b.timestamp - a.timestamp);

  if (entries.length === 0) {
    return <EmptyState icon={<ActivityIcon className="h-4 w-4" />} text="No activity yet" />;
  }

  return (
    <div className="space-y-0.5">
      {entries.slice(0, 50).map((e) => (
        <div key={e.id} className="flex items-start gap-2 rounded-md px-2 py-1 hover:bg-sidebar-accent/50">
          <span className="mt-0.5 shrink-0">{e.icon}</span>
          <div className="min-w-0 flex-1">
            <div className="truncate text-[11px] font-medium text-sidebar-foreground">{e.label}</div>
            {e.detail && (
              <div className="truncate text-[10px] text-sidebar-foreground/50">{e.detail}</div>
            )}
          </div>
        </div>
      ))}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Changes Panel
// ---------------------------------------------------------------------------

function ChangesPanel({
  sessionId,
  availableSessions,
}: {
  sessionId: string | null;
  availableSessions: { session_id: string; change_stats?: { added: number; removed: number; files: string[] } }[];
}) {
  const current = availableSessions.find((s) => s.session_id === sessionId);

  if (!current?.change_stats || (current.change_stats.added === 0 && current.change_stats.removed === 0)) {
    return <EmptyState icon={<FileDiff className="h-4 w-4" />} text="No changes yet" />;
  }

  const stats = current.change_stats;

  return (
    <div className="space-y-2">
      <div className="flex items-center gap-3 text-[10px]">
        <span className="flex items-center gap-1 text-green-500">
          <span className="font-medium">+{stats.added}</span> added
        </span>
        <span className="flex items-center gap-1 text-red-500">
          <span className="font-medium">−{stats.removed}</span> removed
        </span>
        <span className="text-sidebar-foreground/50">
          {stats.files.length} file{stats.files.length !== 1 ? "s" : ""}
        </span>
      </div>
      <div className="space-y-0.5">
        {stats.files.map((f) => (
          <div key={f} className="truncate rounded-md px-2 py-1 text-[10px] text-sidebar-foreground/70 hover:bg-sidebar-accent/50">
            {f.split("/").pop() ?? f}
            <span className="ml-1 text-sidebar-foreground/30">{f}</span>
          </div>
        ))}
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Tab Button
// ---------------------------------------------------------------------------

function TabButton({
  label,
  count,
  active,
  onClick,
}: {
  label: string;
  count?: number;
  active: boolean;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      className={cn(
        "relative flex items-center gap-1.5 rounded-md px-2 py-1 text-[11px] font-medium transition-colors",
        active
          ? "bg-sidebar-accent text-sidebar-accent-foreground"
          : "text-sidebar-foreground/60 hover:bg-sidebar-accent hover:text-sidebar-accent-foreground",
      )}
    >
      {label}
      {count !== undefined && count > 0 && (
        <span className="flex h-4 min-w-4 items-center justify-center rounded-full bg-primary/10 px-1 text-[9px] text-primary">
          {count}
        </span>
      )}
    </button>
  );
}

// ---------------------------------------------------------------------------
// Existing panels (Todos, Subagents, Questions)
// ---------------------------------------------------------------------------

function TodoRailPanel({ todos }: { todos: TodoItem[] }) {
  const [expandedId, setExpandedId] = useState<number | null>(null);

  if (todos.length === 0) {
    return <EmptyState icon={<CheckCircle2 className="h-4 w-4" />} text="No todos yet" />;
  }

  return (
    <div className="space-y-0.5">
      <div className="mb-1.5 flex items-center justify-between text-[10px] uppercase tracking-wide text-sidebar-foreground/50">
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
    <div className="rounded-md border border-transparent hover:border-sidebar-border hover:bg-sidebar-accent/30">
      <button
        onClick={onToggle}
        className="flex w-full items-start gap-2 px-2 py-1.5 text-left"
      >
        <span className={cn("mt-0.5 shrink-0", color)}>{icon}</span>
        <span
          className={cn(
            "flex-1 text-xs",
            todo.status === "done" && "text-sidebar-foreground/50 line-through",
            todo.status === "blocked" && "text-destructive",
            todo.status === "in_progress" && "font-medium text-sidebar-foreground",
          )}
        >
          {todo.content}
        </span>
      </button>
      {expanded && deps.length > 0 && (
        <div className="pb-2 pl-7 pr-2">
          <div className="relative border-l-2 border-sidebar-border pl-3 text-[10px] text-sidebar-foreground/50">
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
      return { icon: <PlayCircle className="h-3.5 w-3.5" />, color: "text-sidebar-foreground/40" };
  }
}

function SubagentRailPanel({ subagents }: { subagents: Map<string, SubagentInfo> }) {
  const subs = [...subagents.values()];
  if (subs.length === 0) {
    return <EmptyState icon={<Loader2 className="h-4 w-4" />} text="No subagents yet" />;
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
    cancelled: "bg-sidebar-foreground/40",
    failed: "bg-destructive",
  }[sub.status] ?? "bg-sidebar-foreground/40";

  return (
    <div className="rounded-md border border-sidebar-border bg-sidebar/50 px-2 py-1.5">
      <div className="flex items-center gap-2">
        <span className={cn("h-1.5 w-1.5 shrink-0 rounded-full", dotColor)} />
        <span className="truncate text-xs font-medium text-sidebar-foreground">
          {sub.displayName ?? sub.name}
        </span>
        {sub.status !== "running" && (
          <span className="ml-auto text-[10px] capitalize text-sidebar-foreground/50">{sub.status}</span>
        )}
      </div>
      <div className="mt-0.5 flex items-center gap-1 text-[10px] text-sidebar-foreground/50">
        <span className="font-mono">{sub.name}</span>
        {sub.childSessionId && <span>· {sub.childSessionId.slice(0, 8)}</span>}
      </div>
      {sub.lastProgress && (
        <div className="mt-1 truncate text-[11px] text-sidebar-foreground/50">
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

function QuestionsRailPanel({ questions, onResolved }: { questions: PendingAskUser[]; onResolved: () => void }) {
  const resolveAskUser = useSessionStore((s) => s.resolveAskUser);

  if (questions.length === 0) {
    return <EmptyState icon={<HelpCircle className="h-4 w-4" />} text="No questions waiting" />;
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
            onResolved();
          }}
        />
      ))}
    </div>
  );
}

function EmptyState({ icon, text }: { icon: ReactNode; text: string }) {
  return (
    <div className="flex flex-col items-center gap-2 py-10 text-sidebar-foreground/40">
      {icon}
      <span className="text-xs">{text}</span>
    </div>
  );
}
