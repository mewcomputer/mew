import { useSessionStore } from "../stores/session";
import { cn } from "../lib/utils";

/** A bottom status bar giving mew a "tool-like" instrument panel.
 *
 * Left: connection dot + provider/model shortcut (click → model picker trigger).
 * Middle: tokens in/out, cost, active subagent count, pending permission count.
 * Right: last event latency placeholder (for future use).
 *
 * Uses tabular-nums for stable number alignment.
 */
export function StatusFooter({ onModelClick }: { onModelClick?: () => void }) {
  const connectionState = useSessionStore((s) => s.connectionState);
  const currentModel = useSessionStore((s) => s.currentModel);
  const currentProvider = useSessionStore((s) => s.currentProvider);
  const inputTokens = useSessionStore((s) => s.totalInputTokens);
  const outputTokens = useSessionStore((s) => s.totalOutputTokens);
  const cost = useSessionStore((s) => s.totalCost);
  const subagents = useSessionStore((s) => s.subagents);
  const pendingPermissions = useSessionStore((s) => s.pendingPermissions);

  const dotColor = {
    connected: "bg-green-500",
    connecting: "bg-yellow-500",
    reconnecting: "bg-yellow-500",
    disconnected: "bg-red-500",
  }[connectionState] ?? "bg-gray-500";

  const runningSubs = [...subagents.values()].filter((s) => s.status === "running").length;

  const modelLabel = currentModel
    ? `${currentProvider}/${currentModel}`
    : "no model";

  return (
    <footer className="flex shrink-0 items-center justify-between border-t border-border bg-background px-3 py-1 text-[10px] text-muted-foreground tabular-nums">
      {/* Left: connection + model */}
      <div className="flex items-center gap-3">
        <div className="flex items-center gap-1.5">
          <span className={cn("h-1.5 w-1.5 rounded-full", dotColor)} />
          <span className="capitalize">{connectionState}</span>
        </div>
        <button
          onClick={onModelClick}
          className="flex items-center gap-1 rounded px-1 py-0.5 font-mono transition-colors hover:bg-accent hover:text-accent-foreground"
          title="Switch model"
        >
          <svg className="h-3 w-3" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5">
            <path d="M2 4h12M2 8h12M2 12h12" />
          </svg>
          <span className="max-w-[180px] truncate">{modelLabel}</span>
        </button>
      </div>

      {/* Middle: metrics */}
      <div className="flex items-center gap-3">
        <Metric label="in" value={formatTokens(inputTokens)} />
        <Metric label="out" value={formatTokens(outputTokens)} />
        <Metric label="cost" value={`$${cost.toFixed(4)}`} />
        {runningSubs > 0 && (
          <Metric
            label="subagents"
            value={String(runningSubs)}
            tone="active"
          />
        )}
        {pendingPermissions.length > 0 && (
          <Metric
            label="perms"
            value={String(pendingPermissions.length)}
            tone="warning"
          />
        )}
      </div>
    </footer>
  );
}

function Metric({
  label,
  value,
  tone,
}: {
  label: string;
  value: string;
  tone?: "active" | "warning";
}) {
  return (
    <div className="flex items-center gap-1">
      <span className="uppercase tracking-wide opacity-60">{label}</span>
      <span
        className={cn(
          "font-medium",
          tone === "active" && "text-blue-500",
          tone === "warning" && "text-yellow-500",
        )}
      >
        {value}
      </span>
    </div>
  );
}

function formatTokens(n: number): string {
  if (n < 1000) return String(n);
  if (n < 1_000_000) return `${(n / 1000).toFixed(1)}k`;
  return `${(n / 1_000_000).toFixed(2)}M`;
}
