import { FormEvent, useEffect, useState } from "react";
import { Camera, Globe, Loader2, MoreHorizontal, Play, RefreshCw, X } from "lucide-react";
import type { MewClient } from "@mew/web-client";
import type { WorkbenchTab } from "../lib/workbench-tabs";
import { acceptsBrowserEvent } from "../lib/browser-lifecycle";
import { Button } from "./ui/button";
import { cn } from "../lib/utils";

export function BrowserPanel({
  client,
  connected,
  active = true,
  tab,
  onTabChange,
}: {
  client: MewClient | null;
  connected: boolean;
  active?: boolean;
  tab: WorkbenchTab;
  onTabChange: (patch: Partial<WorkbenchTab>) => void;
}) {
  const tabUrl = tab.payload?.url ?? "";
  const [url, setUrl] = useState(tabUrl);
  const [currentUrl, setCurrentUrl] = useState("");
  const [title, setTitle] = useState("");
  const [snapshot, setSnapshot] = useState("");
  const [screenshot, setScreenshot] = useState<string | null>(null);
  const [selector, setSelector] = useState("");
  const [fillText, setFillText] = useState("");
  const [toolsOpen, setToolsOpen] = useState(false);
  const [inspectorOpen, setInspectorOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [browserError, setBrowserError] = useState<string | null>(null);

  useEffect(() => {
    setUrl(tabUrl);
    setCurrentUrl(tabUrl);
    setTitle(tab.title === "New tab" ? "" : tab.title);
    setSnapshot("");
    setScreenshot(null);
    setBrowserError(null);
    if (!tabUrl || !active || !client || !connected) {
      setBusy(false);
      return;
    }
    setBusy(true);
    try {
      client.browserOpen(tabUrl, tab.id);
    } catch (error) {
      setBusy(false);
      setBrowserError(error instanceof Error ? error.message : String(error));
    }
  }, [active, client, connected, tab.id, tabUrl]);

  useEffect(() => {
    if (!client) return;
    const isCurrentEvent = (data: { tabId?: string; url?: string }) => acceptsBrowserEvent(data, {
      tabId: tab.id,
      url: tab.payload?.url ?? "",
      active,
    });
    const onState = (data: { open: boolean; url?: string; title?: string; tabId?: string }) => {
      if (!isCurrentEvent(data)) return;
      setBusy(false);
      setCurrentUrl(data.url ?? "");
      setTitle(data.title ?? "");
      setUrl(data.url ?? "");
      onTabChange({ title: data.title || tab.title, payload: { ...tab.payload, url: data.url } });
    };
    const onSnapshot = (data: { snapshot: string; url: string; title: string; tabId?: string }) => {
      if (!isCurrentEvent(data)) return;
      setBusy(false);
      setSnapshot(formatSnapshot(data.snapshot));
      setCurrentUrl(data.url);
      setTitle(data.title);
      setUrl(data.url);
      onTabChange({ title: data.title || tab.title, payload: { ...tab.payload, url: data.url } });
    };
    const onScreenshot = (data: { data: string; url: string; tabId?: string }) => {
      if (!isCurrentEvent(data)) return;
      setBusy(false);
      setScreenshot(`data:image/png;base64,${data.data}`);
      setCurrentUrl(data.url);
      setUrl(data.url);
      onTabChange({ payload: { ...tab.payload, url: data.url } });
    };
    const onError = (data: { message: string }) => {
      if (!isBrowserError(data.message)) return;
      setBusy(false);
      setBrowserError(data.message);
    };
    const onBrowserError = (data: { message: string; tabId?: string }) => {
      if (data.tabId ? data.tabId !== tab.id : !active) return;
      setBusy(false);
      setBrowserError(data.message);
    };
    client.on("browser-state", onState);
    client.on("browser-snapshot", onSnapshot);
    client.on("browser-screenshot", onScreenshot);
    client.on("browser-error", onBrowserError);
    client.on("errorMessage", onError);
    return () => {
      client.off("browser-state", onState);
      client.off("browser-snapshot", onSnapshot);
      client.off("browser-screenshot", onScreenshot);
      client.off("browser-error", onBrowserError);
      client.off("errorMessage", onError);
    };
  }, [active, client, onTabChange, tab]);

  const run = (action: () => void) => {
    setBusy(true);
    setBrowserError(null);
    try {
      action();
    } catch (error) {
      setBusy(false);
      setBrowserError(error instanceof Error ? error.message : String(error));
    }
  };

  const open = (event: FormEvent) => {
    event.preventDefault();
    if (!canUseBrowser || !url.trim()) return;
    setBrowserError(null);
    onTabChange({ title: tabTitleForUrl(url.trim()), payload: { ...tab.payload, url: url.trim() } });
  };

  const close = () => {
    setToolsOpen(false);
    if (client && connected) run(() => client.browserClose(tab.id));
  };

  const canUseBrowser = Boolean(client) && connected;

  const closeTools = () => {
    setToolsOpen(false);
    setInspectorOpen(false);
  };

  const openTools = () => {
    setToolsOpen((value) => !value);
  };

  return (
    <div className="flex h-full min-h-0 flex-col bg-background">
      <div className="relative shrink-0 border-b border-border px-3 py-2">
        <form onSubmit={open} className="flex min-w-0 items-center">
          <div className="flex min-w-0 flex-1 items-center gap-2 rounded-full border border-border bg-muted/35 px-3 py-1.5 focus-within:border-ring focus-within:ring-2 focus-within:ring-ring/20">
            <Globe className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
            <input
              value={url}
              onChange={(event) => setUrl(event.target.value)}
              className="min-w-0 flex-1 bg-transparent text-xs outline-none placeholder:text-muted-foreground"
              placeholder={currentUrl || "Search or enter URL"}
              aria-label="Browser URL"
              spellCheck={false}
            />
            {busy && <Loader2 className="h-3.5 w-3.5 shrink-0 animate-spin text-muted-foreground" />}
            <Button type="submit" size="icon-sm" variant="ghost" className="shrink-0 rounded-full" disabled={!canUseBrowser || busy} aria-label="Open URL">
              <Play className="h-3.5 w-3.5" />
            </Button>
            <button
              type="button"
              className={cn("motion-pressable shrink-0 rounded-full p-1.5 text-muted-foreground hover:bg-accent hover:text-foreground", toolsOpen && "bg-accent text-foreground")}
              onClick={openTools}
              aria-label="Open browser tools"
              aria-expanded={toolsOpen}
              title="Browser tools"
            >
              <MoreHorizontal className="h-3.5 w-3.5" />
            </button>
          </div>
        </form>

        {browserError && (
          <div role="alert" className="mt-2 rounded-md border border-destructive/30 bg-destructive/10 px-2.5 py-2 text-[11px] text-destructive">
            {browserError}
          </div>
        )}

        {toolsOpen && (
          <div className="motion-enter absolute right-3 top-full z-50 mt-2 w-[min(24rem,calc(100vw-1.5rem))] rounded-xl border border-border bg-popover p-2 text-popover-foreground shadow-2xl">
            <div className="flex items-center justify-between px-2 py-1.5">
              <div>
                <p className="text-xs font-medium">Browser tools</p>
                <p className="text-[10px] text-muted-foreground">Inspect the visible page without taking space from it.</p>
              </div>
              <button type="button" className="rounded p-1 text-muted-foreground hover:bg-accent hover:text-foreground" onClick={closeTools} aria-label="Close browser tools">
                <X className="h-3.5 w-3.5" />
              </button>
            </div>

            <div className="grid grid-cols-2 gap-1.5 p-1">
              <Button size="sm" variant="ghost" className="justify-start gap-2 text-xs" disabled={!client || !connected} onClick={() => client && connected && run(() => client.browserSnapshot(tab.id))}>
                <RefreshCw className="h-3.5 w-3.5" />
                Text snapshot
              </Button>
              <Button size="sm" variant="ghost" className="justify-start gap-2 text-xs" disabled={!client || !connected} onClick={() => client && connected && run(() => client.browserScreenshot(false, tab.id))}>
                <Camera className="h-3.5 w-3.5" />
                Screenshot
              </Button>
            </div>

            {(snapshot || screenshot) && (
              <div className="mt-1 space-y-2 border-t border-border p-2">
                {snapshot && <details open className="rounded-md border border-border bg-muted/25 text-[10px]">
                  <summary className="cursor-pointer px-2 py-1.5 font-medium text-muted-foreground">Text snapshot</summary>
                  <pre className="max-h-40 overflow-auto whitespace-pre-wrap border-t border-border p-2 font-mono leading-relaxed text-foreground/85">{snapshot}</pre>
                </details>}
                {screenshot && <img src={screenshot} alt={title ? `Screenshot of ${title}` : "Browser screenshot"} className="max-h-40 w-full rounded-md border border-border object-contain" />}
              </div>
            )}

            <div className="mt-1 border-t border-border pt-1">
              <button type="button" className="flex w-full items-center justify-between rounded-md px-2 py-1.5 text-left text-xs font-medium hover:bg-accent" onClick={() => setInspectorOpen((value) => !value)} aria-expanded={inspectorOpen}>
                Inspect and interact
                <span className="text-[10px] text-muted-foreground">{inspectorOpen ? "Hide" : "Show"}</span>
              </button>
              {inspectorOpen && <div className="space-y-1.5 p-2">
                <input
                  value={selector}
                  onChange={(event) => setSelector(event.target.value)}
                  className="h-8 w-full rounded-md border border-border bg-background px-2.5 text-[11px] outline-none focus-visible:ring-2 focus-visible:ring-ring"
                  placeholder="element ref or selector, e.g. @e3"
                  aria-label="Browser element selector"
                />
                <div className="flex gap-1.5">
                  <input
                    value={fillText}
                    onChange={(event) => setFillText(event.target.value)}
                    className="h-8 min-w-0 flex-1 rounded-md border border-border bg-background px-2.5 text-[11px] outline-none focus-visible:ring-2 focus-visible:ring-ring"
                    placeholder="text to fill"
                    aria-label="Browser fill text"
                  />
                  <Button size="sm" variant="secondary" className="h-8 px-2.5 text-[11px]" disabled={!client || !connected || !selector.trim()} onClick={() => client && connected && run(() => client.browserFill(selector.trim(), fillText, tab.id))}>Fill</Button>
                  <Button size="sm" variant="secondary" className="h-8 px-2.5 text-[11px]" disabled={!client || !connected || !selector.trim()} onClick={() => client && connected && run(() => client.browserClick(selector.trim(), tab.id))}>Click</Button>
                </div>
              </div>}
            </div>

            <button type="button" className="mt-1 flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-xs text-muted-foreground hover:bg-accent hover:text-foreground" onClick={close}>
              <X className="h-3.5 w-3.5" />
              Hide browser page
            </button>
          </div>
        )}
      </div>

      <div className="min-h-0 flex-1 overflow-auto bg-muted/20">
        <div className="flex h-full min-h-32 flex-col items-center justify-center gap-2 px-6 text-center text-[11px] text-muted-foreground">
          <Globe className="h-5 w-5 opacity-50" />
          <p>open a page, then use Browser tools to inspect or capture it.</p>
        </div>
      </div>
    </div>
  );
}

function tabTitleForUrl(value: string): string {
  try {
    const parsed = new URL(value);
    return parsed.hostname.replace(/^www\./, "") || "New tab";
  } catch {
    return value || "New tab";
  }
}

function formatSnapshot(value: string): string {
  try {
    return JSON.stringify(JSON.parse(value), null, 2);
  } catch {
    return value;
  }
}

function isBrowserError(message: string): boolean {
  return /\bbrowser\b|browser_/i.test(message);
}
