import { useSessionStore } from "../stores/session";
import { cn } from "../lib/utils";
import { connectionDotClass } from "../lib/format";

export function TitleStrip() {
  const sessionId = useSessionStore((s) => s.sessionId);
  const connectionState = useSessionStore((s) => s.connectionState);
  const sessionTitles = useSessionStore((s) => s.sessionTitles);

  const title = sessionId ? sessionTitles.get(sessionId) : null;

  return (
    <div className="flex shrink-0 items-center gap-2 px-4 py-1.5">
      <span
        className={cn(
          "h-1.5 w-1.5 shrink-0 rounded-full",
          connectionDotClass(connectionState),
        )}
      />
      <span className="truncate font-mono text-[11px] text-muted-foreground">
        {title ?? (sessionId ? sessionId.slice(0, 16) : "no session")}
      </span>
    </div>
  );
}
