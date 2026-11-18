import { useEffect, useState, type RefObject } from "react";
import { useSessionStore } from "../stores/session";
import { getClient, SESSION_ID_KEY } from "./client";

/** Connection hook — manages WebSocket lifecycle. */
export function useMewConnection() {
  const connected = useSessionStore((s) => s.connectionState === "connected");
  const retryToken = useSessionStore((s) => s.connectionRetryToken);

  useEffect(() => {
    const client = getClient();
    let cancelled = false;
    let attempt = 0;
    let reconnectTimer: ReturnType<typeof setTimeout> | null = null;

    const scheduleRetry = () => {
      if (cancelled || reconnectTimer) return;
      const delay = Math.min(1000 * 2 ** Math.min(attempt, 5), 30000);
      useSessionStore.getState().setConnectionState("reconnecting");
      reconnectTimer = setTimeout(() => {
        reconnectTimer = null;
        void doConnect();
      }, delay);
    };

    const doConnect = async () => {
      if (cancelled) return;
      useSessionStore
        .getState()
        .setConnectionState(attempt === 0 ? "connecting" : "reconnecting");

      try {
        await client.connect();
        if (cancelled) return;
        const version = await client.ping();
        if (cancelled) return;
        attempt = 0;
        useSessionStore.getState().setDaemonVersion(version);
        useSessionStore.getState().setConnectionState("connected");
        useSessionStore.getState().setConnectionError(null);
      } catch (error) {
        if (cancelled) return;
        client.disconnect();
        if (!reconnectTimer) attempt += 1;
        useSessionStore.getState().setConnectionError(connectionErrorMessage(error));
        scheduleRetry();
      }
    };

    const handleClose = () => {
      if (cancelled || reconnectTimer) return;
      attempt += 1;
      useSessionStore.getState().setConnectionError("The daemon connection closed.");
      scheduleRetry();
    };

    client.on("close", handleClose);
    void doConnect();

    return () => {
      cancelled = true;
      if (reconnectTimer) clearTimeout(reconnectTimer);
      client.off("close", handleClose);
      client.disconnect();
    };
  }, [retryToken]);

  return connected;
}

function connectionErrorMessage(error: unknown): string {
  if (error instanceof Error && error.message) return error.message;
  if (typeof error === "string" && error) return error;
  return "The local daemon could not be reached.";
}

/** Trap Cmd/Ctrl+L to focus the composer. */
export function useComposerFocusShortcut(inputRef: RefObject<HTMLTextAreaElement | null>) {
  useEffect(() => {
    const handler = (e: globalThis.KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "l") {
        e.preventDefault();
        inputRef.current?.focus();
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [inputRef]);
}

/** Attach the selected session again after each confirmed connection. */
export function useSessionAttach(sessionId: string) {
  const connected = useSessionStore((s) => s.connectionState === "connected");
  const [attachError, setAttachError] = useState<string | null>(null);

  useEffect(() => {
    setAttachError(null);
    if (!connected) return;
    const client = getClient();
    const store = useSessionStore.getState();
    if (store.sessionId !== sessionId) store.reset();
    let current = true;
    void client.attachSession(sessionId).then(() => {
      if (!current) return;
      localStorage.setItem(SESSION_ID_KEY, sessionId);
      store.clearAlertsForSession(sessionId);
    }).catch((error: unknown) => {
      if (!current || !client.isConnected()) return;
      setAttachError(error instanceof Error ? error.message : "Could not open the conversation.");
    });
    return () => { current = false; };
  }, [sessionId, connected]);
  return attachError;
}
