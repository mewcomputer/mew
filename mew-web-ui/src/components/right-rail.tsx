import { useState, type ReactNode } from "react";
import { CheckCircle2, HelpCircle, Loader2, PlayCircle, XCircle, AlertCircle } from "lucide-react";
import { useSessionStore, type TodoItem, type SubagentInfo, type PendingAskUser } from "../stores/session";
import { cn } from "../lib/utils";
import { AskUserForm } from "./ask-user-card";
import { getClient } from "../lib/client-ref";
import {
  Sheet,
  SheetContent,
  SheetHeader,
  SheetTitle,
  SheetDescription,
} from "@/components/ui/sheet";

type TabKey = "todos" | "subagents" | "questions";

interface RightRailProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}

export function RightRail({ open, onOpenChange }: RightRailProps) {
  const [activeTab, setActiveTab] = useState<TabKey>("subagents");
  const todos = useSessionStore((s) => s.todos);
  const subagents = useSessionStore((s) => s.subagents);
  const questions = useSessionStore((s) => s.pendingAskUser);

  const activeCounts = {
    todos: todos.filter((t) => t.status === "in_progress" || t.status === "pending").length,
    subagents: [...subagents.values()].filter((s) => s.status === "running").length,
    questions: questions.length,
  };
  const totalActive = activeCounts.todos + activeCounts.subagents + activeCounts.questions;

  return (
    <Sheet open={open} onOpenChange={onOpenChange}>
      <SheetContent side="right" className="w-80 p-0 sm:max-w-sm">
        <SheetHeader className="border-b border-border px-3 py-2">
          <div className="flex items-center justify-between">
            <SheetTitle className="text-xs">Activity</SheetTitle>
            {totalActive > 0 && (
              <span className="flex h-4 min-w-4 items-center justify-center rounded-full bg-primary px-1 text-[9px] font-medium text-primary-foreground">
                {totalActive}
              </span>
            )}
          </div>
          <SheetDescription className="sr-only">
            Todos, subagents, and pending questions.
          </SheetDescription>
        </SheetHeader>

        {/* Tabs */}
        <div className="flex items-center gap-2 border-b border-border px-3 py-1.5">
          <TabButton
            label="Todos"
            count={activeCounts.todos}
            active={activeTab === "todos"}
            onClick={() => setActiveTab("todos")}
          />
          <TabButton
            label="Subagents"
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
        </div>

        {/* Content */}
        <div className="flex-1 overflow-y-auto p-2">
          {activeTab === "todos" && <TodoRailPanel todos={todos} />}
          {activeTab === "subagents" && <SubagentRailPanel subagents={subagents} />}
          {activeTab === "questions" && (
            <QuestionsRailPanel
              questions={questions}
              onResolved={() => onOpenChange(false)}
            />
          )}
        </div>
      </SheetContent>
    </Sheet>
  );
}

function TabButton({
  label,
  count,
  active,
  onClick,
}: {
  label: string;
  count: number;
  active: boolean;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      className={cn(
        "relative flex items-center gap-1.5 rounded-md px-2 py-1 text-[11px] font-medium transition-colors",
        active
          ? "bg-accent text-accent-foreground"
          : "text-muted-foreground hover:bg-accent hover:text-accent-foreground",
      )}
    >
      {label}
      {count > 0 && (
        <span className="flex h-4 min-w-4 items-center justify-center rounded-full bg-primary/10 px-1 text-[9px] text-primary">
          {count}
        </span>
      )}
    </button>
  );
}

function TodoRailPanel({ todos }: { todos: TodoItem[] }) {
  if (todos.length === 0) {
    return <EmptyState icon={<CheckCircle2 className="h-4 w-4" />} text="No todos yet" />;
  }

  const [expandedId, setExpandedId] = useState<number | null>(null);

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
    <div className="flex flex-col items-center gap-2 py-10 text-muted-foreground">
      {icon}
      <span className="text-xs">{text}</span>
    </div>
  );
}
