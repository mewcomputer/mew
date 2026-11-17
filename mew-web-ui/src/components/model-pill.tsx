import { useEffect, useState } from "react";
import { ChevronDown, Loader2 } from "lucide-react";
import type { ModelInfo } from "@mew/web-client";
import { useSessionStore } from "../stores/session";
import { shortModel } from "../lib/format";
import { getClient } from "../lib/client-ref";
import {
  DropdownMenu, DropdownMenuContent, DropdownMenuLabel, DropdownMenuRadioGroup,
  DropdownMenuRadioItem, DropdownMenuSeparator, DropdownMenuTrigger,
} from "./ui/dropdown-menu";

export function ModelPill() {
  const availableModels = useSessionStore((s) => s.availableModels);
  const currentModel = useSessionStore((s) => s.currentModel);
  const currentProvider = useSessionStore((s) => s.currentProvider);
  const currentThinkingVariant = useSessionStore((s) => s.currentThinkingVariant);
  const [open, setOpen] = useState(false);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const sessionId = useSessionStore((s) => s.sessionId);
  useEffect(() => { setOpen(false); setError(null); }, [sessionId]);

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
    setPending(true);
    setError(null);
    try {
      const resolved = await client.setThinkingVariant(variant);
      if (useSessionStore.getState().sessionId !== sessionId) return;
      useSessionStore.getState().setCurrentThinkingVariant(resolved);
      setOpen(false);
    } catch (error) {
      if (useSessionStore.getState().sessionId === sessionId) {
        setError(error instanceof Error ? error.message : "Could not change thinking. Try again.");
      }
    } finally {
      setPending(false);
    }
  };

  const byProvider = new Map<string, ModelInfo[]>();
  for (const model of availableModels) {
    const list = byProvider.get(model.provider) ?? [];
    list.push(model);
    byProvider.set(model.provider, list);
  }
  const current = availableModels.find((model) => model.provider === currentProvider && model.model === currentModel);
  const variants = current?.thinking_variants ?? [];
  const label = current?.display_name || (currentModel ? shortModel(currentModel) : "Choose model");

  return (
    <DropdownMenu open={open} onOpenChange={(value) => { setOpen(value); setError(null); }}>
      <DropdownMenuTrigger asChild>
        <button type="button" title="Switch model" className="motion-pressable flex h-7 min-w-0 max-w-56 items-center gap-1.5 rounded-md px-1.5 text-[11px] text-muted-foreground hover:bg-accent hover:text-accent-foreground data-[state=open]:bg-accent">
          <span className="truncate" title={label}>{label}</span>
          {currentThinkingVariant && <span className="max-w-16 truncate text-[10px] text-muted-foreground">{currentThinkingVariant}</span>}
          {pending ? <Loader2 className="h-3 w-3 shrink-0 animate-spin" /> : <ChevronDown className="h-3 w-3 shrink-0" />}
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent side="top" align="end" sideOffset={10} aria-label="Model" aria-busy={pending} className="w-72 max-w-[calc(100vw-2rem)] rounded-xl p-1.5">
        {error && <p role="alert" className="px-2 py-2 text-xs text-text-error">{error}</p>}
        {variants.length > 0 && <>
          <DropdownMenuLabel className="text-[11px] font-normal text-muted-foreground">Thinking</DropdownMenuLabel>
          <DropdownMenuRadioGroup value={currentThinkingVariant ?? ""} onValueChange={(value) => void setVariant(value)} className="flex flex-wrap gap-1 px-1 pb-1">
            {[{ name: "", label: "Off" }, ...variants.map((variant) => ({ name: variant.name, label: variant.name }))].map((variant) => (
              <DropdownMenuRadioItem key={variant.name} value={variant.name} disabled={pending} onSelect={(event) => event.preventDefault()} className="min-h-7 rounded-md text-xs capitalize data-[state=checked]:bg-accent">{variant.label}</DropdownMenuRadioItem>
            ))}
          </DropdownMenuRadioGroup>
          <DropdownMenuSeparator />
        </>}
        <DropdownMenuRadioGroup value={`${currentProvider}/${currentModel}`} onValueChange={(id) => {
          const model = availableModels.find((item) => item.id === id);
          if (model) void switchModel(model);
        }}>
          {availableModels.length === 0 && <p className="px-2 py-4 text-xs text-muted-foreground">No models available. Check your provider settings.</p>}
          {[...byProvider.keys()].sort().map((provider) => (
            <div key={provider}>
              <DropdownMenuLabel className="text-[11px] font-normal text-muted-foreground">{provider}</DropdownMenuLabel>
              {byProvider.get(provider)!.map((model) => (
                <DropdownMenuRadioItem key={model.id} value={model.id} disabled={pending} onSelect={(event) => event.preventDefault()} className="min-h-9 rounded-lg text-xs">
                  <span className="min-w-0 flex-1">
                    <span className="block truncate font-medium">{model.display_name || model.model}</span>
                    {model.description && <span className="block truncate text-[11px] text-muted-foreground">{model.description}</span>}
                  </span>
                </DropdownMenuRadioItem>
              ))}
            </div>
          ))}
        </DropdownMenuRadioGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
