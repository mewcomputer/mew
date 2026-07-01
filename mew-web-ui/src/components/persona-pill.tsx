import { useRef, useState } from "react";
import { User } from "lucide-react";
import { cn } from "../lib/utils";
import { useSessionStore } from "../stores/session";
import { useOutsideClick } from "../lib/useOutsideClick";

const OPTIONS = ["default", "code-reviewer", "explainer"];

export function PersonaPill() {
  const persona = useSessionStore((s) => s.currentPersona);
  const setPersona = useSessionStore((s) => s.setCurrentPersona);
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  useOutsideClick(ref, open, () => setOpen(false));

  const label = persona ?? "default";

  return (
    <div ref={ref} className="relative">
      <button
        onClick={() => setOpen((o) => !o)}
        className="flex items-center gap-1 rounded-md border border-border px-2 py-1 text-[10px] text-muted-foreground transition-colors hover:bg-accent hover:text-accent-foreground"
        title="Persona"
      >
        <User className="h-3 w-3" />
        <span className="capitalize">{label}</span>
      </button>

      {open && (
        <div className="absolute bottom-full right-0 z-50 mb-1 w-36 rounded-lg border border-border bg-popover shadow-lg">
          {OPTIONS.map((p) => (
            <button
              key={p}
              onClick={() => {
                setPersona(p === "default" ? null : p);
                setOpen(false);
              }}
              className={cn(
                "flex w-full items-center justify-between px-2 py-1 text-left text-[11px] transition-colors hover:bg-accent",
                label === p && "bg-accent",
              )}
            >
              <span className="capitalize text-foreground">{p}</span>
              {label === p && (
                <svg
                  className="ml-2 h-3 w-3 shrink-0 text-primary"
                  viewBox="0 0 16 16"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="2"
                >
                  <path d="M3 8L6.5 11.5L13 5" />
                </svg>
              )}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
