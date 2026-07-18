import { useEffect, useMemo, useState } from "react";
import { ChevronRight, ChevronDown, Terminal, CheckCircle2, XCircle, Loader2, Copy, FileEdit, FilePlus } from "lucide-react";
import { useSessionStore, type MessagePart } from "../stores/session";
import { cn } from "../lib/utils";

type Sensitivity = "readonly" | "mutating" | "dangerous";

function inferSensitivity(toolName: string): Sensitivity {
  const t = toolName.toLowerCase();
  if (t.includes("read") || t.includes("glob") || t.includes("grep") || t.includes("find") || t.includes("ls")) {
    return "readonly";
  }
  if (t.includes("bash") || t.includes("shell") || t.includes("exit")) {
    return "dangerous";
  }
  return "mutating";
}

function sensitivityMeta(s: Sensitivity) {
  switch (s) {
    case "readonly":
      return { label: "ReadOnly", className: "bg-muted text-muted-foreground" };
    case "mutating":
      return { label: "Mutating", className: "bg-amber-500/10 text-amber-600 dark:text-amber-400" };
    case "dangerous":
      return { label: "Dangerous", className: "bg-red-500/10 text-red-600 dark:text-red-400" };
  }
}

/** Extract a human-readable summary of the tool input for the collapsed view. */
function inputSummary(input: unknown): string {
  if (!input || input === null) return "";
  if (typeof input === "object") {
    const obj = input as Record<string, unknown>;
    const path = obj.path as string | undefined;
    const command = obj.command as string | undefined;
    const pattern = obj.pattern as string | undefined;
    const text = obj.text as string | undefined;
    const oldStr = obj.old_string as string | undefined;
    if (command) return command.length > 60 ? command.slice(0, 60) + "…" : command;
    if (path) return path;
    if (pattern) return pattern;
    if (text) return text.length > 60 ? text.slice(0, 60) + "…" : text;
    if (oldStr) return "old: " + (oldStr.length > 40 ? oldStr.slice(0, 40) + "…" : oldStr);
  }
  return "";
}

function useElapsed(startMs: number | undefined, endMs: number | null | undefined, running: boolean) {
  const [now, setNow] = useState(Date.now);
  useEffect(() => {
    if (!running || startMs == null) return;
    const id = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(id);
  }, [running, startMs]);

  if (startMs == null) return null;
  const end = endMs ?? now;
  const ms = Math.max(0, end - startMs);
  if (ms < 1000) return `${ms}ms`;
  return `${(ms / 1000).toFixed(1)}s`;
}

export function ToolCallCard({ part }: { part: Extract<MessagePart, { type: "tool-call" }> }) {
  const [expanded, setExpanded] = useState(false);
  const [copied, setCopied] = useState(false);
  const toolState = useSessionStore((s) => s.toolStates.get(part.callId));
  const toolOutput = useSessionStore((s) => s.toolOutputs.get(part.callId)) ?? part.output;

  const state = toolState ?? part.state ?? "pending";
  const hasOutput = toolOutput && toolOutput.length > 0;
  const summary = inputSummary(part.input);
  const sensitivity = inferSensitivity(part.toolName);
  const sens = sensitivityMeta(sensitivity);
  const elapsed = useElapsed(part.time?.start, part.time?.end, state === "running");

  const handleCopy = async () => {
    if (!toolOutput) return;
    try {
      await navigator.clipboard.writeText(toolOutput);
      setCopied(true);
      setTimeout(() => setCopied(false), 1200);
    } catch {
      // ignore
    }
  };

  return (
    <div
      className={cn(
        "max-w-[85%] rounded-lg border bg-card",
        state === "error" && "border-destructive",
        state !== "error" && "border-border",
      )}
    >
      <button
        onClick={() => setExpanded(!expanded)}
        className="motion-pressable flex w-full items-center gap-2 px-4 py-2 text-left"
      >
        {expanded ? (
          <ChevronDown className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
        ) : (
          <ChevronRight className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
        )}
        <StateIcon state={state} />
        <Terminal className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
        <span className="font-mono text-sm">{part.toolName}</span>
        {summary && (
          <span className="truncate text-xs text-muted-foreground">{summary}</span>
        )}
        <span className={cn("ml-auto shrink-0 rounded px-1.5 py-0.5 text-[9px] font-medium uppercase", sens.className)}>
          {sens.label}
        </span>
        <span
          className={cn(
            "shrink-0 text-xs capitalize tabular-nums",
            state === "error" && "text-destructive",
            state === "completed" && "text-green-500",
            state === "running" && "text-blue-500",
            state === "pending" && "text-muted-foreground",
          )}
        >
          {state}
        </span>
        {elapsed && <span className="shrink-0 text-[10px] text-muted-foreground">{elapsed}</span>}
      </button>

      {hasOutput && !expanded && (
        <div className="border-t border-border">
          <div className="group relative">
            <button
              onClick={handleCopy}
              className="motion-pressable absolute right-2 top-2 rounded border border-border bg-background p-1 opacity-0 transition-opacity group-hover:opacity-100 focus-visible:opacity-100"
              title="Copy output"
            >
              {copied ? <CheckCircle2 className="h-3 w-3 text-green-500" /> : <Copy className="h-3 w-3" />}
            </button>
            <pre className="max-h-32 overflow-auto px-4 py-2 text-xs text-muted-foreground">
              {toolOutput}
            </pre>
          </div>
        </div>
      )}

      {expanded && (
        <div className="border-t border-border px-4 py-2">
          {part.input != null && (
            <div className="mb-2">
              <div className="flex items-center justify-between">
                <span className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
                  Input
                </span>
                {(part.toolName === "edit" || part.toolName === "write") && (
                  <span className="flex items-center gap-1 text-[10px] text-muted-foreground">
                    <FileEdit className="h-3 w-3" />
                    diff view
                  </span>
                )}
              </div>
              {(part.toolName === "edit" || part.toolName === "write") && part.input && typeof part.input === "object" ? (
                <ToolInputDiff toolName={part.toolName} input={part.input as Record<string, unknown>} />
              ) : (
                <pre className="mt-1 overflow-x-auto rounded bg-muted p-2 text-xs">
                  {JSON.stringify(part.input, null, 2)}
                </pre>
              )}
            </div>
          )}
          {hasOutput && (
            <div>
              <div className="flex items-center justify-between">
                <span className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
                  Output
                </span>
                <button
                  onClick={handleCopy}
                  className="motion-pressable flex items-center gap-1 rounded border border-border px-1.5 py-0.5 text-[10px] hover:bg-accent"
                >
                  {copied ? <CheckCircle2 className="h-3 w-3 text-green-500" /> : <Copy className="h-3 w-3" />}
                  {copied ? "copied" : "copy"}
                </button>
              </div>
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

function StateIcon({ state }: { state: Extract<MessagePart, { type: "tool-call" }>["state"] }) {
  switch (state) {
    case "pending":
      return <Loader2 className="h-3.5 w-3.5 animate-spin text-muted-foreground" />;
    case "running":
      return <Loader2 className="h-3.5 w-3.5 animate-spin text-blue-500" />;
    case "completed":
      return <CheckCircle2 className="h-3.5 w-3.5 text-green-500" />;
    case "error":
      return <XCircle className="h-3.5 w-3.5 text-destructive" />;
    default:
      return null;
  }
}

function ToolInputDiff({ toolName, input }: { toolName: string; input: Record<string, unknown> }) {
  if (toolName === "edit") {
    const oldStr = String(input.old_string ?? "");
    const newStr = String(input.new_string ?? "");
    const path = String(input.path ?? "");
    return (
      <div className="mt-1 space-y-1 rounded border border-border bg-muted/50 p-2 text-xs">
        {path && <div className="font-mono text-muted-foreground">{path}</div>}
        <Diff oldStr={oldStr} newStr={newStr} />
      </div>
    );
  }

  if (toolName === "write") {
    const text = String(input.text ?? "");
    const path = String(input.path ?? "");
    return (
      <div className="mt-1 space-y-1 rounded border border-border bg-muted/50 p-2 text-xs">
        {path && <div className="font-mono text-muted-foreground">{path}</div>}
        <div className="flex items-center gap-2 text-muted-foreground">
          <FilePlus className="h-3.5 w-3.5" />
          <span>Write {text.length} characters</span>
        </div>
        <pre className="max-h-40 overflow-auto rounded bg-muted p-2 text-[10px]">{text}</pre>
      </div>
    );
  }

  return (
    <pre className="mt-1 overflow-x-auto rounded bg-muted p-2 text-xs">
      {JSON.stringify(input, null, 2)}
    </pre>
  );
}

function Diff({ oldStr, newStr }: { oldStr: string; newStr: string }) {
  const lines = useMemo(() => computeLineDiff(oldStr, newStr), [oldStr, newStr]);

  if (oldStr === newStr) {
    return <div className="text-muted-foreground">No changes</div>;
  }

  return (
    <div className="max-h-48 overflow-auto rounded border border-border bg-background font-mono text-[10px]">
      {lines.map((line, i) => (
        <div
          key={i}
          className={cn(
            "flex",
            line.kind === "old" && "bg-red-500/10 text-red-600 dark:text-red-400",
            line.kind === "new" && "bg-green-500/10 text-green-600 dark:text-green-400",
          )}
        >
          <span className="w-5 shrink-0 select-none pl-1 text-muted-foreground">
            {line.kind === "old" ? "-" : line.kind === "new" ? "+" : " "}
          </span>
          <span className="min-w-0 whitespace-pre-wrap">{line.text}</span>
        </div>
      ))}
    </div>
  );
}

function computeLineDiff(oldStr: string, newStr: string): { kind: "old" | "new" | "same"; text: string }[] {
  const oldLines = oldStr.split("\n");
  const newLines = newStr.split("\n");
  const result: { kind: "old" | "new" | "same"; text: string }[] = [];
  let oi = 0;
  let ni = 0;
  while (oi < oldLines.length || ni < newLines.length) {
    const ol: string | undefined = oldLines[oi];
    const nl: string | undefined = newLines[ni];
    if (ol === nl && ol !== undefined) {
      result.push({ kind: "same", text: ol });
      oi++;
      ni++;
    } else if (nl !== undefined && (ol === undefined || !oldLines.includes(nl))) {
      result.push({ kind: "new", text: nl });
      ni++;
    } else if (ol !== undefined && (nl === undefined || !newLines.includes(ol))) {
      result.push({ kind: "old", text: ol });
      oi++;
    } else if (ol !== undefined && nl !== undefined) {
      result.push({ kind: "old", text: ol });
      result.push({ kind: "new", text: nl });
      oi++;
      ni++;
    } else {
      break;
    }
  }
  return result;
}
