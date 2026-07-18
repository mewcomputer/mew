import { invoke } from "@tauri-apps/api/core";

type WebLocation = Pick<Location, "protocol" | "host">;

export type NativeBrowserRect = {
  owner: string;
  x: number;
  y: number;
  width: number;
  height: number;
  visible: boolean;
};

export type NativeBrowserEvent =
  | { kind: "address_changed"; owner?: string; url: string }
  | { kind: "title_changed"; owner?: string; title: string; url: string };

let websocketUrl: string | null = null;
let initialization: Promise<void> | null = null;

export function isDesktopHost(): boolean {
  return (
    typeof window !== "undefined" &&
    Object.prototype.hasOwnProperty.call(window, "__TAURI_INTERNALS__")
  );
}

export function browserWebSocketUrl(location: WebLocation): string {
  const protocol = location.protocol === "https:" ? "wss" : "ws";
  return `${protocol}://${location.host}/ws`;
}

/** Resolve native host state before mounting the React tree. */
export function initializeHost(): Promise<void> {
  if (websocketUrl) return Promise.resolve();
  if (initialization) return initialization;

  if (!isDesktopHost()) {
    websocketUrl = browserWebSocketUrl(window.location);
    return Promise.resolve();
  }

  initialization = invoke<string>("daemon_ws_url")
    .then((url) => {
      websocketUrl = url;
    })
    .catch((error: unknown) => {
      initialization = null;
      throw error;
    });
  return initialization;
}

/** Clear cached native host state so a failed bootstrap can be retried. */
export function resetHost(): void {
  websocketUrl = null;
  initialization = null;
}

export function getWebSocketUrl(): string {
  if (websocketUrl) return websocketUrl;

  if (!isDesktopHost()) {
    websocketUrl = browserWebSocketUrl(window.location);
    return websocketUrl;
  }

  throw new Error("mew desktop host is not initialized");
}

export function cefBrowserAvailable(): Promise<boolean> {
  if (!isDesktopHost()) return Promise.resolve(false);
  return invoke<boolean>("cef_browser_available");
}

export function setCefBrowserRect(rect: NativeBrowserRect): Promise<void> {
  return invoke<void>("cef_browser_set_rect", { rect });
}

export function setCefBrowserVisible(visible: boolean, owner: string): Promise<void> {
  return invoke<void>("cef_browser_set_visible", { payload: { visible, owner } });
}

export function navigateCefBrowser(url: string, owner: string): Promise<void> {
  return invoke<void>("cef_browser_navigate", { url, owner });
}

export function closeCefBrowser(owner: string): Promise<void> {
  return invoke<void>("cef_browser_close", { owner });
}

export function listenCefBrowserEvents(
  handler: (event: NativeBrowserEvent) => void,
): Promise<() => void> {
  if (!isDesktopHost()) return Promise.resolve(() => undefined);
  return import("@tauri-apps/api/event").then(({ listen }) =>
    listen<NativeBrowserEvent>("cef-browser-event", (event) => handler(event.payload)),
  );
}
