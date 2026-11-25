import { createFileRoute, useRouter } from "@tanstack/react-router";
import { FrameTitlebar } from "@/components/workspace-frame";
import { useState, useMemo, useEffect } from "react";
import { useSessionStore } from "@/stores/session";
import { getClient, SESSION_ID_KEY } from "@/lib/client";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { ChevronLeft, Search, Trash2, MessageSquare, Pencil, Check, X } from "lucide-react";
import { formatRelativeAge } from "@/lib/format";

export const Route = createFileRoute("/settings/sessions")({
  component: SessionsRouteComponent,
});

function SessionsRouteComponent() {
  const router = useRouter();
  const sessionId = useSessionStore((s) => s.sessionId);
  const connected = useSessionStore((s) => s.connectionState === "connected");
  const sessions = useSessionStore((s) => s.availableSessions);
  const titles = useSessionStore((s) => s.sessionTitles);
  const summaries = useSessionStore((s) => s.sessionSummaries);
  const [query, setQuery] = useState("");
  const [confirmDelete, setConfirmDelete] = useState<string | null>(null);
  const [deletingSession, setDeletingSession] = useState<string | null>(null);
  const [deleteError, setDeleteError] = useState<string | null>(null);
  const [editingTitle, setEditingTitle] = useState<string | null>(null);
  const [titleDraft, setTitleDraft] = useState("");

  useEffect(() => {
    if (!connected) return;
    const client = getClient();
    void client.listSessions().then((list) => {
      useSessionStore.getState().setAvailableSessions(list);
    }).catch((error: unknown) => {
      if (client.isConnected()) {
        useSessionStore.getState().onError(error instanceof Error ? error.message : "Could not load sessions.");
      }
    });
  }, [connected]);

  const filtered = useMemo(() => {
    const q = query.toLowerCase();
    const sorted = [...sessions].sort((a, b) => {
      const aT = a.last_message_at ?? a.created_at;
      const bT = b.last_message_at ?? b.created_at;
      return bT - aT;
    });
    if (q === "") return sorted;
    return sorted.filter(
      (s) =>
        s.session_id.toLowerCase().includes(q) ||
        (titles.get(s.session_id) ?? "").toLowerCase().includes(q) ||
        (s.model ?? "").toLowerCase().includes(q),
    );
  }, [sessions, titles, query]);

  const handleDelete = async (sid: string) => {
    if (!connected || deletingSession !== null) return;
    setDeletingSession(sid);
    setDeleteError(null);
    try {
      await getClient().deleteSessionConfirmed(sid);
      const store = useSessionStore.getState();
      store.setAvailableSessions(store.availableSessions.filter((session) => session.session_id !== sid));
      if (localStorage.getItem(SESSION_ID_KEY) === sid) localStorage.removeItem(SESSION_ID_KEY);
      setConfirmDelete(null);
      if (store.sessionId === sid) {
        store.clearSession();
        const route = router.state.location.pathname;
        if (route === "/settings/sessions" || route === `/session/${sid}`) void router.navigate({ to: "/" });
      }
    } catch (error) {
      setDeleteError(error instanceof Error ? error.message : "The daemon returned an unexpected error.");
    } finally {
      setDeletingSession(null);
    }
  };

  const handleOpen = (sid: string) => {
    router.navigate({ to: "/session/$sessionId", params: { sessionId: sid } });
  };

  const handleStartEdit = (sid: string) => {
    const current = titles.get(sid) ?? sid.slice(0, 12) + "…";
    setEditingTitle(sid);
    setTitleDraft(current);
  };

  const handleSaveTitle = () => {
    if (!editingTitle || !titleDraft.trim()) {
      setEditingTitle(null);
      return;
    }
    getClient().renameSession(editingTitle, titleDraft.trim());
    // Update local store immediately.
    useSessionStore.getState().onSessionTitleChanged(
      editingTitle,
      titleDraft.trim(),
    );
    setEditingTitle(null);
  };

  return (
    <>
      <FrameTitlebar>
      <div className="mew-conversation-header mew-drag-region flex h-9 shrink-0 items-center gap-2 px-3">
        <Button
          variant="ghost"
          size="icon"
          className="h-7 w-7"
          onClick={() => router.navigate({ to: "/settings" })}
          title="Back to settings"
        >
          <ChevronLeft className="h-4 w-4" />
        </Button>
        <span className="text-xs font-medium text-muted-foreground">
          Sessions
        </span>
      </div>
      </FrameTitlebar>

      <div className="flex-1 overflow-y-auto px-6 pb-8 pt-8">
        <div className="mx-auto max-w-2xl space-y-5">
          {/* Search */}
          <div className="relative">
            <Search className="pointer-events-none absolute left-3 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground" />
            <Input
              type="text"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="Search sessions…"
              className="h-9 rounded-lg pl-8"
            />
          </div>

          {/* Session count */}
          <div className="px-1 text-xs font-medium text-muted-foreground">
            {filtered.length} of {sessions.length} sessions
          </div>
          {deleteError && confirmDelete && (
            <p role="alert" className="px-1 text-xs text-destructive">
              Could not delete {titles.get(confirmDelete) ?? confirmDelete}. {deleteError}
            </p>
          )}

          {/* Session list */}
          {filtered.length === 0 ? (
            <div className="rounded-xl border border-border/80 bg-card/40 py-10 text-center text-xs text-muted-foreground">
              {query ? `No sessions match "${query}"` : "No sessions yet"}
            </div>
          ) : (
            <div className="divide-y divide-border/60 overflow-hidden rounded-xl border border-border/80 bg-card/40">
              {filtered.map((s) => {
                const title = titles.get(s.session_id) ?? s.session_id.slice(0, 12) + "…";
                const isCurrent = s.session_id === sessionId;
                const isConfirming = confirmDelete === s.session_id;
                const isDeleting = deletingSession === s.session_id;
                const isEditing = editingTitle === s.session_id;
                return (
                  <div
                    key={s.session_id}
                    className={cn(
                      "flex items-center gap-3 px-4 py-3 transition-colors duration-150 ease-out",
                      isCurrent ? "bg-accent/50" : "hover:bg-accent/35",
                    )}
                  >
                    {isEditing ? (
                      <div className="flex flex-1 items-center gap-2">
                        <MessageSquare className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
                        <Input
                          autoFocus
                          value={titleDraft}
                          onChange={(e) => setTitleDraft(e.target.value)}
                          onKeyDown={(e) => {
                            if (e.key === "Enter") handleSaveTitle();
                            if (e.key === "Escape") setEditingTitle(null);
                          }}
                          className="h-7 text-xs"
                        />
                        <Button
                          variant="ghost"
                          size="icon-sm"
                          onClick={handleSaveTitle}
                          title="Save"
                        >
                          <Check className="h-3.5 w-3.5" />
                        </Button>
                        <Button
                          variant="ghost"
                          size="icon-sm"
                          onClick={() => setEditingTitle(null)}
                          title="Cancel"
                        >
                          <X className="h-3.5 w-3.5" />
                        </Button>
                      </div>
                    ) : (
                      <button
                        onClick={() => handleOpen(s.session_id)}
                        className="flex min-w-0 flex-1 flex-col gap-1 text-left"
                      >
                        <div className="flex items-center gap-2">
                          <MessageSquare className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
                          <span className="truncate text-[13px] font-medium text-foreground">
                            {title}
                          </span>
                          {isCurrent && (
                            <span className="shrink-0 rounded-full bg-primary/10 px-1.5 py-0.5 text-[10px] font-medium text-primary">
                              current
                            </span>
                          )}
                        </div>
                        <div className="flex items-center gap-2 pl-5.5 text-xs text-muted-foreground">
                          <span
                            className={cn(
                              "shrink-0 rounded-full px-1.5 py-0.5 text-[10px] font-medium uppercase",
                              s.state === "active"
                                ? "bg-green-500/15 text-green-600 dark:text-green-400"
                                : "bg-muted text-muted-foreground",
                            )}
                          >
                            {s.state}
                          </span>
                          {s.model && (
                            <span className="truncate font-mono">
                              {s.model.split("/").pop() ?? s.model}
                            </span>
                          )}
                          <span className="shrink-0">
                            · {formatRelativeAge(s.last_message_at ?? s.created_at)}
                          </span>
                          {s.client_count > 0 && (
                            <span className="shrink-0">· {s.client_count}c</span>
                          )}
                        </div>
                        {summaries.get(s.session_id) && !isEditing && (
                          <div className="mt-0.5 truncate pl-5.5 text-xs italic text-muted-foreground/80">
                            {summaries.get(s.session_id)}
                          </div>
                        )}
                      </button>
                    )}

                    {/* Action buttons: pencil + delete (only when not editing) */}
                    {!isEditing && (
                      <div className="flex shrink-0 items-center gap-1">
                        <Button
                          variant="ghost"
                          size="icon-sm"
                          className="text-muted-foreground"
                          onClick={() => handleStartEdit(s.session_id)}
                          title="Rename session"
                        >
                          <Pencil className="h-3.5 w-3.5" />
                        </Button>
                        {isConfirming ? (
                          <>
                            <Button
                              variant="destructive"
                              size="sm"
                              className="h-7 text-xs"
                              onClick={() => handleDelete(s.session_id)}
                              disabled={!connected || deletingSession !== null}
                              aria-busy={isDeleting}
                            >
                              {isDeleting ? "Deleting…" : "Delete"}
                            </Button>
                            <Button
                              variant="ghost"
                              size="sm"
                              className="h-7 text-xs"
                              onClick={() => {
                                setConfirmDelete(null);
                                setDeleteError(null);
                              }}
                              disabled={isDeleting}
                            >
                              Cancel
                            </Button>
                          </>
                        ) : (
                          <Button
                            variant="ghost"
                            size="icon-sm"
                            className="text-muted-foreground hover:text-destructive"
                            onClick={() => {
                              setConfirmDelete(s.session_id);
                              setDeleteError(null);
                            }}
                            disabled={!connected || deletingSession !== null}
                            title="Delete session"
                          >
                            <Trash2 className="h-3.5 w-3.5" />
                          </Button>
                        )}
                      </div>
                    )}
                  </div>
                );
              })}
            </div>
          )}
        </div>
      </div>
    </>
  );
}
