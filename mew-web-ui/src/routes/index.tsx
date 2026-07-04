import { createFileRoute, useRouter } from "@tanstack/react-router";
import { useEffect } from "react";
import { getClient, SESSION_ID_KEY } from "@/lib/client";
import { FakeHeader } from "@/components/fake-header";

export const Route = createFileRoute("/")({
  component: HomeComponent,
});

function HomeComponent() {
  const router = useRouter();

  useEffect(() => {
    const client = getClient();
    const prevSessionId = localStorage.getItem(SESSION_ID_KEY);

    const createNew = () => {
      client.newSession().then((newId) => {
        try {
          localStorage.setItem(SESSION_ID_KEY, newId);
        } catch {
          /* localStorage may be unavailable (e.g. private mode); ignore */
        }
        router.navigate({ to: "/session/$sessionId", params: { sessionId: newId } });
      });
    };

    if (prevSessionId) {
      client
        .attachSession(prevSessionId)
        .then(() => {
          router.navigate({ to: "/session/$sessionId", params: { sessionId: prevSessionId } });
        })
        .catch(createNew);
    } else {
      createNew();
    }
  }, [router]);

  return (
    <>
      <FakeHeader />
      <div className="flex flex-1 flex-col items-center justify-center gap-2 text-muted-foreground">
        <div className="h-6 w-6 animate-spin rounded-full border-2 border-current border-t-transparent" />
        <span className="text-xs">connecting…</span>
      </div>
    </>
  );
}
