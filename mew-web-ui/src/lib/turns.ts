import type { ChatMessage, MessagePart } from "../stores/session";

/** One user prompt plus every assistant/tool message produced while answering it.
 *
 *  The daemon stores one assistant message per provider round, so a single
 *  answer spans several messages. The transcript renders turns, not rounds. */
export interface Turn {
  key: string;
  /** Index of this turn's first message, used for scroll targeting. */
  messageIndex: number;
  messages: ChatMessage[];
}

function startsTurn(message: ChatMessage): boolean {
  return message.role === "user" && message.parts.some((part) => part.type === "text" && part.text.trim());
}

/** Group messages into turns. Tool results arrive as user messages with no parts,
 *  so they stay with the turn they belong to instead of starting a new one. */
export function buildTurns(messages: ChatMessage[]): Turn[] {
  const turns: Turn[] = [];
  for (const [index, message] of messages.entries()) {
    const current = turns[turns.length - 1];
    if (!current || startsTurn(message)) {
      turns.push({ key: message.id, messageIndex: index, messages: [message] });
    } else {
      current.messages.push(message);
    }
  }
  return turns;
}

/** Split a turn's assistant parts into folded work and the trailing answer.
 *
 *  Everything before the last assistant message's closing text run is work:
 *  reasoning, tool calls, and the narration between them. */
export function splitTurnParts(messages: ChatMessage[]): { work: MessagePart[]; response: MessagePart[] } {
  const assistants = messages.filter((message) => message.role === "assistant");
  const last = assistants[assistants.length - 1];
  if (!last) return { work: [], response: [] };

  let start = last.parts.length;
  while (start > 0) {
    const part = last.parts[start - 1];
    if (part?.type === "text" && part.text.trim()) start -= 1;
    else break;
  }

  const earlier = assistants.slice(0, -1).flatMap((message) => message.parts);
  return {
    work: [...earlier, ...last.parts.slice(0, start)].filter((part) => part.type !== "compaction"),
    response: last.parts.slice(start).filter((part) => part.type !== "compaction"),
  };
}

export type WorkSegment =
  | { kind: "text"; part: Extract<MessagePart, { type: "text" }> }
  | { kind: "steps"; parts: MessagePart[] };

/** Group a turn's work parts into visible intermediate prose beats and
 *  collapsed step runs (reasoning, tool calls, attachments, etc.). */
export function groupWorkSegments(work: MessagePart[]): WorkSegment[] {
  const segments: WorkSegment[] = [];
  for (const part of work) {
    if (part.type === "text") {
      if (!part.streaming && !part.text?.trim()) continue;
      segments.push({ kind: "text", part });
      continue;
    }
    const prev = segments[segments.length - 1];
    if (prev?.kind === "steps") {
      prev.parts.push(part);
    } else {
      segments.push({ kind: "steps", parts: [part] });
    }
  }
  return segments;
}

export function formatDuration(ms: number): string {
  const totalSeconds = Math.max(0, Math.round(ms / 1000));
  if (totalSeconds < 60) return `${totalSeconds}s`;
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  if (minutes < 60) return `${minutes}m ${seconds}s`;
  return `${Math.floor(minutes / 60)}h ${minutes % 60}m`;
}
