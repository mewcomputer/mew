import { createRootRoute, Outlet, useRouter } from "@tanstack/react-router";
import { SidebarProvider, SidebarInset } from "@/components/ui/sidebar";
import { SessionRail } from "@/components/session-rail";
import { getClient } from "@/lib/client";
import { useSessionStore } from "@/stores/session";
import { useSessionNavigation } from "@/lib/hooks";
import { MobileNav } from "@/components/mobile-nav";

export const Route = createRootRoute({
  component: RootComponent,
});

function RootComponent() {
  const router = useRouter();
  useSessionNavigation();
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
    </SidebarProvider>
  );
}
