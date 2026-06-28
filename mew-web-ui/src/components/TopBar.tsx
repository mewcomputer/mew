import type { MewClient } from "@mew/web-client";
import { useSessionStore } from "../stores/session";
import { cn } from "../lib/utils";
import { ModelPicker } from "./ModelPicker";
import { ThemeToggle } from "./ThemeToggle";

interface TopBarProps {
  connectionState: string;
  client: MewClient | null;
  onOpenSessions: () => void;
}

export function TopBar({ connectionState, client, onOpenSessions }: TopBarProps) {
  const sessionId = useSessionStore((s) => s.sessionId);
  const cost = useSessionStore((s) => s.totalCost);
  const inputTokens = useSessionStore((s) => s.totalInputTokens);
  const outputTokens = useSessionStore((s) => s.totalOutputTokens);

  const dotColor = {
    connected: "bg-green-500",
    connecting: "bg-yellow-500",
    reconnecting: "bg-yellow-500",
    disconnected: "bg-red-500",
  }[connectionState] ?? "bg-gray-500";

  return (
    <header className="flex items-center justify-between border-b border-border px-4 py-2">
      <div className="flex items-center gap-3">
        <button
          onClick={onOpenSessions}
          className="flex items-center gap-1.5 rounded-md border border-border px-2 py-1 text-xs text-muted-foreground transition-colors hover:bg-accent hover:text-accent-foreground"
          title="Sessions"
        >
          <svg className="h-3.5 w-3.5" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5">
            <path d="M2 4h12M2 8h12M2 12h12" />
          </svg>
          <span className="max-w-[120px] truncate font-mono">
            {sessionId ? sessionId.slice(0, 12) + "…" : "no session"}
          </span>
        </button>
        <div className="flex items-center gap-1.5">
          <div className={cn("h-2 w-2 rounded-full", dotColor)} />
          <span className="text-xs text-muted-foreground">{connectionState}</span>
        </div>
      </div>
      <div className="flex items-center gap-4 text-xs text-muted-foreground">
        <ModelPicker client={client} />
        <span>{inputTokens.toLocaleString()} in / {outputTokens.toLocaleString()} out</span>
        <span className="font-mono">${cost.toFixed(4)}</span>
        <ThemeToggle />
      </div>
    </header>
  );
}
