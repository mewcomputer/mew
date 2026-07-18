import { FormEvent, useEffect, useLayoutEffect, useRef, useState } from "react";
import { Camera, Globe, Loader2, Play, RefreshCw, X } from "lucide-react";
import type { MewClient } from "@mew/web-client";
import type { WorkbenchTab } from "../lib/workbench-tabs";
import { acceptsBrowserEvent, acceptsNativeBrowserEvent } from "../lib/browser-lifecycle";
import { Button } from "./ui/button";
import {
  cefBrowserAvailable,
  isDesktopHost,
  listenCefBrowserEvents,
  navigateCefBrowser,
  setCefBrowserRect,
  setCefBrowserVisible,
} from "../lib/host";
import { cn } from "../lib/utils";

export function BrowserPanel({
  client,
  active = true,
  tab,
  onTabChange,
}: {
  client: MewClient | null;
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
  const [busy, setBusy] = useState(false);
  const [browserError, setBrowserError] = useState<string | null>(null);
  const [nativeAvailable, setNativeAvailable] = useState(false);
  const [nativeVisible, setNativeVisible] = useState(false);
  const nativeVisibleRef = useRef(nativeVisible);
  const busyRef = useRef(busy);
  const viewportRef = useRef<HTMLDivElement>(null);
  nativeVisibleRef.current = nativeVisible;
  busyRef.current = busy;

  useEffect(() => {
    if (!isDesktopHost()) return;
    let mounted = true;
    void cefBrowserAvailable()
      .then((available) => {
        if (mounted) {
          setNativeAvailable(available);
          setNativeVisible(available && Boolean(tabUrl));
        }
      })
      .catch(() => {
        if (mounted) setNativeAvailable(false);
      });
    return () => {
      mounted = false;
    };
  }, []);

  useEffect(() => {
    setUrl(tabUrl);
    setCurrentUrl(tabUrl);
    setTitle(tab.title === "New tab" ? "" : tab.title);
    setSnapshot("");
    setScreenshot(null);
    setBrowserError(null);
    setBusy(false);
    setNativeVisible(nativeAvailable && Boolean(tabUrl) && active);

    if (!tabUrl || !active) return;
    setBusy(true);
    if (client) {
      client.browserOpen(tabUrl, tab.id);
    } else {
      void navigateCefBrowser(tabUrl, tab.id).finally(() => setBusy(false));
    }
  }, [active, nativeAvailable, tab.id]);

  useLayoutEffect(() => {
    if (!nativeAvailable || !viewportRef.current) return;

    const updateBounds = () => {
      const viewport = viewportRef.current;
      if (!viewport) return;
      const bounds = viewport.getBoundingClientRect();
      const viewportHeight = window.visualViewport?.height ?? window.innerHeight;
      void setCefBrowserRect({
        owner: tab.id,
        x: bounds.left,
        y: viewportHeight - bounds.bottom,
        width: bounds.width,
        height: bounds.height,
        visible: active && nativeVisibleRef.current,
      }).catch(() => undefined);
    };

    updateBounds();
    const observer = typeof ResizeObserver === "undefined"
      ? null
      : new ResizeObserver(updateBounds);
    observer?.observe(viewportRef.current);
    window.addEventListener("resize", updateBounds);
    window.visualViewport?.addEventListener("resize", updateBounds);
    return () => {
      observer?.disconnect();
      window.removeEventListener("resize", updateBounds);
      window.visualViewport?.removeEventListener("resize", updateBounds);
      void setCefBrowserVisible(false, tab.id).catch(() => undefined);
    };
  }, [active, nativeAvailable, tab.id]);

  useLayoutEffect(() => {
    if (!nativeAvailable) return;
    void setCefBrowserVisible(active && nativeVisible, tab.id).catch(() => undefined);
  }, [active, nativeAvailable, nativeVisible, tab.id]);

  useEffect(() => {
    if (!nativeAvailable) return;
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void listenCefBrowserEvents((event) => {
      if (disposed) return;
      if (!acceptsNativeBrowserEvent(event, {
        tabId: tab.id,
        url: tab.payload?.url ?? "",
        active,
        visible: nativeVisibleRef.current,
        loading: busyRef.current,
      })) return;
      if (event.kind === "address_changed") {
        setBusy(false);
        setCurrentUrl(event.url);
        setUrl(event.url);
        onTabChange({ payload: { ...tab.payload, url: event.url } });
      } else {
        setTitle(event.title);
        onTabChange({ title: event.title || tab.title });
      }
    }).then((cleanup) => {
      if (disposed) cleanup();
      else unlisten = cleanup;
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [active, nativeAvailable, onTabChange, tab]);

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
    if ((!client && !nativeAvailable) || !url.trim()) return;
    setBrowserError(null);
    if (nativeAvailable) {
      setNativeVisible(true);
    }
    onTabChange({ title: tabTitleForUrl(url.trim()), payload: { ...tab.payload, url: url.trim() } });
    if (client) {
      run(() => client.browserOpen(url.trim(), tab.id));
    } else {
      setBusy(true);
      void navigateCefBrowser(url.trim(), tab.id).finally(() => setBusy(false));
    }
  };

  const close = () => {
    if (nativeAvailable) {
      setNativeVisible(false);
      return;
    }
    if (client) run(() => client.browserClose(tab.id));
  };

  return (
    <div className="flex h-full min-h-0 flex-col gap-3">
      <form onSubmit={open} className="flex items-center gap-1.5">
        <div className="flex min-w-0 flex-1 items-center gap-2 rounded-md border border-border bg-background px-2.5 py-1.5">
          <Globe className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
          <input
            value={url}
            onChange={(event) => setUrl(event.target.value)}
            className="min-w-0 flex-1 bg-transparent text-xs outline-none placeholder:text-muted-foreground"
            placeholder="https://..."
            aria-label="Browser URL"
            spellCheck={false}
          />
        </div>
        <Button type="submit" size="icon" variant="secondary" className="h-8 w-8" disabled={(!client && !nativeAvailable) || busy} aria-label="Open URL">
          {busy ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Play className="h-3.5 w-3.5" />}
        </Button>
      </form>

      {browserError && (
        <div role="alert" className="rounded-md border border-destructive/30 bg-destructive/10 px-2.5 py-2 text-[11px] text-destructive">
          {browserError}
        </div>
      )}

      <div className="flex min-w-0 items-center gap-2 text-[10px] text-muted-foreground">
        <span className="min-w-0 flex-1 truncate" title={currentUrl}>{title || currentUrl || "No page open"}</span>
        <button type="button" className="motion-pressable rounded p-1 hover:bg-accent hover:text-foreground" onClick={() => client && run(() => client.browserSnapshot(tab.id))} aria-label="Refresh page snapshot" title="Refresh text snapshot">
          <RefreshCw className="h-3 w-3" />
        </button>
        <button type="button" className="motion-pressable rounded p-1 hover:bg-accent hover:text-foreground" onClick={() => client && run(() => client.browserScreenshot(false, tab.id))} aria-label="Take browser screenshot" title="Take screenshot">
          <Camera className="h-3 w-3" />
        </button>
        <button type="button" className="motion-pressable rounded p-1 hover:bg-accent hover:text-foreground" onClick={close} aria-label="Hide browser page" title="Hide browser page">
          <X className="h-3 w-3" />
        </button>
      </div>

      {screenshot && (
        <div className="overflow-hidden rounded-lg border border-border bg-muted/20">
          <img src={screenshot} alt={title ? `Screenshot of ${title}` : "Browser screenshot"} className="block h-auto max-h-56 w-full object-contain" />
        </div>
      )}

      {nativeAvailable ? (
        <>
          <div ref={viewportRef} className="relative min-h-32 flex-1 overflow-hidden rounded-lg border border-border bg-muted/20">
            {!nativeVisible && (
              <div className="flex h-full min-h-32 flex-col items-center justify-center gap-2 px-6 text-center text-[11px] text-muted-foreground">
                <Globe className="h-5 w-5 opacity-50" />
                <p>the browser surface is closed. open a URL to bring it back.</p>
              </div>
            )}
          </div>
          {snapshot && (
            <details className="max-h-44 overflow-auto rounded-lg border border-border bg-muted/20 text-[10px]">
              <summary className="cursor-pointer px-3 py-2 font-medium text-muted-foreground">text snapshot</summary>
              <pre className="whitespace-pre-wrap border-t border-border p-3 font-mono leading-relaxed text-foreground/85">{snapshot}</pre>
            </details>
          )}
        </>
      ) : (
        <div className="min-h-0 flex-1 overflow-auto rounded-lg border border-border bg-muted/20">
          {snapshot ? (
            <pre className="whitespace-pre-wrap p-3 font-mono text-[10px] leading-relaxed text-foreground/85">{snapshot}</pre>
          ) : (
            <div className="flex h-full min-h-32 flex-col items-center justify-center gap-2 px-6 text-center text-[11px] text-muted-foreground">
              <Globe className="h-5 w-5 opacity-50" />
              <p>open a page, then inspect its text structure or capture a screenshot.</p>
            </div>
          )}
        </div>
      )}

      <div className="grid gap-1.5 border-t border-border pt-3">
        <input
          value={selector}
          onChange={(event) => setSelector(event.target.value)}
          className="h-8 rounded-md border border-border bg-background px-2.5 text-[11px] outline-none focus-visible:ring-2 focus-visible:ring-ring"
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
          <Button size="sm" variant="secondary" className={cn("h-8 px-2.5 text-[11px]")} disabled={!client || !selector.trim()} onClick={() => client && run(() => client.browserFill(selector.trim(), fillText, tab.id))}>
            Fill
          </Button>
          <Button size="sm" variant="secondary" className="h-8 px-2.5 text-[11px]" disabled={!client || !selector.trim()} onClick={() => client && run(() => client.browserClick(selector.trim(), tab.id))}>
            Click
          </Button>
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
