import { memo, useEffect, useMemo, useState } from "react";
import { ChevronRight } from "lucide-react";
import { selectSessionRunning, useSessionStore, type MessagePart } from "../stores/session";
import { Mascot, mascotStateForActivity } from "./mascot";
import { MessageActions, MessageItem, PartList, messageCopyText } from "./message-item";
import { formatDuration, groupWorkSegments, splitTurnParts, type Turn } from "../lib/turns";
import { cn } from "../lib/utils";

function useTick(active: boolean) {
  const [now, setNow] = useState(Date.now);
  useEffect(() => {
    if (!active) return;
    const id = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(id);
  }, [active]);
  return now;
}

function usePresence(visible: boolean, exitMs = 180): { mounted: boolean; state: "open" | "closed" } {
  const [mounted, setMounted] = useState(visible);

  useEffect(() => {
    if (visible) {
      setMounted(true);
      return;
    }
    const isJsdom = typeof navigator !== "undefined" && /jsdom/i.test(navigator.userAgent);
    const reduced =
      typeof window !== "undefined" &&
      typeof window.matchMedia === "function" &&
      window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    if (isJsdom || reduced) {
      setMounted(false);
      return;
    }
    const id = setTimeout(() => setMounted(false), exitMs);
    return () => clearTimeout(id);
  }, [visible, exitMs]);

  return {
    mounted: visible || mounted,
    state: visible ? "open" : "closed",
  };
}

function describeActiveStep(
  toolName: string | null,
  description: string | null,
  streamingReasoning: boolean,
): string | null {
  if (description) return description;
  if (toolName) {
    const t = toolName.toLowerCase();
    if (t === "read") return "Reading file";
    if (t.startsWith("edit") || t.startsWith("write")) return "Editing file";
    if (t === "grep" || t === "glob") return "Searching";
    if (t === "web_fetch") return "Fetching URL";
    if (t === "python") return "Running Python";
    if (t.startsWith("browser")) return "Using browser";
    if (t === "bash" || t === "shell") return "Running command";
    return toolName;
  }
  return streamingReasoning ? "Thinking" : null;
}

function summarizeSteps(parts: MessagePart[], toolStates: Map<string, string>) {
  let thoughts = 0;
  let tools = 0;
  let failed = 0;
  for (const part of parts) {
    if (part.type === "reasoning") thoughts += 1;
    else if (part.type === "tool-call") {
      tools += 1;
      if ((toolStates.get(part.callId) ?? part.state) === "error") failed += 1;
    }
  }
  const summary: string[] = [];
  if (thoughts > 0) summary.push(`${thoughts} ${thoughts === 1 ? "thought" : "thoughts"}`);
  if (tools > 0) summary.push(`${tools} ${tools === 1 ? "tool call" : "tool calls"}`);
  return {
    text: summary.length > 0 ? summary.join(", ") : "Steps",
    failed,
  };
}

/** Collapsed run of non-prose steps (thoughts, tool calls) rendered inline
 *  between intermediate assistant messages during a live turn. */
function WorkStepGroup({ parts }: { parts: MessagePart[] }) {
  const [open, setOpen] = useState(false);
  const toolStates = useSessionStore((s) => s.toolStates);
  const { text, failed } = summarizeSteps(parts, toolStates);

  return (
    <div className="flex min-w-0 max-w-full flex-col gap-1.5">
      <button
        type="button"
        onClick={() => setOpen((value) => !value)}
        aria-expanded={open}
        aria-label={open ? `Hide ${text}` : `Show ${text}`}
        className="-ml-1 flex min-w-0 items-center gap-2 self-start rounded-md py-1 pl-1 pr-2 text-xs text-muted-foreground hover:bg-muted/50 hover:text-foreground"
      >
        <ChevronRight className={cn("h-3 w-3 shrink-0 transition-transform", open && "rotate-90")} aria-hidden="true" />
        <span>{text}</span>
        {failed > 0 && <span className="text-destructive">{failed} failed</span>}
      </button>
      {open && (
        <div
          className={cn(
            "motion-enter flex min-w-0 max-w-full flex-col gap-1.5 border-l pl-3",
            failed > 0 ? "border-destructive/40" : "border-border/70",
          )}
        >
          <PartList parts={parts} isUser={false} />
        </div>
      )}
    </div>
  );
}

const LIVE_LINE_CLASS =
  "-ml-1 flex min-w-0 max-w-full items-center gap-2 self-start rounded-md py-1 pl-1 pr-2 text-xs text-muted-foreground";

/** Standalone live indicator rendered at the bottom of the transcript when
 *  manual /compact runs on an otherwise idle session. */
export function CompactingIndicator({ startedAt }: { startedAt: number | null }) {
  const now = useTick(true);
  const start = startedAt ?? now;
  const duration = formatDuration(Math.max(0, now - start));
  return (
    <div className={LIVE_LINE_CLASS}>
      <Mascot state="thinking" size={32} className="shrink-0" />
      <span className="shrink-0">
        Compacting context… <span className="tabular-nums">{duration}</span>
      </span>
    </div>
  );
}

/** One prompt and everything that answered it.
 *
 *  While a turn is live, intermediate prose stays visible inline and the
 *  thought/tool runs between them stay collapsed by default until opened. Once
 *  the turn finishes with a final answer, all work above that answer folds into
 *  a single "Worked for ..." line so the transcript reads as a conversation. */
export const TurnItem = memo(function TurnItem({ turn, isLast = true }: { turn: Turn; isLast?: boolean }) {
  const [expanded, setExpanded] = useState(false);
  const toolStates = useSessionStore((s) => s.toolStates);
  const compacting = useSessionStore((s) => (isLast ? s.compacting : false));
  const retryWait = useSessionStore((s) => (isLast ? s.retryWait : null));
  const hasPendingPermissions = useSessionStore((s) => (isLast ? s.pendingPermissions.length > 0 : false));
  const hasPendingAskUser = useSessionStore((s) => (isLast ? s.pendingAskUser.length > 0 : false));
  const subagents = useSessionStore((s) => (isLast ? s.subagents : null));
  const runningSubs = useMemo(() => {
    if (!subagents) return [];
    return [...subagents.values()].filter((sub) => sub.status === "running");
  }, [subagents]);
  const sessionRunning = useSessionStore((s) => (isLast ? selectSessionRunning(s) : false));
  const { work, response } = useMemo(() => splitTurnParts(turn.messages), [turn.messages]);

  const assistants = turn.messages.filter((message) => message.role === "assistant");
  const last = assistants[assistants.length - 1];
  const compactionParts = useMemo(() => {
    const beforeResponse: MessagePart[] = [];
    const afterResponse: MessagePart[] = [];
    const responseOwnerIdx =
      response.length > 0 && last ? turn.messages.lastIndexOf(last) : -1;
    turn.messages.forEach((message, idx) => {
      for (const part of message.parts) {
        if (part.type !== "compaction") continue;
        if (responseOwnerIdx >= 0 && idx < responseOwnerIdx) {
          beforeResponse.push(part);
        } else {
          afterResponse.push(part);
        }
      }
    });
    return { beforeResponse, afterResponse };
  }, [turn.messages, last, response.length]);
  const streaming = assistants.some((message) =>
    message.parts.some((part) => (part.type === "text" || part.type === "reasoning") && part.streaming),
  );
  const streamingReasoning = assistants.some((message) =>
    message.parts.some((part) => part.type === "reasoning" && part.streaming),
  );
  let runningToolName: string | null = null;
  let runningCommand: string | null = null;
  let runningDescription: string | null = null;
  for (const message of turn.messages) {
    for (const part of message.parts) {
      if (part.type === "tool-call" && (toolStates.get(part.callId) ?? part.state) === "running") {
        runningToolName = part.toolName;
        const input = part.input as Record<string, unknown> | undefined;
        runningCommand = typeof input?.command === "string" ? input.command : null;
        const desc = typeof input?.description === "string"
          ? input.description.trim()
          : typeof input?.reason === "string"
            ? input.reason.trim()
            : "";
        runningDescription = desc || null;
        break;
      }
    }
    if (runningToolName) break;
  }
  const running = runningToolName !== null;
  const live = isLast && (sessionRunning || streaming || running || (last !== undefined && last.completedAt == null));
  const liveSubagentLabel = (() => {
    if (runningSubs.length === 0) return null;
    if (runningSubs.length === 1 && runningSubs[0]!.lastProgress) {
      const sub = runningSubs[0]!;
      return `${sub.displayName ?? sub.name}: ${sub.lastProgress}`;
    }
    return `Waiting on ${runningSubs.length} subagent${runningSubs.length === 1 ? "" : "s"}`;
  })();
  const activeLabel = live
    ? compacting
      ? "Compacting context…"
      : retryWait
        ? `Retrying (${retryWait.attempt}/${retryWait.maxAttempts}) in ${retryWait.delaySecs}s — ${retryWait.reason}`
        : hasPendingPermissions
          ? "Waiting for approval"
          : hasPendingAskUser
            ? "Waiting for answer"
            : liveSubagentLabel ?? describeActiveStep(runningToolName, runningDescription, streamingReasoning)
    : null;
  const now = useTick(live);
  const livePresence = usePresence(live);
  const settledPresence = usePresence(!live);

  const failed = work.filter(
    (part) => part.type === "tool-call" && (toolStates.get(part.callId) ?? part.state) === "error",
  ).length;

  const startedAt = turn.messages[0]?.timestamp ?? now;
  const endedAt = live ? now : (last?.completedAt ?? last?.timestamp ?? startedAt);
  const duration = formatDuration(endedAt - startedAt);
  const lineClass = LIVE_LINE_CLASS;

  const liveRow = (
    <div className={lineClass}>
      <Mascot
        state={mascotStateForActivity({
          streamingReasoning,
          runningToolName,
          runningCommand,
        })}
        size={32}
        className="shrink-0"
      />
      <span className="shrink-0">
        Working for <span className="tabular-nums">{duration}</span>
      </span>
      {activeLabel && (
        <span className="min-w-0 truncate text-muted-foreground/80">{activeLabel}</span>
      )}
      {failed > 0 && (
        <span className="shrink-0 text-destructive">{failed} failed</span>
      )}
    </div>
  );

  // A single text-only answer is just a reply: no work log to fold.
  // While waiting on the first token of an active prompt (or standalone live
  // states like compacting / retry / approval), keep the live indicator visible.
  const showFastPathLive =
    live &&
    livePresence.mounted &&
    (assistants.length === 0 ||
      compacting ||
      retryWait != null ||
      hasPendingPermissions ||
      hasPendingAskUser ||
      runningSubs.length > 0);
  if (work.length === 0 && assistants.length <= 1) {
    return (
      <div className="group flex min-w-0 max-w-full flex-col gap-2 typeset">
        {turn.messages
          .filter(
            (message) =>
              message.parts.length > 0 &&
              (message.role !== "system" || message.parts.some((part) => part.type === "compaction")),
          )
          .map((message) => <MessageItem key={message.id} message={message} />)}
        {!live && last?.cancelled && (
          <div className={lineClass}>
            <span className="shrink-0 text-muted-foreground/80">Cancelled</span>
          </div>
        )}
        {showFastPathLive && (
          <div className="motion-presence" data-state={livePresence.state}>
            <div className="min-h-0 overflow-hidden">{liveRow}</div>
          </div>
        )}
      </div>
    );
  }

  // Fold work behind a disclosure once the turn has a final answer, or when a
  // turn stopped/cancelled mid-reasoning without producing a visible error.
  const hasErrorPart = work.some((part) => part.type === "error");
  const hasReasoning = work.some((part) => part.type === "reasoning");
  const collapsible =
    work.length > 0 &&
    (response.length > 0 || (!live && !hasErrorPart && (Boolean(last?.cancelled) || hasReasoning)));
  const open = live || expanded || !collapsible;

  const settledRow = (
    <>
      {collapsible && (
        <ChevronRight className={cn("h-3 w-3 shrink-0 transition-transform", open && "rotate-90")} aria-hidden="true" />
      )}
      <span className="shrink-0">
        Worked for <span className="tabular-nums">{duration}</span>
      </span>
      {!live && last?.cancelled && (
        <span className="shrink-0 text-muted-foreground/80">Cancelled</span>
      )}
      {failed > 0 && (
        <span className="shrink-0 text-destructive">{failed} failed</span>
      )}
    </>
  );

  return (
    <div className="group flex min-w-0 max-w-full flex-col gap-2 typeset">
      {turn.messages
        .filter((message) => message.role === "user" && message.parts.length > 0)
        .map((message) => <MessageItem key={message.id} message={message} />)}
      <div className="flex min-w-0 max-w-full flex-col gap-2 items-stretch">
        {settledPresence.mounted && (
          <div className="motion-presence" data-state={settledPresence.state}>
            <div className="min-h-0 overflow-hidden">
              {collapsible ? (
                <button
                  type="button"
                  onClick={() => setExpanded((value) => !value)}
                  aria-expanded={open}
                  aria-label={open ? "Hide work" : "Show work"}
                  className={cn(lineClass, "hover:bg-muted/50 hover:text-foreground")}
                >
                  {settledRow}
                </button>
              ) : (
                <div className={lineClass}>{settledRow}</div>
              )}
            </div>
          </div>
        )}
        {open && work.length > 0 && (
          live ? (
            <div className="flex min-w-0 max-w-full flex-col gap-2">
              {(() => {
                const segments = groupWorkSegments(work);
                return segments.map((segment, index) => {
                  if (segment.kind === "text") {
                    return <PartList key={`text-${index}`} parts={[segment.part]} isUser={false} />;
                  }
                  if (segments.length > 1 && segment.parts.every((part) => part.type === "reasoning")) {
                    return null;
                  }
                  return <WorkStepGroup key={`steps-${index}`} parts={segment.parts} />;
                });
              })()}
            </div>
          ) : (
            <div
              className={cn(
                "motion-enter flex min-w-0 max-w-full flex-col gap-2 border-l pl-3",
                failed > 0 ? "border-destructive/40" : "border-border",
              )}
            >
              <PartList parts={work} isUser={false} />
            </div>
          )
        )}
        {compactionParts.beforeResponse.length > 0 && (
          <PartList parts={compactionParts.beforeResponse} isUser={false} />
        )}
        {response.length > 0 && <PartList parts={response} isUser={false} />}
        {compactionParts.afterResponse.length > 0 && (
          <PartList parts={compactionParts.afterResponse} isUser={false} />
        )}
        {livePresence.mounted && (
          <div className="motion-presence" data-state={livePresence.state}>
            <div className="min-h-0 overflow-hidden">{liveRow}</div>
          </div>
        )}
        <MessageActions
          meta={last?.assistantMeta}
          text={messageCopyText(response.length > 0 ? response : turn.messages.flatMap((message) => message.parts))}
        />
      </div>
    </div>
  );
});
