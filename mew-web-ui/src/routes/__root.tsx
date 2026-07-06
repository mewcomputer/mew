import { createRootRoute, Outlet, useRouter } from "@tanstack/react-router";
import { useState, useEffect } from "react";
import { SidebarProvider, SidebarInset } from "@/components/ui/sidebar";
import { SessionRail } from "@/components/session-rail";
import { CommandPalette } from "@/components/command-palette";
import { getClient } from "@/lib/client";
import { routerRef } from "@/lib/router-ref";
import { useSessionStore } from "@/stores/session";
import { useSessionNavigation } from "@/lib/hooks";
import { MobileNav } from "@/components/mobile-nav";

export const Route = createRootRoute({
  component: RootComponent,
});

function RootComponent() {
  const router = useRouter();
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
        useSessionStore.getState().reset();
        client.newSession().then((newId) => {
          localStorage.setItem("mew.sessionId", newId);
          router.navigate({
            to: "/session/$sessionId",
            params: { sessionId: newId },
          });
        });
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [router]);

  return (
    <SidebarProvider
      style={
        {
          "--sidebar-width": "calc(var(--spacing) * 72)",
          "--header-height": "calc(var(--spacing) * 12)",
        } as React.CSSProperties
      }
      defaultOpen
    >
      <SessionRail client={getClient()} />
      <SidebarInset className="flex flex-1 flex-col h-screen">
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
      </SidebarInset>
      <CommandPalette
        client={getClient()}
        open={paletteOpen}
        onOpenChange={setPaletteOpen}
      />
    </SidebarProvider>
  );
}
