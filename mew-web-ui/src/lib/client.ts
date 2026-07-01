import { MewClient } from "@mew/web-client";
import { bridgeClientToStore } from "../stores/session";
import { setClient } from "./client-ref";

const WS_URL = (() => {
  const proto = location.protocol === "https:" ? "wss" : "ws";
  return `${proto}://${location.host}/ws`;
})();

let clientInstance: MewClient | null = null;

export function getClient(): MewClient {
  if (!clientInstance) {
    clientInstance = new MewClient(WS_URL, { debug: false });
    setClient(clientInstance);
    bridgeClientToStore(clientInstance);
  }
  return clientInstance;
}

export const SESSION_ID_KEY = "mew.sessionId";
