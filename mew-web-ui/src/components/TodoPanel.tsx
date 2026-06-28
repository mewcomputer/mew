import { useSessionStore } from "../stores/session";
import { cn } from "../lib/utils";
import type { TodoItem } from "../stores/session";

/** Renders the agent's todo list. Shows items with their status and
 * dependency relationships. Collapsible — hidden when empty. */
export function TodoPanel() {
  const todos = useSessionStore((s) => s.todos);

  if (todos.length === 0) return null;

  const done = todos.filter((t) => t.status === "done").length;
  const inProgress = todos.filter((t) => t.status === "in_progress").length;

  return (
    <div className="border-t border-border bg-muted/30 px-3 py-2">
      <div className="mb-1.5 flex items-center justify-between">
        <span className="text-[10px] font-semibold uppercase tracking-wide text-muted-foreground">
          Todos
        </span>
        <span className="text-[10px] text-muted-foreground">
          {done}/{todos.length} done{inProgress > 0 ? ` · ${inProgress} active` : ""}
        </span>
      </div>
      <div className="space-y-0.5">
        {todos.map((todo) => (
          <TodoRow key={todo.id} todo={todo} todos={todos} />
        ))}
      </div>
    </div>
  );
}

function TodoRow({ todo, todos }: { todo: TodoItem; todos: TodoItem[] }) {
  const icon = {
    done: "✅",
    in_progress: "🔄",
    pending: "⬜",
    blocked: "⛔",
  }[todo.status] ?? "⬜";

  // Show dependency labels: if this todo depends on others, show their
  // content truncated.
  const deps = todo.dependsOn
    .map((depId) => todos.find((t) => t.id === depId))
    .filter(Boolean) as TodoItem[];

  return (
    <div className="flex items-start gap-2 py-0.5 text-xs">
      <span className="shrink-0 text-[11px]">{icon}</span>
      <div className="min-w-0 flex-1">
        <span
          className={cn(
            todo.status === "done" && "text-muted-foreground line-through",
            todo.status === "in_progress" && "font-medium text-foreground",
            todo.status === "blocked" && "text-red-500",
          )}
        >
          {todo.content}
        </span>
        {deps.length > 0 && (
          <span className="ml-1.5 text-[10px] text-muted-foreground">
            (needs: {deps.map((d) => d.content.slice(0, 20)).join(", ")})
          </span>
        )}
      </div>
    </div>
  );
}
