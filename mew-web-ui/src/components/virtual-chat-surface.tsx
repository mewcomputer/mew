import { useEffect, useMemo, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { selectSessionRunning, useSessionStore } from "../stores/session";
import { CompactingIndicator, TurnItem } from "./turn-item";
import { buildTurns } from "../lib/turns";
import { ErrorBoundary } from "./error-boundary";
import { ChatSurface } from "./chat-surface";
import { ConversationMinimap } from "./conversation-minimap";
import { JumpToLatest } from "./scroll-to-latest";
import { LoadEarlier } from "./load-earlier";
import { useFollowScroll } from "../lib/use-follow-scroll";
import { useComposerHeight } from "../lib/composer-height";
import { getClient } from "../lib/client";

/** Virtualized chat surface. Follow-scrolling lives in `useFollowScroll` rather
 *  than the virtualizer's anchoring, because streaming grows an existing row
 *  instead of appending one. */
/** A page of history has to add at least this much visible material before it
 *  counts as "loaded"; tool-result-only pages add nothing. */
const MIN_PAGE_GROWTH_PX = 480;

export function VirtualChatSurface() {
  const messages = useSessionStore((s) => s.messages);
  const sessionId = useSessionStore((s) => s.sessionId);
  const sessionIndex = useSessionStore((s) => s.sessionIndex);
  const streamingText = useSessionStore((s) => s.streamingText);
  const compacting = useSessionStore((s) => s.compacting);
  const compactingStartedAt = useSessionStore((s) => s.compactingStartedAt);
  const sessionRunning = useSessionStore(selectSessionRunning);
  const attached = useSessionStore((s) => s.connectionState === "connected" && s.confirmedSessionId === s.sessionId);
  const historyCursor = useSessionStore((s) => s.historyCursor);
  const [loadingCursor, setLoadingCursor] = useState<number | null>(null);
  const turns = useMemo(() => buildTurns(messages), [messages]);
  const composerHeight = useComposerHeight();
  const lastLoadTick = useRef(-1);
  const pageBaseHeight = useRef<number | null>(null);
  // The composer is a dependency: growing it pushes the floating box up, and a
  // pinned transcript has to follow so nothing ends up behind it.
  const scroll = useFollowScroll<HTMLDivElement>([
    turns.length,
    messages,
    streamingText,
    compacting,
    composerHeight,
  ]);

  const virtualizer = useVirtualizer({
    count: turns.length,
    getScrollElement: () => scroll.ref.current,
    estimateSize: () => 100,
    getItemKey: (index) => turns[index]?.key ?? index,
    overscan: 6,
    useAnimationFrameWithResizeObserver: true,
  });

  // A session switch starts at its newest message.
  useEffect(() => {
    scroll.jumpToLatest("auto");
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sessionId]);

  // Older messages load on their own: once the reader reaches the top, and on
  // open while the page the daemon sent does not yet fill the viewport.
  useEffect(() => {
    if (!attached || historyCursor === null || loadingCursor !== null) return;
    const element = scroll.ref.current;
    if (!element) return;
    const atTop = element.scrollTop <= 320;
    const fillsViewport = element.scrollHeight > element.clientHeight + 40;
    // One page per gesture: a chain that reloads on every commit would walk back
    // through the whole session.
    const movedSinceLoad = lastLoadTick.current !== scroll.scrollTick;
    if (fillsViewport && !(atTop && movedSinceLoad)) return;
    lastLoadTick.current = scroll.scrollTick;
    requestOlderPage();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [scroll.scrollTick, scroll.scrollable, attached, historyCursor, loadingCursor]);

  // A page is 24 messages, and most of them are tool results that render nothing.
  // Keep pulling until the page added something to look at, or history runs out.
  useEffect(() => {
    if (pageBaseHeight.current === null) return;
    // The in-flight flag is cleared by an effect that runs after this one, so
    // wait for it rather than deciding on a stale value.
    if (loadingCursor !== null || !attached) return;
    const element = scroll.ref.current;
    if (!element) return;
    const baseline = pageBaseHeight.current;
    pageBaseHeight.current = null;
    if (element.scrollHeight - baseline >= MIN_PAGE_GROWTH_PX) return;
    if (historyCursor === null || element.scrollTop > 320) return;
    lastLoadTick.current = scroll.scrollTick;
    requestOlderPage();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [historyCursor, loadingCursor]);

  useEffect(() => {
    if (loadingCursor === null) return;
    if (!attached || loadingCursor !== historyCursor) {
      setLoadingCursor(null);
      return;
    }
    const timeout = setTimeout(() => setLoadingCursor(null), 10000);
    return () => clearTimeout(timeout);
  }, [attached, historyCursor, loadingCursor, sessionId]);

  /** Record the height a page arrives on top of, so little-visible pages can
   *  ask for another one. */
  function requestOlderPage() {
    pageBaseHeight.current = scroll.ref.current?.scrollHeight ?? null;
    loadEarlier();
  }

  function loadEarlier() {
    if (!sessionId || !attached || historyCursor === null || loadingCursor !== null) return;
    const client = getClient();
    if (!client.isSessionAttached(sessionId)) return;
    try {
      // Older rows land above the viewport; remember where the reader was.
      scroll.holdPosition();
      client.loadSessionHistory(sessionId, historyCursor);
      setLoadingCursor(historyCursor);
    } catch (error) {
      useSessionStore.getState().onError(error instanceof Error ? error.message : "Could not load earlier messages.");
    }
  }

  const virtualItems = virtualizer.getVirtualItems();
  const scrollTop = scroll.ref.current?.scrollTop ?? 0;
  const visibleItem =
    virtualItems.find((item) => item.end > scrollTop + 24) ??
    virtualItems[0];
  const activeMessageIndex = (visibleItem && turns[visibleItem.index]?.messageIndex) ?? Math.max(0, messages.length - 1);

  // A selected message index resolves to the turn that contains it.
  const handleSelectMessageIndex = (index: number) => {
    const messageIndex = Math.max(0, Math.min(messages.length - 1, index));
    let turnIndex = 0;
    turns.forEach((turn, i) => { if (turn.messageIndex <= messageIndex) turnIndex = i; });
    virtualizer.scrollToIndex(turnIndex, { align: "start" });
    const el = scroll.ref.current?.querySelector<HTMLElement>(`[data-message-index="${turns[turnIndex]?.messageIndex ?? messageIndex}"]`);
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
        sessionIndex={sessionIndex}
        onLoadEarlier={requestOlderPage}
        loadingEarlier={loadingCursor !== null}
      />
      {!scroll.atBottom && <JumpToLatest onClick={() => scroll.jumpToLatest()} />}
      {(loadingCursor !== null || scroll.nearTop) && historyCursor !== null && (
        <LoadEarlier loading={loadingCursor !== null} onClick={requestOlderPage} />
      )}
      <div
        ref={scroll.setRef}
        className="chat-scroll-container mew-conversation-gutters min-h-0 min-w-0 flex-1 overflow-x-hidden overflow-y-auto pt-6 pb-[calc(var(--composer-height,0px)_+_3rem)]"
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
            const turn = turns[virtualItem.index];
            if (!turn) return null;
            return (
              <div
                key={virtualItem.key}
                data-index={virtualItem.index}
                data-message-index={turn.messageIndex}
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
                    <TurnItem turn={turn} isLast={virtualItem.index === turns.length - 1} />
                  </ErrorBoundary>
                </div>
              </div>
            );
          })}
        </div>
        {compacting && !sessionRunning && (
          <div className="mew-conversation-width mx-auto min-w-0">
            <CompactingIndicator startedAt={compactingStartedAt} />
          </div>
        )}
      </div>
    </div>
  );
  return transcript;
}
