import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type CSSProperties, type ReactNode } from "react";
import { ComposerHeightContext } from "../lib/composer-height";
import { ConversationHeader } from "./conversation-header";
import { ConversationSummaryContent } from "./conversation-summary";
import { useWorkspaceFrame } from "./workspace-frame";

/** Pane width below which the centred transcript column reaches into the pinned
 *  summary panel. Derived from `.mew-conversation-width` (46rem), the panel's
 *  width (16rem), and a 24px gap, at the browser's 16px root size. */
const SUMMARY_INTRUDES_WIDTH = 46 * 16 + 2 * (16 * 16 + 24);

/** Chat region: transcript, floating composer, and the conversation summary.
 *
 *  The transcript scrolls the full width of the chat region so its scrollbar
 *  lands on the region's right edge, past the summary panel, and it runs all the
 *  way down behind the composer. The content column clears those overlays
 *  through `--summary-inset` and `--composer-height`; the inset is only set
 *  while the panel would otherwise overlap the column, so a pane wide enough for
 *  both leaves the chat centred. */
export function ConversationLayout({ children, footer }: { children: ReactNode; footer?: ReactNode }) {
  const pane = useRef<HTMLDivElement>(null);
  const footerRef = useRef<HTMLDivElement>(null);
  const footerObserver = useRef<ResizeObserver | null>(null);
  const [footerHeight, setFooterHeight] = useState(0);
  // A callback ref, not a mount effect: the composer is remounted on session
  // switches, and an observer left on the detached node would freeze the height
  // it reports — the transcript would then clear the wrong box.
  const setFooterRef = useCallback((node: HTMLDivElement | null) => {
    footerObserver.current?.disconnect();
    footerRef.current = node;
    if (!node) return;
    const observer = new ResizeObserver(([entry]) => {
      // Border box, so the composer's own padding counts as occupied space.
      const block = entry?.borderBoxSize?.[0]?.blockSize ?? entry?.contentRect.height ?? 0;
      setFooterHeight(Math.round(block));
    });
    observer.observe(node);
    footerObserver.current = observer;
  }, []);
  const summary = useRef<HTMLElement>(null);
  const pinned = useRef(false);
  const measured = useRef(false);
  const animateChange = useRef(false);
  const returnSummaryFocus = useRef<"card" | "trigger" | null>(null);
  const positions = useRef<[HTMLElement, number][]>([]);
  const [summaryPinned, setSummaryPinned] = useState(false);
  const inset = useRef(false);
  const [summaryInset, setSummaryInset] = useState(false);
  const { openWorkbenchSurface } = useWorkspaceFrame();

  useEffect(() => {
    const element = pane.current;
    if (!element) return;
    const observer = new ResizeObserver(([entry]) => {
      if (!entry) return;
      const width = entry.contentRect.width;
      // Leave a full transcript column beside the summary, even with tools open.
      const nextPinned = width >= 1040;
      // Shifting the column off centre only buys anything while the panel is in
      // its way; past `SUMMARY_INTRUDES_WIDTH` the column clears it anyway.
      const nextInset = nextPinned && width < SUMMARY_INTRUDES_WIDTH;
      const pinnedChanged = nextPinned !== pinned.current;
      if (pinnedChanged || nextInset !== inset.current) {
        animateChange.current = measured.current
          && !window.matchMedia("(prefers-reduced-motion: reduce)").matches;
        positions.current = animateChange.current
          ? Array.from(element.querySelectorAll<HTMLElement>(".mew-conversation-width"), (column) => [column, column.getBoundingClientRect().left])
          : [];
        returnSummaryFocus.current = null;
        if (pinnedChanged) {
          const focused = document.activeElement;
          if (!nextPinned && summary.current?.contains(focused)) returnSummaryFocus.current = "trigger";
          if (nextPinned && (focused?.matches('button[aria-label="Open summary"], button[aria-label="Close summary"]') || focused?.closest('[role="dialog"][aria-label="Conversation summary"]'))) {
            returnSummaryFocus.current = "card";
          }
        }
        pinned.current = nextPinned;
        inset.current = nextInset;
        setSummaryPinned(nextPinned);
        setSummaryInset(nextInset);
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
        : document.querySelector<HTMLButtonElement>('[aria-label="Open summary"]');
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
  }, [summaryPinned, summaryInset]);

  const vars = useMemo(() => ({
    "--composer-height": `${footerHeight}px`,
    "--summary-inset": summaryInset ? "16rem" : "0px",
  }) as CSSProperties, [footerHeight, summaryInset]);

  return (
    <div ref={pane} role="region" aria-label="Chat" data-summary-animate={animateChange.current} className="flex h-full min-h-0 min-w-0 flex-col">
      <ConversationHeader summaryPinned={summaryPinned} />
      <div className="relative flex min-h-0 flex-1" style={vars}>
        <div className="flex min-h-0 min-w-0 flex-1 flex-col">
          <ComposerHeightContext.Provider value={footerHeight}>{children}</ComposerHeightContext.Provider>
        </div>
        {footer && (
          <>
            <div
              aria-hidden="true"
              className="pointer-events-none absolute inset-x-0 bottom-0 z-10 bg-gradient-to-t from-background via-background/80 to-transparent"
              style={{ height: footerHeight + 44 }}
            />
            <div ref={setFooterRef} className="absolute inset-x-0 bottom-0 z-20 pb-2">{footer}</div>
          </>
        )}
        <aside
          ref={summary}
          aria-label="Conversation summary"
          aria-hidden={!summaryPinned}
          inert={!summaryPinned}
          data-state={summaryPinned ? "open" : "closed"}
          className="mew-summary-aside absolute right-0 top-0 z-10 overflow-hidden"
          style={{ width: summaryPinned ? "16rem" : 0, bottom: footerHeight }}
        >
          <div className="mew-summary-content absolute inset-y-0 right-0 w-64 overflow-y-auto p-3 pl-0">
            <div className="rounded-2xl border border-border/60 bg-popover p-3 text-popover-foreground">
              <ConversationSummaryContent onOpenSurface={openWorkbenchSurface} />
            </div>
          </div>
        </aside>
      </div>
    </div>
  );
}
