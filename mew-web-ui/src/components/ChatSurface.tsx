import { useEffect, useRef } from "react";
import { useSessionStore } from "../stores/session";
import { MessageItem } from "./MessageItem";
import { ErrorBoundary } from "./ErrorBoundary";

export function ChatSurface() {
  const messages = useSessionStore((s) => s.messages);
  const streamingText = useSessionStore((s) => s.streamingText);
  const streamingReasoningText = useSessionStore(
    (s) => s.streamingReasoningText,
  );
  const scrollRef = useRef<HTMLDivElement>(null);
  const autoScroll = useRef(true);

  // Track whether the user is scrolled to the bottom
  const handleScroll = () => {
    const el = scrollRef.current;
    if (!el) return;
    const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 100;
    autoScroll.current = atBottom;
  };

  // Auto-scroll to bottom when new content arrives (if user was at bottom)
  useEffect(() => {
    if (autoScroll.current && scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
  }, [messages, streamingText, streamingReasoningText]);

  return (
    <div
      ref={scrollRef}
      onScroll={handleScroll}
      className="flex-1 overflow-y-auto px-4 py-4"
    >
      <div className="mx-auto max-w-3xl space-y-4">
        {messages.length === 0 && (
          <div className="flex h-full items-center justify-center pt-20 text-muted-foreground">
            <p>Send a message to start chatting with mew.</p>
          </div>
        )}
        {messages.map((msg) => (
          <ErrorBoundary
            key={msg.id}
            title="Message failed to render"
            fallback={
              <div className="max-w-[85%] rounded-lg border border-destructive/50 bg-destructive/10 px-4 py-2.5 text-sm text-destructive">
                A message in this conversation failed to render.
              </div>
            }
          >
            <MessageItem
              message={msg}
              streamingText={streamingText}
              streamingReasoningText={streamingReasoningText}
            />
          </ErrorBoundary>
        ))}
      </div>
    </div>
  );
}
