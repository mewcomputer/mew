import { Folder as FolderIcon, GitBranch } from "lucide-react";
import { useSessionStore } from "../stores/session";
import { getClient } from "../lib/client-ref";
import { cn } from "../lib/utils";
import { WorkspaceFileWorkbench } from "./file-workbench";
import type { GitEntry } from "@mew/web-client";

/** The Files workbench entry point. Changes remains a sibling surface. */
export function FileTreePanel({ hasWorkspace }: { hasWorkspace: boolean }) {
  return <WorkspaceFileWorkbench hasWorkspace={hasWorkspace} />;
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

export function parentPath(path: string | null): string | undefined {
  const normalized = path?.replace(/^\/+|\/+$/g, "") ?? "";
  if (!normalized) return undefined;
  const separator = normalized.lastIndexOf("/");
  return separator === -1 ? undefined : normalized.slice(0, separator) || undefined;
}

export function joinPath(base: string | null, name: string): string {
  const normalizedBase = base?.replace(/^\/+|\/+$/g, "") ?? "";
  return normalizedBase ? `${normalizedBase}/${name}` : name;
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
