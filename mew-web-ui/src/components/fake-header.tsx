import { useState } from "react";
import { useRouter } from "@tanstack/react-router";
import { useSessionStore } from "../stores/session";
import { useSidebar } from "@/components/ui/sidebar";
import { Button } from "@/components/ui/button";
import { RightRail } from "../components/right-rail";
import { PanelLeft, PanelRight, Activity, Settings } from "lucide-react";

/** Fake header — borderless, natural extension of the chat surface. */
export function FakeHeader() {
  const { toggleSidebar, open, isMobile } = useSidebar();
  const [rightSheetOpen, setRightSheetOpen] = useState(false);
  const sessionId = useSessionStore((s) => s.sessionId);
  const titles = useSessionStore((s) => s.sessionTitles);
  const router = useRouter();

  const title = sessionId
    ? (titles.get(sessionId) ?? sessionId.slice(0, 12) + "…")
    : "mew";

  return (
    <>
      <div className="mx-auto flex w-full shrink-0 items-center gap-2 px-3 py-1.5">
        <Button
          variant="ghost"
          size="icon"
          className="h-7 w-7"
          onClick={toggleSidebar}
          title={open ? "Collapse sidebar" : "Expand sidebar"}
        >
          {open ? (
            <PanelLeft className="h-3.5 w-3.5" />
          ) : (
            <PanelRight className="h-3.5 w-3.5" />
          )}
        </Button>

        <span className="truncate text-xs font-medium text-muted-foreground">
          {title}
        </span>

        <div className="flex-1" />

        {isMobile && (
          <Button
            variant="ghost"
            size="icon"
            className="h-7 w-7"
            onClick={() => setRightSheetOpen(true)}
            title="Activity"
          >
            <Activity className="h-3.5 w-3.5" />
          </Button>
        )}
        <Button
          variant="ghost"
          size="icon"
          className="h-7 w-7"
          onClick={() => router.navigate({ to: "/settings" })}
          title="Settings"
        >
          <Settings className="h-3.5 w-3.5" />
        </Button>
      </div>
      {/* Right rail: mobile sheet only — desktop renders the docked panel in the layout */}
      {isMobile && (
        <RightRail mobileOpen={rightSheetOpen} onMobileOpenChange={setRightSheetOpen} />
      )}
    </>
  );
}
