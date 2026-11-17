import { useEffect, useState, type RefObject } from "react";
import { useRouter } from "@tanstack/react-router";
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
        attempt = 0;
        useSessionStore.getState().setConnectionState("connected");
        useSessionStore.getState().setConnectionError(null);
        client
          .ping()
          .then((version) => useSessionStore.getState().setDaemonVersion(version))
          .catch(() => {
            /* daemon may not support ping yet — ignore */
          });
      } catch (error) {
        console.error("[mew] connect failed:", error);
        if (cancelled) return;
        attempt += 1;
        useSessionStore.getState().setConnectionError(connectionErrorMessage(error));
        scheduleRetry();
      }
    };

    const handleClose = () => {
      if (cancelled) return;
      attempt = Math.max(attempt + 1, 1);
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

/** Attach to a session by ID, with cleanup. */
export function useSessionAttach(sessionId: string) {
  const router = useRouter();
  const connected = useSessionStore((state) => state.connectionState === "connected");
  const attachedSession = useSessionStore((state) => state.sessionId);
  const [settled, setSettled] = useState(true);

  useEffect(() => {
    if (!connected) return;
    const client = getClient();
    const store = useSessionStore.getState();
    let active = true;
    localStorage.setItem(SESSION_ID_KEY, sessionId);
    store.clearAlertsForSession(sessionId);

    // New-session flows receive SessionReady before navigation. The client
    // is already attached in that case, even if React has not observed the
    // store update yet; attaching again races the daemon and can surface a
    // misleading "session not found" error.
    if (client.isSessionStable(sessionId)) {
      if (store.sessionId !== sessionId) store.setSessionId(sessionId);
      setSettled(true);
      return;
    }

    setSettled(false);
    if (store.sessionId !== sessionId) store.reset();
    client.attachSession(sessionId).catch(() => {
      if (active && client.isConnected()) router.navigate({ to: "/" });
    }).finally(() => { if (active) setSettled(true); });
    return () => { active = false; };
  }, [sessionId, router, connected, attachedSession]);
  return connected && attachedSession === sessionId && settled && getClient().isSessionStable(sessionId);
}
