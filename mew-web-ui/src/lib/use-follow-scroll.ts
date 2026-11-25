import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";

/** How close to either end still counts as "reading that end". Slightly more
 *  than a line of text, so re-pinning does not fight measurement drift. */
const BOTTOM_SLACK_PX = 64;
const TOP_SLACK_PX = 320;

/** Keeps a transcript pinned to its newest content while the reader is already
 *  at the bottom, holds their place when older messages are prepended, and
 *  reports when they sit at either end.
 *
 *  `dependencies` changes re-pin only when the reader has not scrolled away, so
 *  scrolling back through history is never yanked forward by streaming output. */
export function useFollowScroll<T extends HTMLElement>(dependencies: unknown[]) {
  const ref = useRef<T>(null);
  const [attachedElement, setAttachedElement] = useState(0);
  // The transcript scroller mounts after the first messages arrive, which is
  // later than this hook's first render, so listeners follow the element.
  const setRef = useCallback((node: T | null) => {
    ref.current = node;
    setAttachedElement((version) => version + 1);
  }, []);
  const pinned = useRef(true);
  const held = useRef<{ height: number } | null>(null);
  const [atBottom, setAtBottom] = useState(true);
  const [nearTop, setNearTop] = useState(false);
  const [scrollable, setScrollable] = useState(false);
  const [scrollTick, setScrollTick] = useState(0);

  const measure = useCallback(() => {
    const element = ref.current;
    if (!element) return;
    const distance = element.scrollHeight - element.scrollTop - element.clientHeight;
    pinned.current = distance <= (pinned.current ? BOTTOM_SLACK_PX : 1);
    setAtBottom(pinned.current);
    setNearTop(element.scrollTop <= TOP_SLACK_PX);
    setScrollable(element.scrollHeight > element.clientHeight + 40);
  }, []);

  useEffect(() => {
    const element = ref.current;
    if (!element) return;
    measure();
    element.addEventListener("scroll", measure, { passive: true });
    const stopFollowing = (event: WheelEvent) => {
      if (event.deltaY < 0) {
        pinned.current = false;
        setAtBottom(false);
      }
    };
    element.addEventListener("wheel", stopFollowing, { passive: true });
    // A counter of real scroll events, so callers can tell "the reader moved"
    // apart from "our own re-pin moved".
    const bump = () => setScrollTick((tick) => tick + 1);
    element.addEventListener("scroll", bump, { passive: true });
    return () => {
      element.removeEventListener("scroll", measure);
      element.removeEventListener("wheel", stopFollowing);
      element.removeEventListener("scroll", bump);
    };
  }, [measure, attachedElement]);

  /** Call immediately before requesting older messages. The prepended rows grow
   *  the scroll height above the viewport by exactly as much as `scrollTop` then
   *  needs to move, so the rows being read stay put. */
  const holdPosition = useCallback(() => {
    const element = ref.current;
    if (!element) return;
    held.current = { height: element.scrollHeight };
  }, []);

  useLayoutEffect(() => {
    const element = ref.current;
    if (!element) return;

    if (held.current !== null) {
      // A virtualized list grows a frame or two after the rows are committed,
      // so wait for the height to land before shifting.
      let frames = 0;
      let cancelled = false;
      const settle = () => {
        if (cancelled) return;
        const node = ref.current;
        const hold = held.current;
        if (!node || !hold) return;
        const delta = node.scrollHeight - hold.height;
        if (delta <= 0 && frames < 30) {
          frames += 1;
          requestAnimationFrame(settle);
          return;
        }
        if (delta > 0) node.scrollTop += delta;
        held.current = null;
        measure();
      };
      settle();
      return () => { cancelled = true; };
    }

    if (!pinned.current) return;
    element.scrollTop = element.scrollHeight;
    // Virtual rows, fonts, and images can settle a frame later and push the
    // bottom down again, so re-pin once the browser has laid the frame out.
    const frame = requestAnimationFrame(() => {
      const node = ref.current;
      if (node && pinned.current) node.scrollTop = node.scrollHeight;
    });
    return () => cancelAnimationFrame(frame);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, dependencies);

  const jumpToLatest = useCallback((behavior: ScrollBehavior = "smooth") => {
    const element = ref.current;
    if (!element) return;
    pinned.current = true;
    setAtBottom(true);
    element.scrollTo({ top: element.scrollHeight, behavior });
  }, []);

  return { ref, setRef, atBottom, nearTop, scrollable, scrollTick, holdPosition, jumpToLatest };
}
