import { FormEvent, useEffect, useState, useRef, type ReactNode } from "react";
import { Camera, Globe, Loader2, MoreHorizontal, ArrowRight, RefreshCw, X } from "lucide-react";
import type { MewClient } from "@mew/web-client";
import type { WorkbenchTab } from "../lib/workbench-tabs";
import { acceptsBrowserEvent } from "../lib/browser-lifecycle";
import { Button } from "./ui/button";
import { NativeBrowserPage } from "./native-browser-page";
import { isDesktopHost } from "../lib/host";
import { cn } from "../lib/utils";
import { useOutsideClick } from "../lib/useOutsideClick";

export function BrowserPanel({
  client,
  connected,
  active = true,
  present = active,
  tab,
  onTabChange,
  sessionId,
  launcher,
}: {
  client: MewClient | null;
  connected: boolean;
  active?: boolean;
  present?: boolean;
  tab: WorkbenchTab;
  sessionId?: string;
  launcher?: ReactNode;
  onTabChange: (patch: Partial<WorkbenchTab>) => void;
}) {
  const native = isDesktopHost();
  const urlInput = useRef<HTMLInputElement>(null);
  const toolbar = useRef<HTMLDivElement>(null);
  const toolsTrigger = useRef<HTMLButtonElement>(null);
  const addressDirty = useRef(false);
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
  const [nativePageInstance, setNativePageInstance] = useState<string | null>(null);
  const browserIdentity = native ? nativePageInstance : tab.id;
  const [browserError, setBrowserError] = useState<string | null>(null);

  useEffect(() => {
    if (!active || !connected) setNativePageInstance(null);
  }, [active, connected]);

  useEffect(() => {
    if (active && !tabUrl && tab.id !== "launcher") urlInput.current?.focus();
  }, [active, tab.id, tabUrl]);

  useEffect(() => {
    if (!addressDirty.current) setUrl(tabUrl);
    setCurrentUrl(tabUrl);
    setTitle(tab.title === "New tab" ? "" : tab.title);
    setSnapshot("");
    setScreenshot(null);
    setBrowserError(null);
    if (native || !tabUrl || !active || !client || !connected) {
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
  }, [active, client, connected, native, tab.id, tabUrl]);

  useEffect(() => {
    if (!client) return;
    const isCurrentEvent = (data: { tabId?: string; url?: string }) => native
      ? Boolean(browserIdentity) && data.tabId === browserIdentity
      : acceptsBrowserEvent(data, {
      tabId: tab.id,
      url: tab.payload?.url ?? "",
      active,
    });
    const onState = (data: { open: boolean; url?: string; title?: string; tabId?: string }) => {
      if (!isCurrentEvent(data)) return;
      setBusy(false);
      setCurrentUrl(data.url ?? "");
      setTitle(data.title ?? "");
      if (!addressDirty.current) setUrl(data.url ?? "");
      onTabChange({ title: data.title || tab.title, payload: { ...tab.payload, url: data.url } });
    };
    const onSnapshot = (data: { snapshot: string; url: string; title: string; tabId?: string }) => {
      if (!isCurrentEvent(data)) return;
      setBusy(false);
      setSnapshot(formatSnapshot(data.snapshot));
      setCurrentUrl(data.url);
      setTitle(data.title);
      if (!addressDirty.current) setUrl(data.url);
      onTabChange({ title: data.title || tab.title, payload: { ...tab.payload, url: data.url } });
    };
    const onScreenshot = (data: { data: string; url: string; tabId?: string }) => {
      if (!isCurrentEvent(data)) return;
      setBusy(false);
      setScreenshot(`data:image/png;base64,${data.data}`);
      setCurrentUrl(data.url);
      if (!addressDirty.current) setUrl(data.url);
      onTabChange({ payload: { ...tab.payload, url: data.url } });
    };
    const onError = (data: { message: string }) => {
      if (!isBrowserError(data.message)) return;
      setBusy(false);
      setBrowserError(data.message);
    };
    const onBrowserError = (data: { message: string; tabId?: string }) => {
      if (native ? !browserIdentity || data.tabId !== browserIdentity : data.tabId ? data.tabId !== tab.id : !active) return;
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
  }, [active, browserIdentity, client, native, onTabChange, tab]);

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
    addressDirty.current = false;
    const value = url.trim();
    const destination = /^(https?:\/\/)/i.test(value) ? value : value.includes(".") || value.startsWith("localhost") ? `http${value.startsWith("localhost") || value.startsWith("127.") ? "" : "s"}://${value}` : `https://www.google.com/search?q=${encodeURIComponent(value)}`;
    onTabChange({ title: tabTitleForUrl(destination), payload: { ...tab.payload, url: destination } });
  };

  const close = () => {
    setToolsOpen(false);
    if (native) { setNativePageInstance(null); onTabChange({ title: "New tab", payload: { url: "" } }); }
    else if (client && connected) run(() => client.browserClose(tab.id));
  };

  const canUseBrowser = Boolean(client) && connected && (!native || Boolean(sessionId));
  const canInspectBrowser = Boolean(client) && connected && Boolean(browserIdentity);

  const closeTools = () => {
    setToolsOpen(false);
    setInspectorOpen(false);
  };

  useOutsideClick(toolbar, toolsOpen, closeTools);
  useEffect(() => {
    if (!toolsOpen) return;
    const dismiss = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      setToolsOpen(false);
      setInspectorOpen(false);
      toolsTrigger.current?.focus();
    };
    window.addEventListener("keydown", dismiss);
    return () => window.removeEventListener("keydown", dismiss);
  }, [toolsOpen]);

  const openTools = () => {
    setToolsOpen((value) => !value);
  };

  return (
    <div className="flex h-full min-h-0 flex-col bg-background">
      <div ref={toolbar} className="relative shrink-0 border-b border-border/70 px-3 py-2">
        <form onSubmit={open} className="flex min-w-0 items-center gap-2">
          <div className="mx-auto flex h-8 min-w-0 max-w-3xl flex-1 items-center gap-2 rounded-lg border border-border/80 bg-muted/60 pl-3 pr-1 transition-colors hover:bg-muted focus-within:border-ring/70 focus-within:ring-2 focus-within:ring-ring/15">
            {tabUrl && <Globe className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />}
            <input
              ref={urlInput}
              value={url}
              onChange={(event) => { addressDirty.current = true; setUrl(event.target.value); }}
              className="min-w-0 flex-1 bg-transparent text-center text-xs outline-none placeholder:text-muted-foreground focus:text-left"
              placeholder={currentUrl || "Search or enter URL"}
              aria-label="Browser URL"
              spellCheck={false}
            />
            {busy && <Loader2 className="h-3.5 w-3.5 shrink-0 animate-spin text-muted-foreground" />}
            <Button type="submit" size="icon-sm" variant="ghost" className="shrink-0 rounded-full" disabled={!canUseBrowser || busy} aria-label="Open URL">
              <ArrowRight className="h-3.5 w-3.5" />
            </Button>

          </div>
            <button
              ref={toolsTrigger}
              type="button"
              className={cn("motion-pressable flex h-7 w-7 shrink-0 items-center justify-center rounded-md text-muted-foreground hover:bg-accent hover:text-foreground", toolsOpen && "bg-accent text-foreground")}
              onClick={openTools}
              aria-label="Open browser tools"
              aria-expanded={toolsOpen}
              title="Browser tools"
            >
              <MoreHorizontal className="h-3.5 w-3.5" />
            </button>
        </form>

        {browserError && (
          <div role="alert" className="mt-2 rounded-md border border-destructive/30 bg-destructive/10 px-2.5 py-2 text-[11px] text-destructive">
            {browserError}
          </div>
        )}

        {toolsOpen && (
          <div data-mew-overlay className="motion-enter absolute right-3 top-full z-50 mt-2 max-h-[min(32rem,calc(100dvh-10rem))] w-[min(24rem,calc(100%_-_1.5rem))] overflow-y-auto overscroll-contain rounded-xl border border-border bg-popover p-2 text-popover-foreground shadow-lg">
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
              <Button size="sm" variant="ghost" className="justify-start gap-2 text-xs" disabled={!canInspectBrowser} onClick={() => client && connected && browserIdentity && run(() => client.browserSnapshot(browserIdentity))}>
                <RefreshCw className="h-3.5 w-3.5" />
                Text snapshot
              </Button>
              <Button size="sm" variant="ghost" className="justify-start gap-2 text-xs" disabled={!canInspectBrowser} onClick={() => client && connected && browserIdentity && run(() => client.browserScreenshot(false, browserIdentity))}>
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
                  <Button size="sm" variant="secondary" className="h-8 px-2.5 text-[11px]" disabled={!canInspectBrowser || !selector.trim()} onClick={() => client && connected && browserIdentity && run(() => client.browserFill(selector.trim(), fillText, browserIdentity))}>Fill</Button>
                  <Button size="sm" variant="secondary" className="h-8 px-2.5 text-[11px]" disabled={!canInspectBrowser || !selector.trim()} onClick={() => client && connected && browserIdentity && run(() => client.browserClick(selector.trim(), browserIdentity))}>Click</Button>
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

      {!tabUrl ? <div className="min-h-0 flex-1 overflow-auto">{launcher}</div> : native && client && sessionId ? (
        <NativeBrowserPage client={client} connected={connected} sessionId={sessionId} tabId={tab.id} url={tabUrl} active={active} present={present} obscured={toolsOpen} onError={setBrowserError} onChange={(event) => {
          if (event.kind === "ready") setNativePageInstance(event.pageInstance);
          if (event.kind === "closed") setNativePageInstance(null);
          setBusy(false); setCurrentUrl(event.url); setTitle(event.title);
          if (!addressDirty.current) setUrl(event.url);
          if (event.kind === "changed") onTabChange({ title: event.title || tab.title, payload: { ...tab.payload, url: event.url } });
        }} />
      ) : <div className="min-h-0 flex-1 overflow-auto bg-muted/20">
        <div className="flex h-full min-h-32 flex-col items-center justify-center gap-2 px-6 text-center text-xs text-muted-foreground">
          <Globe className="h-5 w-5 opacity-50" />
          <p>Browser tools can inspect this page. Use the desktop app to interact with a page beside your chat.</p>
          <a href={tabUrl} target="_blank" rel="noreferrer" className="text-primary underline underline-offset-4">Open page in your browser</a>
        </div>
      </div>}

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
