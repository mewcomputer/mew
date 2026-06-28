import type { MewClient } from "@mew/web-client";

/** Module-level singleton ref to the active MewClient instance.
 *  Set by App.tsx on connect; used by components that need to send
 *  messages but don't receive the client as a prop (e.g. AskUserCard). */
let clientRef: MewClient | null = null;

export function setClient(client: MewClient | null): void {
  clientRef = client;
}

export function getClient(): MewClient | null {
  return clientRef;
}
