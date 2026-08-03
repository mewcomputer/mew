type WebLocation = Pick<Location, "protocol" | "host">;

let websocketUrl: string | null = null;
let initialization: Promise<void> | null = null;

export function browserWebSocketUrl(location: WebLocation): string {
  const protocol = location.protocol === "https:" ? "wss" : "ws";
  return `${protocol}://${location.host}/ws`;
}

/** Resolve the browser bridge before mounting the React tree. */
export function initializeHost(): Promise<void> {
  if (websocketUrl) return Promise.resolve();
  if (initialization) return initialization;

  if (typeof window === "undefined") return Promise.resolve();

  websocketUrl = browserWebSocketUrl(window.location);
  initialization = Promise.resolve();
  return initialization;
}

/** Clear cached bridge state so a failed bootstrap can be retried. */
export function resetHost(): void {
  websocketUrl = null;
  initialization = null;
}

export function getWebSocketUrl(): string {
  if (websocketUrl) return websocketUrl;

  if (typeof window !== "undefined") {
    websocketUrl = browserWebSocketUrl(window.location);
    return websocketUrl;
  }

  throw new Error("mew web host is not initialized");
}
