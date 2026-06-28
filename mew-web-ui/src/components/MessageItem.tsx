import { useState } from "react";
import { ChevronRight, Terminal } from "lucide-react";
import { type ChatMessage, type MessagePart } from "../stores/session";
import { ToolCallCard } from "./ToolCallCard";
import { MarkdownBody } from "./MarkdownBody";
import { CopyButton } from "./CopyButton";
import { cn } from "../lib/utils";

/** Group consecutive tool-call parts together so they can be collapsed
 *  into a single card instead of showing N separate cards. */
type PartGroup =
  | { kind: "single"; parts: [MessagePart] }
  | { kind: "tool-group"; parts: Extract<MessagePart, { type: "tool-call" }>[] };

function groupParts(parts: MessagePart[]): PartGroup[] {
  const groups: PartGroup[] = [];
  let toolBuffer: Extract<MessagePart, { type: "tool-call" }>[] = [];

  for (const part of parts) {
    if (part.type === "tool-call") {
      toolBuffer.push(part);
    } else {
      if (toolBuffer.length > 0) {
        groups.push(
          toolBuffer.length === 1
            ? { kind: "single", parts: [toolBuffer[0] as MessagePart] }
            : { kind: "tool-group", parts: toolBuffer },
        );
        toolBuffer = [];
      }
      groups.push({ kind: "single", parts: [part] });
    }
  }
  if (toolBuffer.length > 0) {
    groups.push(
      toolBuffer.length === 1
        ? { kind: "single", parts: [toolBuffer[0] as MessagePart] }
        : { kind: "tool-group", parts: toolBuffer },
    );
  }
  return groups;
}

/** A collapsible group of consecutive tool calls. Shows a summary header
 *  (e.g. "3 tool calls: read, glob, grep") and expands to show individual
 *  ToolCallCards. */
function ToolCallGroup({ parts }: { parts: Extract<MessagePart, { type: "tool-call" }>[] }) {
  const [expanded, setExpanded] = useState(false);
  const toolNames = parts.map((p) => p.toolName);
  const uniqueNames = [...new Set(toolNames)];

  return (
    <div className="max-w-[85%]">
      <button
        onClick={() => setExpanded(!expanded)}
        className="flex w-full items-center gap-2 rounded-lg border border-border bg-card px-4 py-2 text-left"
      >
        <Terminal className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
        <span className="text-sm font-medium">
          {parts.length} tool calls
        </span>
        <span className="truncate text-xs text-muted-foreground">
          {uniqueNames.join(", ")}
        </span>
        <ChevronRight className={cn("ml-auto h-3.5 w-3.5 shrink-0 text-muted-foreground transition-transform", expanded && "rotate-90")} />
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
  streamingText,
  streamingReasoningText,
}: {
  message: ChatMessage;
  streamingText: string;
  streamingReasoningText: string;
}) {
  const isUser = message.role === "user";
  const copyText = message.parts
    .filter((p): p is Extract<MessagePart, { type: "text" }> => p.type === "text")
    .map((p) => p.text)
    .join("\n\n");

  return (
    <div className={cn("group flex flex-col gap-2", isUser ? "items-end" : "items-start")}>
      {!isUser && copyText && (
        <CopyButton text={copyText} className="self-end opacity-0 group-hover:opacity-100" />
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
            "max-w-[85%] rounded-lg px-4 py-2.5",
            isUser
              ? "bg-primary text-primary-foreground"
              : "bg-card border border-border",
          )}
        >
          {part.streaming ? (
            <MarkdownBody highlight={false}>{streamingText || "…"}</MarkdownBody>
          ) : (
            <MarkdownBody highlight={true}>{part.text}</MarkdownBody>
          )}
        </div>
      );

    case "reasoning":
      return (
        <details className="max-w-[85%] rounded-lg border border-border bg-muted/50 px-4 py-2 text-sm text-muted-foreground">
          <summary className="cursor-pointer select-none text-xs font-medium uppercase tracking-wide">
            Reasoning
          </summary>
          <div className="mt-2 whitespace-pre-wrap">
            {part.streaming ? streamingReasoningText || "…" : part.text}
          </div>
        </details>
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
