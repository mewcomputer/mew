import { useRef, useState } from "react";
import type { ModelInfo } from "@mew/web-client";
import { cn } from "../lib/utils";
import { useSessionStore } from "../stores/session";
import { useOutsideClick } from "../lib/useOutsideClick";
import { shortModel } from "../lib/format";
import { getClient } from "../lib/client-ref";

export function ModelPill() {
  const availableModels = useSessionStore((s) => s.availableModels);
  const currentModel = useSessionStore((s) => s.currentModel);
  const currentProvider = useSessionStore((s) => s.currentProvider);
  const currentThinkingVariant = useSessionStore((s) => s.currentThinkingVariant);
  const [open, setOpen] = useState(false);
  const [thinkingOpen, setThinkingOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  useOutsideClick(ref, open, () => {
    setOpen(false);
    setThinkingOpen(false);
  });

  const handleSwitch = async (model: ModelInfo) => {
    const client = getClient();
    if (!client) return;
    setThinkingOpen(false);
    setOpen(false);
    const result = await client.switchModel(model.provider, model.model);
    if (result) {
      useSessionStore.getState().setCurrentModel(result.provider, result.model);
      useSessionStore.getState().setCurrentThinkingVariant(null);
    }
  };

  const handleSetVariant = async (variant: string) => {
    const client = getClient();
    if (!client) return;
    setThinkingOpen(false);
    setOpen(false);
    const resolved = await client.setThinkingVariant(variant);
    useSessionStore.getState().setCurrentThinkingVariant(resolved);
  };

  const byProvider = new Map<string, ModelInfo[]>();
  for (const m of availableModels) {
    const list = byProvider.get(m.provider) ?? [];
    list.push(m);
    byProvider.set(m.provider, list);
  }
  const providers = [...byProvider.keys()].sort();

  const label = currentModel ? shortModel(currentModel) : "model";
  const currentModelInfo = availableModels.find(
    (m) => m.provider === currentProvider && m.model === currentModel,
  );
  const variants = currentModelInfo?.thinking_variants ?? [];

  return (
    <div ref={ref} className="relative">
      <button
        onClick={() => setOpen((o) => !o)}
        className="flex items-center gap-1 rounded-md px-2 py-1 text-[11px] text-muted-foreground transition-colors hover:bg-accent hover:text-accent-foreground"
        title="Switch model"
      >
        <span className="truncate font-mono">{label}</span>
        {currentThinkingVariant && (
          <span className="rounded bg-primary/10 px-1 text-[9px] text-primary">
            {currentThinkingVariant}
          </span>
        )}
        <svg
          className={cn("h-3 w-3 transition-transform", open && "rotate-180")}
          viewBox="0 0 12 12"
          fill="none"
          stroke="currentColor"
          strokeWidth="2"
        >
          <path d="M3 7.5L6 4.5L9 7.5" />
        </svg>
      </button>

      {open && (
        <div className="absolute bottom-full left-0 z-50 mb-1 w-72 rounded-lg border border-border bg-popover shadow-lg">
          {variants.length > 0 && (
            <div className="border-b border-border p-1">
              <button
                onClick={() => setThinkingOpen((o) => !o)}
                className="flex w-full items-center justify-between rounded-md px-2 py-1 text-xs transition-colors hover:bg-accent"
              >
                <span className="text-muted-foreground">Thinking</span>
                <span className="flex items-center gap-1">
                  <span className="font-medium text-foreground">
                    {currentThinkingVariant ?? "off"}
                  </span>
                  <svg
                    className={cn("h-3 w-3 transition-transform", thinkingOpen && "rotate-180")}
                    viewBox="0 0 12 12"
                    fill="none"
                    stroke="currentColor"
                    strokeWidth="2"
                  >
                    <path d="M3 4.5L6 7.5L9 4.5" />
                  </svg>
                </span>
              </button>
              {thinkingOpen && (
                <div className="mt-0.5 space-y-0.5">
                  <VariantRow
                    label="Off"
                    active={currentThinkingVariant === null}
                    onClick={() => handleSetVariant("")}
                  />
                  {variants.map((v) => (
                    <VariantRow
                      key={v.name}
                      label={v.name}
                      active={currentThinkingVariant === v.name}
                      onClick={() => handleSetVariant(v.name)}
                    />
                  ))}
                </div>
              )}
            </div>
          )}
          <div className="max-h-64 overflow-y-auto p-1">
            {availableModels.length === 0 && (
              <div className="px-3 py-4 text-center text-xs text-muted-foreground">
                {getClient() ? "Loading models…" : "Not connected"}
              </div>
            )}
            {providers.map((provider) => (
              <div key={provider}>
                <div className="px-2 py-1 text-[9px] font-semibold uppercase tracking-wide text-muted-foreground">
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
                        "flex w-full items-center justify-between rounded-md px-2 py-1.5 text-left text-xs transition-colors hover:bg-accent",
                        isActive && "bg-accent",
                      )}
                    >
                      <div className="min-w-0 flex-1">
                        <div className="truncate font-medium text-foreground">
                          {m.model}
                        </div>
                        {m.description && (
                          <div className="truncate text-[10px] text-muted-foreground">
                            {m.description}
                          </div>
                        )}
                      </div>
                      {isActive && (
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

function VariantRow({
  label,
  active,
  onClick,
}: {
  label: string;
  active: boolean;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      className={cn(
        "flex w-full items-center justify-between rounded-md px-2 py-1 text-left text-xs transition-colors hover:bg-accent",
        active && "bg-accent",
      )}
    >
      <span className="capitalize text-foreground">{label}</span>
      {active && (
        <svg
          className="h-3 w-3 shrink-0 text-primary"
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
}
