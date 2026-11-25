import { useEffect, useMemo, useRef, useState } from "react";
import { Check, ChevronDown, ChevronLeft, ChevronRight, Clock, Loader2, Search } from "lucide-react";
import type { ModelInfo } from "@mew/web-client";
import { useSessionStore } from "../stores/session";
import { shortModel } from "../lib/format";
import { getClient } from "../lib/client-ref";
import { getHostBootstrap } from "../lib/host";
import { cn } from "../lib/utils";
import { ProviderIcon } from "./provider-icon";
import { Popover, PopoverContent, PopoverTrigger } from "./ui/popover";

const RECENT_MODELS_STORAGE_KEY = "mew.recentModels";
const MAX_RECENT_MODELS = 6;

export function loadRecentModels(): string[] {
  const fromHost = getHostBootstrap()?.recentModels ?? [];
  let fromStorage: string[] = [];
  try {
    const raw = typeof window !== "undefined" ? window.localStorage.getItem(RECENT_MODELS_STORAGE_KEY) : null;
    if (raw) {
      const parsed: unknown = JSON.parse(raw);
      if (Array.isArray(parsed)) {
        fromStorage = parsed.filter((item): item is string => typeof item === "string" && item.includes("/"));
      }
    }
  } catch {
    // Ignore storage read errors in restricted contexts.
  }
  const merged: string[] = [];
  for (const id of [...fromStorage, ...fromHost]) {
    if (!merged.includes(id)) merged.push(id);
    if (merged.length >= MAX_RECENT_MODELS) break;
  }
  return merged;
}

export function recordRecentModel(id: string, previous: string[]): string[] {
  const next = [id, ...previous.filter((item) => item !== id)].slice(0, MAX_RECENT_MODELS);
  try {
    if (typeof window !== "undefined") {
      window.localStorage.setItem(RECENT_MODELS_STORAGE_KEY, JSON.stringify(next));
    }
  } catch {
    // Ignore storage write errors.
  }
  return next;
}

export function ModelPill() {
  const availableModels = useSessionStore((s) => s.availableModels);
  const currentModel = useSessionStore((s) => s.currentModel);
  const currentProvider = useSessionStore((s) => s.currentProvider);
  const currentThinkingVariant = useSessionStore((s) => s.currentThinkingVariant);
  const sessionId = useSessionStore((s) => s.sessionId);

  const [open, setOpen] = useState(false);
  const [stage, setStage] = useState<"effort" | "models">("effort");
  const [query, setQuery] = useState("");
  const [dragEffortIdx, setDragEffortIdx] = useState<number | null>(null);
  const [optimisticVariant, setOptimisticVariant] = useState<string | null | undefined>(undefined);
  const [recentIds, setRecentIds] = useState<string[]>(() => loadRecentModels());
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const searchInputRef = useRef<HTMLInputElement>(null);
  const sliderTrackRef = useRef<HTMLDivElement>(null);

  const current = useMemo(
    () => availableModels.find((model) => model.provider === currentProvider && model.model === currentModel),
    [availableModels, currentProvider, currentModel],
  );
  const variants = current?.thinking_variants ?? [];
  const hasThinking = variants.length > 0;

  useEffect(() => {
    setOpen(false);
    setError(null);
    setQuery("");
    setOptimisticVariant(undefined);
  }, [sessionId]);

  useEffect(() => {
    setOptimisticVariant(undefined);
  }, [currentThinkingVariant]);

  useEffect(() => {
    if (open && stage === "models") {
      requestAnimationFrame(() => searchInputRef.current?.focus());
    }
  }, [open, stage]);

  const handleOpenChange = (nextOpen: boolean) => {
    setOpen(nextOpen);
    setError(null);
    setDragEffortIdx(null);
    setOptimisticVariant(undefined);
    if (nextOpen) {
      setQuery("");
      setRecentIds(loadRecentModels());
      setStage(hasThinking ? "effort" : "models");
    }
  };

  const switchModel = async (model: ModelInfo) => {
    const client = getClient();
    if (!client || pending) return;
    setPending(true);
    setError(null);
    try {
      const result = await client.switchModel(model.provider, model.model);
      if (useSessionStore.getState().sessionId !== sessionId) return;
      if (result) {
        useSessionStore.getState().setCurrentModel(result.provider, result.model);
        useSessionStore.getState().setCurrentThinkingVariant(null);
        setRecentIds((prev) => recordRecentModel(`${result.provider}/${result.model}`, prev));
        setOpen(false);
      }
    } catch (error) {
      if (useSessionStore.getState().sessionId === sessionId) {
        setError(error instanceof Error ? error.message : "Could not switch models. Try again.");
      }
    } finally {
      setPending(false);
    }
  };

  const setVariant = async (variant: string) => {
    const client = getClient();
    if (!client || pending) return;
    setOptimisticVariant(variant || null);
    setPending(true);
    setError(null);
    try {
      const resolved = await client.setThinkingVariant(variant);
      if (useSessionStore.getState().sessionId !== sessionId) return;
      useSessionStore.getState().setCurrentThinkingVariant(resolved);
    } catch (error) {
      if (useSessionStore.getState().sessionId === sessionId) {
        setOptimisticVariant(undefined);
        setError(error instanceof Error ? error.message : "Could not change thinking. Try again.");
      }
    } finally {
      setPending(false);
    }
  };

  const displayedThinkingVariant = optimisticVariant !== undefined ? optimisticVariant : currentThinkingVariant;

  const effortSteps = useMemo(
    () => [{ name: "", label: "Off" }, ...variants.map((v) => ({ name: v.name, label: capitalize(v.name) }))],
    [variants],
  );
  const selectedEffortIdx = Math.max(
    0,
    effortSteps.findIndex((step) => {
      if (!displayedThinkingVariant) return step.name === "";
      return step.name.toLowerCase() === displayedThinkingVariant.toLowerCase();
    }),
  );
  const activeEffortIdx = dragEffortIdx ?? selectedEffortIdx;
  const activeEffort = effortSteps[activeEffortIdx] ?? effortSteps[0]!;
  const maxStepIdx = Math.max(1, effortSteps.length - 1);
  const fillFraction = effortSteps.length <= 1 ? 1 : activeEffortIdx / maxStepIdx;

  const indexFromClientX = (clientX: number): number => {
    const track = sliderTrackRef.current;
    if (!track || effortSteps.length <= 1) return 0;
    const rect = track.getBoundingClientRect();
    const pad = 14; // 4px padding + 10px thumb radius
    const travel = Math.max(1, rect.width - pad * 2);
    const ratio = Math.min(1, Math.max(0, (clientX - (rect.left + pad)) / travel));
    return Math.min(effortSteps.length - 1, Math.max(0, Math.round(ratio * (effortSteps.length - 1))));
  };

  const recentModels = useMemo(() => {
    if (query.trim()) return [];
    const byId = new Map(availableModels.map((m) => [m.id, m]));
    return recentIds.map((id) => byId.get(id)).filter((m): m is ModelInfo => Boolean(m));
  }, [availableModels, recentIds, query]);

  const filteredByProvider = useMemo(() => {
    const normalized = query.trim().toLowerCase();
    const map = new Map<string, ModelInfo[]>();
    for (const model of availableModels) {
      if (normalized) {
        const haystack = `${model.provider} ${model.display_name ?? ""} ${model.model} ${model.description ?? ""}`.toLowerCase();
        if (!haystack.includes(normalized)) continue;
      }
      const list = map.get(model.provider) ?? [];
      list.push(model);
      map.set(model.provider, list);
    }
    return map;
  }, [availableModels, query]);

  const label = current?.display_name || (currentModel ? shortModel(currentModel) : "Choose model");

  return (
    <Popover open={open} onOpenChange={handleOpenChange}>
      <PopoverTrigger asChild>
        <button
          type="button"
          title="Switch model"
          className="motion-pressable flex h-7 min-w-0 max-w-56 items-center gap-1.5 rounded-full px-2 text-[11px] text-muted-foreground hover:bg-accent hover:text-accent-foreground data-[state=open]:bg-accent data-[state=open]:text-accent-foreground"
        >
          {currentProvider && <ProviderIcon provider={currentProvider} variant="mono" className="h-3 w-3" />}
          <span className="truncate" title={label}>{label}</span>
          {currentThinkingVariant && (
            <span className="max-w-16 truncate text-[10px] capitalize text-muted-foreground/80">
              {currentThinkingVariant}
            </span>
          )}
          {pending ? <Loader2 className="h-3 w-3 shrink-0 animate-spin" /> : <ChevronDown className="h-3 w-3 shrink-0" />}
        </button>
      </PopoverTrigger>
      <PopoverContent
        side="top"
        align="end"
        sideOffset={10}
        role="menu"
        aria-label="Model"
        aria-busy={pending}
        className={cn(
          "max-w-[calc(100vw-2rem)] overflow-hidden rounded-2xl border border-border/80 bg-popover text-popover-foreground shadow-xl transition-[width,padding] duration-200 ease-[cubic-bezier(0.23,1,0.32,1)]",
          stage === "effort" && hasThinking ? "w-[17rem] p-3" : "w-72 p-1.5",
        )}
      >
        {error && <p role="alert" className="px-2 py-1.5 text-xs text-text-error">{error}</p>}

        {stage === "effort" && hasThinking ? (
          <div key="effort" className="motion-enter flex flex-col gap-3">
            <button
              type="button"
              onClick={() => setStage("models")}
              aria-label="Choose model"
              className="motion-pressable group flex w-full min-w-0 flex-col items-center rounded-xl px-3 py-1.5 text-center transition-colors hover:bg-accent"
            >
              <span key={activeEffort.label} className="motion-enter text-xs font-semibold leading-tight text-foreground">
                {activeEffort.label}
              </span>
              <span className="mt-0.5 flex max-w-full items-center gap-0.5 text-[11px] leading-tight text-muted-foreground group-hover:text-foreground">
                <span className="truncate">{label}</span>
                <ChevronRight className="h-3 w-3 shrink-0 opacity-70 transition-transform group-hover:translate-x-0.5" />
              </span>
            </button>

            <div
              ref={sliderTrackRef}
              role="group"
              aria-label="Thinking effort"
              onMouseDown={(event) => {
                if (event.button !== 0 || pending || effortSteps.length <= 1) return;
                // Only initiate drag if moving across the track; let direct clicks on buttons work naturally too.
                let dragged = false;
                const onMove = (moveEvent: MouseEvent) => {
                  dragged = true;
                  setDragEffortIdx(indexFromClientX(moveEvent.clientX));
                };
                const onUp = (upEvent: MouseEvent) => {
                  window.removeEventListener("mousemove", onMove);
                  window.removeEventListener("mouseup", onUp);
                  if (dragged) {
                    const finalIdx = indexFromClientX(upEvent.clientX);
                    setDragEffortIdx(null);
                    const step = effortSteps[finalIdx];
                    if (step && finalIdx !== selectedEffortIdx) {
                      void setVariant(step.name);
                    }
                  }
                };
                window.addEventListener("mousemove", onMove);
                window.addEventListener("mouseup", onUp);
              }}
              className="relative flex h-7 w-full cursor-pointer select-none items-center rounded-full border border-border/50 bg-background/60 p-1 shadow-inner"
            >
              <div className="pointer-events-none relative h-5 w-full">
                {/* Filled pill extending from the left edge (20px circle at idx 0) to 100% width at the final step */}
                <div
                  aria-hidden="true"
                  className="absolute inset-y-0 left-0 flex items-center justify-end rounded-full bg-foreground/95 shadow-xs transition-[width] duration-220 ease-[cubic-bezier(0.22,1,0.36,1)]"
                  style={{
                    width: `calc(1.25rem + (100% - 1.25rem) * ${fillFraction})`,
                  }}
                >
                  <span className="h-5 w-5 shrink-0 rounded-full bg-foreground shadow-sm ring-1 ring-black/15" />
                </div>
              </div>
              <div className="absolute inset-1 flex items-center justify-between">
                {effortSteps.map((step, idx) => {
                  const isSelected = idx === selectedEffortIdx;
                  const isActive = idx === activeEffortIdx;
                  const isCovered = idx < activeEffortIdx;
                  return (
                    <button
                      key={step.name || "off"}
                      type="button"
                      role="menuitemradio"
                      aria-checked={isSelected}
                      aria-label={step.label}
                      disabled={pending}
                      onClick={() => void setVariant(step.name)}
                      className="relative z-10 flex h-5 w-5 shrink-0 cursor-pointer items-center justify-center rounded-full outline-none focus-visible:ring-2 focus-visible:ring-ring"
                    >
                      <span
                        className={cn(
                          "h-1.5 w-1.5 rounded-full bg-muted-foreground/65 transition-all duration-200 ease-out",
                          (isActive || isCovered) ? "scale-0 opacity-0" : "scale-100 opacity-100",
                        )}
                      />
                      <span className="sr-only">{step.label}</span>
                    </button>
                  );
                })}
              </div>
            </div>
          </div>
        ) : (
          <div key="models" className="motion-enter flex flex-col">
            <div className="flex items-center gap-1 border-b border-border/60 px-1.5 pb-1.5 pt-0.5">
              {hasThinking && (
                <button
                  type="button"
                  onClick={() => setStage("effort")}
                  aria-label="Back to thinking effort"
                  title="Back to thinking effort"
                  className="flex h-6 w-6 shrink-0 items-center justify-center rounded-md text-muted-foreground hover:bg-accent hover:text-foreground"
                >
                  <ChevronLeft className="h-3.5 w-3.5" />
                </button>
              )}
              <div className="flex h-7 min-w-0 flex-1 items-center gap-1.5 rounded-lg bg-muted/50 px-2">
                <Search className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
                <input
                  ref={searchInputRef}
                  type="text"
                  value={query}
                  onChange={(e) => setQuery(e.target.value)}
                  placeholder="Search models…"
                  aria-label="Search models"
                  className="min-w-0 flex-1 bg-transparent text-xs outline-none placeholder:text-muted-foreground"
                />
              </div>
            </div>

            <div className="max-h-72 overflow-y-auto pt-1">
              {availableModels.length === 0 && (
                <p className="px-2 py-4 text-xs text-muted-foreground">
                  No models available. Check your provider settings.
                </p>
              )}
              {availableModels.length > 0 && filteredByProvider.size === 0 && (
                <p className="px-2 py-4 text-center text-xs text-muted-foreground">
                  No models match “{query.trim()}”
                </p>
              )}
              {recentModels.length > 0 && (
                <div className="mb-1.5 border-b border-border/50 pb-1.5">
                  <div className="flex items-center gap-1 px-2 py-1 text-[10px] font-medium uppercase tracking-wider text-muted-foreground/75">
                    <Clock className="h-2.5 w-2.5" />
                    <span>Recent</span>
                  </div>
                  {recentModels.map((model) => {
                    const checked = model.provider === currentProvider && model.model === currentModel;
                    return (
                      <button
                        key={`recent-${model.id}`}
                        type="button"
                        disabled={pending}
                        onClick={() => void switchModel(model)}
                        className={cn(
                          "flex w-full items-center gap-2 rounded-lg px-2.5 py-1.5 text-left text-xs outline-none transition-colors hover:bg-accent hover:text-accent-foreground focus-visible:bg-accent",
                          checked && "bg-accent/70 font-medium text-accent-foreground",
                        )}
                      >
                        <span className="min-w-0 flex-1">
                          <span className="flex items-center gap-1.5">
                            <span className="truncate font-medium">{model.display_name || model.model}</span>
                            <span className="inline-flex shrink-0 items-center gap-1 text-[10px] font-normal text-muted-foreground/75">
                              <ProviderIcon provider={model.provider} variant="mono" className="h-3 w-3" />
                              <span>{model.provider}</span>
                            </span>
                          </span>
                          {model.description && (
                            <span className="block truncate text-[11px] font-normal text-muted-foreground">
                              {model.description}
                            </span>
                          )}
                        </span>
                        {checked && <Check className="h-3.5 w-3.5 shrink-0 text-foreground" />}
                      </button>
                    );
                  })}
                </div>
              )}
              {[...filteredByProvider.keys()].sort().map((provider) => (
                <div key={provider} className="mb-1 last:mb-0">
                  <div className="flex items-center gap-1.5 px-2 py-1 text-[10px] font-medium uppercase tracking-wider text-muted-foreground/75">
                    <ProviderIcon provider={provider} variant="color" className="h-3.5 w-3.5" />
                    <span>{provider}</span>
                  </div>
                  {filteredByProvider.get(provider)!.map((model) => {
                    const checked = model.provider === currentProvider && model.model === currentModel;
                    return (
                      <button
                        key={model.id}
                        type="button"
                        role="menuitemradio"
                        aria-checked={checked}
                        disabled={pending}
                        onClick={() => void switchModel(model)}
                        className={cn(
                          "flex w-full items-center gap-2 rounded-lg px-2.5 py-1.5 text-left text-xs outline-none transition-colors hover:bg-accent hover:text-accent-foreground focus-visible:bg-accent",
                          checked && "bg-accent/70 font-medium text-accent-foreground",
                        )}
                      >
                        <span className="min-w-0 flex-1">
                          <span className="block truncate font-medium">{model.display_name || model.model}</span>
                          {model.description && (
                            <span className="block truncate text-[11px] font-normal text-muted-foreground">
                              {model.description}
                            </span>
                          )}
                        </span>
                        {checked && <Check className="h-3.5 w-3.5 shrink-0 text-foreground" />}
                      </button>
                    );
                  })}
                </div>
              ))}
            </div>
          </div>
        )}
      </PopoverContent>
    </Popover>
  );
}

function capitalize(value: string): string {
  if (!value) return value;
  return value.charAt(0).toUpperCase() + value.slice(1);
}
