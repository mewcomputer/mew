import { createFileRoute, useRouter } from "@tanstack/react-router";
import { useState, useMemo, useEffect } from "react";
import { useSessionStore } from "@/stores/session";
import { getClient } from "@/lib/client";
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
  const sessions = useSessionStore((s) => s.availableSessions);
  const titles = useSessionStore((s) => s.sessionTitles);
  const summaries = useSessionStore((s) => s.sessionSummaries);
  const [query, setQuery] = useState("");
  const [confirmDelete, setConfirmDelete] = useState<string | null>(null);
  const [editingTitle, setEditingTitle] = useState<string | null>(null);
  const [titleDraft, setTitleDraft] = useState("");

  // Refresh session list on mount.
  useEffect(() => {
    const client = getClient();
    client.listSessions().then((list) => {
      useSessionStore.getState().setAvailableSessions(list);
    });
  }, []);

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

  const handleDelete = (sid: string) => {
    getClient().deleteSession(sid);
    // Remove from store immediately.
    useSessionStore.getState().setAvailableSessions(
      sessions.filter((s) => s.session_id !== sid),
    );
    setConfirmDelete(null);

    // If we deleted the current session, go home.
    if (sid === sessionId) {
      router.navigate({ to: "/" });
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
      <div className="flex items-center gap-2 px-3 py-1.5">
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

      <div className="flex-1 overflow-y-auto p-4">
        <div className="mx-auto max-w-2xl space-y-4">
          {/* Search */}
          <div className="relative">
            <Search className="pointer-events-none absolute left-3 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground" />
            <Input
              type="text"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="Search sessions…"
              className="h-9 pl-8"
            />
          </div>

          {/* Session count */}
          <div className="text-[10px] text-muted-foreground">
            {filtered.length} of {sessions.length} sessions
          </div>

          {/* Session list */}
          {filtered.length === 0 && (
            <div className="py-8 text-center text-xs text-muted-foreground">
              {query ? `No sessions match "${query}"` : "No sessions yet"}
            </div>
          )}

          <div className="space-y-1.5">
            {filtered.map((s) => {
              const title = titles.get(s.session_id) ?? s.session_id.slice(0, 12) + "…";
              const isCurrent = s.session_id === sessionId;
              const isConfirming = confirmDelete === s.session_id;
              const isEditing = editingTitle === s.session_id;
              return (
                <div
                  key={s.session_id}
                  className={cn(
                    "flex items-center gap-3 rounded-lg border p-3 transition-colors",
                    isCurrent
                      ? "border-primary/50 bg-accent/50"
                      : "border-border hover:bg-accent/30",
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
                      className="flex min-w-0 flex-1 flex-col gap-0.5 text-left"
                    >
                      <div className="flex items-center gap-2">
                        <MessageSquare className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
                        <span className="truncate text-xs font-medium text-foreground">
                          {title}
                        </span>
                        {isCurrent && (
                          <span className="shrink-0 rounded-full bg-primary/10 px-1.5 py-0.5 text-[9px] font-medium text-primary">
                            current
                          </span>
                        )}
                      </div>
                      <div className="flex items-center gap-2 pl-5.5 text-[10px] text-muted-foreground">
                        <span
                          className={cn(
                            "shrink-0 rounded-full px-1.5 py-0.5 text-[9px] font-medium uppercase",
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
                        <div className="pl-5.5 mt-0.5 truncate text-[10px] italic text-muted-foreground/80">
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
                            className="h-7 text-[10px]"
                            onClick={() => handleDelete(s.session_id)}
                          >
                            Delete
                          </Button>
                          <Button
                            variant="ghost"
                            size="sm"
                            className="h-7 text-[10px]"
                            onClick={() => setConfirmDelete(null)}
                          >
                            Cancel
                          </Button>
                        </>
                      ) : (
                        <Button
                          variant="ghost"
                          size="icon-sm"
                          className="text-muted-foreground hover:text-destructive"
                          onClick={() => setConfirmDelete(s.session_id)}
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
        </div>
      </div>
    </>
  );
}
