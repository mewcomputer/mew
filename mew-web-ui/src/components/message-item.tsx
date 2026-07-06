import { useState } from "react";
import { ChevronRight, Terminal } from "lucide-react";
import { useSessionStore, type ChatMessage, type MessagePart } from "../stores/session";
import { ToolCallCard } from "./tool-call-card";
import { MarkdownBody } from "./markdown-body";
import { CopyButton } from "./copy-button";
import { ReasoningBlock } from "./reasoning-block";
import { cn } from "../lib/utils";

/** Group consecutive tool-call parts by sensitivity tier.
 *  - readonly + mutating calls batch together within their tier
 *  - dangerous calls are always shown individually
 *  A batch may contain mixed tool names (e.g. "2× read · 1× grep"). */
type PartGroup =
  | { kind: "single"; parts: [MessagePart] }
  | {
      kind: "tool-group";
      parts: Extract<MessagePart, { type: "tool-call" }>[];
    };

function partSensitivity(part: Extract<MessagePart, { type: "tool-call" }>): string {
  return part.sensitivity?.toLowerCase() ?? "dangerous";
}

function groupParts(parts: MessagePart[]): PartGroup[] {
  const groups: PartGroup[] = [];
  let toolBuffer: Extract<MessagePart, { type: "tool-call" }>[] = [];
  let bufferTier: string | null = null;

  function flushBuffer() {
    if (toolBuffer.length > 0) {
      groups.push(
        toolBuffer.length === 1
          ? { kind: "single", parts: [toolBuffer[0] as MessagePart] }
          : { kind: "tool-group", parts: toolBuffer },
      );
      toolBuffer = [];
      bufferTier = null;
    }
  }

  for (const part of parts) {
    if (part.type === "tool-call") {
      const tier = partSensitivity(part);
      if (tier === "dangerous") {
        // Dangerous tools are never grouped — flush existing buffer, then
        // emit this call as a standalone single group.
        flushBuffer();
        groups.push({ kind: "single", parts: [part] });
      } else if (bufferTier === null || bufferTier === tier) {
        // Same tier (or first in buffer) — accumulate.
        toolBuffer.push(part);
        bufferTier = tier;
      } else {
        // Different tier — flush and start a new buffer.
        flushBuffer();
        toolBuffer = [part];
        bufferTier = tier;
      }
    } else {
      flushBuffer();
      groups.push({ kind: "single", parts: [part] });
    }
  }
  flushBuffer();
  return groups;
}

/** A collapsible group of consecutive tool calls. Shows a summary header
 *  (e.g. "3 tool calls: read, glob, grep") and expands to show individual
 *  ToolCallCards. */
function ToolCallGroup({
  parts,
}: {
  parts: Extract<MessagePart, { type: "tool-call" }>[];
}) {
  const [expanded, setExpanded] = useState(false);
  const toolNames = parts.map((p) => p.toolName);
  // Build a count-per-tool summary preserving first-seen order,
  // e.g. "2× read · 1× grep".
  const counts: { name: string; count: number }[] = [];
  for (const name of toolNames) {
    const existing = counts.find((c) => c.name === name);
    if (existing) existing.count++;
    else counts.push({ name, count: 1 });
  }
  const summary = counts.map((c) => `${c.count}× ${c.name}`).join(" · ");

  return (
    <div className="max-w-[85%]">
      <button
        onClick={() => setExpanded(!expanded)}
        className="flex w-full items-center gap-2 rounded-lg border border-border bg-card px-4 py-2 text-left"
      >
        <Terminal className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
        <span className="text-sm font-medium">{parts.length} tool calls</span>
        <span className="truncate text-xs text-muted-foreground">
          {summary}
        </span>
        <ChevronRight
          className={cn(
            "ml-auto h-3.5 w-3.5 shrink-0 text-muted-foreground transition-transform",
            expanded && "rotate-90",
          )}
        />
      </button>
      {expanded && (
        <div className="mt-1 space-y-1 pl-2">
          {parts.map((part, i) => (
            <ToolCallCard key={i} part={part} />
          ))}
        </div>
      )}
    </div>
  );
}

export function MessageItem({
  message,
}: {
  message: ChatMessage;
}) {
  const streamingText = useSessionStore((s) => s.streamingText);
  const streamingReasoningText = useSessionStore((s) => s.streamingReasoningText);
  const isUser = message.role === "user";
  const copyText = message.parts
    .filter(
      (p): p is Extract<MessagePart, { type: "text" }> => p.type === "text",
    )
    .map((p) => p.text)
    .join("\n\n");

  return (
    <div
      className={cn(
        "group flex flex-col gap-2",
        isUser ? "items-end" : "items-start",
      )}
    >
      {!isUser && copyText && (
        <CopyButton
          text={copyText}
          className="self-end opacity-0 group-hover:opacity-100"
        />
      )}
      {groupParts(message.parts).map((group, i) => {
        if (group.kind === "single") {
          const part = group.parts[0];
          // Skip empty text parts — they create blank bubbles.
          if (part.type === "text" && !part.text?.trim()) return null;
          return (
            <PartRenderer
              key={i}
              part={part}
              isUser={isUser}
              streamingText={streamingText}
              streamingReasoningText={streamingReasoningText}
            />
          );
        }
        // Group of consecutive tool calls.
        return <ToolCallGroup key={i} parts={group.parts} />;
      })}
    </div>
  );
}

function PartRenderer({
  part,
  isUser,
  streamingText,
  streamingReasoningText,
}: {
  part: MessagePart;
  isUser: boolean;
  streamingText: string;
  streamingReasoningText: string;
}) {
  switch (part.type) {
    case "text":
      return (
        <div
          className={cn(
            "max-w-[85%] rounded-lg py-2.5",
            isUser ? "bg-primary text-primary-foreground px-4" : "",
          )}
        >
          {part.streaming ? (
            <MarkdownBody highlight={false}>
              {streamingText || "…"}
            </MarkdownBody>
          ) : (
            <MarkdownBody highlight={true}>{part.text}</MarkdownBody>
          )}
        </div>
      );

    case "reasoning":
      return (
        <ReasoningBlock
          text={part.streaming ? streamingReasoningText || "…" : part.text}
          streaming={part.streaming}
        />
      );

    case "tool-call":
      return <ToolCallCard part={part} />;

    case "error":
      return (
        <div className="max-w-[85%] rounded-lg border border-destructive/50 bg-destructive/10 px-4 py-2.5 text-sm text-destructive">
          <span className="font-medium">Error:</span> {part.message}
        </div>
      );

    default:
      return null;
  }
}
