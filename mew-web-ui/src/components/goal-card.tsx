import { getClient } from "../lib/client-ref";
import { useSessionStore } from "../stores/session";

export function GoalCard() {
  const pending = useSessionStore((s) => s.pendingGoals);
  const resolveGoal = useSessionStore((s) => s.resolveGoal);
  if (pending.length === 0) return null;

  const respond = (requestId: string, accepted: boolean) => {
    const client = getClient();
    if (!client) return;
    client.respondToGoal(requestId, accepted);
    resolveGoal(requestId);
  };

  return (
    <div className="motion-enter border-t border-border bg-muted/30 p-3">
      <div className="mb-2 text-[10px] font-semibold uppercase tracking-wide text-muted-foreground">
        Proposed goals
      </div>
      <div className="space-y-3">
        {pending.map((goal) => (
          <div key={goal.requestId} className="rounded-lg border border-border bg-background p-3">
            <p className="whitespace-pre-wrap text-sm text-foreground">{goal.objective}</p>
            <div className="mt-3 flex gap-2">
              <button
                onClick={() => respond(goal.requestId, true)}
                className="motion-pressable rounded-md bg-primary px-3 py-1.5 text-sm font-medium text-primary-foreground hover:bg-primary/90"
              >
                Accept
              </button>
              <button
                onClick={() => respond(goal.requestId, false)}
                className="motion-pressable rounded-md border border-border px-3 py-1.5 text-sm font-medium hover:bg-accent"
              >
                Reject
              </button>
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}
