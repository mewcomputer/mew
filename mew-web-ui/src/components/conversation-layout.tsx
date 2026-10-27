import { useEffect, useRef, useState, type ReactNode } from "react";
import { ConversationHeader } from "./conversation-header";
import { ConversationSummaryContent } from "./conversation-summary";
import { useWorkspaceFrame } from "./workspace-frame";

export function ConversationLayout({ children }: { children: ReactNode }) {
  const pane = useRef<HTMLDivElement>(null);
  const [summaryPinned, setSummaryPinned] = useState(false);
  const { openWorkbenchSurface } = useWorkspaceFrame();

  useEffect(() => {
    const element = pane.current;
    if (!element) return;
    const observer = new ResizeObserver(([entry]) => {
      if (!entry) return;
      // Leave a full transcript column beside the summary, even with tools open.
      setSummaryPinned(entry.contentRect.width >= 1040);
    });
    observer.observe(element);
    return () => observer.disconnect();
  }, []);

  return (
    <div ref={pane} role="region" aria-label="Chat" className="flex h-full min-h-0 min-w-0 flex-col">
      <ConversationHeader summaryPinned={summaryPinned} />
      <div className="flex min-h-0 flex-1">
        <div className="flex min-h-0 min-w-0 flex-1 flex-col">{children}</div>
        {summaryPinned && (
          <aside aria-label="Conversation summary" className="w-64 shrink-0 overflow-y-auto p-2 pl-0 pt-1">
            <div className="rounded-2xl border border-border/60 bg-popover p-3 text-popover-foreground">
              <ConversationSummaryContent onOpenSurface={openWorkbenchSurface} />
            </div>
          </aside>
        )}
      </div>
    </div>
  );
}
