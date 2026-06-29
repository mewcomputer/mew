import { useEffect, type RefObject } from "react";

/** Calls `onOutside` when a mousedown occurs outside `ref`.
 *  Only active while `enabled` is true (avoids unnecessary listeners). */
export function useOutsideClick(
  ref: RefObject<HTMLElement | null>,
  enabled: boolean,
  onOutside: () => void,
) {
  useEffect(() => {
    if (!enabled) return;
    const handler = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) {
        onOutside();
      }
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [ref, enabled, onOutside]);
}
