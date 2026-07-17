import { MewClient } from "@mew/web-client";
import { bridgeClientToStore } from "../stores/session";
import { setClient } from "./client-ref";
import { getWebSocketUrl } from "./host";

let clientInstance: MewClient | null = null;

export function getClient(): MewClient {
  if (!clientInstance) {
    clientInstance = new MewClient(getWebSocketUrl(), { debug: false });
    setClient(clientInstance);
    bridgeClientToStore(clientInstance);
  }
  return clientInstance;
}

export const SESSION_ID_KEY = "mew.sessionId";
