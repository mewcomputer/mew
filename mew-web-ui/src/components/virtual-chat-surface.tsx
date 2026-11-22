import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { useSessionStore } from "../stores/session";
import { MessageItem } from "./message-item";
import { ErrorBoundary } from "./error-boundary";
import { ChatSurface } from "./chat-surface";
import { ConversationMinimap } from "./conversation-minimap";
import { getClient } from "../lib/client";
import { Button } from "./ui/button";

/** Virtualized chat surface using TanStack Virtual's end-anchoring pattern.
 *  See: https://github.com/tanstack/virtual/blob/main/docs/chat.md */
export function VirtualChatSurface() {
  const messages = useSessionStore((s) => s.messages);
  const sessionId = useSessionStore((s) => s.sessionId);
  const streamingText = useSessionStore((s) => s.streamingText);
  const attached = useSessionStore((s) => s.connectionState === "connected" && s.confirmedSessionId === s.sessionId);
  const historyCursor = useSessionStore((s) => s.historyCursor);
  const [loadingCursor, setLoadingCursor] = useState<number | null>(null);
  const parentRef = useRef<HTMLDivElement>(null);
  const [didInitialScroll, setDidInitialScroll] = useState(false);

  const virtualizer = useVirtualizer({
    count: messages.length,
    getScrollElement: () => parentRef.current,
    estimateSize: () => 100,
    getItemKey: (index) => messages[index]?.id ?? index,
    overscan: 6,
    // End-anchoring keeps the viewport pinned to the latest message during
    // streaming and keeps scroll stable when prepending history.
    anchorTo: "end",
    followOnAppend: true,
    useAnimationFrameWithResizeObserver: true,
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

  useEffect(() => {
    if (loadingCursor === null) return;
    if (!attached || loadingCursor !== historyCursor) {
      setLoadingCursor(null);
      return;
    }
    const timeout = setTimeout(() => setLoadingCursor(null), 10000);
    return () => clearTimeout(timeout);
  }, [attached, historyCursor, loadingCursor, sessionId]);

  function loadEarlier() {
    if (!sessionId || !attached || historyCursor === null || loadingCursor !== null) return;
    const client = getClient();
    if (!client.isSessionAttached(sessionId)) return;
    try {
      client.loadSessionHistory(sessionId, historyCursor);
      setLoadingCursor(historyCursor);
    } catch (error) {
      useSessionStore.getState().onError(error instanceof Error ? error.message : "Could not load earlier messages.");
    }
  }

  const virtualItems = virtualizer.getVirtualItems();
  const scrollTop = parentRef.current?.scrollTop ?? 0;
  const visibleItem =
    virtualItems.find((item) => item.end > scrollTop + 24) ??
    virtualItems[0];
  const activeMessageIndex = visibleItem?.index ?? Math.max(0, messages.length - 1);

  const handleSelectMessageIndex = (index: number) => {
    const target = Math.max(0, Math.min(messages.length - 1, index));
    virtualizer.scrollToIndex(target, { align: "start" });
    const el = parentRef.current?.querySelector<HTMLElement>(`[data-index="${target}"]`);
    if (el && typeof el.scrollIntoView === "function") {
      el.scrollIntoView({ block: "start" });
    }
  };

  const transcript = messages.length === 0 ? <ChatSurface /> : (
    <div className="relative flex min-h-0 min-w-0 flex-1 flex-col">
      <ConversationMinimap
        messages={messages}
        sessionId={sessionId}
        streamingText={streamingText}
        activeMessageIndex={activeMessageIndex}
        onSelectMessageIndex={handleSelectMessageIndex}
      />
      <div
        ref={parentRef}
        className="chat-scroll-container min-h-0 min-w-0 flex-1 overflow-x-hidden overflow-y-auto px-4 pb-6 pt-6 sm:pl-9 sm:pr-6"
      >
        <div
          className="min-w-0 max-w-full"
          style={{
            height: `${virtualizer.getTotalSize()}px`,
            width: "100%",
            position: "relative",
          }}
        >
          {virtualItems.map((virtualItem) => {
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
                <div className="mew-conversation-width mx-auto min-w-0 pb-6">
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
    </div>
  );
  return (
    <>
      {historyCursor !== null && (
        <div className="flex shrink-0 justify-center px-4 pt-2">
          <Button onClick={loadEarlier} disabled={!attached || loadingCursor !== null} variant="ghost" size="sm">
            {loadingCursor !== null ? "Loading earlier messages…" : "Load earlier messages"}
          </Button>
        </div>
      )}
      {transcript}
    </>
  );
}
