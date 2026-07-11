import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { useSessionStore } from "../stores/session";
import { MessageItem } from "./message-item";
import { ErrorBoundary } from "./error-boundary";
import { ChatSurface } from "./chat-surface";

/** Virtualized chat surface using TanStack Virtual's end-anchoring pattern.
 *  See: https://github.com/tanstack/virtual/blob/main/docs/chat.md */
export function VirtualChatSurface() {
  const messages = useSessionStore((s) => s.messages);
  const sessionId = useSessionStore((s) => s.sessionId);
  const parentRef = useRef<HTMLDivElement>(null);
  const [didInitialScroll, setDidInitialScroll] = useState(false);

  const virtualizer = useVirtualizer({
    count: messages.length,
    getScrollElement: () => parentRef.current,
    estimateSize: () => 100,
    getItemKey: (index) => messages[index]?.id ?? index,
    overscan: 6,
    // End-anchoring: keeps viewport pinned to latest message during
    // streaming, and keeps scroll stable when prepending history.
    // followOnAppend auto-scrolls to new messages only when already at bottom.
    followOnAppend: true,
  });

  // Start at the latest message on mount / session switch.
  useLayoutEffect(() => {
    if (didInitialScroll || messages.length === 0) return;
    virtualizer.scrollToEnd();
    setDidInitialScroll(true);
  }, [didInitialScroll, virtualizer, messages.length]);

  // reset didInitialScroll when sessionId changes
  useEffect(() => {
    setDidInitialScroll(false);
  }, [sessionId]);

  if (messages.length === 0) {
    return <ChatSurface />;
  }

  return (
    <div
      ref={parentRef}
      className="min-h-0 flex-1 overflow-y-auto px-3 py-4 sm:px-4"
    >
      <div
        style={{
          height: `${virtualizer.getTotalSize()}px`,
          width: "100%",
          position: "relative",
        }}
      >
        {virtualizer.getVirtualItems().map((virtualItem) => {
          const msg = messages[virtualItem.index];
          if (!msg) return null;
          return (
            <div
              key={virtualItem.key}
              data-index={virtualItem.index}
              ref={virtualizer.measureElement}
              style={{
                position: "absolute",
                top: 0,
                left: 0,
                width: "100%",
                transform: `translateY(${virtualItem.start}px)`,
              }}
            >
              <div className="mx-auto max-w-3xl pb-4">
                <ErrorBoundary
                  title="Message failed to render"
                  fallback={
                    <div className="max-w-[85%] rounded-lg border border-destructive/50 bg-destructive/10 px-4 py-2.5 text-sm text-destructive">
                      A message in this conversation failed to render.
                    </div>
                  }
                >
                  <MessageItem
                    message={msg}
                  />
                </ErrorBoundary>
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}
