import { useSessionStore } from "../stores/session";
import { cn } from "../lib/utils";
import { formatTokens } from "../lib/format";
import { getClient } from "../lib/client-ref";
import { Users, Hand } from "lucide-react";

/** A slim status bar that sits beneath the input area as a natural
 *  extension of the composer. Left: connection dot + metrics.
 *  Right: presence chips + yield control + daemon version. */
export function StatusFooter() {
  const connectionState = useSessionStore((s) => s.connectionState);
  const inputTokens = useSessionStore((s) => s.totalInputTokens);
  const outputTokens = useSessionStore((s) => s.totalOutputTokens);
  const cost = useSessionStore((s) => s.totalCost);
  const subagents = useSessionStore((s) => s.subagents);
  const pendingPermissions = useSessionStore((s) => s.pendingPermissions);
  const daemonVersion = useSessionStore((s) => s.daemonVersion);
  const attachedClients = useSessionStore((s) => s.attachedClients);
  const yieldedByClient = useSessionStore((s) => s.yieldedByClient);

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

  const handleYield = () => {
    getClient()?.yieldControl();
  };

  // "Take control" is advisory: any prompt implicitly re-activates this
  // client, so we just clear the local yielded flag. There is no dedicated
  // wire message for reclaiming — prompting (or cancelling) does it.
  const handleTakeControl = () => {
    useSessionStore.getState().clearYieldedControl();
  };

  return (
    <div className="shrink-0">
      {/* Yielded-control banner */}
      {yieldedByClient !== null && (
        <div className="flex items-center justify-between gap-2 border-t border-amber-500/30 bg-amber-500/10 px-3 py-1 text-[10px] text-amber-600 dark:text-amber-400 sm:px-4">
          <span className="flex items-center gap-1">
            <Hand className="h-3 w-3" />
            Control yielded{attachedClients.length > 1 ? " to another client" : ""}
          </span>
          <button
            onClick={handleTakeControl}
            className="font-medium underline-offset-2 hover:underline"
            title="Any prompt will reclaim control; this just clears the banner."
          >
            Take control
          </button>
        </div>
      )}

      <footer className="mx-auto flex w-full items-center justify-between px-3 pb-2 pt-1 text-[10px] text-muted-foreground tabular-nums sm:px-4">
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
        {/* Right: presence chips + yield + version */}
        <div className="flex items-center gap-2">
          {attachedClients.length > 0 && (
            <div className="flex items-center gap-1">
              <Users className="h-3 w-3 opacity-50" />
              <div className="flex items-center gap-1">
                {attachedClients.map((c) => (
                  <PresenceChip key={c.id} id={c.id} kind={c.kind} />
                ))}
              </div>
            </div>
          )}
          {yieldedByClient === null && (
            <button
              onClick={handleYield}
              className="flex items-center gap-1 rounded px-1.5 py-0.5 text-[10px] text-muted-foreground/80 transition-colors hover:bg-accent hover:text-foreground"
              title="Let another attached client drive the session (advisory)."
            >
              <Hand className="h-3 w-3" />
              Yield
            </button>
          )}
          {daemonVersion && (
            <span className="text-muted-foreground/60">v{daemonVersion}</span>
          )}
        </div>
      </footer>
    </div>
  );
}

/** A small chip showing an attached client's id and kind. */
function PresenceChip({ id, kind }: { id: number; kind: string }) {
  const initial = kind.charAt(0).toUpperCase() || "?";
  const tone =
    kind === "web"
      ? "bg-blue-500/15 text-blue-600 dark:text-blue-400"
      : kind === "tui"
        ? "bg-purple-500/15 text-purple-600 dark:text-purple-400"
        : "bg-muted text-muted-foreground";
  return (
    <span
      className={cn(
        "inline-flex h-4 min-w-4 items-center justify-center rounded px-1 font-medium uppercase",
        tone,
      )}
      title={`${kind} client #${id}`}
    >
      {initial}
      <span className="ml-0.5 text-[8px] opacity-60">{id}</span>
    </span>
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
