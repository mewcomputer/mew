import { useSessionStore } from "../stores/session";
import { useRef, useState } from "react";
import { useOutsideClick } from "../lib/useOutsideClick";
import { cn } from "../lib/utils";
import { contextPercent, formatTokens } from "../lib/format";
import { Users } from "lucide-react";

/** Context-window occupancy as a small ring, sitting beside the model name.
 *
 *  Clicking it opens the session details that used to hang off a bare "…"
 *  button under the composer: connection, tokens, cost, attached clients,
 *  and the daemon version. */
export function ContextIndicator() {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  useOutsideClick(ref, open, () => setOpen(false));
  const connectionState = useSessionStore((s) => s.connectionState);
  const contextTokens = useSessionStore((s) => s.contextTokens);
  const contextTokensKnown = useSessionStore((s) => s.contextTokensKnown);
  const contextWindow = useSessionStore((s) => s.contextWindow);
  const cost = useSessionStore((s) => s.totalCost);
  const subagents = useSessionStore((s) => s.subagents);
  const pendingPermissions = useSessionStore((s) => s.pendingPermissions);
  const daemonVersion = useSessionStore((s) => s.daemonVersion);
  const attachedClients = useSessionStore((s) => s.attachedClients);

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

  const percent = contextTokensKnown && contextWindow > 0
    ? contextPercent(contextTokens, contextWindow)
    : null;
  const tone = percent == null
    ? "text-muted-foreground/40"
    : percent >= 90
      ? "text-destructive"
      : percent >= 70
        ? "text-text-warning"
        : "text-text-accent";

  return (
    <div ref={ref} className="relative flex shrink-0 items-center">
      <button
        type="button"
        aria-label={percent == null ? "Context usage unknown, show session details" : `Context ${percent}% used, show session details`}
        aria-expanded={open}
        title={percent == null ? "Context usage unknown" : `Context ${contextLabel(contextTokens, contextWindow)}`}
        onClick={() => setOpen((value) => !value)}
        className="motion-pressable flex h-7 items-center rounded-md px-1.5 text-muted-foreground hover:bg-accent hover:text-accent-foreground"
      >
        <ContextRing tone={tone} filled={percent == null ? 0 : percent / 100} />
      </button>
      {open && <footer data-mew-overlay="" className="absolute bottom-full right-0 z-30 mb-2 flex min-w-72 flex-wrap items-center justify-between gap-3 rounded-lg border border-border bg-popover p-3 text-[10px] text-muted-foreground shadow-lg tabular-nums">
        {/* Left: connection dot + metrics */}
        <div className="flex min-w-0 items-center gap-2 sm:gap-3">
          <div className="flex items-center gap-1.5">
            <span className={cn("h-1.5 w-1.5 rounded-full", dotColor)} />
            <span className="capitalize">{connectionState}</span>
          </div>
          <div className="hidden items-center gap-2 sm:flex">
            {(!contextTokensKnown || contextTokens > 0 || contextWindow > 0) && (
              <Metric label="ctx" value={contextTokensKnown ? contextLabel(contextTokens, contextWindow) : "?"} />
            )}
            <Metric label="cost" value={`$${cost.toFixed(4)}`} />
          </div>
          {runningSubs > 0 && (
            <span className="hidden items-center gap-1 text-blue-500 sm:flex">
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
        {/* Right: presence chips + version */}
        <div className="flex items-center gap-2">
          {attachedClients.length > 0 && (
            <div className="hidden items-center gap-1 sm:flex">
              <Users className="h-3 w-3 opacity-50" />
              <div className="flex items-center gap-1">
                {attachedClients.map((c) => (
                  <PresenceChip key={c.id} id={c.id} kind={c.kind} />
                ))}
              </div>
            </div>
          )}
          {daemonVersion && (
            <span className="hidden text-muted-foreground/60 sm:inline">v{daemonVersion}</span>
          )}
        </div>
      </footer>}
    </div>
  );
}

/** Occupancy ring: amount filled is context used, colour is how close it is. */
function ContextRing({ filled, tone }: { filled: number; tone: string }) {
  const radius = 5;
  const circumference = 2 * Math.PI * radius;
  const used = Math.min(1, Math.max(0, filled)) * circumference;
  return (
    <svg viewBox="0 0 14 14" className={cn("h-3.5 w-3.5 -rotate-90", tone)} aria-hidden="true">
      <circle cx="7" cy="7" r={radius} fill="none" strokeWidth={2.5} stroke="currentColor" opacity={0.2} />
      {used > 0 && (
        <circle
          cx="7" cy="7" r={radius} fill="none" strokeWidth={2.5} stroke="currentColor"
          strokeDasharray={`${used} ${circumference}`} strokeLinecap="round"
        />
      )}
    </svg>
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

/** "12.3k / 200k · 6%" when the window is known, otherwise just the count. */
function contextLabel(used: number, window: number): string {
  if (window > 0) {
    const pct = contextPercent(used, window);
    return `${formatTokens(used)} / ${formatTokens(window)} · ${pct ?? 0}%`;
  }
  return formatTokens(used);
}
