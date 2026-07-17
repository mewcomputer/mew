import { useEffect, useState } from "react";
import {
  ChevronRight,
  File as FileIcon,
  Folder as FolderIcon,
  ArrowUp,
  ExternalLink,
  Loader2,
  FileText,
  GitBranch,
} from "lucide-react";
import { useSessionStore } from "../stores/session";
import { getClient } from "../lib/client-ref";
import { cn } from "../lib/utils";
import { CodeBlock } from "./code-block";
import type { DirEntry, GitEntry } from "@mew/web-client";

/** The Files tab: a navigable directory tree with a file-preview pane.
 *
 * On mount it fetches the workspace root via `client.listDir(sessionId)`.
 * Clicking a directory re-fetches with its path; clicking a file calls
 * `readFilePreview` and renders the returned content (highlighted via the
 * existing CodeBlock + shiki path using `filePreview.language`).
 */
export function FileTreePanel({ hasWorkspace }: { hasWorkspace: boolean }) {
  const sessionId = useSessionStore((s) => s.sessionId);
  const dirListing = useSessionStore((s) => s.dirListing);
  const dirListingPath = useSessionStore((s) => s.dirListingPath);
  const filePreview = useSessionStore((s) => s.filePreview);
  const [loading, setLoading] = useState(false);

  // Fetch the root listing on mount / when session changes.
  useEffect(() => {
    const client = getClient();
    if (!client || !sessionId || !hasWorkspace) return;
    setLoading(true);
    client.listDir(sessionId);
    // The dir-listing wire event clears loading indirectly; we use a short
    // timer fallback so the spinner doesn't stick if no response arrives.
    const t = setTimeout(() => setLoading(false), 4000);
    return () => clearTimeout(t);
  }, [sessionId, hasWorkspace]);

  // Clear loading once a listing actually arrives.
  useEffect(() => {
    if (dirListing) setLoading(false);
  }, [dirListing, dirListingPath]);

  const handleOpenDir = (path: string) => {
    const client = getClient();
    const sid = useSessionStore.getState().sessionId;
    if (!client || !sid) return;
    setLoading(true);
    client.listDir(sid, path);
  };

  const handleOpenFile = (path: string) => {
    const client = getClient();
    const sid = useSessionStore.getState().sessionId;
    if (!client || !sid) return;
    client.readFilePreview(sid, path);
  };

  const handleOpenExternal = (path: string) => {
    const client = getClient();
    const sid = useSessionStore.getState().sessionId;
    if (!client || !sid) return;
    client.openPath(sid, path);
  };

  const handleUp = () => {
    if (!dirListingPath) return;
    const parent = dirListingPath.replace(/\/+$/, "").split("/").slice(0, -1).join("/");
    handleOpenDir(parent || "/");
  };

  const entries = dirListing ?? [];

  if (!hasWorkspace) {
    return <WorkspaceEmptyState kind="files" />;
  }

  return (
    <div className="space-y-2">
      {/* Breadcrumb / path + up button */}
      <div className="flex items-center gap-1 text-[10px] text-muted-foreground">
        <button
          onClick={handleUp}
          disabled={!dirListingPath}
          className="rounded p-0.5 hover:bg-accent disabled:opacity-30"
          title="Parent directory"
        >
          <ArrowUp className="h-3 w-3" />
        </button>
        <span className="truncate font-mono">
          {dirListingPath ?? "/"}
        </span>
      </div>

      {/* Directory listing */}
      {loading && entries.length === 0 ? (
        <div className="flex items-center justify-center gap-1.5 py-6 text-[11px] text-muted-foreground">
          <Loader2 className="h-3.5 w-3.5 animate-spin" />
          Loading…
        </div>
      ) : entries.length === 0 ? (
        <div className="py-6 text-center text-[11px] text-muted-foreground">
          Empty directory
        </div>
      ) : (
        <div className="space-y-0.5">
          {sortEntries(entries).map((entry) => (
            <FileTreeRow
              key={entry.name}
              entry={entry}
              onOpenDir={() => handleOpenDir(joinPath(dirListingPath, entry.name))}
              onOpenFile={() => handleOpenFile(joinPath(dirListingPath, entry.name))}
            />
          ))}
        </div>
      )}

      {/* File preview */}
      {filePreview && (
        <div className="mt-2 space-y-1">
          <div className="flex items-center justify-between gap-1 border-t border-border pt-2">
            <span className="flex items-center gap-1 truncate text-[10px] text-muted-foreground">
              <FileText className="h-3 w-3 shrink-0" />
              <span className="truncate font-mono">{filePreview.path.split("/").pop()}</span>
            </span>
            <button
              onClick={() => handleOpenExternal(filePreview.path)}
              className="flex shrink-0 items-center gap-0.5 rounded px-1 py-0.5 text-[9px] text-muted-foreground hover:bg-accent hover:text-foreground"
              title="Open in editor"
            >
              <ExternalLink className="h-2.5 w-2.5" />
            </button>
          </div>
          <div className="max-h-72 overflow-auto rounded-md border border-border">
            <CodeBlock code={filePreview.content} lang={filePreview.language ?? "text"} />
          </div>
          {filePreview.truncated && (
            <span className="text-[9px] italic text-muted-foreground">
              Preview truncated
            </span>
          )}
        </div>
      )}
    </div>
  );
}

function FileTreeRow({
  entry,
  onOpenDir,
  onOpenFile,
}: {
  entry: DirEntry;
  onOpenDir: () => void;
  onOpenFile: () => void;
}) {
  const filePreview = useSessionStore((s) => s.filePreview);
  const isActive = filePreview?.path.split("/").pop() === entry.name;

  if (entry.is_dir) {
    return (
      <button
        onClick={onOpenDir}
        className="flex w-full items-center gap-1 rounded px-1 py-1 text-left text-[11px] hover:bg-accent"
      >
        <ChevronRight className="h-3 w-3 text-muted-foreground" />
        <FolderIcon className="h-3.5 w-3.5 shrink-0 text-blue-500/70" />
        <span className="truncate">{entry.name}</span>
      </button>
    );
  }

  return (
    <button
      onClick={onOpenFile}
      className={cn(
        "flex w-full items-center gap-1 rounded px-1 py-1 text-left text-[11px] hover:bg-accent",
        isActive && "bg-accent",
      )}
    >
      <span className="w-3 shrink-0" />
      <FileIcon className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
      <span className="truncate">{entry.name}</span>
    </button>
  );
}

/** The Changes tab: live git status entries from the daemon. */
export function ChangesPanel({
  gitStatus,
  hasWorkspace,
}: {
  gitStatus: GitEntry[];
  hasWorkspace: boolean;
}) {
  if (!hasWorkspace) {
    return <WorkspaceEmptyState kind="changes" />;
  }

  if (gitStatus.length === 0) {
    return (
      <div className="py-8 text-center text-[11px] text-muted-foreground">
        No changes
      </div>
    );
  }

  return (
    <div className="space-y-0.5">
      <div className="mb-1.5 text-[10px] uppercase tracking-wide text-muted-foreground">
        Working tree ({gitStatus.length})
      </div>
      {gitStatus.map((entry) => (
        <GitStatusRow key={entry.path} entry={entry} />
      ))}
    </div>
  );
}

function WorkspaceEmptyState({ kind }: { kind: "files" | "changes" }) {
  const files = kind === "files";
  return (
    <div className="rounded-lg border border-dashed border-border bg-muted/20 px-4 py-8 text-center">
      <div className="mx-auto flex h-8 w-8 items-center justify-center rounded-full bg-muted text-muted-foreground">
        {files ? <FolderIcon className="h-4 w-4" /> : <GitBranch className="h-4 w-4" />}
      </div>
      <p className="mt-3 text-xs font-medium text-foreground">
        {files ? "No workspace selected" : "No workspace for changes"}
      </p>
      <p className="mx-auto mt-1 max-w-[14rem] text-[11px] leading-relaxed text-muted-foreground">
        {files
          ? "Choose a project when starting a session to browse its files."
          : "Choose a project when starting a session to see its working tree."}
      </p>
    </div>
  );
}

function GitStatusRow({ entry }: { entry: GitEntry }) {
  const { color, icon } = gitStatusMeta(entry.status);
  const handleOpen = () => {
    const client = getClient();
    const sid = useSessionStore.getState().sessionId;
    if (client && sid) client.readFilePreview(sid, entry.path);
  };

  return (
    <button
      onClick={handleOpen}
      className="flex w-full items-center gap-1.5 rounded px-1 py-1 text-left text-[11px] hover:bg-accent"
      title={`Preview ${entry.path}`}
    >
      <span className={cn("w-1.5 shrink-0 rounded-full", color)} />
      <span className={cn("shrink-0 font-mono text-[9px] uppercase", color.replace("bg-", "text-"))}>
        {icon}
      </span>
      <span className="truncate">{entry.path.split("/").pop()}</span>
      <span className="ml-auto truncate text-[9px] text-muted-foreground">
        {entry.path}
      </span>
    </button>
  );
}

// --- helpers ---

function sortEntries(entries: DirEntry[]): DirEntry[] {
  return [...entries].sort((a, b) => {
    if (a.is_dir !== b.is_dir) return a.is_dir ? -1 : 1;
    return a.name.localeCompare(b.name);
  });
}

function joinPath(base: string | null, name: string): string {
  if (!base) return name;
  return `${base.replace(/\/+$/, "")}/${name}`;
}

function gitStatusMeta(status: GitEntry["status"]): {
  color: string;
  icon: string;
} {
  switch (status) {
    case "added":
      return { color: "bg-green-500", icon: "A" };
    case "modified":
      return { color: "bg-amber-500", icon: "M" };
    case "deleted":
      return { color: "bg-red-500", icon: "D" };
    case "renamed":
      return { color: "bg-purple-500", icon: "R" };
    case "untracked":
      return { color: "bg-muted-foreground", icon: "?" };
    default:
      return { color: "bg-muted-foreground", icon: "·" };
  }
}
