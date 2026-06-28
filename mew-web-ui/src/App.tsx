import { useCallback, useEffect, useRef, useState } from "react";
import { MewClient } from "@mew/web-client";
import { bridgeClientToStore, permissionResponders, useSessionStore } from "./stores/session";
import { setClient } from "./lib/client-ref";
import { ChatSurface } from "./components/ChatSurface";
import { InputArea } from "./components/InputArea";
import { TopBar } from "./components/TopBar";
import { PermissionToast } from "./components/PermissionToast";
import { ErrorBoundary } from "./components/ErrorBoundary";
import { SessionRail } from "./components/SessionRail";
import { StatusFooter } from "./components/StatusFooter";
import { SubagentPanel } from "./components/SubagentPanel";
import { AskUserCard } from "./components/AskUserCard";
import { TodoPanel } from "./components/TodoPanel";

const WS_URL = (() => {
  // In dev: Vite proxy forwards /ws to the bridge at 127.0.0.1:9847.
  // In prod: the bridge serves everything on one origin.
  const proto = location.protocol === "https:" ? "wss" : "ws";
  return `${proto}://${location.host}/ws`;
})();

const SESSION_ID_KEY = "mew.sessionId";

export default function App() {
  const clientRef = useRef<MewClient | null>(null);
  const [connected, setConnected] = useState(false);
  const [railCollapsed, setRailCollapsed] = useState(false);
  const connectionState = useSessionStore((s) => s.connectionState);
  const reconnectAttemptRef = useRef(0);
  const intentionalDisconnectRef = useRef(false);
  const reconnectTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  // Connect (or reconnect) to the daemon with exponential backoff.
  // On a successful connect, re-attach to the previous session if we had one.
  const doConnect = useCallback(async (client: MewClient) => {
    const attempt = reconnectAttemptRef.current;
    if (attempt > 0) {
      const delay = Math.min(1000 * 2 ** attempt, 30000); // 2s, 4s, 8s, ... cap 30s
      useSessionStore.getState().setConnectionState("reconnecting");
      console.log(`[mew] reconnecting in ${delay}ms (attempt ${attempt + 1})`);
      await new Promise((r) => {
        reconnectTimerRef.current = setTimeout(r, delay);
      });
    }

    try {
      await client.connect();
      reconnectAttemptRef.current = 0;
      setConnected(true);

      // Re-attach to the previous session if we had one.
      const prevSessionId = localStorage.getItem(SESSION_ID_KEY);
      if (prevSessionId) {
        try {
          await client.attachSession(prevSessionId);
          return;
        } catch {
          // Session no longer exists; create a new one.
        }
      }
      await client.newSession();
    } catch (e) {
      console.error("[mew] connect failed:", e);
      if (!intentionalDisconnectRef.current) {
        reconnectAttemptRef.current += 1;
        doConnect(client);
      }
    }
  }, []);

  useEffect(() => {
    const client = new MewClient(WS_URL, { debug: false });
    clientRef.current = client;
    setClient(client);
    bridgeClientToStore(client);

    intentionalDisconnectRef.current = false;
    doConnect(client);

    // Listen for unexpected disconnects and trigger reconnect.
    client.on("close", () => {
      setConnected(false);
      if (!intentionalDisconnectRef.current) {
        reconnectAttemptRef.current += 1;
        doConnect(client);
      }
    });

    return () => {
      intentionalDisconnectRef.current = true;
      if (reconnectTimerRef.current) clearTimeout(reconnectTimerRef.current);
      client.disconnect();
      clientRef.current = null;
      setClient(null);
    };
  }, [doConnect]);

  // Persist the session ID whenever it changes so a reload can reattach.
  const sessionId = useSessionStore((s) => s.sessionId);
  useEffect(() => {
    if (sessionId) {
      localStorage.setItem(SESSION_ID_KEY, sessionId);
    }
  }, [sessionId]);

  const handleSend = (text: string) => {
    const store = useSessionStore.getState();
    store.addUserMessage(text);
    clientRef.current?.prompt(text);
  };

  const handleCancel = () => {
    clientRef.current?.cancel();
  };

  const handlePermission = (requestId: number, decision: "allow_once" | "allow_session" | "deny") => {
    const respond = permissionResponders.get(requestId);
    if (respond) {
      respond(decision);
      permissionResponders.delete(requestId);
    }
    useSessionStore.getState().resolvePermission(requestId);
  };

  return (
    <ErrorBoundary title="App crashed">
      <div className="flex h-screen flex-col bg-background text-foreground">
        <TopBar
          connectionState={connectionState}
          client={clientRef.current}
          onOpenSessions={() => setRailCollapsed((c) => !c)}
        />
        <div className="flex flex-1 overflow-hidden">
          <SessionRail
            client={clientRef.current}
            collapsed={railCollapsed}
            onToggle={() => setRailCollapsed((c) => !c)}
          />
          <main className="flex flex-1 flex-col overflow-hidden">
            <ChatSurface />
            <TodoPanel />
            <SubagentPanel />
            <AskUserCard />
            <InputArea onSend={handleSend} onCancel={handleCancel} connected={connected} />
          </main>
        </div>
        <StatusFooter />
        <PermissionToast onResolve={handlePermission} />
      </div>
    </ErrorBoundary>
  );
}