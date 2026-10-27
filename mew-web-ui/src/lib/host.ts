type WebLocation = Pick<Location, "protocol" | "host">;

export interface HostTheme {
  id: string;
  mode: "light" | "dark";
  css: string;
}

export interface HostBootstrap {
  daemonUrl: string;
  themeMode: "system" | "light" | "dark";
  lightTheme: string;
  darkTheme: string;
  themes: HostTheme[];
}

export interface NativeBrowserEvent {
  sessionId: string;
  tabId: string;
  pageInstance: string;
  url: string;
  title: string;
  kind: "ready" | "changed" | "closed" | "error";
  cdpEndpoint?: string;
  targetId?: string;
  message?: string;
}

export type HostRequest =
  | { type: "bootstrap" }
  | { type: "set_theme"; themeMode: "system" | "light" | "dark"; lightTheme: string; darkTheme: string }
  | { type: "browser_update"; sessionId: string; tabId: string; url: string; rect: { x: number; y: number; width: number; height: number }; visible: boolean; interactive?: boolean }
  | { type: "browser_close"; sessionId: string; tabId: string };

declare global {
  interface Window {
    mewQuery?: (query: { request: string; onSuccess: (response: string) => void; onFailure: (code: number, message: string) => void }) => void;
  }
}

let websocketUrl: string | null = null;
let bootstrap: HostBootstrap | null = null;
let initialization: Promise<void> | null = null;

export function browserWebSocketUrl(location: WebLocation): string {
  const protocol = location.protocol === "https:" ? "wss" : "ws";
  return `${protocol}://${location.host}/ws`;
}

export function isDesktopHost(): boolean {
  return typeof window !== "undefined" && typeof window.mewQuery === "function";
}

export function hostRequest<T = unknown>(request: HostRequest): Promise<T> {
  return new Promise((resolve, reject) => {
    if (!window.mewQuery) return reject(new Error("The native mew host is unavailable."));
    window.mewQuery({
      request: JSON.stringify(request),
      onSuccess: (response) => {
        try { resolve(JSON.parse(response) as T); }
        catch { reject(new Error("The native host returned an invalid response.")); }
      },
      onFailure: (_code, message) => reject(new Error(message)),
    });
  });
}

/** Resolve the host before mounting the React tree or opening its daemon connection. */
export function initializeHost(): Promise<void> {
  if (websocketUrl) return Promise.resolve();
  if (initialization) return initialization;
  if (typeof window === "undefined") return Promise.resolve();
  initialization = isDesktopHost()
    ? hostRequest<HostBootstrap>({ type: "bootstrap" }).then((result) => {
        if (!/^wss?:\/\//.test(result.daemonUrl) || !Array.isArray(result.themes)) {
          throw new Error("The native host returned invalid startup settings.");
        }
        bootstrap = result;
        websocketUrl = result.daemonUrl;
      })
    : Promise.resolve().then(() => { websocketUrl = browserWebSocketUrl(window.location); });
  return initialization;
}

export function getHostBootstrap(): HostBootstrap | null { return bootstrap; }

/** Clear cached bridge state so a failed bootstrap can be retried. */
export function resetHost(): void {
  websocketUrl = null;
  bootstrap = null;
  initialization = null;
}

export function getWebSocketUrl(): string {
  if (websocketUrl) return websocketUrl;
  if (typeof window !== "undefined" && !isDesktopHost()) {
    websocketUrl = browserWebSocketUrl(window.location);
    return websocketUrl;
  }
  throw new Error("mew host is not initialized");
}
