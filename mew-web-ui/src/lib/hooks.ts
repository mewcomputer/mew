import { useEffect, useRef, useState, type RefObject } from "react";
import { useRouter } from "@tanstack/react-router";
import { useSessionStore } from "../stores/session";
import { getClient, SESSION_ID_KEY } from "./client";

/** Connection hook — manages WebSocket lifecycle. */
export function useMewConnection() {
  const [connected, setConnected] = useState(false);
  const reconnectAttemptRef = useRef(0);
  const intentionalDisconnectRef = useRef(false);
  const reconnectTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    const client = getClient();
    intentionalDisconnectRef.current = false;

    const doConnect = async () => {
      const attempt = reconnectAttemptRef.current;
      if (attempt > 0) {
        const delay = Math.min(1000 * 2 ** attempt, 30000);
        useSessionStore.getState().setConnectionState("reconnecting");
        await new Promise((r) => {
          reconnectTimerRef.current = setTimeout(r, delay);
        });
      } else {
        useSessionStore.getState().setConnectionState("connecting");
      }

      try {
        await client.connect();
        reconnectAttemptRef.current = 0;
        setConnected(true);
        client.listModels();
      } catch (e) {
        console.error("[mew] connect failed:", e);
        if (!intentionalDisconnectRef.current) {
          reconnectAttemptRef.current += 1;
          doConnect();
        }
      }
    };

    doConnect();

    client.on("close", () => {
      setConnected(false);
      if (!intentionalDisconnectRef.current) {
        reconnectAttemptRef.current += 1;
        doConnect();
      }
    });

    return () => {
      intentionalDisconnectRef.current = true;
      if (reconnectTimerRef.current) clearTimeout(reconnectTimerRef.current);
      client.disconnect();
    };
  }, []);

  return connected;
}

/** Trap Cmd/Ctrl+K to focus the composer. */
export function useComposerFocusShortcut(inputRef: RefObject<HTMLTextAreaElement | null>) {
  useEffect(() => {
    const handler = (e: globalThis.KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") {
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

    if (store.sessionId === sessionId) return;

    store.reset();
    localStorage.setItem(SESSION_ID_KEY, sessionId);

    client.attachSession(sessionId).catch(() => {
      router.navigate({ to: "/" });
    });
  }, [sessionId, router]);
}
