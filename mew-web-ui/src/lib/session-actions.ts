import type { MewClient } from "@mew/web-client";
import { toast } from "sonner";
import { SESSION_ID_KEY } from "./client";

interface SessionRouter {
  state: { location: unknown };
  navigate: (opts: { to: "/session/$sessionId"; params: { sessionId: string } }) => unknown;
}

/** Create a session and navigate to it unless the user navigated elsewhere in-flight. */
export async function createAndOpenSession(
  client: MewClient,
  router: SessionRouter,
  options?: { cwd?: string | null; scratch?: boolean; errorMessage?: string },
): Promise<string | null> {
  const location = router.state.location;
  try {
    const newId = await client.newSession(options?.cwd ?? null, { scratch: options?.scratch });
    if (router.state.location !== location) return null;
    localStorage.setItem(SESSION_ID_KEY, newId);
    router.navigate({ to: "/session/$sessionId", params: { sessionId: newId } });
    return newId;
  } catch (error) {
    toast.error(
      error instanceof Error
        ? error.message
        : (options?.errorMessage ?? "Could not create a session."),
    );
    return null;
  }
}
