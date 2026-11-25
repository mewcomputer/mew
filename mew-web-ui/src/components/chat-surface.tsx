import { useMemo, useState } from "react";
import { useRouter } from "@tanstack/react-router";
import { selectSessionRunning, useSessionStore } from "../stores/session";
import { CompactingIndicator, TurnItem } from "./turn-item";
import { buildTurns } from "../lib/turns";
import { useFollowScroll } from "../lib/use-follow-scroll";
import { useComposerHeight } from "../lib/composer-height";
import { ErrorBoundary } from "./error-boundary";
import { ConversationMinimap } from "./conversation-minimap";
import { ChevronDown, Folder, FolderPlus, MessageSquare } from "lucide-react";
import { formatRelativeAge } from "../lib/format";
import { SESSION_ID_KEY } from "../lib/client";
import { getClient } from "../lib/client-ref";
import { createAndOpenSession } from "../lib/session-actions";
import { ProjectPickerModal } from "./session-rail";

export function WorkspaceSelectorPill({
  selectedCwd,
  onSelectCwd,
}: {
  selectedCwd?: string | null;
  onSelectCwd?: (cwd: string) => void;
}) {
  const router = useRouter();
  const sessionId = useSessionStore((s) => s.sessionId);
  const sessionCwd = useSessionStore((s) => s.sessionCwd);
  const sessions = useSessionStore((s) => s.availableSessions);
  const projects = useSessionStore((s) => s.projects);
  const connectionState = useSessionStore((s) => s.connectionState);
  const [pickerOpen, setPickerOpen] = useState(false);

  const session = sessionId ? sessions.find((item) => item.session_id === sessionId) : undefined;
  const effectiveCwd = selectedCwd !== undefined ? selectedCwd : (sessionCwd ?? session?.cwd ?? null);
  const folderName = effectiveCwd ? deriveFolderName(effectiveCwd) : null;

  const recentWorkspaces = useMemo(() => {
    const map = new Map<string, { path: string; name: string; lastUsed: number }>();
    for (const p of projects) {
      map.set(p.path, {
        path: p.path,
        name: p.display_name || deriveFolderName(p.path),
        lastUsed: p.last_used_at ?? 0,
      });
    }
    for (const s of sessions) {
      if (!s.cwd) continue;
      const last = s.last_message_at ?? s.created_at;
      const existing = map.get(s.cwd);
      if (!existing || last > existing.lastUsed) {
        map.set(s.cwd, {
          path: s.cwd,
          name: existing?.name || deriveFolderName(s.cwd),
          lastUsed: last,
        });
      }
    }
    return [...map.values()]
      .sort((a, b) => b.lastUsed - a.lastUsed)
      .slice(0, 5);
  }, [projects, sessions]);

  const handleChooseWorkspace = async (cwd: string) => {
    setPickerOpen(false);
    if (onSelectCwd) {
      onSelectCwd(cwd);
      return;
    }
    if (cwd === effectiveCwd) return;
    const client = getClient();
    if (!client || connectionState !== "connected") return;
    await createAndOpenSession(client, router, {
      cwd,
      errorMessage: "Could not open workspace.",
    });
  };

  return (
    <div className="flex flex-col items-start gap-2.5">
      <div className="flex flex-wrap items-center gap-1.5">
        <button
          type="button"
          onClick={() => {
            getClient()?.listProjects();
            setPickerOpen(true);
          }}
          title={effectiveCwd ?? "Choose a project folder"}
          aria-label={folderName ? `Project folder: ${folderName}` : "Choose project folder"}
          className="motion-pressable inline-flex h-8 items-center gap-1.5 rounded-full border border-border/80 bg-accent/40 px-3 text-xs font-medium text-foreground shadow-2xs transition-colors hover:border-border hover:bg-accent"
        >
          <Folder className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
          <span className="max-w-52 truncate">{folderName ?? "Choose project…"}</span>
          <ChevronDown className="h-3 w-3 shrink-0 text-muted-foreground" />
        </button>

        {recentWorkspaces
          .filter((w) => w.path !== effectiveCwd)
          .slice(0, 3)
          .map((w) => (
            <button
              key={w.path}
              type="button"
              onClick={() => void handleChooseWorkspace(w.path)}
              title={w.path}
              className="motion-pressable inline-flex h-7 items-center gap-1.5 rounded-full border border-border/50 bg-transparent px-2.5 text-[11px] text-muted-foreground transition-colors hover:border-border hover:bg-accent/50 hover:text-foreground"
            >
              <Folder className="h-3 w-3 shrink-0 opacity-70" />
              <span className="max-w-36 truncate">{w.name}</span>
            </button>
          ))}

        {recentWorkspaces.length === 0 && !folderName && (
          <button
            type="button"
            onClick={() => {
              getClient()?.listProjects();
              setPickerOpen(true);
            }}
            className="motion-pressable inline-flex h-7 items-center gap-1 rounded-full px-2 text-[11px] text-muted-foreground hover:bg-accent/50 hover:text-foreground"
          >
            <FolderPlus className="h-3 w-3" />
            <span>Browse folders</span>
          </button>
        )}
      </div>

      <ProjectPickerModal
        open={pickerOpen}
        onOpenChange={setPickerOpen}
        projects={projects}
        onSelect={(cwd) => void handleChooseWorkspace(cwd)}
      />
    </div>
  );
}

function deriveFolderName(cwd: string): string {
  const parts = cwd.replace(/\/+$/, "").split("/");
  return parts[parts.length - 1] || cwd;
}

function EmptyChatSurface() {
  const sessions = useSessionStore((s) => s.availableSessions);
  const titles = useSessionStore((s) => s.sessionTitles);
  const router = useRouter();
  const recent = [...sessions]
    .sort(
      (a, b) =>
        (b.last_message_at ?? b.created_at) -
        (a.last_message_at ?? a.created_at),
    )
    .slice(0, 3);

  return (
    <div className="flex gap-2 px-1 pb-12 pt-16">
      <div className="flex h-full flex-col items-start justify-center text-start">
        <div className="mb-3 flex items-center">
          <h2 className="text-xl font-medium tracking-tight text-foreground">What can we work on?</h2>
        </div>
        <WorkspaceSelectorPill />
        {recent.length > 0 && (
          <div className="mt-6 w-full max-w-sm space-y-1 text-left">
            <div className="mb-1 text-[10px] font-semibold uppercase tracking-wide text-muted-foreground">
              Recent sessions
            </div>
            {recent.map((s) => (
              <button
                key={s.session_id}
                type="button"
                onClick={() => {
                  localStorage.setItem(SESSION_ID_KEY, s.session_id);
                  router.navigate({ to: "/session/$sessionId", params: { sessionId: s.session_id } });
                }}
                className="flex w-full items-center gap-2 rounded-md px-2 py-2 text-left transition-colors hover:bg-accent"
                aria-label={"Open session " + (titles.get(s.session_id) ?? s.title ?? s.first_message ?? s.session_id)}
              >
                <MessageSquare className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
                <span className="flex-1 truncate text-xs text-foreground">
                  {titles.get(s.session_id) ?? s.title ?? s.first_message ?? s.model ?? "Untitled session"}
                </span>
                <span className="text-[10px] text-muted-foreground">
                  {formatRelativeAge(s.last_message_at ?? s.created_at)}
                </span>
              </button>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}

export function ChatSurface() {
  const messages = useSessionStore((s) => s.messages);
  const turns = useMemo(() => buildTurns(messages), [messages]);
  const sessionId = useSessionStore((s) => s.sessionId);
  const sessionIndex = useSessionStore((s) => s.sessionIndex);
  const streamingText = useSessionStore((s) => s.streamingText);
  const streamingReasoningText = useSessionStore(
    (s) => s.streamingReasoningText,
  );
  const compacting = useSessionStore((s) => s.compacting);
  const compactingStartedAt = useSessionStore((s) => s.compactingStartedAt);
  const sessionRunning = useSessionStore(selectSessionRunning);
  const composerHeight = useComposerHeight();
  const scroll = useFollowScroll<HTMLDivElement>([
    turns.length,
    messages,
    streamingText,
    streamingReasoningText,
    compacting,
    composerHeight,
  ]);

  const handleSelectMessageIndex = (index: number) => {
    const el = scroll.ref.current?.querySelector<HTMLElement>(`[data-message-index="${index}"]`);
    if (el && typeof el.scrollIntoView === "function") {
      el.scrollIntoView({ block: "start", behavior: "smooth" });
    }
  };

  return (
    <div className="relative flex min-h-0 min-w-0 flex-1 flex-col">
      {messages.length > 0 && (
        <ConversationMinimap
          messages={messages}
          sessionId={sessionId}
          streamingText={streamingText}
          activeMessageIndex={Math.max(0, messages.length - 1)}
          onSelectMessageIndex={handleSelectMessageIndex}
          sessionIndex={sessionIndex}
        />
      )}
      <div
        ref={scroll.setRef}
        className="chat-scroll-container mew-conversation-gutters min-w-0 flex-1 overflow-x-hidden overflow-y-auto pt-6 pb-[calc(var(--composer-height,0px)_+_3rem)]"
      >
        <div className="mew-conversation-width mx-auto min-w-0 space-y-6">
          {messages.length === 0 && <EmptyChatSurface />}
          {turns.map((turn, i) => (
            <div key={turn.key} data-message-index={turn.messageIndex}>
              <ErrorBoundary
                title="Message failed to render"
                fallback={
                  <div className="max-w-[85%] rounded-lg border border-destructive/50 bg-destructive/10 px-4 py-2.5 text-sm text-destructive">
                    A message in this conversation failed to render.
                  </div>
                }
              >
                <TurnItem turn={turn} isLast={i === turns.length - 1} />
              </ErrorBoundary>
            </div>
          ))}
          {compacting && !sessionRunning && (
            <CompactingIndicator startedAt={compactingStartedAt} />
          )}
        </div>
      </div>
    </div>
  );
}
