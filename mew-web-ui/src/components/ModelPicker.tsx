import { useEffect, useRef, useState } from "react";
import type { MewClient, ModelInfo } from "@mew/web-client";
import { useSessionStore } from "../stores/session";
import { cn } from "../lib/utils";

interface ModelPickerProps {
  client: MewClient | null;
}

export function ModelPicker({ client }: ModelPickerProps) {
  const availableModels = useSessionStore((s) => s.availableModels);
  const currentModel = useSessionStore((s) => s.currentModel);
  const currentProvider = useSessionStore((s) => s.currentProvider);
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const ref = useRef<HTMLDivElement>(null);

  // Fetch model list on mount (and when client connects).
  useEffect(() => {
    if (client) {
      client.listModels().then((models) => {
        useSessionStore.getState().setAvailableModels(models);
      });
    }
  }, [client]);

  // Close on outside click.
  useEffect(() => {
    if (!open) return;
    const handler = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) {
        setOpen(false);
        setQuery("");
      }
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [open]);

  const handleSwitch = async (model: ModelInfo) => {
    if (!client) return;
    setOpen(false);
    setQuery("");
    const result = await client.switchModel(model.provider, model.model);
    if (result) {
      useSessionStore.getState().setCurrentModel(result.provider, result.model);
    }
  };

  // Group models by provider.
  const filtered = availableModels.filter(
    (m) =>
      m.id.toLowerCase().includes(query.toLowerCase()) ||
      m.model.toLowerCase().includes(query.toLowerCase()) ||
      m.provider.toLowerCase().includes(query.toLowerCase()),
  );
  const byProvider = new Map<string, ModelInfo[]>();
  for (const m of filtered) {
    const list = byProvider.get(m.provider) ?? [];
    list.push(m);
    byProvider.set(m.provider, list);
  }
  const providers = [...byProvider.keys()].sort();

  const displayLabel = currentModel
    ? `${currentProvider}/${currentModel}`
    : "Select model";

  return (
    <div ref={ref} className="relative">
      <button
        onClick={() => setOpen((o) => !o)}
        className="flex items-center gap-1.5 rounded-md border border-border px-2 py-1 text-xs text-muted-foreground transition-colors hover:bg-accent hover:text-accent-foreground"
        title="Switch model"
      >
        <span className="max-w-[200px] truncate font-mono">{displayLabel}</span>
        <svg
          className={cn("h-3 w-3 transition-transform", open && "rotate-180")}
          viewBox="0 0 12 12"
          fill="none"
          stroke="currentColor"
          strokeWidth="2"
        >
          <path d="M3 4.5L6 7.5L9 4.5" />
        </svg>
      </button>

      {open && (
        <div className="absolute right-0 top-full z-50 mt-1 w-80 rounded-lg border border-border bg-popover shadow-lg">
          <div className="border-b border-border p-2">
            <input
              type="text"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="Search models…"
              className="w-full rounded-md bg-background px-2 py-1 text-sm outline-hidden ring-1 ring-border focus:ring-2 focus:ring-ring"
              autoFocus
            />
          </div>
          <div className="max-h-80 overflow-y-auto p-1">
            {availableModels.length === 0 && (
              <div className="px-3 py-4 text-center text-xs text-muted-foreground">
                {client ? "Loading models…" : "Not connected"}
              </div>
            )}
            {providers.map((provider) => (
              <div key={provider}>
                <div className="px-2 py-1 text-[10px] font-semibold uppercase tracking-wide text-muted-foreground">
                  {provider}
                </div>
                {byProvider.get(provider)!.map((m) => {
                  const isActive =
                    currentProvider === m.provider && currentModel === m.model;
                  return (
                    <button
                      key={m.id}
                      onClick={() => handleSwitch(m)}
                      className={cn(
                        "flex w-full items-center justify-between rounded-md px-2 py-1.5 text-left text-sm transition-colors hover:bg-accent",
                        isActive && "bg-accent",
                      )}
                    >
                      <div className="min-w-0 flex-1">
                        <div className="truncate font-medium text-foreground">
                          {m.model}
                        </div>
                        {m.description && (
                          <div className="truncate text-xs text-muted-foreground">
                            {m.description}
                          </div>
                        )}
                      </div>
                      {isActive && (
                        <svg
                          className="ml-2 h-3.5 w-3.5 shrink-0 text-primary"
                          viewBox="0 0 16 16"
                          fill="none"
                          stroke="currentColor"
                          strokeWidth="2"
                        >
                          <path d="M3 8L6.5 11.5L13 5" />
                        </svg>
                      )}
                    </button>
                  );
                })}
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
