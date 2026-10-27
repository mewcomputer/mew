import { useEffect, useRef, useState } from "react";
import type { MewClient } from "@mew/web-client";
import { hostRequest, type NativeBrowserEvent } from "../lib/host";

// The daemon has one browser target per connection. Drain its acknowledgement
// before selecting or destroying another page, including across mounted tabs.
let registrationQueue: Promise<void> = Promise.resolve();
const mountedPages = new Map<string, symbol>();
const readyPages = new Map<string, NativeBrowserEvent>();

function queueRegistration(action: () => Promise<void>, onError: (message: string) => void) {
  registrationQueue = registrationQueue.then(action).catch((error: unknown) => {
    onError(error instanceof Error ? error.message : String(error));
  });
}

export function NativeBrowserPage({ client, connected, sessionId, tabId, url, active, obscured = false, onChange, onError }: {
  client: MewClient;
  connected: boolean;
  sessionId: string;
  tabId: string;
  url: string;
  active: boolean;
  obscured?: boolean;
  onChange: (event: NativeBrowserEvent) => void;
  onError: (message: string) => void;
}) {
  const pageKey = `${sessionId}/${tabId}`;
  const container = useRef<HTMLDivElement>(null);
  const currentPage = useRef<NativeBrowserEvent | null>(readyPages.get(pageKey) ?? null);
  const callbacks = useRef({ onChange, onError });
  callbacks.current = { onChange, onError };
  const [page, setPage] = useState<NativeBrowserEvent | null>(() => readyPages.get(pageKey) ?? null);

  useEffect(() => {
    const key = `${sessionId}/${tabId}`;
    const identity = Symbol(key);
    mountedPages.set(key, identity);
    const receive = (event: Event) => {
      const detail = (event as CustomEvent<NativeBrowserEvent>).detail;
      if (detail.sessionId !== sessionId || detail.tabId !== tabId) return;
      if (detail.kind !== "ready" && currentPage.current && detail.pageInstance !== currentPage.current.pageInstance) return;
      if (detail.kind === "error") { callbacks.current.onError(detail.message ?? "The browser page could not be opened."); return; }
      if (detail.kind === "ready") { readyPages.set(key, detail); currentPage.current = detail; setPage(detail); }
      if (detail.kind === "closed") { readyPages.delete(key); currentPage.current = null; setPage(null); }
      if (detail.kind !== "ready") callbacks.current.onChange(detail);
    };
    window.addEventListener("mew:browser-event", receive);
    return () => {
      window.removeEventListener("mew:browser-event", receive);
      queueMicrotask(() => queueRegistration(async () => {
        if (mountedPages.get(key) !== identity) return;
        mountedPages.delete(key);
        readyPages.delete(key);
        await hostRequest({ type: "browser_close", sessionId, tabId });
      }, (message) => callbacks.current.onError(message)));
    };
  }, [sessionId, tabId]);

  useEffect(() => {
    if (!active || !connected || !page?.cdpEndpoint || !page.targetId) return;
    let selected = true;
    let registered = false;
    queueRegistration(async () => {
      if (!selected) return;
      await client.registerBrowserPage(sessionId, page.pageInstance, page.cdpEndpoint!, page.targetId!);
      registered = true;
      if (selected) callbacks.current.onChange(page);
    }, (message) => callbacks.current.onError(message));
    return () => {
      selected = false;
      queueRegistration(async () => {
        if (registered && client.isConnected()) await client.unregisterBrowserPage(sessionId, page.pageInstance);
      }, (message) => callbacks.current.onError(message));
    };
  }, [active, client, connected, page, sessionId]);

  useEffect(() => {
    const element = container.current;
    if (!element) return;
    let lastUpdate = "";
    const update = () => {
      const bounds = element.getBoundingClientRect();
      const overlay = Array.from(document.querySelectorAll('[role="dialog"], [role="alertdialog"], [role="menu"], [role="listbox"], [cmdk-dialog], [data-mew-overlay]')).some((node) => {
        if (node.contains(element)) return false;
        const rect = node.getBoundingClientRect();
        if (rect.width <= 0 || rect.height <= 0) return false;
        return node.getAttribute("aria-modal") === "true"
          || (rect.left < bounds.right && rect.right > bounds.left && rect.top < bounds.bottom && rect.bottom > bounds.top);
      });
      const request = {
        type: "browser_update" as const, sessionId, tabId, url,
        rect: { x: bounds.x, y: bounds.y, width: bounds.width, height: bounds.height },
        visible: active && !obscured && !document.hidden && !overlay && bounds.width > 0 && bounds.height > 0,
      };
      const serialized = JSON.stringify(request);
      if (serialized === lastUpdate) return;
      lastUpdate = serialized;
      void hostRequest(request).catch((error: unknown) => callbacks.current.onError(error instanceof Error ? error.message : String(error)));
    };
    const resize = new ResizeObserver(update);
    resize.observe(element);
    const mutations = new MutationObserver(update);
    mutations.observe(document.body, { childList: true, subtree: true, attributes: true, attributeFilter: ["data-state", "role", "aria-modal"] });
    window.addEventListener("resize", update);
    document.addEventListener("visibilitychange", update);
    document.addEventListener("scroll", update, true);
    update();
    return () => {
      resize.disconnect(); mutations.disconnect();
      window.removeEventListener("resize", update);
      document.removeEventListener("visibilitychange", update);
      document.removeEventListener("scroll", update, true);
      void hostRequest({ type: "browser_update", sessionId, tabId, url, rect: { x: 0, y: 0, width: 0, height: 0 }, visible: false }).catch(() => {});
    };
  }, [active, obscured, sessionId, tabId, url]);

  return <div ref={container} className="h-full min-h-0 w-full flex-1 bg-background" aria-label="Browser page" />;
}
