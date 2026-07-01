import { useSessionStore } from "../stores/session";
import { cn } from "../lib/utils";

/** Shows running and recently-finished subagents as a collapsible list.
 * Rendered in the sidebar or below the chat surface. */
export function SubagentPanel() {
  const subagents = useSessionStore((s) => s.subagents);

  if (subagents.size === 0) return null;

  const subs = [...subagents.values()];

  return (
    <div className="border-t border-border bg-muted/30 p-2">
      <div className="mb-1 text-[10px] font-semibold uppercase tracking-wide text-muted-foreground">
        Subagents ({subs.filter((s) => s.status === "running").length} running)
      </div>
      <div className="space-y-1">
        {subs.map((sub) => (
          <SubagentRow key={sub.parentCallId} sub={sub} />
        ))}
      </div>
    </div>
  );
}

function SubagentRow({ sub }: { sub: ReturnType<typeof useSessionStore.getState>["subagents"] extends Map<string, infer V> ? V : never }) {
  const dotColor = {
    running: "bg-blue-500 animate-pulse",
    completed: "bg-green-500",
    cancelled: "bg-gray-500",
    failed: "bg-red-500",
  }[sub.status] ?? "bg-gray-500";

  return (
    <div className="flex items-start gap-2 rounded-md px-2 py-1 text-xs">
      <div className={cn("mt-1 h-2 w-2 shrink-0 rounded-full", dotColor)} />
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-1.5">
          <span className="font-medium text-foreground">
            {sub.displayName ?? sub.name}
          </span>
          <span className="text-[10px] text-muted-foreground">({sub.name})</span>
        </div>
        {sub.lastProgress && (
          <div className="truncate text-[11px] text-muted-foreground">
            ↳ {sub.lastProgress}
          </div>
        )}
        {sub.outcome?.type === "failed" && (
          <div className="truncate text-[11px] text-red-500">
            ✗ {sub.outcome.reason}
          </div>
        )}
      </div>
    </div>
  );
}
