import { useEffect, useState, useMemo } from "react";
import { useRouter } from "@tanstack/react-router";
import { Command } from "cmdk";
import type { MewClient, SessionInfo } from "@mew/web-client";
import { useSessionStore } from "../stores/session";
import { useTheme, THEMES } from "../lib/theme";
import { cn } from "../lib/utils";
import {
  Plus,
  Sun,
  Settings,
  Layers,
  Archive,
  Square,
  CornerDownLeft,
  Search,
} from "lucide-react";

interface CommandPaletteProps {
  client: MewClient | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}

type CommandItem = {
  id: string;
  label: string;
  icon?: React.ReactNode;
  group: string;
  hint?: string;
  action: () => void;
};

export function CommandPalette({ client, open, onOpenChange }: CommandPaletteProps) {
  const router = useRouter();
  const sessions = useSessionStore((s) => s.availableSessions);
  const sessionTitles = useSessionStore((s) => s.sessionTitles);
  const currentSessionId = useSessionStore((s) => s.sessionId);
  const { themeId, setThemeId } = useTheme();
  const [query, setQuery] = useState("");

  // Close on Escape
  useEffect(() => {
    if (!open) setQuery("");
  }, [open]);

  const handleNewSession = async () => {
    if (!client) return;
    useSessionStore.getState().reset();
    const newId = await client.newSession();
    localStorage.setItem("mew.sessionId", newId);
    router.navigate({ to: "/session/$sessionId", params: { sessionId: newId } });
    onOpenChange(false);
  };

  const handleAttach = (sessionId: string) => {
    localStorage.setItem("mew.sessionId", sessionId);
    router.navigate({ to: "/session/$sessionId", params: { sessionId } });
    onOpenChange(false);
  };

  const handleCancel = () => {
    client?.cancel();
    onOpenChange(false);
  };

  const handleArchive = () => {
    if (currentSessionId && client) {
      client.archiveSession(currentSessionId, true);
      onOpenChange(false);
    }
  };

  const handleToggleTheme = () => {
    // Cycle through available themes via the ThemeProvider context.
    const allThemes = THEMES;
    const currentIdx = allThemes.findIndex((t) => t.id === themeId);
    const nextTheme = allThemes[(currentIdx + 1) % allThemes.length];
    if (nextTheme) {
      setThemeId(nextTheme.id);
    }
    onOpenChange(false);
  };

  const handleSettings = () => {
    router.navigate({ to: "/settings" });
    onOpenChange(false);
  };

  // Build command items
  const items = useMemo<CommandItem[]>(() => {
    const actions: CommandItem[] = [
      {
        id: "new-session",
        label: "New session",
        icon: <Plus className="h-4 w-4" />,
        group: "Actions",
        hint: "⌘N",
        action: handleNewSession,
      },
      {
        id: "cancel-turn",
        label: "Cancel current turn",
        icon: <Square className="h-4 w-4" />,
        group: "Actions",
        action: handleCancel,
      },
      {
        id: "toggle-theme",
        label: "Toggle theme",
        icon: <Sun className="h-4 w-4" />,
        group: "Actions",
        action: handleToggleTheme,
      },
      {
        id: "settings",
        label: "Open settings",
        icon: <Settings className="h-4 w-4" />,
        group: "Actions",
        action: handleSettings,
      },
      {
        id: "generate-wiki",
        label: "Generate repo wiki",
        icon: <Layers className="h-4 w-4" />,
        group: "Actions",
        action: () => {
          client?.slashCommand("/wiki");
          onOpenChange(false);
        },
      },
    ];

    if (currentSessionId) {
      actions.push({
        id: "archive-session",
        label: "Archive current session",
        icon: <Archive className="h-4 w-4" />,
        group: "Actions",
        action: handleArchive,
      });
    }

    // Session items
    const sessionItems: CommandItem[] = sessions
      .filter((s) => !s.archived)
      .slice(0, 20)
      .map((s) => ({
        id: `session-${s.session_id}`,
        label: sessionTitles.get(s.session_id) ?? deriveTitle(s),
        icon: <Layers className="h-4 w-4" />,
        group: "Sessions",
        hint: s.state === "running" ? "running" : s.state === "active" ? "active" : "idle",
        action: () => handleAttach(s.session_id),
      }));

    return [...actions, ...sessionItems];
  }, [sessions, sessionTitles, currentSessionId, client]);

  // Filter by query
  const filtered = useMemo(() => {
    if (!query.trim()) return items;
    const q = query.toLowerCase();
    return items.filter((item) => item.label.toLowerCase().includes(q));
  }, [items, query]);

  // Group filtered items
  const grouped = useMemo(() => {
    const map = new Map<string, CommandItem[]>();
    for (const item of filtered) {
      const list = map.get(item.group) ?? [];
      list.push(item);
      map.set(item.group, list);
    }
    return Array.from(map.entries());
  }, [filtered]);

  if (!open) return null;

  return (
    <div
      className="fixed inset-0 z-50 flex items-start justify-center pt-[15vh]"
      onClick={() => onOpenChange(false)}
    >
      <div className="absolute inset-0 bg-black/40" />
      <div
        className="relative w-full max-w-xl rounded-xl border bg-background shadow-lg"
        onClick={(e) => e.stopPropagation()}
      >
        <Command shouldFilter={false} className="flex flex-col">
          <div className="flex items-center gap-2 border-b px-3 py-2">
            <Search className="h-4 w-4 text-muted-foreground" />
            <Command.Input
              autoFocus
              placeholder="Type a command or search…"
              value={query}
              onValueChange={setQuery}
              className="flex-1 bg-transparent text-sm outline-none placeholder:text-muted-foreground"
            />
          </div>
          <Command.List className="max-h-[400px] overflow-y-auto p-1">
            {grouped.length === 0 && (
              <Command.Empty className="py-6 text-center text-sm text-muted-foreground">
                No results found
              </Command.Empty>
            )}
            {grouped.map(([group, groupItems]) => (
              <Command.Group
                key={group}
                heading={group}
                className={cn(
                  "[&_[cmdk-group-heading]]:px-2 [&_[cmdk-group-heading]]:py-1.5",
                  "[&_[cmdk-group-heading]]:text-[10px] [&_[cmdk-group-heading]]:font-semibold",
                  "[&_[cmdk-group-heading]]:text-muted-foreground",
                )}
              >
                {groupItems.map((item) => (
                  <Command.Item
                    key={item.id}
                    onSelect={() => item.action()}
                    className="flex cursor-pointer items-center gap-2 rounded-md px-2 py-1.5 text-sm aria-selected:bg-accent"
                  >
                    {item.icon}
                    <span className="flex-1">{item.label}</span>
                    {item.hint && (
                      <span className="text-[10px] text-muted-foreground">
                        {item.hint}
                      </span>
                    )}
                  </Command.Item>
                ))}
              </Command.Group>
            ))}
          </Command.List>
          <div className="flex items-center justify-between border-t px-3 py-1.5 text-[10px] text-muted-foreground">
            <span>↑↓ to navigate · ↵ to select · esc to close</span>
            <CornerDownLeft className="h-3 w-3" />
          </div>
        </Command>
      </div>
    </div>
  );
}

function deriveTitle(s: SessionInfo): string {
  if (s.summary) return s.summary;
  if (s.model) return s.model.split("/").pop() ?? s.model;
  return s.session_id.slice(0, 8);
}
