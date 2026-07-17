import { FormEvent, useEffect, useLayoutEffect, useRef, useState } from "react";
import { Camera, Globe, Loader2, Play, RefreshCw, X } from "lucide-react";
import type { MewClient } from "@mew/web-client";
import { Button } from "./ui/button";
import {
  cefBrowserAvailable,
  isDesktopHost,
  navigateCefBrowser,
  setCefBrowserRect,
  setCefBrowserVisible,
} from "../lib/host";
import { cn } from "../lib/utils";

export function BrowserPanel({ client }: { client: MewClient | null }) {
  const [url, setUrl] = useState("https://example.com");
  const [currentUrl, setCurrentUrl] = useState("");
  const [title, setTitle] = useState("");
  const [snapshot, setSnapshot] = useState("");
  const [screenshot, setScreenshot] = useState<string | null>(null);
  const [selector, setSelector] = useState("");
  const [fillText, setFillText] = useState("");
  const [busy, setBusy] = useState(false);
  const [nativeAvailable, setNativeAvailable] = useState(false);
  const [nativeVisible, setNativeVisible] = useState(false);
  const nativeVisibleRef = useRef(nativeVisible);
  const viewportRef = useRef<HTMLDivElement>(null);
  nativeVisibleRef.current = nativeVisible;

  useEffect(() => {
    if (!isDesktopHost()) return;
    let mounted = true;
    void cefBrowserAvailable()
      .then((available) => {
        if (mounted) {
          setNativeAvailable(available);
          setNativeVisible(available);
        }
      })
      .catch(() => {
        if (mounted) setNativeAvailable(false);
      });
    return () => {
      mounted = false;
    };
  }, []);

  useLayoutEffect(() => {
    if (!nativeAvailable || !viewportRef.current) return;

    const updateBounds = () => {
      const viewport = viewportRef.current;
      if (!viewport) return;
      const bounds = viewport.getBoundingClientRect();
      const viewportHeight = window.visualViewport?.height ?? window.innerHeight;
      void setCefBrowserRect({
        x: bounds.left,
        y: viewportHeight - bounds.bottom,
        width: bounds.width,
        height: bounds.height,
        visible: nativeVisibleRef.current,
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
      void setCefBrowserVisible(false).catch(() => undefined);
    };
  }, [nativeAvailable]);

  useLayoutEffect(() => {
    if (!nativeAvailable) return;
    void setCefBrowserVisible(nativeVisible).catch(() => undefined);
  }, [nativeAvailable, nativeVisible]);

  useEffect(() => {
    if (!client) return;
    const onState = (data: { open: boolean; url?: string; title?: string }) => {
      setBusy(false);
      setCurrentUrl(data.url ?? "");
      setTitle(data.title ?? "");
    };
    const onSnapshot = (data: { snapshot: string; url: string; title: string }) => {
      setBusy(false);
      setSnapshot(formatSnapshot(data.snapshot));
      setCurrentUrl(data.url);
      setTitle(data.title);
    };
    const onScreenshot = (data: { data: string; url: string }) => {
      setBusy(false);
      setScreenshot(`data:image/png;base64,${data.data}`);
      setCurrentUrl(data.url);
    };
    client.on("browser-state", onState);
    client.on("browser-snapshot", onSnapshot);
    client.on("browser-screenshot", onScreenshot);
    return () => {
      client.off("browser-state", onState);
      client.off("browser-snapshot", onSnapshot);
      client.off("browser-screenshot", onScreenshot);
    };
  }, [client]);

  const run = (action: () => void) => {
    setBusy(true);
    action();
  };

  const open = (event: FormEvent) => {
    event.preventDefault();
    if ((!client && !nativeAvailable) || !url.trim()) return;
    if (nativeAvailable) {
      setNativeVisible(true);
    }
    if (client) {
      run(() => client.browserOpen(url.trim()));
    } else {
      setBusy(true);
      void navigateCefBrowser(url.trim()).finally(() => setBusy(false));
    }
  };

  const close = () => {
    if (nativeAvailable) {
      setNativeVisible(false);
      return;
    }
    if (client) run(() => client.browserClose());
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

      <div className="flex min-w-0 items-center gap-2 text-[10px] text-muted-foreground">
        <span className="min-w-0 flex-1 truncate" title={currentUrl}>{title || currentUrl || "No page open"}</span>
        <button type="button" className="rounded p-1 hover:bg-accent hover:text-foreground" onClick={() => client && run(() => client.browserSnapshot())} aria-label="Refresh page snapshot" title="Refresh text snapshot">
          <RefreshCw className="h-3 w-3" />
        </button>
        <button type="button" className="rounded p-1 hover:bg-accent hover:text-foreground" onClick={() => client && run(() => client.browserScreenshot(false))} aria-label="Take browser screenshot" title="Take screenshot">
          <Camera className="h-3 w-3" />
        </button>
        <button type="button" className="rounded p-1 hover:bg-accent hover:text-foreground" onClick={close} aria-label="Close browser" title="Close browser">
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
          <Button size="sm" variant="secondary" className={cn("h-8 px-2.5 text-[11px]")} disabled={!client || !selector.trim()} onClick={() => client && run(() => client.browserFill(selector.trim(), fillText))}>
            Fill
          </Button>
          <Button size="sm" variant="secondary" className="h-8 px-2.5 text-[11px]" disabled={!client || !selector.trim()} onClick={() => client && run(() => client.browserClick(selector.trim()))}>
            Click
          </Button>
        </div>
      </div>
    </div>
  );
}

function formatSnapshot(value: string): string {
  try {
    return JSON.stringify(JSON.parse(value), null, 2);
  } catch {
    return value;
  }
}
