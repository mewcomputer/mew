import { createFileRoute } from "@tanstack/react-router";
import { useRef } from "react";
import type { Attachment } from "@mew/web-client";
import { useSessionStore, permissionResponders } from "@/stores/session";
import { getClient } from "@/lib/client";
import { useComposerFocusShortcut, useSessionAttach } from "@/lib/hooks";
import { FakeHeader } from "@/components/fake-header";
import { VirtualChatSurface } from "@/components/virtual-chat-surface";
import { InputArea } from "@/components/input-area";
import { StatusFooter } from "@/components/status-footer";
import { PermissionToast } from "@/components/permission-toast";
import { AskUserCard } from "@/components/ask-user-card";
import { PlanApprovalCard } from "@/components/plan-approval-card";

export const Route = createFileRoute("/session/$sessionId")({
  component: SessionRouteComponent,
});

function SessionRouteComponent() {
  const { sessionId } = Route.useParams();
  const inputRef = useRef<HTMLTextAreaElement | null>(null);
  const connected = useSessionStore((s) => s.connectionState === "connected");

  useSessionAttach(sessionId);
  useComposerFocusShortcut(inputRef);

  const handleSend = (text: string, attachments: Attachment[] = []) => {
    const store = useSessionStore.getState();
    store.addUserMessage(text);
    getClient().prompt(text, attachments);
  };

  const handleSlash = async (command: string) => {
    const result = await getClient().slashCommand(command);
    if (result) {
      useSessionStore.getState().onSlashResult(result);
    }
  };

  const handleCancel = () => {
    getClient().cancel();
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
    <>
      <FakeHeader />
      <div className="relative flex min-h-0 flex-1 flex-col">
        <VirtualChatSurface />
      </div>
      <StatusFooter />
      <div className="pointer-events-none absolute inset-x-0 bottom-0 z-20">
        <div
          aria-hidden="true"
          className="absolute inset-x-0 bottom-0 h-40 bg-gradient-to-t from-background via-background/85 to-transparent"
        />
        <div className="relative flex flex-col">
          <div className="pointer-events-auto">
            <AskUserCard />
            <PlanApprovalCard />
          </div>
          <InputArea
            ref={inputRef}
            onSend={handleSend}
            onSlash={handleSlash}
            onCancel={handleCancel}
            connected={connected}
          />
        </div>
      </div>
      <PermissionToast onResolve={handlePermission} />
    </>
  );
}
