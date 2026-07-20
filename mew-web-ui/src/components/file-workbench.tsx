import { useEffect, useState } from "react";
import {
  ChevronDown,
  ChevronRight,
  ExternalLink,
  File as FileIcon,
  Folder as FolderIcon,
  FolderOpen,
  Loader2,
  PanelLeft,
  Search,
  WrapText,
  X,
} from "lucide-react";
import type { DirEntry } from "@mew/web-client";
import { useSessionStore } from "../stores/session";
import { getClient } from "../lib/client-ref";
import { cn } from "../lib/utils";
import { CodeBlock } from "./code-block";
import {
  ResizableHandle,
  ResizablePanel,
  ResizablePanelGroup,
} from "./ui/resizable";

type FilePreview = {
  path: string;
  content: string;
  truncated: boolean;
  language?: string;
};

/** A small editor workbench: lazy explorer on the left, read-only documents on the right. */
export function WorkspaceFileWorkbench({ hasWorkspace }: { hasWorkspace: boolean }) {
  const sessionId = useSessionStore((s) => s.sessionId);
  const dirListing = useSessionStore((s) => s.dirListing);
  const dirListingPath = useSessionStore((s) => s.dirListingPath);
  const filePreview = useSessionStore((s) => s.filePreview);
  const sessionCwd = useSessionStore((s) => s.sessionCwd);
  const [directories, setDirectories] = useState<Record<string, DirEntry[]>>({});
  const [expandedPaths, setExpandedPaths] = useState<Set<string>>(() => new Set([""]));
  const [loadingPaths, setLoadingPaths] = useState<Set<string>>(() => new Set());
  const [openPaths, setOpenPaths] = useState<string[]>([]);
  const [activePath, setActivePath] = useState<string | null>(null);
  const [previews, setPreviews] = useState<Record<string, FilePreview>>({});
  const [filter, setFilter] = useState("");
  const [explorerVisible, setExplorerVisible] = useState(true);

  useEffect(() => {
    const client = getClient();
    if (!client || !sessionId || !hasWorkspace) return;
    setDirectories({});
    setExpandedPaths(new Set([""]));
    setLoadingPaths(new Set([""]));
    setOpenPaths([]);
    setActivePath(null);
    setPreviews({});
    setFilter("");
    client.listDir(sessionId);
  }, [sessionId, hasWorkspace]);

  useEffect(() => {
    if (dirListing === null || dirListingPath === null) return;
    const path = normalizeRelativePath(dirListingPath);
    setDirectories((current) => ({ ...current, [path]: dirListing }));
    setLoadingPaths((current) => withoutPath(current, path));
  }, [dirListing, dirListingPath]);

  useEffect(() => {
    if (!filePreview) return;
    setPreviews((current) => ({ ...current, [filePreview.path]: filePreview }));
    setOpenPaths((current) => current.includes(filePreview.path) ? current : [...current, filePreview.path]);
    setActivePath(filePreview.path);
  }, [filePreview]);

  const requestDirectory = (path: string) => {
    const client = getClient();
    const sid = useSessionStore.getState().sessionId;
    if (!client || !sid) return;
    const normalized = normalizeRelativePath(path);
    if (directories[normalized]) return;
    setLoadingPaths((current) => withPath(current, normalized));
    client.listDir(sid, normalized || undefined);
  };

  const toggleDirectory = (path: string) => {
    const normalized = normalizeRelativePath(path);
    setExpandedPaths((current) => {
      const next = new Set(current);
      if (next.has(normalized)) next.delete(normalized);
      else next.add(normalized);
      return next;
    });
    requestDirectory(normalized);
  };

  const openFile = (path: string) => {
    setActivePath(path);
    setOpenPaths((current) => current.includes(path) ? current : [...current, path]);
    if (previews[path]) return;
    const client = getClient();
    const sid = useSessionStore.getState().sessionId;
    if (!client || !sid) return;
    client.readFilePreview(sid, path);
  };

  const openExternal = (path: string) => {
    const client = getClient();
    const sid = useSessionStore.getState().sessionId;
    if (client && sid) client.openPath(sid, path);
  };

  const closeFile = (path: string) => {
    const index = openPaths.indexOf(path);
    const next = openPaths.filter((openPath) => openPath !== path);
    setOpenPaths(next);
    if (activePath === path) setActivePath(next[Math.max(0, index - 1)] ?? next[0] ?? null);
  };

  if (!hasWorkspace) {
    return (
      <div className="flex h-full min-h-48 flex-col items-center justify-center px-6 text-center">
        <FolderOpen className="h-7 w-7 text-muted-foreground/60" />
        <h3 className="mt-3 text-sm font-semibold text-foreground">No workspace selected</h3>
        <p className="mt-1 max-w-[18rem] text-[11px] leading-relaxed text-muted-foreground">
          Choose a project when starting a session to browse its files.
        </p>
      </div>
    );
  }

  return (
    <div className="flex h-full min-h-0 min-w-0 flex-col overflow-hidden bg-background">
      <div className="flex h-10 shrink-0 items-center justify-between gap-3 border-b border-border px-3">
        <div className="flex min-w-0 items-center gap-2">
          <FolderOpen className="h-3.5 w-3.5 shrink-0 text-primary" />
          <span className="truncate text-[11px] font-semibold">{workspaceName(sessionCwd)}</span>
          {activePath && <span className="truncate text-[10px] text-muted-foreground">/ {activePath}</span>}
        </div>
        <button
          type="button"
          onClick={() => setExplorerVisible((visible) => !visible)}
          className={cn(
            "rounded p-1.5 text-muted-foreground transition-colors hover:bg-accent hover:text-foreground",
            explorerVisible && "bg-accent/70 text-foreground",
          )}
          aria-label={explorerVisible ? "Hide file explorer" : "Show file explorer"}
          title={explorerVisible ? "Hide file explorer" : "Show file explorer"}
          aria-pressed={explorerVisible}
        >
          <PanelLeft className="h-3.5 w-3.5" />
        </button>
      </div>

      <ResizablePanelGroup orientation="horizontal" className="min-h-0 min-w-0 flex-1 overflow-hidden">
        {explorerVisible && (
          <>
            <ResizablePanel id="file-explorer" defaultSize="32%" minSize="11rem" maxSize="48%" className="min-w-0">
              <div className="flex h-full min-h-0 flex-col">
                <div className="flex h-10 shrink-0 items-center gap-2 border-b border-border px-3">
                  <Search className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
                  <input
                    value={filter}
                    onChange={(event) => setFilter(event.target.value)}
                    placeholder="Filter files…"
                    aria-label="Filter files"
                    className="min-w-0 flex-1 bg-transparent text-[11px] text-foreground outline-none placeholder:text-muted-foreground"
                  />
                  {filter && (
                    <button
                      type="button"
                      onClick={() => setFilter("")}
                      className="rounded p-0.5 text-muted-foreground hover:bg-accent hover:text-foreground"
                      aria-label="Clear file filter"
                    >
                      <X className="h-3 w-3" />
                    </button>
                  )}
                </div>
                <div className="min-h-0 flex-1 overflow-auto py-2">
                  {directories[""] ? (
                    <ExplorerTree
                      path=""
                      entries={directories[""]}
                      directories={directories}
                      expandedPaths={expandedPaths}
                      loadingPaths={loadingPaths}
                      filter={filter}
                      activePath={activePath}
                      onToggleDirectory={toggleDirectory}
                      onOpenFile={openFile}
                    />
                  ) : (
                    <div className="flex items-center justify-center gap-2 px-3 py-8 text-[11px] text-muted-foreground">
                      <Loader2 className="h-3.5 w-3.5 animate-spin" />
                      Loading workspace…
                    </div>
                  )}
                </div>
              </div>
            </ResizablePanel>
            <ResizableHandle withHandle aria-label="Resize file explorer" />
          </>
        )}

        <ResizablePanel id="file-editor" defaultSize={explorerVisible ? "68%" : "100%"} minSize="12rem" className="min-w-0">
          <div className="flex h-full min-h-0 min-w-0 w-full flex-1 flex-col overflow-hidden">
            <div className="flex min-h-10 shrink-0 items-end overflow-x-auto border-b border-border px-1">
              {openPaths.map((path) => (
                <div key={path} className={cn("group flex h-9 shrink-0 items-center border-b-2", activePath === path ? "border-primary" : "border-transparent")}>
                  <button
                    type="button"
                    onClick={() => setActivePath(path)}
                    className={cn("max-w-44 truncate px-2.5 text-[11px]", activePath === path ? "text-foreground" : "text-muted-foreground hover:text-foreground")}
                    aria-label={`Open ${path}`}
                  >
                    {fileName(path)}
                  </button>
                  <button
                    type="button"
                    onClick={() => closeFile(path)}
                    className="mr-1 rounded p-1 text-muted-foreground opacity-0 hover:bg-accent hover:text-foreground focus-visible:opacity-100 focus-visible:outline-none group-hover:opacity-100"
                    aria-label={`Close ${fileName(path)}`}
                  >
                    <X className="h-3 w-3" />
                  </button>
                </div>
              ))}
            </div>
            {activePath && previews[activePath] ? (
              <FileDocument preview={previews[activePath]} onOpenExternal={() => openExternal(activePath)} />
            ) : (
              <FileEditorEmptyState hasOpenFiles={openPaths.length > 0} />
            )}
          </div>
        </ResizablePanel>
      </ResizablePanelGroup>
    </div>
  );
}

function ExplorerTree({
  path,
  entries,
  directories,
  expandedPaths,
  loadingPaths,
  filter,
  activePath,
  onToggleDirectory,
  onOpenFile,
  depth = 0,
}: {
  path: string;
  entries: DirEntry[];
  directories: Record<string, DirEntry[]>;
  expandedPaths: Set<string>;
  loadingPaths: Set<string>;
  filter: string;
  activePath: string | null;
  onToggleDirectory: (path: string) => void;
  onOpenFile: (path: string) => void;
  depth?: number;
}) {
  const normalizedFilter = filter.trim().toLowerCase();
  const visibleEntries = sortEntries(entries).filter((entry) => (
    !normalizedFilter || entry.name.toLowerCase().includes(normalizedFilter)
  ));

  if (visibleEntries.length === 0) {
    return <div className="px-3 py-6 text-center text-[11px] text-muted-foreground">No matching files</div>;
  }

  return (
    <div>
      {visibleEntries.map((entry) => {
        const entryPath = joinRelativePath(path, entry.name);
        const isExpanded = expandedPaths.has(entryPath);
        const children = directories[entryPath];
        const isLoading = loadingPaths.has(entryPath);
        const isActive = activePath === entryPath;
        return (
          <div key={entryPath}>
            <button
              type="button"
              onClick={() => entry.is_dir ? onToggleDirectory(entryPath) : onOpenFile(entryPath)}
              className={cn(
                "flex w-full items-center gap-1.5 py-1 text-left text-[11px] transition-colors hover:bg-accent/70",
                isActive && "bg-accent text-foreground",
              )}
              style={{ paddingLeft: `${8 + depth * 14}px`, paddingRight: "8px" }}
              aria-expanded={entry.is_dir ? isExpanded : undefined}
            >
              {entry.is_dir ? (
                isExpanded ? <ChevronDown className="h-3 w-3 shrink-0 text-muted-foreground" /> : <ChevronRight className="h-3 w-3 shrink-0 text-muted-foreground" />
              ) : <span className="w-3 shrink-0" />}
              {entry.is_dir ? <FolderIcon className="h-3.5 w-3.5 shrink-0 text-primary/75" /> : <FileIcon className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />}
              <span className="min-w-0 flex-1 truncate">{entry.name}</span>
              {isLoading && <Loader2 className="h-3 w-3 shrink-0 animate-spin text-muted-foreground" />}
            </button>
            {entry.is_dir && isExpanded && children && (
              <ExplorerTree
                path={entryPath}
                entries={children}
                directories={directories}
                expandedPaths={expandedPaths}
                loadingPaths={loadingPaths}
                filter={filter}
                activePath={activePath}
                onToggleDirectory={onToggleDirectory}
                onOpenFile={onOpenFile}
                depth={depth + 1}
              />
            )}
          </div>
        );
      })}
    </div>
  );
}

function FileDocument({ preview, onOpenExternal }: { preview: FilePreview; onOpenExternal: () => void }) {
  const [wrapLines, setWrapLines] = useState(true);

  return (
    <div className="flex min-h-0 min-w-0 w-full flex-1 flex-col overflow-hidden">
      <div className="flex h-9 shrink-0 items-center justify-between gap-2 border-b border-border px-3 text-[10px] text-muted-foreground">
        <span className="truncate font-mono">{preview.path}</span>
        <div className="flex shrink-0 items-center gap-1">
          <button
            type="button"
            onClick={() => setWrapLines((wrapped) => !wrapped)}
            className={cn(
              "flex items-center gap-1 rounded px-1.5 py-1 hover:bg-accent hover:text-foreground",
              wrapLines && "bg-accent text-foreground",
            )}
            title={wrapLines ? "Stop wrapping lines" : "Wrap long lines"}
            aria-label={wrapLines ? "Stop wrapping lines" : "Wrap lines"}
            aria-pressed={wrapLines}
          >
            <WrapText className="h-3 w-3" />
            Wrap
          </button>
          <button
            type="button"
            onClick={onOpenExternal}
            className="flex items-center gap-1 rounded px-1.5 py-1 hover:bg-accent hover:text-foreground"
            title="Open in default editor"
          >
            <ExternalLink className="h-3 w-3" />
            Open externally
          </button>
        </div>
      </div>
      <div className="flex min-h-0 min-w-0 flex-1 flex-col overflow-auto bg-muted/10 py-3">
        <CodeBlock
          code={preview.content}
          lang={preview.language ?? "text"}
          showHeader={false}
          lineNumbers
          wrapLines={wrapLines}
          flush
          fill
        />
        {preview.truncated && <p className="mt-2 px-3 text-[10px] italic text-muted-foreground">Preview truncated</p>}
      </div>
    </div>
  );
}

function FileEditorEmptyState({ hasOpenFiles }: { hasOpenFiles: boolean }) {
  return (
    <div className="flex min-h-0 flex-1 flex-col items-center justify-center px-6 text-center">
      <FolderOpen className="h-7 w-7 text-muted-foreground/60" />
      <h3 className="mt-3 text-sm font-semibold text-foreground">{hasOpenFiles ? "Loading file" : "Open a file"}</h3>
      <p className="mt-1 max-w-[18rem] text-[11px] leading-relaxed text-muted-foreground">
        {hasOpenFiles ? "Fetching a preview from the workspace." : "Select a file from the explorer to open a read-only preview."}
      </p>
    </div>
  );
}

function normalizeRelativePath(path: string): string {
  return path.replace(/^\/+|\/+$/g, "");
}

function joinRelativePath(base: string, name: string): string {
  const normalizedBase = normalizeRelativePath(base);
  return normalizedBase ? `${normalizedBase}/${name}` : name;
}

function withPath(paths: Set<string>, path: string): Set<string> {
  const next = new Set(paths);
  next.add(path);
  return next;
}

function withoutPath(paths: Set<string>, path: string): Set<string> {
  const next = new Set(paths);
  next.delete(path);
  return next;
}

function sortEntries(entries: DirEntry[]): DirEntry[] {
  return [...entries].sort((a, b) => {
    if (a.is_dir !== b.is_dir) return a.is_dir ? -1 : 1;
    return a.name.localeCompare(b.name);
  });
}

function workspaceName(cwd: string | null): string {
  if (!cwd) return "Workspace";
  return cwd.split("/").filter(Boolean).pop() ?? cwd;
}

function fileName(path: string): string {
  return path.split("/").pop() || path;
}
