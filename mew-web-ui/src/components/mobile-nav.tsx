import { cn } from "../lib/utils";
import { useSidebar } from "@/components/ui/sidebar";
import { MessageSquare, LayoutGrid, MoreHorizontal } from "lucide-react";

interface MobileNavProps {
  active: "chat" | "sessions" | "more";
  onChat: () => void;
  onSessions: () => void;
  onMore: () => void;
}

/** Bottom navigation bar for mobile. Hidden on desktop (md:hidden).
 *  Uses the sidebar context to toggle the session sidebar open on mobile. */
export function MobileNav({ active, onChat, onSessions, onMore }: MobileNavProps) {
  const { setOpenMobile, openMobile } = useSidebar();

  const handleSessions = () => {
    setOpenMobile(!openMobile);
    onSessions();
  };

  return (
    <nav
      className="flex shrink-0 items-center justify-around border-t border-border bg-background md:hidden"
      style={{ paddingBottom: "env(safe-area-inset-bottom)" }}
    >
      <NavButton
        label="Chat"
        isActive={active === "chat" && !openMobile}
        onClick={onChat}
        icon={<MessageSquare className="h-5 w-5" />}
      />
      <NavButton
        label="Sessions"
        isActive={openMobile}
        onClick={handleSessions}
        icon={<LayoutGrid className="h-5 w-5" />}
      />
      <NavButton
        label="More"
        isActive={active === "more"}
        onClick={onMore}
        icon={<MoreHorizontal className="h-5 w-5" />}
      />
    </nav>
  );
}

function NavButton({
  label,
  isActive,
  onClick,
  icon,
}: {
  label: string;
  isActive: boolean;
  onClick: () => void;
  icon: React.ReactNode;
}) {
  return (
    <button
      onClick={onClick}
      className={cn(
        "flex flex-1 flex-col items-center gap-0.5 py-2 transition-colors",
        isActive ? "text-primary" : "text-muted-foreground",
      )}
    >
      {icon}
      <span className="text-[10px] font-medium">{label}</span>
    </button>
  );
}
