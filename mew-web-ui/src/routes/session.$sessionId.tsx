import { createFileRoute } from "@tanstack/react-router";
import { useRef } from "react";
import type { Attachment } from "@mew/web-client";
import { useSessionStore, permissionResponders } from "@/stores/session";
import { getClient } from "@/lib/client";
import { useComposerFocusShortcut, useSessionAttach } from "@/lib/hooks";
import { ConversationLayout } from "@/components/conversation-layout";
import { VirtualChatSurface } from "@/components/virtual-chat-surface";
import { InputArea } from "@/components/input-area";
import { StatusFooter } from "@/components/status-footer";
import { PermissionToast } from "@/components/permission-toast";
import { AskUserCard } from "@/components/ask-user-card";
import { GoalCard } from "@/components/goal-card";

export const Route = createFileRoute("/session/$sessionId")({
  component: SessionRouteComponent,
});

function SessionRouteComponent() {
  const { sessionId } = Route.useParams();
  const inputRef = useRef<HTMLTextAreaElement | null>(null);
  const connected = useSessionStore((s) => s.connectionState === "connected" && s.confirmedSessionId === sessionId);
  const current = useSessionStore((s) => s.sessionId === sessionId);

  const attachError = useSessionAttach(sessionId);
  useComposerFocusShortcut(inputRef);

  const handleSend = (text: string, attachments: Attachment[] = []) =>
    getClient().submitPrompt(sessionId, text, attachments);

  const handleSlash = async (command: string) => {
    const result = await getClient().slashCommand(command);
    if (result) {
      useSessionStore.getState().onSlashResult(result);
    }
  };

  const handleCancel = () => {
    const client = getClient();
    if (client.isSessionAttached(sessionId)) client.cancel();
  };

  const handlePermission = (
    requestId: string,
    decision: "allow_once" | "allow_session" | "deny",
  ) => {
    const respond = permissionResponders.get(requestId);
    if (respond) {
      respond(decision);
      permissionResponders.delete(requestId);
    }
    useSessionStore.getState().resolvePermission(requestId);
  };

  return (
    <ConversationLayout>
      <div className="relative flex min-h-0 flex-1 flex-col">
        {current && <VirtualChatSurface />}
        {attachError && <p role="alert" className="px-4 py-2 text-sm text-destructive">{attachError}</p>}
      </div>
      <div className="shrink-0">
        {connected && <><AskUserCard /><GoalCard /></>}
        <InputArea ref={inputRef} onSend={handleSend} onSlash={handleSlash} onCancel={handleCancel} connected={connected} draftSessionId={sessionId} />
        <StatusFooter />
      </div>
      {connected && <PermissionToast onResolve={handlePermission} />}
    </ConversationLayout>
  );
}
