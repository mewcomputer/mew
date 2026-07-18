import { ShieldCheck, ShieldX, ShieldAlert } from "lucide-react";
import { useSessionStore } from "../stores/session";

export function PermissionToast({
  onResolve,
}: {
  onResolve: (requestId: string, decision: "allow_once" | "allow_session" | "deny") => void;
}) {
  const pending = useSessionStore((s) => s.pendingPermissions);

  if (pending.length === 0) return null;

  const latest = pending[pending.length - 1]!;

  return (
    <div className="motion-enter fixed bottom-24 left-3 right-3 z-50 w-auto rounded-lg border border-yellow-500/50 bg-yellow-500/10 p-3 shadow-lg backdrop-blur-sm sm:bottom-4 sm:left-auto sm:right-4 sm:w-96 sm:p-4">
      <div className="flex items-start gap-3">
        <ShieldAlert className="mt-0.5 h-5 w-5 shrink-0 text-yellow-500" />
        <div className="flex-1">
          <p className="text-sm font-medium">
            Permission requested: <span className="font-mono">{latest.toolName}</span>
          </p>
          <pre className="mt-2 max-h-32 overflow-auto rounded bg-muted/50 p-2 text-xs">
            {JSON.stringify(latest.input, null, 2)}
          </pre>
          <div className="mt-3 flex flex-wrap gap-2">
            <button
              onClick={() => onResolve(latest.requestId, "allow_once")}
              className="motion-pressable flex items-center gap-1 rounded-md border border-green-500/50 bg-green-500/10 px-3 py-1 text-xs font-medium text-green-500 hover:bg-green-500/20"
            >
              <ShieldCheck className="h-3 w-3" />
              Allow Once
            </button>
            <button
              onClick={() => onResolve(latest.requestId, "allow_session")}
              className="motion-pressable flex items-center gap-1 rounded-md border border-border bg-secondary px-3 py-1 text-xs font-medium hover:bg-secondary/80"
            >
              Allow Session
            </button>
            <button
              onClick={() => onResolve(latest.requestId, "deny")}
              className="motion-pressable flex items-center gap-1 rounded-md border border-red-500/50 bg-red-500/10 px-3 py-1 text-xs font-medium text-red-500 hover:bg-red-500/20"
            >
              <ShieldX className="h-3 w-3" />
              Deny
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
