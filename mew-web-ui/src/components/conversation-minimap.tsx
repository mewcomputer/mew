import { useEffect, useMemo, useRef, useState } from "react";
import { Bookmark } from "lucide-react";
import type { ChatMessage } from "../stores/session";
import { cn } from "../lib/utils";

export interface ConversationTurn {
  id: string;
  messageIndex: number;
  promptText: string;
  replyText: string;
}

const BOOKMARK_STORAGE_PREFIX = "mew:bookmarked-turns:";

function loadBookmarks(sessionId: string | null): Set<string> {
  if (!sessionId || typeof window === "undefined") return new Set();
  try {
    const raw = window.localStorage.getItem(`${BOOKMARK_STORAGE_PREFIX}${sessionId}`);
    if (!raw) return new Set();
    const parsed = JSON.parse(raw);
    return Array.isArray(parsed) ? new Set(parsed.filter((x): x is string => typeof x === "string")) : new Set();
  } catch {
    return new Set();
  }
}

function saveBookmarks(sessionId: string | null, bookmarks: Set<string>) {
  if (!sessionId || typeof window === "undefined") return;
  try {
    window.localStorage.setItem(`${BOOKMARK_STORAGE_PREFIX}${sessionId}`, JSON.stringify([...bookmarks]));
  } catch {
    // ignore storage errors
  }
}

function extractMessageText(msg: ChatMessage, streamingText?: string): string {
  const parts: string[] = [];
  for (const part of msg.parts) {
    if (part.type === "text") {
      const text = part.streaming ? (streamingText || part.text) : part.text;
      if (text?.trim()) parts.push(text.trim());
    } else if (part.type === "error" && part.message.trim()) {
      parts.push(`Error: ${part.message.trim()}`);
    }
  }
  return parts.join(" ").replace(/\s+/g, " ").trim();
}

function extractToolFallback(msg: ChatMessage): string {
  const tools = msg.parts
    .filter((p): p is Extract<typeof p, { type: "tool-call" }> => p.type === "tool-call")
    .map((p) => p.toolName);
  if (tools.length === 0) return "";
  const unique = [...new Set(tools)];
  return `Ran ${unique.slice(0, 3).join(", ")}${unique.length > 3 ? "…" : ""}`;
}

/** Pair each user message with its subsequent assistant messages into navigable turns. */
export function buildConversationTurns(
  messages: ChatMessage[],
  streamingText = "",
): ConversationTurn[] {
  const turns: ConversationTurn[] = [];
  let i = 0;

  while (i < messages.length) {
    const msg = messages[i];
    if (!msg || msg.role !== "user") {
      i += 1;
      continue;
    }

    const promptText = extractMessageText(msg) || "User message";
    const replyChunks: string[] = [];
    const startIndex = i;
    i += 1;

    while (i < messages.length && messages[i]?.role !== "user") {
      const next = messages[i];
      if (next && next.role === "assistant") {
        const text = extractMessageText(next, streamingText) || extractToolFallback(next);
        if (text) replyChunks.push(text);
      }
      i += 1;
    }

    turns.push({
      id: msg.id,
      messageIndex: startIndex,
      promptText,
      replyText: replyChunks.join(" ").trim(),
    });
  }

  return turns;
}

/** Format inline markdown bold (**text**) into strong spans while stripping noisy syntax for the 3-line preview. */
function renderReplyPreview(text: string) {
  if (!text) {
    return <span className="italic opacity-70">Working on response…</span>;
  }
  const cleaned = text
    .replace(/```[\s\S]*?```/g, "[code]")
    .replace(/`([^`]+)`/g, "$1")
    .replace(/^#+\s+/gm, "");

  const segments = cleaned.split(/(\*\*[^*]+\*\*)/g);
  return segments.map((segment, idx) => {
    if (segment.startsWith("**") && segment.endsWith("**") && segment.length > 4) {
      return (
        <strong key={idx} className="font-semibold text-foreground/85">
          {segment.slice(2, -2)}
        </strong>
      );
    }
    return <span key={idx}>{segment}</span>;
  });
}

export function ConversationMinimap({
  messages,
  sessionId,
  streamingText,
  activeMessageIndex = 0,
  onSelectMessageIndex,
}: {
  messages: ChatMessage[];
  sessionId: string | null;
  streamingText?: string;
  activeMessageIndex?: number;
  onSelectMessageIndex: (index: number) => void;
}) {
  const turns = useMemo(
    () => buildConversationTurns(messages, streamingText),
    [messages, streamingText],
  );
  const [bookmarks, setBookmarks] = useState<Set<string>>(() => loadBookmarks(sessionId));
  const [hoveredTurnIndex, setHoveredTurnIndex] = useState<number | null>(null);
  const [popoverTopPx, setPopoverTopPx] = useState<number>(0);
  const containerRef = useRef<HTMLDivElement>(null);
  const railRef = useRef<HTMLDivElement>(null);
  const hideTimerRef = useRef<number | null>(null);

  useEffect(() => {
    setBookmarks(loadBookmarks(sessionId));
  }, [sessionId]);

  useEffect(() => {
    return () => {
      if (hideTimerRef.current !== null) {
        window.clearTimeout(hideTimerRef.current);
      }
    };
  }, []);

  const activeTurnIndex = useMemo(() => {
    if (turns.length === 0) return 0;
    let best = 0;
    for (let i = 0; i < turns.length; i++) {
      if (turns[i]!.messageIndex <= activeMessageIndex) {
        best = i;
      } else {
        break;
      }
    }
    return best;
  }, [turns, activeMessageIndex]);

  if (turns.length === 0) return null;

  const clearHideTimer = () => {
    if (hideTimerRef.current !== null) {
      window.clearTimeout(hideTimerRef.current);
      hideTimerRef.current = null;
    }
  };

  const scheduleHide = () => {
    clearHideTimer();
    hideTimerRef.current = window.setTimeout(() => {
      setHoveredTurnIndex(null);
    }, 110);
  };

  const selectHoveredTurn = (idx: number, tickEl?: HTMLElement | null) => {
    clearHideTimer();
    setHoveredTurnIndex(idx);
    const container = containerRef.current;
    if (container && tickEl) {
      const containerRect = container.getBoundingClientRect();
      const tickRect = tickEl.getBoundingClientRect();
      if (containerRect.height > 0) {
        setPopoverTopPx(tickRect.top + tickRect.height / 2 - containerRect.top - 36);
        return;
      }
    }
    const rail = railRef.current;
    if (rail && turns.length > 1) {
      setPopoverTopPx((idx / (turns.length - 1)) * rail.clientHeight);
    } else {
      setPopoverTopPx(0);
    }
  };

  const updatePointer = (clientY: number) => {
    const rail = railRef.current;
    if (!rail || turns.length === 0) return;
    const buttons = rail.querySelectorAll<HTMLElement>("[data-turn-tick]");
    if (buttons.length === 0) return;

    let closestIdx = 0;
    let closestDist = Infinity;
    let closestEl: HTMLElement | null = null;
    buttons.forEach((btn, idx) => {
      const rect = btn.getBoundingClientRect();
      const center = rect.top + rect.height / 2;
      const dist = Math.abs(clientY - center);
      if (dist < closestDist) {
        closestDist = dist;
        closestIdx = idx;
        closestEl = btn;
      }
    });
    selectHoveredTurn(closestIdx, closestEl);
  };

  const handleToggleBookmark = (turnId: string, e: React.MouseEvent) => {
    e.stopPropagation();
    setBookmarks((prev) => {
      const next = new Set(prev);
      if (next.has(turnId)) {
        next.delete(turnId);
      } else {
        next.add(turnId);
      }
      saveBookmarks(sessionId, next);
      return next;
    });
  };

  const previewTurn = hoveredTurnIndex !== null ? turns[hoveredTurnIndex] : null;

  return (
    <div
      ref={containerRef}
      aria-label="Conversation timeline"
      onMouseEnter={clearHideTimer}
      onMouseLeave={scheduleHide}
      className="pointer-events-auto absolute inset-y-6 left-1.5 z-20 hidden select-none sm:flex items-center"
    >
      {/* Vertical tick ruler: 1 tick per turn */}
      <div
        ref={railRef}
        onMouseMove={(e) => updatePointer(e.clientY)}
        className="flex max-h-full w-7 flex-col justify-center gap-1 overflow-y-auto py-2 pl-1.5 pr-1 [scrollbar-width:none] [&::-webkit-scrollbar]:hidden"
      >
        {turns.map((turn, idx) => {
          const isBookmarked = bookmarks.has(turn.id);
          const isCurrentTurn = idx === activeTurnIndex;
          const isHovered = hoveredTurnIndex === idx;

          // Smooth proximity wave across turns when hovering
          let widthPx = 6;
          if (hoveredTurnIndex !== null) {
            const dist = Math.abs(idx - hoveredTurnIndex);
            if (dist === 0) {
              widthPx = 18;
            } else if (dist === 1) {
              widthPx = 13;
            } else if (dist === 2) {
              widthPx = 9.5;
            } else if (dist === 3) {
              widthPx = 7.5;
            }
          } else if (isCurrentTurn) {
            widthPx = 11;
          } else if (isBookmarked) {
            widthPx = 9;
          }

          return (
            <button
              key={turn.id}
              type="button"
              data-turn-tick={idx}
              aria-label={`Jump to turn ${idx + 1}: ${turn.promptText}`}
              onFocus={(e) => selectHoveredTurn(idx, e.currentTarget)}
              onBlur={scheduleHide}
              onClick={() => onSelectMessageIndex(turn.messageIndex)}
              className="group flex h-2.5 w-full cursor-pointer items-center focus:outline-none"
            >
              <span
                style={{ width: `${widthPx}px` }}
                className={cn(
                  "block h-[1.5px] rounded-full transition-[width,background-color,opacity] duration-150 ease-out",
                  isHovered
                    ? "bg-foreground opacity-100"
                    : isCurrentTurn
                      ? "bg-foreground/75 opacity-90"
                      : isBookmarked
                        ? "bg-primary opacity-85"
                        : hoveredTurnIndex !== null && Math.abs(idx - hoveredTurnIndex) <= 2
                          ? "bg-muted-foreground/70"
                          : "bg-muted-foreground/35",
                )}
              />
            </button>
          );
        })}
      </div>

      {/* Floating preview card */}
      {previewTurn && (
        <div
          role="dialog"
          aria-label="Turn preview"
          onMouseEnter={clearHideTimer}
          onClick={() => onSelectMessageIndex(previewTurn.messageIndex)}
          style={{
            top: `clamp(0.5rem, calc(${popoverTopPx}px), calc(100% - 5.75rem))`,
          }}
          className="motion-surface absolute left-8 z-30 w-72 cursor-pointer rounded-xl border border-border/80 bg-popover/95 px-3.5 py-2.5 text-popover-foreground shadow-lg backdrop-blur-md transition-[top] duration-100 ease-out"
        >
          <div className="flex items-center justify-between gap-2">
            <p className="truncate text-xs font-medium text-foreground">
              {previewTurn.promptText}
            </p>
            <button
              type="button"
              aria-label={
                bookmarks.has(previewTurn.id) ? "Remove turn bookmark" : "Bookmark turn"
              }
              aria-pressed={bookmarks.has(previewTurn.id)}
              onClick={(e) => handleToggleBookmark(previewTurn.id, e)}
              className={cn(
                "motion-pressable -mr-1 inline-flex h-5 w-5 shrink-0 items-center justify-center rounded-md transition-colors hover:bg-accent",
                bookmarks.has(previewTurn.id)
                  ? "text-foreground"
                  : "text-muted-foreground/70 hover:text-foreground",
              )}
            >
              <Bookmark
                className={cn(
                  "h-3.5 w-3.5",
                  bookmarks.has(previewTurn.id) && "fill-current",
                )}
              />
            </button>
          </div>
          <p className="mt-1 line-clamp-3 text-[11px] leading-relaxed text-muted-foreground">
            {renderReplyPreview(previewTurn.replyText)}
          </p>
        </div>
      )}
    </div>
  );
}
