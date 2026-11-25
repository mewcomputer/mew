import { useEffect, useState } from "react";
import { Dialog, DialogContent, DialogTitle } from "@/components/ui/dialog";
import { SettingsPanel } from "@/components/settings-panel";

/** Opened from the rail and command palette; the /settings route stays for deep links. */
export function SettingsDialog() {
  const [open, setOpen] = useState(false);

  useEffect(() => {
    const openSettings = () => setOpen(true);
    const closeSettings = () => setOpen(false);
    window.addEventListener("mew:open-settings", openSettings);
    window.addEventListener("mew:close-settings", closeSettings);
    return () => {
      window.removeEventListener("mew:open-settings", openSettings);
      window.removeEventListener("mew:close-settings", closeSettings);
    };
  }, []);

  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <DialogContent
        aria-label="Settings"
        className="flex h-[min(40rem,calc(100vh-4rem))] w-[min(54rem,calc(100vw-2rem))] max-w-none flex-col gap-0 overflow-hidden rounded-xl border-border/80 p-0 shadow-2xl [&>button:last-child]:right-3.5 [&>button:last-child]:top-3 [&>button:last-child]:z-20 [&>button:last-child]:rounded-md [&>button:last-child]:p-1 [&>button:last-child]:hover:bg-accent"
      >
        <DialogTitle className="sr-only">Settings</DialogTitle>
        <SettingsPanel inDialog />
      </DialogContent>
    </Dialog>
  );
}
