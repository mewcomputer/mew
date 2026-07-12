import { useState } from "react";
import { ChevronRight, ChevronDown } from "lucide-react";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import type { TurnManifest, Segment } from "@mew/web-client";

/** Map segment kind to CSS variable color. */
function segmentColor(kind: string): string {
  const map: Record<string, string> = {
    scaffold: "var(--inspector-scaffold)",
    context_file: "var(--inspector-context)",
    skill: "var(--inspector-skill)",
    tools: "var(--inspector-tools)",
    message: "var(--inspector-message)",
    part: "var(--inspector-part)",
    compaction_summary: "var(--inspector-compaction)",
  };
  return map[kind] ?? "var(--inspector-scaffold)";
}

/** Format token count with k suffix for large numbers. */
function formatTokens(n: number): string {
  if (n >= 1000) {
    return `${(n / 1000).toFixed(1)}k`;
  }
  return String(n);
}

/** Format percentage from a fraction. */
function formatPct(fraction: number): string {
  if (fraction <= 0) return "0%";
  if (fraction >= 1) return "100%";
  return `${(fraction * 100).toFixed(0)}%`;
}

export function MessageInspector({ manifest }: { manifest: TurnManifest }) {
  const [open, setOpen] = useState(false);

  const inputTokens = manifest.input_tokens;
  const outputTokens = manifest.output_tokens;
  const cacheRead = manifest.cache_read_tokens;
  const contextWindow = manifest.context_window;

  // Summary line
  const isError = inputTokens == null;
  const utilization =
    inputTokens != null && contextWindow > 0
      ? inputTokens / contextWindow
      : 0;
  const cacheWarmth =
    inputTokens != null && cacheRead != null && inputTokens > 0
      ? cacheRead / inputTokens
      : null;

  return (
    <div>
      <div className="mt-1 flex items-center gap-2 text-xs text-muted-foreground">
        <button
          onClick={() => setOpen(!open)}
          className="flex items-center gap-1 hover:text-foreground transition-colors"
          aria-expanded={open}
        >
          {open ? (
            <ChevronDown className="size-3" />
          ) : (
            <ChevronRight className="size-3" />
          )}
        </button>

        {isError ? (
          <span className="text-destructive/70">error · structure below</span>
        ) : (
          <TooltipProvider>
            <Tooltip>
              <TooltipTrigger asChild>
                <span className="cursor-help tabular-nums">
                  {cacheWarmth != null && cacheWarmth > 0 && (
                    <span className="mr-1">
                      {formatPct(cacheWarmth)} warm ·{" "}
                    </span>
                  )}
                  ~{formatTokens(inputTokens!)} ↓ · ~
                  {formatTokens(outputTokens ?? 0)} ↑ · {formatPct(utilization)}{" "}
                  (~{formatTokens(inputTokens!)}/
                  {formatTokens(contextWindow)})
                </span>
              </TooltipTrigger>
              <TooltipContent side="top" className="max-w-xs">
                <p>
                  Per-segment counts are local-tokenizer estimates scaled to the
                  provider-reported total. The <code>~</code> prefix signals
                  approximation.
                </p>
              </TooltipContent>
            </Tooltip>
          </TooltipProvider>
        )}
      </div>

      {open && (
        <div className="mt-2 space-y-2">
          {/* Stacked bar */}
          {inputTokens != null && (
            <StackedBar
              segments={manifest.segments}
              inputTokens={inputTokens}
              contextWindow={contextWindow}
            />
          )}

          {/* Segment tree */}
          <div className="space-y-0.5">
            {manifest.segments.map((seg, i) => (
              <SegmentRow key={`${seg.label}-${i}`} segment={seg} depth={0} />
            ))}
          </div>
        </div>
      )}
    </div>
  );
}

/** Stacked bar showing proportional segment widths. */
function StackedBar({
  segments,
  inputTokens,
  contextWindow,
}: {
  segments: Segment[];
  inputTokens: number;
  contextWindow: number;
}) {
  const totalScaled = segments.reduce((sum, s) => sum + s.tokens_scaled, 0);
  const freeSpace = Math.max(0, contextWindow - inputTokens);

  if (totalScaled === 0 && inputTokens === 0) return null;

  return (
    <div
      className="flex h-2 w-full overflow-hidden rounded-full"
      title={`~${formatTokens(inputTokens)} of ${formatTokens(contextWindow)} context window`}
    >
      {segments.map((seg, i) => {
        const width =
          contextWindow > 0 ? (seg.tokens_scaled / contextWindow) * 100 : 0;
        if (width === 0) return null;
        return (
          <div
            key={i}
            style={{
              width: `${width}%`,
              backgroundColor: segmentColor(seg.kind),
            }}
            title={`${seg.label}: ~${formatTokens(seg.tokens_scaled)}`}
          />
        );
      })}
      {/* Free space tail */}
      {freeSpace > 0 && contextWindow > 0 && (
        <div
          style={{
            width: `${(freeSpace / contextWindow) * 100}%`,
            backgroundColor: "var(--inspector-free)",
          }}
        />
      )}
    </div>
  );
}

/** A single segment row in the tree, with optional expandable children. */
function SegmentRow({
  segment,
  depth,
}: {
  segment: Segment;
  depth: number;
}) {
  const [expanded, setExpanded] = useState(false);
  const hasChildren = segment.children.length > 0;

  return (
    <div>
      <div
        className="flex items-center gap-1.5 py-0.5 text-xs hover:bg-muted/30 rounded-sm"
        style={{ paddingLeft: `${depth * 12 + 4}px` }}
      >
        {hasChildren ? (
          <button
            onClick={() => setExpanded(!expanded)}
            className="flex size-3 items-center justify-center text-muted-foreground hover:text-foreground"
          >
            {expanded ? (
              <ChevronDown className="size-2.5" />
            ) : (
              <ChevronRight className="size-2.5" />
            )}
          </button>
        ) : (
          <span className="size-3" />
        )}

        <span
          className="size-2 rounded-full shrink-0"
          style={{ backgroundColor: segmentColor(segment.kind) }}
        />

        <span className="flex-1 truncate text-muted-foreground">
          {segment.label}
        </span>

        <span className="tabular-nums text-muted-foreground/70">
          ~{formatTokens(segment.tokens_scaled)}
        </span>
      </div>

      {hasChildren && expanded && (
        <div>
          {segment.children.map((child, i) => (
            <SegmentRow key={`${child.label}-${i}`} segment={child} depth={depth + 1} />
          ))}
        </div>
      )}
    </div>
  );
}
