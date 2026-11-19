import { useEffect, useState } from "react";
import { ChevronLeft, RotateCw } from "lucide-react";
import type { MewClientEvents } from "@mew/web-client";
import { getClient } from "../lib/client-ref";
import {
  useSessionStore,
  EMPTY_SUBAGENT_DRAFT,
  sendSubagentGuidance,
  subagentDraftKey,
  wireMessageToChatMessage,
  type ChatMessage,
  type SubagentInfo,
} from "../stores/session";
import { Button } from "./ui/button";
import { MessageItem } from "./message-item";

export default function SubagentConversation({ parentId, agent, active, onBack }: {
  parentId: string;
  agent: SubagentInfo;
  active: boolean;
  onBack: () => void;
}) {
  const childId = agent.childSessionId;
  const draftKey = subagentDraftKey(parentId, childId);
  const name = agent.displayName ?? agent.name;
  const draft = useSessionStore((s) => s.subagentDrafts.get(draftKey) ?? EMPTY_SUBAGENT_DRAFT);
  const setDraft = useSessionStore((s) => s.setSubagentDraft);
  const attached = useSessionStore((s) => s.connectionState === "connected" && s.confirmedSessionId === parentId);
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [loading, setLoading] = useState(false);
  const [historyError, setHistoryError] = useState<string | null>(null);
  const [refresh, setRefresh] = useState(0);

  useEffect(() => {
    const client = getClient();
    if (!active || !attached || !client?.isSessionAttached(parentId)) {
      setLoading(false);
      return;
    }
    const requestId = crypto.randomUUID();
    const cleanup = () => {
      clearTimeout(timer);
      client.off("subagent-history", onHistory);
    };
    const fail = (message: string) => {
      cleanup();
      setLoading(false);
      setHistoryError(message);
    };
    const onHistory: MewClientEvents["subagent-history"] = (history) => {
      if (history.session_id !== parentId || history.child_session_id !== childId || history.request_id !== requestId) return;
      if (history.error) {
        fail(history.error);
        return;
      }
      cleanup();
      setLoading(false);
      setMessages(history.messages.map(wireMessageToChatMessage).filter((message) => message.parts.length > 0 || message.assistantMeta));
    };
    const timer = setTimeout(() => fail("The agent history did not load. Refresh to try again."), 10000);
    setLoading(true);
    setHistoryError(null);
    client.on("subagent-history", onHistory);
    try {
      client.readSubagentHistory(parentId, childId, requestId);
    } catch (error) {
      fail(error instanceof Error ? error.message : "Could not load the agent history.");
    }
    return cleanup;
  }, [active, attached, parentId, childId, refresh, agent.status, draft.savedRequestId]);

  const canSend = active && attached && agent.status === "running";
  const sending = draft.submission?.status === "sending";
  const checking = draft.submission?.status === "checking";
  function sendGuidance() {
    const client = getClient();
    if (canSend && client) {
      void sendSubagentGuidance(client, parentId, childId, agent.parentCallId);
    }
  }

  return (
    <div className="flex h-full min-h-0 flex-col">
      <header className="flex shrink-0 items-center gap-2 border-b border-border px-2 py-1">
        <Button type="button" size="icon-sm" variant="ghost" aria-label="Back to agents" onClick={onBack}>
          <ChevronLeft />
        </Button>
        <h2 className="min-w-0 flex-1 truncate text-xs font-medium">{name}</h2>
        <Button type="button" size="icon-sm" variant="ghost" aria-label="Refresh agent history" disabled={!attached || loading} onClick={() => setRefresh((count) => count + 1)}>
          <RotateCw />
        </Button>
      </header>
      <div className="min-h-0 flex-1 space-y-4 overflow-y-auto px-3 py-3">
        {historyError && <p role="alert" className="text-xs text-destructive">{historyError}</p>}
        {loading && <p role="status" className="text-xs text-muted-foreground">Loading agent history…</p>}
        {!loading && !historyError && messages.length === 0 && (
          <p className="text-xs text-muted-foreground">{attached ? "No messages yet." : "Connect to the parent conversation to load agent history."}</p>
        )}
        {messages.map((message) => <MessageItem key={message.id} message={message} />)}
      </div>
      {(agent.status === "running" || draft.text || draft.savedRequestId) && (
        <form className="shrink-0 space-y-2 border-t border-border p-3" onSubmit={(event) => { event.preventDefault(); sendGuidance(); }}>
          {(agent.status === "running" || draft.text) && (
            <textarea
              className="block w-full resize-y rounded-md border border-input bg-transparent px-2 py-1.5 text-xs focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
              aria-label={`Guidance for ${name}`}
              placeholder="Guide this agent"
              rows={2}
              value={draft.text}
              readOnly={agent.status !== "running" || sending}
              onChange={(event) => {
                const current = useSessionStore.getState().subagentDrafts.get(draftKey) ?? EMPTY_SUBAGENT_DRAFT;
                setDraft(draftKey, { ...current, text: event.currentTarget.value });
              }}
            />
          )}
          {draft.error && !checking && <p role="alert" className="text-xs text-destructive">{draft.error}</p>}
          {draft.savedRequestId && <p role="status" className="text-xs text-muted-foreground">Guidance saved.</p>}
          {agent.status === "running" && (
            <Button type="submit" size="sm" disabled={!canSend || sending || checking || !draft.text.trim()} aria-busy={sending || checking}>
              {sending ? "Sending…" : checking ? "Checking delivery…" : "Send guidance"}
            </Button>
          )}
        </form>
      )}
    </div>
  );
}
