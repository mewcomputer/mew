import { createElement, useEffect, useLayoutEffect, useRef, useState } from "react";
import type { MewClient } from "@mew/web-client";
import { hostRequest, isElectronHost, type NativeBrowserEvent } from "../lib/host";

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

export function NativeBrowserPage({ client, connected, sessionId, tabId, url, active, present = active, obscured = false, onChange, onError }: {
  client: MewClient;
  connected: boolean;
  sessionId: string;
  tabId: string;
  url: string;
  active: boolean;
  present?: boolean;
  obscured?: boolean;
  onChange: (event: NativeBrowserEvent) => void;
  onError: (message: string) => void;
}) {
  const pageKey = `${sessionId}/${tabId}`;
  const electron = isElectronHost();
  const container = useRef<HTMLDivElement>(null);
  const guest = useRef<(HTMLElement & { getWebContentsId: () => number }) | null>(null);
  const currentPage = useRef<NativeBrowserEvent | null>(electron ? null : readyPages.get(pageKey) ?? null);
  const callbacks = useRef({ onChange, onError });
  callbacks.current = { onChange, onError };
  const [page, setPage] = useState<NativeBrowserEvent | null>(() => electron ? null : readyPages.get(pageKey) ?? null);
  const [permit, setPermit] = useState<{ partition: string; pageInstance: string; url: string } | null>(null);
  const [sessionConfirmed, setSessionConfirmed] = useState(false);
  const issuedPermit = useRef(permit);

  useEffect(() => {
    const ready = (data: { session_id: string }) => {
      if (data.session_id === sessionId) setSessionConfirmed(client.hasConfirmedSession(sessionId));
    };
    const closed = () => setSessionConfirmed(client.hasConfirmedSession(sessionId));
    setSessionConfirmed(connected && client.hasConfirmedSession(sessionId));
    client.on("session-ready", ready);
    client.on("close", closed);
    return () => {
      client.off("session-ready", ready);
      client.off("close", closed);
    };
  }, [client, connected, sessionId]);

  useEffect(() => {
    const key = `${sessionId}/${tabId}`;
    const identity = Symbol(key);
    mountedPages.set(key, identity);
    const receive = (event: Event) => {
      const detail = (event as CustomEvent<NativeBrowserEvent>).detail;
      if (detail.sessionId !== sessionId || detail.tabId !== tabId) return;
      if (electron && detail.pageInstance !== issuedPermit.current?.pageInstance) return;
      if (detail.kind !== "ready" && currentPage.current && detail.pageInstance !== currentPage.current.pageInstance) return;
      if (detail.kind === "error") { callbacks.current.onError(detail.message ?? "The browser page could not be opened."); return; }
      if (detail.kind === "ready") {
        // Removing a DOM webview destroys its guest; only CEF pages survive a remount.
        if (!electron) readyPages.set(key, detail);
        currentPage.current = detail; setPage(detail);
      }
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
  }, [sessionId, tabId, electron]);

  useLayoutEffect(() => {
    const element = guest.current;
    if (!element || !permit) return;
    const bind = () => {
      try {
        void hostRequest({ type: "browser_bind", sessionId, tabId, pageInstance: permit.pageInstance, guestId: element.getWebContentsId() })
          .catch((error: unknown) => callbacks.current.onError(error instanceof Error ? error.message : String(error)));
      } catch (error) {
        callbacks.current.onError(error instanceof Error ? error.message : String(error));
      }
    };
    element.addEventListener("dom-ready", bind);
    return () => element.removeEventListener("dom-ready", bind);
  }, [permit, sessionId, tabId]);

  useEffect(() => {
    if (!active || !connected || !sessionConfirmed || !page?.cdpEndpoint || !page.targetId) return;
    let selected = true;
    let registered = false;
    queueRegistration(async () => {
      if (!selected || !client.hasConfirmedSession(sessionId)) return;
      if (page.focusUrl === undefined) await client.registerBrowserPage(sessionId, page.pageInstance, page.cdpEndpoint!, page.targetId!);
      else await client.registerBrowserPage(sessionId, page.pageInstance, page.cdpEndpoint!, page.targetId!, page.focusUrl);
      registered = true;
      if (selected) callbacks.current.onChange(page);
    }, (message) => callbacks.current.onError(message));
    return () => {
      selected = false;
      queueRegistration(async () => {
        if (registered && client.isConnected() && client.hasConfirmedSession(sessionId)) await client.unregisterBrowserPage(sessionId, page.pageInstance);
      }, (message) => callbacks.current.onError(message));
    };
  }, [active, client, connected, page, sessionId, sessionConfirmed]);

  useEffect(() => {
    const element = container.current;
    if (!element) return;
    const identity = mountedPages.get(pageKey);
    let lastUpdate = "";
    const update = () => {
      const bounds = element.getBoundingClientRect();
      const overlay = !electron && Array.from(document.querySelectorAll('[role="dialog"], [role="alertdialog"], [role="menu"], [role="listbox"], [cmdk-dialog], [data-mew-overlay]')).some((node) => {
        if (node.contains(element)) return false;
        const rect = node.getBoundingClientRect();
        if (rect.width <= 0 || rect.height <= 0) return false;
        return node.getAttribute("aria-modal") === "true"
          || (rect.left < bounds.right && rect.right > bounds.left && rect.top < bounds.bottom && rect.bottom > bounds.top);
      });
      const visible = present && (electron || (!obscured && !overlay)) && !document.hidden && bounds.width > 0 && bounds.height > 0;
      const request = {
        type: "browser_update" as const, sessionId, tabId, url,
        rect: { x: bounds.x, y: bounds.y, width: bounds.width, height: bounds.height },
        visible,
        interactive: active && visible,
      };
      const serialized = JSON.stringify(request);
      if (serialized === lastUpdate) return;
      lastUpdate = serialized;
      void hostRequest<{ partition?: string; pageInstance?: string }>(request).then((result) => {
        if (electron && result.partition && !issuedPermit.current && mountedPages.get(pageKey) === identity) {
          if (!result.pageInstance) throw new Error("The desktop host returned an invalid browser permit.");
          const next = { partition: result.partition, pageInstance: result.pageInstance, url: request.url };
          issuedPermit.current = next;
          setPermit(next);
        }
      }).catch((error: unknown) => callbacks.current.onError(error instanceof Error ? error.message : String(error)));
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
      void hostRequest({ type: "browser_update", sessionId, tabId, url, rect: { x: 0, y: 0, width: 0, height: 0 }, visible: false, interactive: false }).catch(() => {});
    };
  }, [active, present, obscured, sessionId, tabId, url, electron, pageKey]);

  return <div ref={container} className="h-full min-h-0 w-full flex-1 bg-background" aria-label="Browser page">
    {electron && permit && createElement("webview", { ref: guest, partition: permit.partition, src: permit.url, className: "h-full w-full", tabIndex: active ? 0 : -1, style: { pointerEvents: active ? "auto" : "none" } })}
  </div>;
}
