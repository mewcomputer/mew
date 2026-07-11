import { useRef, useState } from "react";
import { User } from "lucide-react";
import { cn } from "../lib/utils";
import { useSessionStore } from "../stores/session";
import { useOutsideClick } from "../lib/useOutsideClick";

export function PersonaPill() {
  const persona = useSessionStore((s) => s.currentPersona);
  const availablePersonas = useSessionStore((s) => s.availablePersonas);
  const selectPersona = useSessionStore((s) => s.selectPersona);
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  useOutsideClick(ref, open, () => setOpen(false));

  const label = persona ?? "default";

  const options: { name: string; description?: string; color?: string }[] = [
    { name: "default" },
    ...availablePersonas.map((p) => ({
      name: p.name,
      description: p.description,
      color: p.color,
    })),
  ];

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
          {options.map((p) => (
            <button
              key={p.name}
              title={p.description}
              onClick={() => {
                selectPersona(p.name);
                setOpen(false);
              }}
              className={cn(
                "flex w-full items-center justify-between px-2 py-1 text-left text-[11px] transition-colors hover:bg-accent",
                label === p.name && "bg-accent",
              )}
            >
              <span className="flex items-center gap-1 capitalize text-foreground">
                {p.color && (
                  <span
                    className="h-2 w-2 rounded-full"
                    style={{ backgroundColor: p.color }}
                  />
                )}
                {p.name}
              </span>
              {label === p.name && (
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
