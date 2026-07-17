import { useEffect, type RefObject } from "react";
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

/** Sync store sessionId → router navigation. */
export function useSessionNavigation() {
  const router = useRouter();
  useEffect(() => {
    if (!router) return;
    const unsub = useSessionStore.subscribe((state, prev) => {
      if (state.sessionId && state.sessionId !== prev.sessionId) {
        const current = router.state.location.pathname;
        const expected = `/session/${state.sessionId}`;
        if (current !== expected) {
          router.navigate({ to: "/session/$sessionId", params: { sessionId: state.sessionId } });
        }
      }
    });
    return unsub;
  }, [router]);
}

/** Attach to a session by ID, with cleanup. */
export function useSessionAttach(sessionId: string) {
  const router = useRouter();

  useEffect(() => {
    const client = getClient();
    const store = useSessionStore.getState();

    // New-session flows receive SessionReady before navigation. The client
    // is already attached in that case, even if React has not observed the
    // store update yet; attaching again races the daemon and can surface a
    // misleading "session not found" error.
    if (store.sessionId === sessionId || client.getSessionId() === sessionId) {
      if (store.sessionId !== sessionId) store.setSessionId(sessionId);
      return;
    }

    store.reset();
    localStorage.setItem(SESSION_ID_KEY, sessionId);
    // Clear alerts for this session since we're now viewing it.
    store.clearAlertsForSession(sessionId);

    client.attachSession(sessionId).catch(() => {
      router.navigate({ to: "/" });
    });
  }, [sessionId, router]);
}
