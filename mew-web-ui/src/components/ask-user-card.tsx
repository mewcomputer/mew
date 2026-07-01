import { useState } from "react";
import { useSessionStore } from "../stores/session";
import type { PendingAskUser } from "../stores/session";

/** The MewClient instance is passed down via a React context set up in App.
 * For now, we grab it from a module-level ref that App sets. */
import { getClient } from "../lib/client-ref";

/** Renders pending AskUserRequest modals. Each request shows its questions
 * with selectable options or free-text input. When the user submits, the
 * answers are sent back to the daemon via the client's respondToAskUser. */
export function AskUserCard() {
  const pending = useSessionStore((s) => s.pendingAskUser);

  if (pending.length === 0) return null;

  return (
    <div className="border-t border-border bg-muted/30 p-3">
      <div className="mb-2 text-[10px] font-semibold uppercase tracking-wide text-muted-foreground">
        Questions
      </div>
      <div className="space-y-3">
        {pending.map((req) => (
          <AskUserForm key={req.requestId} req={req} />
        ))}
      </div>
    </div>
  );
}

export function AskUserForm({
  req,
  onSubmit,
}: {
  req: PendingAskUser;
  onSubmit?: (answers: string[]) => void;
}) {
  const resolveAskUser = useSessionStore((s) => s.resolveAskUser);
  // One answer per question, indexed by question position.
  const [answers, setAnswers] = useState<string[]>(
    () => req.questions.map(() => ""),
  );

  const handleSubmit = () => {
    if (onSubmit) {
      onSubmit(answers);
    } else {
      const client = getClient();
      if (client) {
        client.respondToAskUser(req.requestId, answers);
      }
      resolveAskUser(req.requestId);
    }
  };

  return (
    <div className="rounded-lg border border-border bg-background p-3">
      <div className="space-y-3">
        {req.questions.map((q, qi) => (
          <div key={qi}>
            <div className="mb-1.5 text-sm font-medium text-foreground">
              {q.prompt}
            </div>
            {q.options.length > 0 ? (
              <div className="space-y-1">
                {q.options.map((opt) => (
                  <button
                    key={opt.label}
                    onClick={() => {
                      const next = [...answers];
                      next[qi] = opt.label;
                      setAnswers(next);
                    }}
                    className={`flex w-full items-start gap-2 rounded-md px-2 py-1.5 text-left text-sm transition-colors hover:bg-accent ${
                      answers[qi] === opt.label ? "bg-accent ring-1 ring-ring" : ""
                    }`}
                  >
                    <div className="min-w-0 flex-1">
                      <div className="font-medium text-foreground">{opt.label}</div>
                      {opt.description && (
                        <div className="text-xs text-muted-foreground">{opt.description}</div>
                      )}
                    </div>
                  </button>
                ))}
              </div>
            ) : (
              <input
                type="text"
                value={answers[qi]}
                onChange={(e) => {
                  const next = [...answers];
                  next[qi] = e.target.value;
                  setAnswers(next);
                }}
                placeholder="Type your answer…"
                className="w-full rounded-md bg-background px-2 py-1.5 text-sm outline-hidden ring-1 ring-border focus:ring-2 focus:ring-ring"
              />
            )}
          </div>
        ))}
      </div>
      <button
        onClick={handleSubmit}
        disabled={answers.some((a) => !a)}
        className="mt-3 rounded-md bg-primary px-3 py-1.5 text-sm font-medium text-primary-foreground transition-colors hover:bg-primary/90 disabled:opacity-50"
      >
        Submit
      </button>
    </div>
  );
}
