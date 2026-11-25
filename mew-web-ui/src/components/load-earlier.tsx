import { Loader2 } from "lucide-react";

/** Floating control for older history, shown once the reader nears the top.
 *
 *  It floats over the transcript instead of occupying a strip above it, so the
 *  list keeps its full height. While a page is in flight it becomes a small
 *  skeleton card: the same place the loaded rows are about to appear. */
export function LoadEarlier({ loading, onClick }: { loading: boolean; onClick: () => void }) {
  return (
    <div className="pointer-events-none absolute inset-x-0 top-2 z-30 flex flex-col items-center gap-2 px-4">
      {loading ? (
        <div
          aria-live="polite"
          className="pointer-events-auto flex w-full max-w-md flex-col gap-2 rounded-xl border border-border/70 bg-card/95 p-3 shadow-md backdrop-blur"
        >
          <span className="flex items-center gap-2 text-[11px] text-muted-foreground">
            <Loader2 className="h-3 w-3 animate-spin" aria-hidden="true" />
            Loading earlier messages…
          </span>
          <div className="space-y-1.5" aria-hidden="true">
            <div className="mew-skeleton h-2.5 w-[92%] rounded-full bg-muted-foreground/15" />
            <div className="mew-skeleton h-2.5 w-[78%] rounded-full bg-muted-foreground/15" />
            <div className="mew-skeleton h-2.5 w-[85%] rounded-full bg-muted-foreground/15" />
          </div>
        </div>
      ) : (
        <button
          type="button"
          onClick={onClick}
          className="pointer-events-auto rounded-full border border-border/80 bg-card px-3 py-1 text-[11px] text-muted-foreground shadow-md transition-colors hover:bg-accent hover:text-accent-foreground"
        >
          Load earlier messages
        </button>
      )}
    </div>
  );
}
