import { useEffect, useMemo, useState } from "react";
import { useRouter } from "@tanstack/react-router";
import type { MewClient, SessionInfo } from "@mew/web-client";
import { CalendarDays, FolderOpen, Layers, Plus, Search, Settings, Square, Sun, Archive, FileText, CornerDownLeft } from "lucide-react";
import { Command, CommandEmpty, CommandGroup, CommandInput, CommandItem, CommandList, CommandShortcut } from "../components/ui/command";
import { Dialog, DialogContent } from "../components/ui/dialog";
import { useSessionStore } from "../stores/session";
import { useTheme, THEMES } from "../lib/theme";
import { cn } from "../lib/utils";
import { filterProjects, filterSessions, formatSessionDate, getSessionSearchText, projectName } from "../lib/session-search";

interface CommandPaletteProps {
  client: MewClient | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}

type PaletteAction = {
  id: string;
  label: string;
  icon: React.ReactNode;
  hint?: string;
  action: () => void | Promise<void>;
};

export function CommandPalette({ client, open, onOpenChange }: CommandPaletteProps) {
  const router = useRouter();
  const sessions = useSessionStore((s) => s.availableSessions);
  const projects = useSessionStore((s) => s.projects);
  const sessionTitles = useSessionStore((s) => s.sessionTitles);
  const sessionSummaries = useSessionStore((s) => s.sessionSummaries);
  const messages = useSessionStore((s) => s.messages);
  const currentSessionId = useSessionStore((s) => s.sessionId);
  const { themeId, setThemeId } = useTheme();
  const [query, setQuery] = useState("");

  useEffect(() => {
    if (!open) setQuery("");
    if (open) client?.listProjects();
  }, [client, open]);

  const currentContent = useMemo(
    () =>
      messages
        .flatMap((message) =>
          message.parts.flatMap((part) => {
            if (part.type === "text" || part.type === "reasoning") return [part.text];
            if (part.type === "tool-call") return [part.toolName];
            if (part.type === "error") return [part.message];
            return [];
          }),
        )
        .join(" "),
    [messages],
  );

  const content = useMemo(
    () => (currentSessionId ? new Map([[currentSessionId, currentContent]]) : undefined),
    [currentContent, currentSessionId],
  );

  const handleNewSession = async (cwd: string | null = null) => {
    if (!client) return;
    try {
      useSessionStore.getState().reset();
      const newId = await client.newSession(cwd);
      localStorage.setItem("mew.sessionId", newId);
      router.navigate({ to: "/session/$sessionId", params: { sessionId: newId } });
      onOpenChange(false);
    } catch (error) {
      useSessionStore.getState().onError(error instanceof Error ? error.message : "Could not create a session.");
    }
  };

  const handleAttach = (sessionId: string) => {
    localStorage.setItem("mew.sessionId", sessionId);
    router.navigate({ to: "/session/$sessionId", params: { sessionId } });
    onOpenChange(false);
  };

  const actions = useMemo<PaletteAction[]>(() => {
    const items: PaletteAction[] = [
      { id: "new-session", label: "New session", icon: <Plus />, hint: "⌘N", action: () => handleNewSession() },
      { id: "cancel-turn", label: "Cancel current turn", icon: <Square />, action: () => { client?.cancel(); onOpenChange(false); } },
      {
        id: "toggle-theme",
        label: "Toggle theme",
        icon: <Sun />,
        action: () => {
          const index = THEMES.findIndex((theme) => theme.id === themeId);
          const next = THEMES[(index + 1) % THEMES.length];
          if (next) setThemeId(next.id);
          onOpenChange(false);
        },
      },
      { id: "settings", label: "Open settings", icon: <Settings />, action: () => { router.navigate({ to: "/settings" }); onOpenChange(false); } },
      { id: "generate-wiki", label: "Generate repo wiki", icon: <Layers />, action: () => { client?.slashCommand("/wiki"); onOpenChange(false); } },
    ];
    if (currentSessionId) {
      items.push({ id: "archive-session", label: "Archive current session", icon: <Archive />, action: () => { client?.archiveSession(currentSessionId, true); onOpenChange(false); } });
    }
    return items;
  }, [client, currentSessionId, onOpenChange, router, setThemeId, themeId]);

  const normalizedQuery = query.trim().toLowerCase();
  const filteredActions = useMemo(
    () => normalizedQuery ? actions.filter((action) => action.label.toLowerCase().includes(normalizedQuery)) : actions,
    [actions, normalizedQuery],
  );
  const filteredSessions = useMemo(
    () => filterSessions(sessions.filter((session) => !session.archived), query, sessionTitles, sessionSummaries, content)
      .sort((a, b) => (b.last_message_at ?? b.created_at) - (a.last_message_at ?? a.created_at))
      .slice(0, 30),
    [content, query, sessionSummaries, sessionTitles, sessions],
  );
  const filteredProjects = useMemo(() => filterProjects(projects, query).slice(0, 12), [projects, query]);

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="overflow-hidden rounded-2xl border-border/80 p-0 shadow-2xl sm:max-w-2xl">
        <Command shouldFilter={false} className="bg-popover">
          <CommandInput autoFocus placeholder="Search sessions, projects, dates, and content…" value={query} onValueChange={setQuery} />
          <CommandList className="max-h-[min(28rem,65vh)] p-2">
            <CommandEmpty className="py-12 text-center text-sm text-muted-foreground">
              <Search className="mx-auto mb-2 h-5 w-5 opacity-40" />
              No matching sessions or projects
            </CommandEmpty>

            {filteredActions.length > 0 && (
              <CommandGroup heading="Actions">
                {filteredActions.map((action) => (
                  <CommandItem key={action.id} value={action.label} onSelect={() => action.action()} className="min-h-10 px-3">
                    <span className="flex h-7 w-7 items-center justify-center rounded-md bg-muted text-muted-foreground">{action.icon}</span>
                    <span>{action.label}</span>
                    {action.hint && <CommandShortcut>{action.hint}</CommandShortcut>}
                  </CommandItem>
                ))}
              </CommandGroup>
            )}

            {filteredSessions.length > 0 && (
              <CommandGroup heading={`Sessions${query ? ` · ${filteredSessions.length}` : ""}`}>
                {filteredSessions.map((session) => (
                  <SessionItem key={session.session_id} session={session} title={sessionTitles.get(session.session_id)} onSelect={() => handleAttach(session.session_id)} />
                ))}
              </CommandGroup>
            )}

            {filteredProjects.length > 0 && (
              <CommandGroup heading="Projects">
                {filteredProjects.map((project) => (
                  <CommandItem key={project.path} value={`${project.display_name} ${project.path}`} onSelect={() => handleNewSession(project.path)} className="min-h-10 px-3">
                    <span className="flex h-7 w-7 items-center justify-center rounded-md bg-muted text-muted-foreground"><FolderOpen className="h-4 w-4" /></span>
                    <span className="min-w-0 flex-1 truncate">{project.display_name}</span>
                    <span className="max-w-[16rem] truncate text-xs text-muted-foreground">{project.path}</span>
                    <span className="text-[11px] tabular-nums text-muted-foreground">{project.session_count}</span>
                  </CommandItem>
                ))}
              </CommandGroup>
            )}
          </CommandList>
          <div className="flex items-center justify-between border-t px-4 py-2 text-[11px] text-muted-foreground">
            <span><kbd className="rounded border bg-muted px-1 py-0.5 font-mono">↑↓</kbd> navigate <kbd className="ml-2 rounded border bg-muted px-1 py-0.5 font-mono">↵</kbd> open</span>
            <span className="flex items-center gap-1"><FileText className="h-3 w-3" /> titles, summaries, and loaded content</span>
          </div>
        </Command>
      </DialogContent>
    </Dialog>
  );
}

function SessionItem({ session, title, onSelect }: { session: SessionInfo; title?: string; onSelect: () => void }) {
  const label = title ?? deriveTitle(session);
  const folder = projectName(session.cwd);
  return (
    <CommandItem value={getSessionSearchText(session, label)} onSelect={onSelect} className={cn("min-h-12 items-start px-3 py-2", session.state === "running" && "bg-primary/5")}>
      <span className="mt-0.5 flex h-7 w-7 shrink-0 items-center justify-center rounded-md bg-muted text-muted-foreground"><Layers className="h-4 w-4" /></span>
      <span className="min-w-0 flex-1">
        <span className="block truncate font-medium">{label}</span>
        <span className="mt-0.5 flex min-w-0 items-center gap-2 truncate text-xs text-muted-foreground">
          <span className="flex min-w-0 items-center gap-1 truncate"><FolderOpen className="h-3 w-3 shrink-0" />{folder}</span>
          <span className="flex shrink-0 items-center gap-1"><CalendarDays className="h-3 w-3" />{formatSessionDate(session.last_message_at ?? session.created_at)}</span>
        </span>
      </span>
      {session.state === "running" && <span className="text-[11px] text-primary">running</span>}
      {session.session_id && <CornerDownLeft className="mt-1 h-3.5 w-3.5 shrink-0 text-muted-foreground/60" />}
    </CommandItem>
  );
}

function deriveTitle(session: SessionInfo): string {
  if (session.summary) return session.summary;
  if (session.first_message) return session.first_message;
  if (session.model) return session.model.split("/").pop() ?? session.model;
  return session.session_id.slice(0, 8);
}
