import { useEffect, useMemo, useState } from "react";
import { ChevronRight, Clock3, CheckCircle2, XCircle, Loader2, Copy, FilePlus } from "lucide-react";
import { useSessionStore, type MessagePart } from "../stores/session";
import { cn } from "../lib/utils";
import { CodeBlock } from "./code-block";
import { formatPythonForDisplay, MAX_PYTHON_DISPLAY_LENGTH } from "../lib/python-display";
import { describePythonCalls, parsePythonCalls } from "../lib/python-calls";

type Sensitivity = "readonly" | "mutating" | "dangerous";
type ToolPart = Extract<MessagePart, { type: "tool-call" }>;

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

function asRecord(input: unknown): Record<string, unknown> {
  return input && typeof input === "object" ? (input as Record<string, unknown>) : {};
}

function stringField(input: unknown, ...keys: string[]): string {
  const record = asRecord(input);
  for (const key of keys) {
    const value = record[key];
    if (typeof value === "string" && value.trim()) return value;
  }
  return "";
}

/** Drop the workspace prefix so long paths stay scannable in a one-line row.
 *  Paths outside the workspace keep their last two segments; the full path is
 *  still on the row's title. */
function relativePath(path: string, cwd: string | null): string {
  if (!path) return path;
  if (cwd) {
    const root = cwd.endsWith("/") ? cwd : `${cwd}/`;
    if (path.startsWith(root)) return path.slice(root.length);
  }
  const segments = path.split("/").filter(Boolean);
  if (segments.length <= 3) return path;
  return `…/${segments.slice(-2).join("/")}`;
}

function patchPath(patch: string): string {
  const match = patch.match(/^\[([^#\]\n]+)#/m);
  return match ? match[1]!.trim() : "";
}

/** The target path for tools that carry one, including hash-anchored patches. */
function inputPath(input: unknown): string {
  return stringField(input, "path", "file_path", "file") || patchPath(stringField(input, "patch"));
}

/** The collapsed row states what the call did ("Edited src/app.tsx") instead of
 *  echoing the tool id and its JSON payload. */
function describeCall(part: ToolPart, cwd: string | null): { verb: string; detail: string; title: string } {
  const name = part.toolName;
  const path = inputPath(part.input);
  const shown = relativePath(path, cwd);
  if (name === "read") return { verb: "Read", detail: shown, title: path };
  if (name.startsWith("edit")) return { verb: "Edited", detail: shown, title: path };
  if (name.startsWith("write")) return { verb: "Wrote", detail: shown, title: path };
  if (name === "grep") {
    const pattern = stringField(part.input, "pattern");
    return { verb: "Searched", detail: pattern ? `"${pattern}"` : "", title: pattern };
  }
  if (name === "glob") {
    const pattern = stringField(part.input, "pattern");
    return { verb: "Listed", detail: pattern, title: pattern };
  }
  if (name === "bash") {
    const command = stringField(part.input, "command");
    return { verb: "Ran", detail: command, title: command };
  }
  if (name === "web_fetch") {
    const url = stringField(part.input, "url");
    return { verb: "Fetched", detail: url, title: url };
  }
  if (name === "python") {
    // After the run the nested calls are known; before it, read them off the
    // source so the row still says what the cell is about to do.
    const nested = [...new Set((part.nestedCalls ?? []).map((call) => call.tool_name))];
    const detail = nested.length
      ? nested.join(", ")
      : describePythonCalls(parsePythonCalls(stringField(part.input, "source")));
    return { verb: "Python", detail, title: detail };
  }
  return { verb: name, detail: "", title: "" };
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

/** One line per tool call, with detail folded away.
 *
 *  All tool calls stay collapsed by default until the user manually opens
 *  them so live turns and settled transcripts stay scannable. */
export function ToolCallCard({ part }: { part: ToolPart }) {
  const [expanded, setExpanded] = useState(false);
  const [copied, setCopied] = useState(false);
  const [copyError, setCopyError] = useState(false);
  const toolState = useSessionStore((s) => s.toolStates.get(part.callId));
  const toolOutput = useSessionStore((s) => s.toolOutputs.get(part.callId)) ?? part.output;
  const cwd = useSessionStore((s) => s.sessionCwd);

  const state = toolState ?? part.state ?? "pending";
  const hasOutput = Boolean(toolOutput);
  const { verb, detail, title } = describeCall(part, cwd);
  const callDescription = stringField(part.input, "description", "reason");
  const source = part.toolName === "python" ? stringField(part.input, "source") : "";
  const displaySource = useMemo(() => (source ? formatPythonForDisplay(source) : null), [source]);
  const nested = part.nestedCalls ?? [];
  const delta = useMemo(
    () => countDiffLines(part.input),
    [part.input],
  );
  // Historic calls record a start but no end, so a computed duration would keep
  // growing. Only show a duration the tool actually reported, or a live one.
  const elapsed = useElapsed(part.time?.start, part.time?.end, state === "running");
  const duration = part.time?.end != null || state === "running" ? elapsed : null;
  const sens = sensitivityMeta(inferSensitivity(part.toolName));

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
    <div className="group/tool w-full min-w-0 max-w-full">
      <button
        type="button"
        onClick={() => setExpanded(!expanded)}
        aria-label={`${expanded ? "Hide" : "Show"} ${part.toolName} details`}
        aria-expanded={expanded}
        className="motion-pressable -ml-1 flex w-full min-w-0 items-center gap-2 rounded-md py-1 pl-1 pr-2 text-left text-xs text-muted-foreground hover:bg-muted/50 hover:text-foreground"
      >
        <StateIcon state={state} />
        <span className={cn("min-w-0 flex-1 truncate", state === "error" && "text-destructive")}>
          {callDescription ? (
            <span title={title || detail || undefined}>{callDescription}</span>
          ) : (
            <>
              <span>{verb}</span>
              {detail && <span title={title || undefined} className="pl-1.5 font-mono text-[11px] text-muted-foreground">{detail}</span>}
            </>
          )}
        </span>
        {delta && (delta.added > 0 || delta.removed > 0) && (
          <span className="shrink-0 font-mono text-[10px] tabular-nums">
            <span className="text-text-success">+{delta.added}</span>{" "}
            <span className="text-destructive">-{delta.removed}</span>
          </span>
        )}
        {duration && <span className="shrink-0 text-[10px] tabular-nums">{duration}</span>}
        <ChevronRight
          className={cn("h-3 w-3 shrink-0 transition-transform", expanded && "rotate-90")}
          aria-hidden="true"
        />
        <span className="sr-only">{state}</span>
      </button>

      {expanded && (
        <div
          className={cn(
            "motion-enter mb-1 mt-0.5 min-w-0 space-y-2 border-l pl-3",
            state === "error" ? "border-destructive/40" : "border-border",
          )}
        >
          {displaySource !== null && (
            <div className="max-h-40 overflow-auto text-[11px] leading-relaxed" aria-label="Python code">
              {displaySource.length > MAX_PYTHON_DISPLAY_LENGTH
                ? <pre>{displaySource}</pre>
                : <CodeBlock key={displaySource} code={displaySource} lang="python" showHeader={false} flush />}
            </div>
          )}

          {nested.length > 0 && (
            <div className="space-y-1 text-[11px]" aria-label="Nested tools">
              {nested.map((call) => {
                const target = stringField(call.input, "path", "pattern", "command");
                return (
                  <div key={call.call_id} className="flex min-w-0 flex-wrap items-baseline gap-x-2">
                    <span className="font-mono text-muted-foreground">{call.tool_name}</span>
                    <span className={cn("text-muted-foreground", call.status === "error" && "text-destructive", call.status === "completed" && "text-text-success")}>{call.status}</span>
                    {target && <span className="min-w-0 truncate text-muted-foreground">{target}</span>}
                    {call.error && <span className="min-w-0 basis-full whitespace-pre-wrap text-destructive">{call.error}</span>}
                  </div>
                );
              })}
            </div>
          )}

          <>
              {displaySource !== null && displaySource !== source && (
                <div>
                  <div className="text-[10px] text-muted-foreground">Original Python</div>
                  <pre className="max-h-40 overflow-auto text-[11px]">{source}</pre>
                </div>
              )}
              {part.input != null && !source && (
                <div>
                  <div className="text-[10px] text-muted-foreground">
                    Input <span className={sens.className}>· {sens.label}</span>
                  </div>
                  {isStringEdit(part.input) || typeof asRecord(part.input).text === "string"
                    ? <ToolInputDetail input={asRecord(part.input)} />
                    : <pre className="mt-1 max-h-64 min-w-0 max-w-full overflow-auto rounded bg-muted/60 p-2 text-[11px] leading-relaxed">
                        {JSON.stringify(part.input, null, 2)}
                      </pre>}
                </div>
              )}
              {hasOutput && (
                <div>
                  <div className="flex items-center justify-between">
                    <span className="text-[10px] text-muted-foreground">Output</span>
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
                  <pre className="mt-1 max-h-64 min-w-0 max-w-full overflow-auto whitespace-pre-wrap bg-transparent p-0 font-mono text-[11px] leading-relaxed text-muted-foreground">
                    {toolOutput}
                  </pre>
                </div>
              )}
          </>
        </div>
      )}
      {copied && <span role="status" className="sr-only">Output copied</span>}
      {copyError && <p role="alert" className="pl-1 text-[11px] text-destructive">Could not copy output. Try again.</p>}
    </div>
  );
}

function StateIcon({ state }: { state: ToolPart["state"] }) {
  switch (state) {
    case "pending":
      return <Clock3 className="h-3 w-3 shrink-0 text-muted-foreground" aria-hidden="true" />;
    case "running":
      return <Loader2 className="h-3 w-3 shrink-0 animate-spin text-text-accent" aria-hidden="true" />;
    case "completed":
      return <CheckCircle2 className="h-3 w-3 shrink-0 text-text-success/70" aria-hidden="true" />;
    case "error":
      return <XCircle className="h-3 w-3 shrink-0 text-destructive" aria-hidden="true" />;
    default:
      return null;
  }
}

/** Pick the input view from the payload shape, not the tool name: write-shaped
 *  input gets a diff or file preview, everything else falls back to raw JSON. */
function ToolInputDetail({ input }: { input: Record<string, unknown> }) {
  // The expanded detail is where the exact path matters; the row compacts it.
  const path = inputPath(input);

  if (isStringEdit(input)) {
    return (
      <div className="mt-1 space-y-1">
        {path && <div className="break-all font-mono text-[10px] text-muted-foreground">{path}</div>}
        <Diff oldStr={String(input.old_string)} newStr={String(input.new_string)} />
      </div>
    );
  }

  const text = String(input.text);
  return (
    <div className="mt-1 space-y-1">
      {path && <div className="break-all font-mono text-[10px] text-muted-foreground">{path}</div>}
      <div className="flex items-center gap-2 text-muted-foreground">
        <FilePlus className="h-3 w-3" aria-hidden="true" />
        <span>Write {text.length} characters</span>
      </div>
      <pre className="max-h-40 max-w-full overflow-auto rounded bg-muted/60 p-2 text-[10px]">{text}</pre>
    </div>
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

function isStringEdit(input: unknown): boolean {
  const record = asRecord(input);
  return typeof record.old_string === "string" && typeof record.new_string === "string";
}

/** Added and removed line counts for an edit, matching what the diff view shows. */
function countDiffLines(input: unknown): { added: number; removed: number } | null {
  if (!isStringEdit(input)) return null;
  const record = asRecord(input);
  const lines = computeLineDiff(String(record.old_string), String(record.new_string));
  let added = 0;
  let removed = 0;
  for (const line of lines) {
    if (line.kind === "new") added++;
    else if (line.kind === "old") removed++;
  }
  return { added, removed };
}

function computeLineDiff(oldStr: string, newStr: string): { kind: "old" | "new" | "same"; text: string }[] {  const oldLines = oldStr.split("\n");
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
