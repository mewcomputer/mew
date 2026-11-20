import { useEffect, useMemo, useState } from "react";
import { ChevronRight, ChevronDown, Clock3, CheckCircle2, XCircle, Loader2, Copy, FileEdit, FilePlus } from "lucide-react";
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
      return { label: "Read only", className: "text-muted-foreground" };
    case "mutating":
      return { label: "Mutating", className: "text-text-warning" };
    case "dangerous":
      return { label: "Dangerous", className: "text-destructive" };
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
    if (command) return command;
    if (path) return path;
    if (pattern) return pattern;
    if (text) return text;
    if (oldStr) return "old: " + oldStr;
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
  const [copyError, setCopyError] = useState(false);
  const toolState = useSessionStore((s) => s.toolStates.get(part.callId));
  const toolOutput = useSessionStore((s) => s.toolOutputs.get(part.callId)) ?? part.output;

  const state = toolState ?? part.state ?? "pending";
  const hasOutput = toolOutput && toolOutput.length > 0;
  const summary = inputSummary(part.input);
  const sensitivity = inferSensitivity(part.toolName);
  const source = part.toolName === "python" && part.input && typeof part.input === "object"
    ? (part.input as Record<string, unknown>).source : undefined;
  const sens = sensitivityMeta(sensitivity);
  const elapsed = useElapsed(part.time?.start, part.time?.end, state === "running");

  const handleCopy = async () => {
    if (!toolOutput) return;
    setCopyError(false);
    try {
      await navigator.clipboard.writeText(toolOutput);
      setCopied(true);
      setTimeout(() => setCopied(false), 1200);
    } catch {
      setCopied(false);
      setCopyError(true);
    }
  };

  return (
    <div
      className={cn(
        "min-w-0 w-full max-w-full rounded-lg border bg-card",
        state === "error" && "border-destructive",
        state !== "error" && "border-border",
      )}
    >
      <button
        type="button"
        onClick={() => setExpanded(!expanded)}
        aria-label={`${expanded ? "Hide" : "Show"} ${part.toolName} details`}
        aria-expanded={expanded}
        className="motion-pressable flex min-w-0 w-full items-start gap-2 rounded-lg px-3 py-2 text-left hover:bg-muted/40"
      >
        <span className="mt-0.5 shrink-0" aria-hidden="true"><StateIcon state={state} /></span>
        <span className="min-w-0 flex-1">
          <span className="flex flex-wrap items-baseline gap-x-2 gap-y-0.5">
            <span className="min-w-0 break-words font-mono text-xs leading-5 [overflow-wrap:anywhere]">{part.toolName}</span>
            <span className="flex flex-wrap items-center gap-x-2 gap-y-0.5 text-[10px] leading-4 text-muted-foreground">
              <span className={cn("capitalize", state === "error" && "text-destructive", state === "completed" && "text-text-success", state === "running" && "text-text-accent")}>{state}</span>
              {elapsed && <span className="tabular-nums">{elapsed}</span>}
              <span className={sens.className}>{sens.label}</span>
            </span>
          </span>
          {summary && <span className="block truncate text-[11px] leading-4 text-muted-foreground" title={summary}>{summary}</span>}
        </span>
        {expanded ? (
          <ChevronDown className="mt-0.5 h-3.5 w-3.5 shrink-0 text-muted-foreground" aria-hidden="true" />
        ) : (
          <ChevronRight className="mt-0.5 h-3.5 w-3.5 shrink-0 text-muted-foreground" aria-hidden="true" />
        )}
      </button>

      {typeof source === "string" && (
        <pre className="max-h-40 overflow-auto border-t border-border px-3 py-2 text-[11px] leading-relaxed">{source}</pre>
      )}
      {part.nestedCalls && part.nestedCalls.length > 0 && (
        <div className="space-y-2 border-t border-border px-3 py-2 text-xs" aria-label="Nested tools">
          {part.nestedCalls.map((call) => (
            <div key={call.call_id}>
              <div className="flex flex-wrap items-center gap-x-2">
                <span className="font-medium">{call.tool_name}</span>
                <span className={cn("text-muted-foreground", call.status === "error" && "text-destructive", call.status === "completed" && "text-text-success")}>{call.status}</span>
                {inputSummary(call.input) && <span className="min-w-0 break-all text-muted-foreground">{inputSummary(call.input)}</span>}
                {call.output_bytes != null && <span className="text-muted-foreground">{call.output_bytes} B</span>}
              </div>
              {call.error && <div className="mt-1 whitespace-pre-wrap text-destructive">{call.error}</div>}
              {expanded && <pre className="mt-1 max-h-40 overflow-auto rounded bg-muted p-2 text-[11px]">{JSON.stringify(call.input, null, 2)}</pre>}
            </div>
          ))}
        </div>
      )}

      {hasOutput && !expanded && (
        <div className="min-w-0 border-t border-border">
          <div className="group relative min-w-0">
            <button
              type="button"
              onClick={handleCopy}
              aria-label="Copy output"
              className="motion-pressable absolute right-2 top-2 rounded border border-border bg-background p-1 text-muted-foreground opacity-0 transition-opacity hover:text-foreground group-hover:opacity-100 group-focus-within:opacity-100 focus-visible:opacity-100"
              title="Copy output"
            >
              {copied ? <CheckCircle2 className="h-3 w-3 text-text-success" /> : <Copy className="h-3 w-3" />}
            </button>
            <pre className="max-h-32 min-w-0 max-w-full overflow-auto py-2 pl-3 pr-10 text-[11px] leading-relaxed text-muted-foreground">
              {toolOutput}
            </pre>
          </div>
        </div>
      )}

      {expanded && (
        <div className="min-w-0 border-t border-border px-3 py-2">
          {part.input != null && typeof source !== "string" && (
            <div className="mb-2">
              <div className="flex items-center justify-between">
                <span className="text-[11px] font-medium text-muted-foreground">
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
                <pre className="mt-1 max-h-64 min-w-0 max-w-full overflow-auto rounded bg-muted p-2 text-[11px] leading-relaxed">
                  {JSON.stringify(part.input, null, 2)}
                </pre>
              )}
            </div>
          )}
          {hasOutput && (
            <div>
              <div className="flex items-center justify-between">
                <span className="text-[11px] font-medium text-muted-foreground">
                  Output
                </span>
                <button
                  type="button"
                  onClick={handleCopy}
                  aria-label="Copy output"
                  className="motion-pressable flex items-center gap-1 rounded px-1.5 py-0.5 text-[10px] text-muted-foreground hover:bg-accent hover:text-foreground"
                >
                  {copied ? <CheckCircle2 className="h-3 w-3 text-text-success" /> : <Copy className="h-3 w-3" />}
                  {copied ? "Copied" : "Copy"}
                </button>
              </div>
              <pre className="mt-1 max-h-64 min-w-0 max-w-full overflow-auto rounded bg-muted p-2 text-[11px] leading-relaxed">
                {toolOutput}
              </pre>
            </div>
          )}
        </div>
      )}
      {copied && <span role="status" className="sr-only">Output copied</span>}
      {copyError && <p role="alert" className="px-3 pb-2 text-[11px] text-destructive">Could not copy output. Try again.</p>}
    </div>
  );
}

function StateIcon({ state }: { state: Extract<MessagePart, { type: "tool-call" }>["state"] }) {
  switch (state) {
    case "pending":
      return <Clock3 className="h-3.5 w-3.5 text-muted-foreground" />;
    case "running":
      return <Loader2 className="h-3.5 w-3.5 animate-spin text-text-accent" />;
    case "completed":
      return <CheckCircle2 className="h-3.5 w-3.5 text-text-success" />;
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
        {path && <div className="break-all font-mono text-muted-foreground">{path}</div>}
        <Diff oldStr={oldStr} newStr={newStr} />
      </div>
    );
  }

  if (toolName === "write") {
    const text = String(input.text ?? "");
    const path = String(input.path ?? "");
    return (
      <div className="mt-1 space-y-1 rounded border border-border bg-muted/50 p-2 text-xs">
        {path && <div className="break-all font-mono text-muted-foreground">{path}</div>}
        <div className="flex items-center gap-2 text-muted-foreground">
          <FilePlus className="h-3.5 w-3.5" />
          <span>Write {text.length} characters</span>
        </div>
        <pre className="max-h-40 max-w-full overflow-auto rounded bg-muted p-2 text-[10px]">{text}</pre>
      </div>
    );
  }

  return (
    <pre className="mt-1 max-w-full overflow-x-auto rounded bg-muted p-2 text-xs">
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
            line.kind === "old" && "bg-destructive/10 text-destructive",
            line.kind === "new" && "bg-text-success/10 text-text-success",
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
