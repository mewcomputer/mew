import { ChevronDown } from "lucide-react";

/** Floating control that returns to the newest message. It sits above the
 *  composer, which the transcript scrolls behind. The drift is applied to the
 *  arrow, not the button, so the hit target never moves. */
export function JumpToLatest({ onClick }: { onClick: () => void }) {
  return (
    <div
      className="pointer-events-none absolute inset-x-0 z-30 flex justify-center"
      style={{ bottom: "calc(var(--composer-height, 0px) + 2.75rem)" }}
    >
      <button
        type="button"
        onClick={onClick}
        aria-label="Scroll to the latest message"
        className="mew-jump-latest pointer-events-auto flex h-8 w-8 items-center justify-center rounded-full border border-border/80 bg-card text-foreground/80 shadow-md backdrop-blur hover:bg-foreground hover:text-background"
      >
        <ChevronDown className="mew-jump-arrow h-4 w-4" aria-hidden="true" />
      </button>
    </div>
  );
}
