import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { ConversationHeader } from "./conversation-header";
import { ConversationSummaryContent } from "./conversation-summary";
import { useWorkspaceFrame } from "./workspace-frame";

export function ConversationLayout({ children }: { children: ReactNode }) {
  const pane = useRef<HTMLDivElement>(null);
  const summary = useRef<HTMLElement>(null);
  const pinned = useRef(false);
  const measured = useRef(false);
  const animateChange = useRef(false);
  const returnSummaryFocus = useRef<"card" | "trigger" | null>(null);
  const positions = useRef<[HTMLElement, number][]>([]);
  const [summaryPinned, setSummaryPinned] = useState(false);
  const { openWorkbenchSurface } = useWorkspaceFrame();

  useEffect(() => {
    const element = pane.current;
    if (!element) return;
    const observer = new ResizeObserver(([entry]) => {
      if (!entry) return;
      // Leave a full transcript column beside the summary, even with tools open.
      const nextPinned = entry.contentRect.width >= 1040;
      if (nextPinned !== pinned.current) {
        animateChange.current = measured.current
          && !window.matchMedia("(prefers-reduced-motion: reduce)").matches
          && document.activeElement?.getAttribute("role") !== "separator";
        positions.current = animateChange.current
          ? Array.from(element.querySelectorAll<HTMLElement>(".mew-conversation-width"), (column) => [column, column.getBoundingClientRect().left])
          : [];
        const focused = document.activeElement;
        returnSummaryFocus.current = null;
        if (!nextPinned && summary.current?.contains(focused)) returnSummaryFocus.current = "trigger";
        if (nextPinned && (focused?.matches('button[title="Summary"]') || focused?.closest('[role="dialog"][aria-label="Conversation summary"]'))) {
          returnSummaryFocus.current = "card";
        }
        pinned.current = nextPinned;
        setSummaryPinned(nextPinned);
      }
      measured.current = true;
    });
    observer.observe(element);
    return () => observer.disconnect();
  }, []);

  useLayoutEffect(() => {
    if (returnSummaryFocus.current) {
      const target = returnSummaryFocus.current === "card"
        ? summary.current?.querySelector<HTMLButtonElement>("button")
        : pane.current?.querySelector<HTMLButtonElement>('[aria-label="Open summary"]');
      target?.focus();
      returnSummaryFocus.current = null;
    }
    const columns = positions.current;
    positions.current = [];
    if (!columns.length || !pane.current) return;
    const style = getComputedStyle(pane.current);
    const timing = {
      duration: Number.parseFloat(style.getPropertyValue("--motion-menu")),
      easing: style.getPropertyValue("--ease-out").trim(),
    };
    const animations: Animation[] = [];
    for (const [column, left] of columns) {
      if (!column.isConnected || !column.animate) continue;
      const offset = left - column.getBoundingClientRect().left;
      if (Math.abs(offset) < 0.5) continue;
      animations.push(column.animate([{ transform: `translateX(${offset}px)` }, { transform: "translateX(0)" }], timing));
    }
    const reducedMotion = window.matchMedia("(prefers-reduced-motion: reduce)");
    const cancelMovement = () => { for (const animation of animations) animation.cancel(); };
    const motionChanged = () => { if (reducedMotion.matches) cancelMovement(); };
    reducedMotion.addEventListener("change", motionChanged);
    return () => {
      reducedMotion.removeEventListener("change", motionChanged);
      cancelMovement();
    };
  }, [summaryPinned]);

  return (
    <div ref={pane} role="region" aria-label="Chat" data-summary-animate={animateChange.current} className="flex h-full min-h-0 min-w-0 flex-col">
      <ConversationHeader summaryPinned={summaryPinned} />
      <div className="flex min-h-0 flex-1">
        <div className="flex min-h-0 min-w-0 flex-1 flex-col">{children}</div>
        <aside ref={summary} aria-label="Conversation summary" aria-hidden={!summaryPinned} inert={!summaryPinned} data-state={summaryPinned ? "open" : "closed"} className="relative shrink-0" style={{ width: summaryPinned ? "16rem" : 0 }}>
          <div className="mew-summary-content absolute inset-y-0 right-0 w-64 overflow-y-auto p-2 pl-0 pt-1">
            <div className="rounded-2xl border border-border/60 bg-popover p-3 text-popover-foreground">
              <ConversationSummaryContent onOpenSurface={openWorkbenchSurface} />
            </div>
          </div>
        </aside>
      </div>
    </div>
  );
}
