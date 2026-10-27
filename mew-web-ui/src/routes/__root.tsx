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

  useEffect(() => {
    const action = (name: string) => {
      if (name === "search") setPaletteOpen((open) => !open);
      if (name !== "new-chat") return;
      const client = getClient();
      const store = useSessionStore.getState();
      if (store.connectionState !== "connected") {
        store.setConnectionError("Connect to the daemon before creating a chat.");
        return;
      }
      store.reset();
      void client.newSession().then((newId) => {
        localStorage.setItem("mew.sessionId", newId);
        router.navigate({ to: "/session/$sessionId", params: { sessionId: newId } });
      }).catch((error: unknown) => {
        useSessionStore.getState().onError(error instanceof Error ? error.message : "Could not create a chat.");
      });
    };
    const keyboard = (event: KeyboardEvent) => {
      if (!(event.metaKey || event.ctrlKey)) return;
      const key = event.key.toLowerCase();
      if (key !== "k" && key !== "n") return;
      event.preventDefault();
      action(key === "k" ? "search" : "new-chat");
    };
    const menu = (event: Event) => action((event as CustomEvent<string>).detail);
    window.addEventListener("keydown", keyboard);
    window.addEventListener("mew:menu-action", menu);
    return () => { window.removeEventListener("keydown", keyboard); window.removeEventListener("mew:menu-action", menu); };
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
