import { RefreshCw, WifiOff } from "lucide-react";
import { useSessionStore } from "@/stores/session";
import { Button } from "@/components/ui/button";

export function ConnectionBanner() {
  const state = useSessionStore((s) => s.connectionState);
  const error = useSessionStore((s) => s.connectionError);
  const retryConnection = useSessionStore((s) => s.retryConnection);

  if (state === "connected" || state === "connecting") return null;

  const reconnecting = state === "reconnecting";
  return (
    <div className="pointer-events-none fixed inset-x-0 top-2 z-50 flex justify-center px-3">
      <div
        className="motion-enter pointer-events-auto flex w-full max-w-md items-center gap-2 rounded-lg border border-border bg-card px-3 py-2 text-xs shadow-lg"
        role={reconnecting ? "status" : "alert"}
      >
        {reconnecting ? (
          <RefreshCw className="h-3.5 w-3.5 shrink-0 animate-spin text-muted-foreground" />
        ) : (
          <WifiOff className="h-3.5 w-3.5 shrink-0 text-destructive" />
        )}
        <div className="min-w-0 flex-1">
          <p className="font-medium">
            {reconnecting ? "reconnecting to the daemon" : "daemon connection lost"}
          </p>
          {error && <p className="truncate text-muted-foreground">{error}</p>}
        </div>
        {!reconnecting && (
          <Button variant="outline" size="sm" onClick={retryConnection}>
            <RefreshCw className="h-3 w-3" />
            Retry
          </Button>
        )}
      </div>
    </div>
  );
}
