import { useSessionStore } from "../stores/session";
import { cn } from "../lib/utils";
import { formatTokens } from "../lib/format";

/** A slim status bar that sits beneath the input area as a natural
 *  extension of the composer. Left: connection dot + metrics.
 *  Right: persona badge. */
export function StatusFooter() {
  const connectionState = useSessionStore((s) => s.connectionState);
  const inputTokens = useSessionStore((s) => s.totalInputTokens);
  const outputTokens = useSessionStore((s) => s.totalOutputTokens);
  const cost = useSessionStore((s) => s.totalCost);
  const subagents = useSessionStore((s) => s.subagents);
  const pendingPermissions = useSessionStore((s) => s.pendingPermissions);
  const daemonVersion = useSessionStore((s) => s.daemonVersion);

  if (connectionState === "disconnected") {
    return null;
  }

  const dotColor =
    {
      connected: "bg-green-500",
      connecting: "bg-yellow-500",
      reconnecting: "bg-yellow-500",
      disconnected: "bg-red-500",
    }[connectionState] ?? "bg-gray-500";

  const runningSubs = [...subagents.values()].filter(
    (s) => s.status === "running",
  ).length;

  return (
    <footer className="mx-auto flex w-full shrink-0 items-center justify-between px-3 pb-2 pt-1 text-[10px] text-muted-foreground tabular-nums sm:px-4">
      {/* Left: connection dot + metrics */}
      <div className="flex items-center gap-3">
        <div className="flex items-center gap-1.5">
          <span className={cn("h-1.5 w-1.5 rounded-full", dotColor)} />
          <span className="capitalize">{connectionState}</span>
        </div>
        <div className="hidden items-center gap-2 sm:flex">
          <Metric label="in" value={formatTokens(inputTokens)} />
          <Metric label="out" value={formatTokens(outputTokens)} />
          <Metric label="cost" value={`$${cost.toFixed(4)}`} />
        </div>
        {runningSubs > 0 && (
          <span className="flex items-center gap-1 text-blue-500">
            <span className="uppercase tracking-wide opacity-60">subs</span>
            <span className="font-medium">{runningSubs}</span>
          </span>
        )}
        {pendingPermissions.length > 0 && (
          <span className="flex items-center gap-1 text-yellow-500">
            <span className="uppercase tracking-wide opacity-60">perms</span>
            <span className="font-medium">{pendingPermissions.length}</span>
          </span>
        )}
      </div>
      {/* Right: daemon version */}
      <div className="flex items-center gap-2">
        {daemonVersion && (
          <span className="text-muted-foreground/60">v{daemonVersion}</span>
        )}
      </div>
    </footer>
  );
}

function Metric({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex items-center gap-1">
      <span className="uppercase tracking-wide opacity-60">{label}</span>
      <span className="font-medium">{value}</span>
    </div>
  );
}
