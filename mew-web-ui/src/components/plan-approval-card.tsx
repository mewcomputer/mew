import { useState } from "react";
import { useSessionStore } from "../stores/session";
import type { PendingPlanApproval } from "../stores/session";
import { MarkdownBody } from "./markdown-body";
import { getClient } from "../lib/client-ref";

/** Renders pending PlanApprovalRequest cards (from handoff_plan). Each card
 * shows the plan rendered as markdown, an Approve button, and a
 * request-changes textarea. Decisions go back to the daemon via the client's
 * respondToPlanApproval. */
export function PlanApprovalCard() {
  const pending = useSessionStore((s) => s.pendingPlanApprovals);

  if (pending.length === 0) return null;

  return (
    <div className="border-t border-border bg-muted/30 p-3">
      <div className="mb-2 text-[10px] font-semibold uppercase tracking-wide text-muted-foreground">
        Plan approval
      </div>
      <div className="space-y-3">
        {pending.map((req) => (
          <PlanApprovalForm key={req.requestId} req={req} />
        ))}
      </div>
    </div>
  );
}

export function PlanApprovalForm({
  req,
  onApprove,
  onRequestChanges,
}: {
  req: PendingPlanApproval;
  onApprove?: () => void;
  onRequestChanges?: (feedback: string) => void;
}) {
  const resolvePlanApproval = useSessionStore((s) => s.resolvePlanApproval);
  const [requesting, setRequesting] = useState(false);
  const [feedback, setFeedback] = useState("");

  const approve = () => {
    if (onApprove) {
      onApprove();
    } else {
      const client = getClient();
      if (client) client.respondToPlanApproval(req.requestId, true);
      resolvePlanApproval(req.requestId);
    }
  };

  const requestChanges = () => {
    if (onRequestChanges) {
      onRequestChanges(feedback);
    } else {
      const client = getClient();
      if (client) client.respondToPlanApproval(req.requestId, false, feedback);
      resolvePlanApproval(req.requestId);
    }
  };

  return (
    <div className="rounded-lg border border-border bg-background p-3">
      <div className="mb-2 flex items-center gap-2 text-xs text-muted-foreground">
        <span className="font-mono">{req.planPath}</span>
        <span>→</span>
        <span className="font-medium text-foreground">{req.persona}</span>
      </div>
      <div className="max-h-80 overflow-y-auto rounded-md border border-border bg-muted/20 p-2 text-sm">
        {req.planMarkdown.trim() ? (
          <MarkdownBody highlight={false}>{req.planMarkdown}</MarkdownBody>
        ) : (
          <pre className="whitespace-pre-wrap font-mono text-xs">
            (empty plan)
          </pre>
        )}
      </div>

      {requesting ? (
        <div className="mt-3 space-y-2">
          <textarea
            value={feedback}
            onChange={(e) => setFeedback(e.target.value)}
            placeholder="What should change before this plan is approved?"
            rows={3}
            className="w-full rounded-md bg-background px-2 py-1.5 text-sm outline-hidden ring-1 ring-border focus:ring-2 focus:ring-ring"
          />
          <div className="flex gap-2">
            <button
              onClick={requestChanges}
              disabled={!feedback.trim()}
              className="rounded-md bg-primary px-3 py-1.5 text-sm font-medium text-primary-foreground transition-colors hover:bg-primary/90 disabled:opacity-50"
            >
              Send feedback
            </button>
            <button
              onClick={() => setRequesting(false)}
              className="rounded-md px-3 py-1.5 text-sm font-medium text-muted-foreground transition-colors hover:bg-accent"
            >
              Cancel
            </button>
          </div>
        </div>
      ) : (
        <div className="mt-3 flex gap-2">
          <button
            onClick={approve}
            className="rounded-md bg-primary px-3 py-1.5 text-sm font-medium text-primary-foreground transition-colors hover:bg-primary/90"
          >
            Approve
          </button>
          <button
            onClick={() => setRequesting(true)}
            className="rounded-md px-3 py-1.5 text-sm font-medium text-foreground ring-1 ring-border transition-colors hover:bg-accent"
          >
            Request changes
          </button>
        </div>
      )}
    </div>
  );
}
