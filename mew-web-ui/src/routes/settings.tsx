import { createFileRoute, Outlet, useMatchRoute, useRouter } from "@tanstack/react-router";
import { FrameTitlebar } from "@/components/workspace-frame";
import { SettingsPanel } from "@/components/settings-panel";
import { useSessionStore } from "@/stores/session";
import { Button } from "@/components/ui/button";
import { ChevronLeft } from "lucide-react";

export const Route = createFileRoute("/settings")({
  component: SettingsRouteComponent,
});

function SettingsRouteComponent() {
  const router = useRouter();
  const matchRoute = useMatchRoute();
  const isChildRoute = Boolean(matchRoute({ to: "/settings/sessions" }));
  const sessionId = useSessionStore((s) => s.sessionId);

  if (isChildRoute) {
    return <Outlet />;
  }

  return (
    <>
      <FrameTitlebar>
        <div className="mew-conversation-header mew-drag-region flex h-9 shrink-0 items-center gap-2 px-3">
          <Button
            variant="ghost"
            size="icon"
            className="h-7 w-7"
            onClick={() => {
              if (sessionId) {
                router.navigate({ to: "/session/$sessionId", params: { sessionId } });
              } else {
                router.navigate({ to: "/" });
              }
            }}
            title="Back to chat"
          >
            <ChevronLeft className="h-4 w-4" />
          </Button>
          <span className="text-xs font-medium text-muted-foreground">Settings</span>
        </div>
      </FrameTitlebar>
      <SettingsPanel hideSidebarTitle />
    </>
  );
}
