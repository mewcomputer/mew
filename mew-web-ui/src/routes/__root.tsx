import { createRootRoute, Outlet, useRouter } from "@tanstack/react-router";
import { useState, useEffect } from "react";
import { CommandPalette } from "@/components/command-palette";
import { Toaster } from "@/components/ui/sonner";
import { getClient } from "@/lib/client";
import { routerRef } from "@/lib/router-ref";
import { useSessionStore } from "@/stores/session";
import { useMewConnection, useSessionNavigation } from "@/lib/hooks";
import { MobileNav } from "@/components/mobile-nav";
import { ConnectionBanner } from "@/components/connection-banner";
import { WorkspaceFrame } from "@/components/workspace-frame";

export const Route = createRootRoute({
  component: RootComponent,
});

function RootComponent() {
  const router = useRouter();
  useMewConnection();
  useSessionNavigation();
  const [paletteOpen, setPaletteOpen] = useState(false);

  // Populate the module-level router ref for non-React navigation.
  useEffect(() => {
    routerRef.navigate = (sessionId: string) => {
      router.navigate({
        to: "/session/$sessionId",
        params: { sessionId },
      });
    };
    return () => {
      routerRef.navigate = null;
    };
  }, [router]);

  // ⌘K → command palette, ⌘N → new session
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key === "k") {
        e.preventDefault();
        setPaletteOpen((v) => !v);
      }
      if ((e.metaKey || e.ctrlKey) && e.key === "n") {
        e.preventDefault();
        // Delegate to the command palette's new-session action.
        const client = getClient();
        if (!client) return;
        const store = useSessionStore.getState();
        if (store.connectionState !== "connected") {
          store.setConnectionError("Connect to the daemon before creating a session.");
          return;
        }
        store.reset();
        client
          .newSession()
          .then((newId) => {
            localStorage.setItem("mew.sessionId", newId);
            router.navigate({
              to: "/session/$sessionId",
              params: { sessionId: newId },
            });
          })
          .catch((error: unknown) => {
            useSessionStore
              .getState()
              .onError(error instanceof Error ? error.message : "Could not create a session.");
          });
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [router]);

  return (
    <>
      <WorkspaceFrame>
        <Outlet />
        <MobileNav
          active="chat"
          onChat={() => {
            const sid = useSessionStore.getState().sessionId;
            if (sid)
              router.navigate({
                to: "/session/$sessionId",
                params: { sessionId: sid },
              });
          }}
          onSessions={() => {}}
          onMore={() => router.navigate({ to: "/settings" })}
        />
      </WorkspaceFrame>
      <ConnectionBanner />
      <CommandPalette
        client={getClient()}
        open={paletteOpen}
        onOpenChange={setPaletteOpen}
      />
      <Toaster />
    </>
  );
}
