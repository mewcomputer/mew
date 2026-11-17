import { createFileRoute, useRouter } from "@tanstack/react-router";
import { useEffect, useRef, useState, type ReactNode } from "react";
import type { Attachment } from "@mew/web-client";
import { getClient, SESSION_ID_KEY } from "@/lib/client";
import { ConversationLayout } from "@/components/conversation-layout";
import { Button } from "@/components/ui/button";
import { InputArea } from "@/components/input-area";
import { useSessionStore } from "@/stores/session";
import { CircleAlert, LoaderCircle, RefreshCw } from "lucide-react";

export const Route = createFileRoute("/")({
  component: HomeComponent,
});

function HomeComponent() {
  const router = useRouter();
  const connectionState = useSessionStore((s) => s.connectionState);
  const connectionError = useSessionStore((s) => s.connectionError);
  const retryConnection = useSessionStore((s) => s.retryConnection);
  const attemptedResume = useRef(false);
  const newSession = useRef<string | null>(null);
  const [resumeState, setResumeState] = useState<"idle" | "loading" | "failed">("idle");

  useEffect(() => {
    if (connectionState !== "connected" || attemptedResume.current) return;
    attemptedResume.current = true;
    const client = getClient();
    const prevSessionId = localStorage.getItem(SESSION_ID_KEY);

    if (prevSessionId) {
      let active = true;
      let pending = true;
      setResumeState("loading");
      client
        .attachSession(prevSessionId)
        .then(() => {
          pending = false;
          if (active && router.state.location.pathname === "/") {
            router.navigate({ to: "/session/$sessionId", params: { sessionId: prevSessionId } });
          }
        })
        .catch(() => { pending = false; if (active) setResumeState("failed"); });
      return () => {
        active = false;
        if (pending) attemptedResume.current = false;
      };
    }
  }, [router, connectionState]);

  const sendNewMessage = async (text: string, attachments?: Attachment[]) => {
    const client = getClient();
    const newId = newSession.current ?? await client.newSession();
    newSession.current = newId;
    if (client.getSessionId() !== newId || !client.hasConfirmedSession(newId)) {
      await client.attachSession(newId);
    }
    localStorage.setItem(SESSION_ID_KEY, newId);
    await client.submitPrompt(newId, text, attachments);
    if (router.state.location.pathname === "/") {
      await router.navigate({ to: "/session/$sessionId", params: { sessionId: newId } });
    }
  };

  const showResumeFailure = resumeState === "failed";

  return (
    <ConversationLayout>
      <main className="flex flex-1 items-center justify-center px-6 pb-16">
        <section className="w-full max-w-md space-y-5 text-center">
          {connectionState === "connecting" && (
            <HomeStatus icon={<LoaderCircle className="h-5 w-5 animate-spin" />} title="starting the local daemon" detail="mew is preparing your coding workspace." />
          )}
          {connectionState === "reconnecting" && (
            <HomeStatus icon={<RefreshCw className="h-5 w-5 animate-spin" />} title="reconnecting to the daemon" detail={connectionError ?? "Your workspace will be available again shortly."} />
          )}
          {connectionState === "disconnected" && (
            <HomeStatus icon={<CircleAlert className="h-5 w-5 text-destructive" />} title="the local daemon is unavailable" detail={connectionError ?? "Retry when the daemon is ready."} />
          )}
          {connectionState === "connected" && resumeState === "loading" && (
            <HomeStatus icon={<LoaderCircle className="h-5 w-5 animate-spin" />} title="reopening your last workspace" detail="Loading the previous session history." />
          )}
          {connectionState === "connected" && showResumeFailure && (
            <HomeStatus icon={<CircleAlert className="h-5 w-5 text-amber-500" />} title="couldn’t reopen that session" detail="It may have been removed or belongs to another daemon." />
          )}
          {connectionState === "connected" && resumeState !== "loading" && (
            <>
              <div className="space-y-2">
                <h1 className="text-[24px] font-medium tracking-tight">What can we work on?</h1>
                {showResumeFailure && <p className="text-sm text-muted-foreground">Start a new chat to continue.</p>}
              </div>
            </>
          )}
          {connectionState !== "connected" && (
            <Button onClick={retryConnection} disabled={connectionState === "reconnecting"} variant="outline">
              <RefreshCw className="h-3.5 w-3.5" />
              {connectionState === "reconnecting" ? "Retrying…" : "Retry connection"}
            </Button>
          )}
        </section>
      </main>
      <InputArea draftId="new" connected={connectionState === "connected" && resumeState !== "loading"} onSend={sendNewMessage} onCancel={() => getClient().cancel()} />
    </ConversationLayout>
  );
}

function HomeStatus({ icon, title, detail }: { icon: ReactNode; title: string; detail: string }) {
  return (
    <div className="space-y-3" role="status" aria-live="polite">
      <div className="mx-auto flex h-10 w-10 items-center justify-center rounded-full bg-muted text-muted-foreground">
        {icon}
      </div>
      <div className="space-y-1">
        <h1 className="text-base font-semibold">{title}</h1>
        <p className="text-sm text-muted-foreground">{detail}</p>
      </div>
    </div>
  );
}
