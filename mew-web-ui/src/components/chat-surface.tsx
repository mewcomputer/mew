import { useEffect, useRef } from "react";
import { useRouter } from "@tanstack/react-router";
import { useSessionStore } from "../stores/session";
import { MessageItem } from "./message-item";
import { ErrorBoundary } from "./error-boundary";
import { MessageSquare } from "lucide-react";
import { formatRelativeAge } from "../lib/format";
import { SESSION_ID_KEY } from "../lib/client";

function EmptyChatSurface() {
  const sessions = useSessionStore((s) => s.availableSessions);
  const titles = useSessionStore((s) => s.sessionTitles);
  const router = useRouter();
  const recent = [...sessions]
    .sort(
      (a, b) =>
        (b.last_message_at ?? b.created_at) -
        (a.last_message_at ?? a.created_at),
    )
    .slice(0, 3);

  return (
    <div className="flex gap-2 px-4 pb-12 pt-20">
      <div className="flex h-full flex-col items-start justify-center text-start">
        <div className="mb-4 flex gap-4 items-center">
          <img
            src="/mew-transparent-closeup.png"
            className="size-12"
            alt=""
            aria-hidden="true"
          />
          <h2 className="mb-2 text-xl text-foreground mt-4">Where to next?</h2>
        </div>
        {recent.length > 0 && (
          <div className="mt-6 w-full max-w-sm space-y-1 text-left">
            <div className="mb-1 text-[10px] font-semibold uppercase tracking-wide text-muted-foreground">
              Recent sessions
            </div>
            {recent.map((s) => (
              <button
                key={s.session_id}
                type="button"
                onClick={() => {
                  localStorage.setItem(SESSION_ID_KEY, s.session_id);
                  router.navigate({ to: "/session/$sessionId", params: { sessionId: s.session_id } });
                }}
                className="flex w-full items-center gap-2 rounded-md border border-border bg-card px-3 py-2 text-left transition-[background-color,border-color] duration-150 ease-out hover:border-foreground/20 hover:bg-accent"
                aria-label={"Open session " + (titles.get(s.session_id) ?? s.summary ?? s.first_message ?? s.session_id)}
              >
                <MessageSquare className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
                <span className="flex-1 truncate text-xs text-foreground">
                  {titles.get(s.session_id) ?? s.summary ?? s.first_message ?? s.model ?? "Untitled session"}
                </span>
                <span className="text-[10px] text-muted-foreground">
                  {formatRelativeAge(s.last_message_at ?? s.created_at)}
                </span>
              </button>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}

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
      className="chat-scroll-container min-w-0 flex-1 overflow-x-hidden overflow-y-auto px-3 pb-32 pt-4 sm:px-4 sm:pb-28"
    >
      <div className="mx-auto min-w-0 max-w-3xl space-y-4">
        {messages.length === 0 && <EmptyChatSurface />}
        {messages.filter((msg) => msg.role !== "system").map((msg) => (
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
            />
          </ErrorBoundary>
        ))}
      </div>
    </div>
  );
}
