import { useEffect } from "react";
import type { MewClient } from "@mew/web-client";
import { useSessionStore } from "../stores/session";
import type { SessionInfo } from "@mew/web-client";
import { cn } from "../lib/utils";

interface SessionRailProps {
  client: MewClient | null;
  collapsed: boolean;
  onToggle: () => void;
}

/** A persistent (collapsible) left rail showing all daemon sessions.
 *
 * Unlike the old drawer, this is always mounted so the user sees their
 * workspace at a glance. It can be collapsed to a narrow strip.
 *
 * Each row shows a derived title, model, client count, and relative age.
 * The active session is marked with a vertical accent bar.
 *
 * If there is a previous non-current session, a "Continue latest" hero
 * button appears at the top for one-click resume.
 */
export function SessionRail({ client, collapsed, onToggle }: SessionRailProps) {
  const sessions = useSessionStore((s) => s.availableSessions);
  const loading = useSessionStore((s) => s.sessionsLoading);
  const currentSessionId = useSessionStore((s) => s.sessionId);

  // Fetch the session list on mount and whenever the client connects.
  useEffect(() => {
    if (client) {
      useSessionStore.getState().setSessionsLoading(true);
      client.listSessions().then((list) => {
        useSessionStore.getState().setAvailableSessions(list);
        useSessionStore.getState().setSessionsLoading(false);
      });
    }
  }, [client]);

  // Also re-fetch when the rail is expanded from collapsed, so the
  // list is fresh after the user has been working in another session.
  useEffect(() => {
    if (client && !collapsed) {
      client.listSessions().then((list) => {
        useSessionStore.getState().setAvailableSessions(list);
      });
    }
  }, [client, collapsed]);

  const handleNewSession = async () => {
    if (!client) return;
    useSessionStore.getState().reset();
    await client.newSession();
  };

  const handleAttach = async (sessionId: string) => {
    if (!client) return;
    useSessionStore.getState().reset();
    try {
      await client.attachSession(sessionId);
    } catch (e) {
      useSessionStore.getState().onError(`Failed to attach: ${e}`);
    }
  };

  // Sort: active first, then idle; newest activity first within each group.
  const sorted = [...sessions].sort((a, b) => {
    if (a.state !== b.state) return a.state === "active" ? -1 : 1;
    const aT = a.last_message_at ?? a.created_at;
    const bT = b.last_message_at ?? b.created_at;
    return bT - aT;
  });

  // "Continue latest" = most recently active session that isn't the current one
  // and isn't empty (heuristic: has a model set, which means at least one turn).
  const continueTarget = sorted.find(
    (s) =>
      s.session_id !== currentSessionId &&
      s.state === "idle" &&
      s.model,
  );

  if (collapsed) {
    return (
      <aside className="flex w-12 shrink-0 flex-col items-center gap-3 border-r border-border bg-background py-3">
        <button
          onClick={onToggle}
          className="rounded-md p-1.5 text-muted-foreground transition-colors hover:bg-accent hover:text-accent-foreground"
          title="Expand sessions"
        >
          <svg className="h-4 w-4" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5">
            <path d="M2 4h12M2 8h12M2 12h12" />
          </svg>
        </button>
        <button
          onClick={handleNewSession}
          disabled={!client}
          className="rounded-md p-1.5 text-muted-foreground transition-colors hover:bg-accent hover:text-accent-foreground disabled:opacity-50"
          title="New session"
        >
          <svg className="h-4 w-4" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="2">
            <path d="M8 3v10M3 8h10" />
          </svg>
        </button>
      </aside>
    );
  }

  return (
    <aside className="flex w-64 shrink-0 flex-col border-r border-border bg-background">
      {/* Header */}
      <div className="flex items-center justify-between border-b border-border px-3 py-2">
        <span className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
          Sessions
        </span>
        <button
          onClick={onToggle}
          className="rounded-md p-1 text-muted-foreground transition-colors hover:bg-accent hover:text-accent-foreground"
          title="Collapse"
        >
          <svg className="h-3.5 w-3.5" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="2">
            <path d="M10 4L6 8l4 4" />
          </svg>
        </button>
      </div>

      {/* New session + continue latest */}
      <div className="space-y-1.5 border-b border-border p-2">
        <button
          onClick={handleNewSession}
          disabled={!client}
          className="flex w-full items-center justify-center gap-1.5 rounded-md bg-primary px-3 py-1.5 text-xs font-medium text-primary-foreground transition-colors hover:bg-primary/90 disabled:opacity-50"
        >
          <svg className="h-3.5 w-3.5" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="2">
            <path d="M8 3v10M3 8h10" />
          </svg>
          New session
        </button>
        {continueTarget && (
          <button
            onClick={() => handleAttach(continueTarget.session_id)}
            className="flex w-full items-center justify-center gap-1.5 rounded-md border border-border bg-secondary px-3 py-1.5 text-xs font-medium text-secondary-foreground transition-colors hover:bg-secondary/80"
            title={`Resume ${deriveTitle(continueTarget)}`}
          >
            <svg className="h-3.5 w-3.5" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="2">
              <path d="M3 8l3-3 3 3M6 5v6h5" />
            </svg>
            Continue latest
          </button>
        )}
      </div>

      {/* Session list */}
      <div className="flex-1 overflow-y-auto p-1.5">
        {loading && sorted.length === 0 && (
          <div className="px-3 py-4 text-center text-xs text-muted-foreground">
            Loading sessions…
          </div>
        )}
        {!loading && sorted.length === 0 && (
          <div className="px-3 py-4 text-center text-xs text-muted-foreground">
            No sessions yet
          </div>
        )}
        {sorted.map((s) => (
          <SessionRow
            key={s.session_id}
            session={s}
            isCurrent={s.session_id === currentSessionId}
            onAttach={() => handleAttach(s.session_id)}
          />
        ))}
      </div>
    </aside>
  );
}

function SessionRow({
  session,
  isCurrent,
  onAttach,
}: {
  session: SessionInfo;
  isCurrent: boolean;
  onAttach: () => void;
}) {
  const isActive = session.state === "active";
  const title = deriveTitle(session);
  const age = formatRelativeAge(session.last_message_at ?? session.created_at);

  return (
    <button
      onClick={onAttach}
      className={cn(
        "group relative mb-0.5 flex w-full flex-col gap-0.5 rounded-md px-2.5 py-1.5 text-left transition-colors hover:bg-accent",
        isCurrent && "bg-accent",
      )}
    >
      {isCurrent && (
        <span className="absolute left-0 top-1/2 h-7 -translate-y-1/2 w-0.5 rounded-r bg-primary" />
      )}
      <div className="flex items-center justify-between gap-1.5">
        <span className="truncate text-xs font-medium text-foreground">
          {title}
        </span>
        <span
          className={cn(
            "shrink-0 rounded-full px-1.5 py-0.5 text-[9px] font-medium uppercase",
            isActive
              ? "bg-green-500/15 text-green-600 dark:text-green-400"
              : "bg-muted text-muted-foreground",
          )}
        >
          {session.state}
        </span>
      </div>
      <div className="flex items-center gap-1.5 text-[10px] text-muted-foreground">
        {session.model && (
          <span className="truncate font-mono">
            {shortModel(session.model)}
          </span>
        )}
        <span className="shrink-0">· {age}</span>
        {isActive && session.client_count > 0 && (
          <span className="shrink-0">· {session.client_count}c</span>
        )}
      </div>
    </button>
  );
}

/** Derive a human-friendly title from session metadata. The daemon doesn't
 *  yet send a title, so we fall back to the model name and a short id. */
function deriveTitle(s: SessionInfo): string {
  if (s.model) {
    // e.g. "glm-4.5-air" from "z-ai/glm-4.5-air"
    const short = s.model.split("/").pop() ?? s.model;
    return short;
  }
  return s.session_id.slice(0, 8);
}

function shortModel(model: string): string {
  return model.split("/").pop() ?? model;
}

function formatRelativeAge(timestamp: number): string {
  const diffMs = Date.now() - timestamp * 1000;
  const sec = Math.floor(diffMs / 1000);
  if (sec < 60) return "now";
  const min = Math.floor(sec / 60);
  if (min < 60) return `${min}m`;
  const hr = Math.floor(min / 60);
  if (hr < 24) return `${hr}h`;
  const day = Math.floor(hr / 24);
  if (day < 7) return `${day}d`;
  return new Date(timestamp * 1000).toLocaleDateString();
}
