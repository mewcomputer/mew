import { createFileRoute, useRouter } from "@tanstack/react-router";
import { useEffect, useRef, useState, type ReactNode } from "react";
import { getClient, SESSION_ID_KEY } from "@/lib/client";
import { FakeHeader } from "@/components/fake-header";
import { Button } from "@/components/ui/button";
import { useSessionStore } from "@/stores/session";
import { CircleAlert, LoaderCircle, Plus, RefreshCw } from "lucide-react";

export const Route = createFileRoute("/")({
  component: HomeComponent,
});

function HomeComponent() {
  const router = useRouter();
  const connectionState = useSessionStore((s) => s.connectionState);
  const connectionError = useSessionStore((s) => s.connectionError);
  const retryConnection = useSessionStore((s) => s.retryConnection);
  const attemptedResume = useRef(false);
  const [resumeState, setResumeState] = useState<"idle" | "loading" | "failed">("idle");
  const [actionState, setActionState] = useState<"idle" | "loading" | "failed">("idle");
  const [actionError, setActionError] = useState<string | null>(null);

  useEffect(() => {
    if (connectionState !== "connected" || attemptedResume.current) return;
    attemptedResume.current = true;
    const client = getClient();
    const prevSessionId = localStorage.getItem(SESSION_ID_KEY);

    if (prevSessionId) {
      setResumeState("loading");
      client
        .attachSession(prevSessionId)
        .then(() => {
          router.navigate({ to: "/session/$sessionId", params: { sessionId: prevSessionId } });
        })
        .catch(() => setResumeState("failed"));
    }
  }, [router, connectionState]);

  const createNew = async () => {
    if (connectionState !== "connected") return;
    setActionState("loading");
    setActionError(null);
    try {
      const newId = await getClient().newSession();
      localStorage.setItem(SESSION_ID_KEY, newId);
      router.navigate({ to: "/session/$sessionId", params: { sessionId: newId } });
    } catch (error) {
      setActionState("failed");
      setActionError(error instanceof Error ? error.message : "Could not create a session.");
    }
  };

  const busy = connectionState !== "connected" || actionState === "loading" || resumeState === "loading";
  const showResumeFailure = resumeState === "failed";

  return (
    <>
      <FakeHeader />
      <main className="flex flex-1 items-center justify-center px-6 py-10">
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
          {connectionState === "connected" && !showResumeFailure && resumeState !== "loading" && (
            <>
              <div className="space-y-2">
                <h1 className="text-xl font-semibold tracking-tight">start a coding session</h1>
                <p className="text-sm text-muted-foreground">
                  Open a local workspace, inspect a codebase, and keep the daemon working beside you.
                </p>
              </div>
              <Button onClick={createNew} disabled={busy} size="lg" className="w-full">
                <Plus className="h-4 w-4" />
                {actionState === "loading" ? "Creating session…" : "New session"}
              </Button>
              {actionError && <p className="text-xs text-destructive">{actionError}</p>}
            </>
          )}
          {connectionState !== "connected" && (
            <Button onClick={retryConnection} disabled={connectionState === "reconnecting"} variant="outline">
              <RefreshCw className="h-3.5 w-3.5" />
              {connectionState === "reconnecting" ? "Retrying…" : "Retry connection"}
            </Button>
          )}
          {connectionState === "connected" && showResumeFailure && (
            <Button onClick={createNew} disabled={busy}>
              <Plus className="h-3.5 w-3.5" />
              Start a new session
            </Button>
          )}
        </section>
      </main>
    </>
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
