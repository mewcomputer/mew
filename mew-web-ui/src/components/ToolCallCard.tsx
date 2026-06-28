import { useState } from "react";
import { ChevronRight, ChevronDown, Terminal, CheckCircle2, XCircle, Loader2 } from "lucide-react";
import { useSessionStore, type MessagePart } from "../stores/session";
import { cn } from "../lib/utils";

/** Extract a human-readable summary of the tool input for the collapsed view.
 * Instead of showing raw JSON, show the key argument (path, command, pattern, etc). */
function inputSummary(_toolName: string, input: unknown): string {
  if (!input || input === null) return "";
  if (typeof input === "object") {
    const obj = input as Record<string, unknown>;
    // Common fields across mew tools.
    const path = obj.path as string | undefined;
    const command = obj.command as string | undefined;
    const pattern = obj.pattern as string | undefined;
    const oldStr = obj.old_string as string | undefined;
    const text = obj.text as string | undefined;
    if (command) return command.length > 60 ? command.slice(0, 60) + "…" : command;
    if (path) return path;
    if (pattern) return pattern;
    if (text) return text.length > 60 ? text.slice(0, 60) + "…" : text;
    if (oldStr) return "old: " + (oldStr.length > 40 ? oldStr.slice(0, 40) + "…" : oldStr);
  }
  return "";
}

export function ToolCallCard({ part }: { part: Extract<MessagePart, { type: "tool-call" }> }) {
  const [expanded, setExpanded] = useState(false);
  const toolState = useSessionStore((s) => s.toolStates.get(part.callId));
  const toolOutput = useSessionStore((s) => s.toolOutputs.get(part.callId)) ?? part.output;

  const state = toolState ?? part.state;
  const hasOutput = toolOutput && toolOutput.length > 0;
  const summary = inputSummary(part.toolName, part.input);

  const stateIcon = {
    pending: <Loader2 className="h-3.5 w-3.5 animate-spin text-muted-foreground" />,
    running: <Loader2 className="h-3.5 w-3.5 animate-spin text-blue-500" />,
    completed: <CheckCircle2 className="h-3.5 w-3.5 text-green-500" />,
    error: <XCircle className="h-3.5 w-3.5 text-red-500" />,
  }[state];

  return (
    <div className="max-w-[85%] rounded-lg border border-border bg-card">
      <button
        onClick={() => setExpanded(!expanded)}
        className="flex w-full items-center gap-2 px-4 py-2 text-left"
      >
        {expanded ? (
          <ChevronDown className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
        ) : (
          <ChevronRight className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
        )}
        <Terminal className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
        <span className="font-mono text-sm">{part.toolName}</span>
        {summary && (
          <span className="truncate text-xs text-muted-foreground">{summary}</span>
        )}
        <span
          className={cn(
            "ml-auto shrink-0 text-xs capitalize",
            state === "error" && "text-red-500",
            state === "completed" && "text-green-500",
            state === "running" && "text-blue-500",
            state === "pending" && "text-muted-foreground",
          )}
        >
          {state}
        </span>
        {stateIcon}
      </button>

      {hasOutput && !expanded && (
        <div className="border-t border-border">
          <pre className="max-h-32 overflow-auto px-4 py-2 text-xs text-muted-foreground">
            {toolOutput}
          </pre>
        </div>
      )}

      {expanded && (
        <div className="border-t border-border px-4 py-2">
          {part.input != null && (
            <div className="mb-2">
              <span className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
                Input
              </span>
              <pre className="mt-1 overflow-x-auto rounded bg-muted p-2 text-xs">
                {JSON.stringify(part.input, null, 2)}
              </pre>
            </div>
          )}
          {hasOutput && (
            <div>
              <span className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
                Output
              </span>
              <pre className="mt-1 max-h-64 overflow-auto rounded bg-muted p-2 text-xs">
                {toolOutput}
              </pre>
            </div>
          )}
        </div>
      )}
    </div>
  );
}