import { useMemo, useState } from "react";
import { BrainCircuit, ChevronRight } from "lucide-react";
import { cn } from "../lib/utils";

interface ReasoningBlockProps {
  text: string;
  streaming?: boolean;
}

/** A collapsed chip that expands into a step-oriented reasoning timeline.
 *
 *  The collapsed surface shows "Reasoning · N steps". Expanded state shows
 *  each step numbered on a vertical timeline. Streaming reasoning appends to
 *  the last step with a live typing indicator. */
export function ReasoningBlock({ text, streaming }: ReasoningBlockProps) {
  const [expanded, setExpanded] = useState(false);
  const steps = useMemo(() => splitSteps(text), [text]);
  const countText = streaming ? `${steps.length} steps · live` : `${steps.length} steps`;

  return (
    <div className="max-w-[85%]">
      <button
        onClick={() => setExpanded((e) => !e)}
        className={cn(
          "flex items-center gap-1.5 rounded-full border px-2.5 py-1 text-xs transition-colors",
          expanded
            ? "border-border bg-muted text-foreground"
            : "border-border bg-card text-muted-foreground hover:bg-muted",
        )}
      >
        <BrainCircuit className="h-3.5 w-3.5" />
        <span>Reasoning</span>
        <span className="text-[10px] opacity-70">· {countText}</span>
        <ChevronRight
          className={cn("h-3 w-3 shrink-0 transition-transform", expanded && "rotate-90")}
        />
      </button>

      {expanded && (
        <div className="mt-2 rounded-lg border border-border bg-card p-3">
          <div className="relative space-y-3">
            {steps.map((step, i) => (
              <div key={i} className="flex gap-3">
                <div className="flex flex-col items-center gap-1">
                  <span className="flex h-5 w-5 shrink-0 items-center justify-center rounded-full bg-muted text-[10px] font-medium text-muted-foreground">
                    {i + 1}
                  </span>
                  {i < steps.length - 1 && (
                    <span className="w-px flex-1 bg-border" />
                  )}
                </div>
                <div className="min-w-0 flex-1 pb-3">
                  <p className="whitespace-pre-wrap text-sm text-muted-foreground">
                    {step}
                  </p>
                </div>
              </div>
            ))}
            {streaming && (
              <div className="flex gap-3">
                <span className="flex h-5 w-5 shrink-0 items-center justify-center rounded-full bg-blue-500/10 text-[10px] text-blue-500">
                  …
                </span>
                <div className="flex items-center gap-1 pt-1">
                  <span className="h-1.5 w-1.5 animate-bounce rounded-full bg-blue-500" />
                  <span className="h-1.5 w-1.5 animate-bounce rounded-full bg-blue-500 [animation-delay:0.1s]" />
                  <span className="h-1.5 w-1.5 animate-bounce rounded-full bg-blue-500 [animation-delay:0.2s]" />
                </div>
              </div>
            )}
          </div>
        </div>
      )}
    </div>
  );
}

function splitSteps(raw: string): string[] {
  const text = String(raw ?? "");
  if (!text.trim()) return [];
  const blocks = text.split(/\n{2,}/).map((s) => s.trim()).filter(Boolean);
  if (blocks.length === 0) return [text.trim()];
  return blocks;
}
