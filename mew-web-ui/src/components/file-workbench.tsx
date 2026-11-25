import { useEffect, useRef, useState, type ReactNode } from "react";
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
import { normalizeFileWorkbenchState, type FileWorkbenchState } from "../lib/workbench-tabs";
import { registerCustomTheme } from "@pierre/diffs";
import { File } from "@pierre/diffs/react";
import { createCssVariablesTheme } from "shiki/core";
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
  error?: string;
};

registerCustomTheme("mew-file", async () => createCssVariablesTheme({ name: "mew-file", variablePrefix: "--mew-code-", fontStyle: true }));

/** A small editor workbench: lazy explorer on the left, read-only documents on the right. */
export function WorkspaceFileWorkbench({ hasWorkspace, fileState, onFileStateChange, refreshRequest, headerActions }: {
  hasWorkspace: boolean;
  fileState?: FileWorkbenchState;
  onFileStateChange?: (state: FileWorkbenchState) => void;
  refreshRequest?: { path: string };
  headerActions?: ReactNode;
}) {
  const sessionId = useSessionStore((s) => s.sessionId);
  const dirResults = useSessionStore((s) => s.dirResults);
  const processedDirectories = useRef(new Map<string, { entries: DirEntry[] | null; error: string | null }>());
  const filePreview = useSessionStore((s) => s.filePreview);
  const sessionCwd = useSessionStore((s) => s.sessionCwd);
  const connected = useSessionStore((s) => s.connectionState === "connected");
  const [directories, setDirectories] = useState<Map<string, DirEntry[]>>(() => new Map());
  const [expandedPaths, setExpandedPaths] = useState<Set<string>>(() => new Set([""]));
  const [loadingPaths, setLoadingPaths] = useState<Set<string>>(() => new Set());
  const [directoryErrors, setDirectoryErrors] = useState<Map<string, string>>(() => new Map());
  const [openPaths, setOpenPaths] = useState<string[]>(() => normalizeFileWorkbenchState(fileState).openPaths);
  const [activePath, setActivePath] = useState<string | null>(() => normalizeFileWorkbenchState(fileState).activePath);
  const [previews, setPreviews] = useState<Map<string, FilePreview>>(() => new Map());
  const [filter, setFilter] = useState("");
  const [explorerVisible, setExplorerVisible] = useState(true);
  const [wrapLines, setWrapLines] = useState(false);
  const previewTimers = useRef(new Map<string, ReturnType<typeof setTimeout>>());
  const directoryTimers = useRef(new Map<string, ReturnType<typeof setTimeout>>());

  const loadDirectory = (sid: string, path: string) => {
    const normalized = normalizeRelativePath(path);
    const previous = directoryTimers.current.get(normalized);
    if (previous) clearTimeout(previous);
    setDirectoryErrors((current) => { const next = new Map(current); next.delete(normalized); return next; });
    setLoadingPaths((current) => withPath(current, normalized));
    const fail = (error: string) => {
      directoryTimers.current.delete(normalized);
      setLoadingPaths((current) => withoutPath(current, normalized));
      setDirectoryErrors((current) => new Map(current).set(normalized, error));
    };
    try {
      const client = getClient();
      if (!client) throw new Error("No connection");
      if (normalized) client.listDir(sid, normalized);
      else client.listDir(sid);
      directoryTimers.current.set(normalized, setTimeout(() => fail("Directory listing timed out. Check the connection and retry."), 15_000));
    } catch { fail("Could not load this directory. Check the connection and retry."); }
  };

  const requestPreview = (sid: string, path: string) => {
    const fail = (error: string) => setPreviews((current) => new Map(current).set(path, { path, content: "", truncated: false, error }));
    const previous = previewTimers.current.get(path);
    if (previous) clearTimeout(previous);
    const client = getClient();
    if (!client || useSessionStore.getState().connectionState !== "connected") {
      fail("Disconnected. Reconnect to load this file.");
      return;
    }
    try {
      client.readFilePreview(sid, path);
      previewTimers.current.set(path, setTimeout(() => {
        previewTimers.current.delete(path);
        fail("File preview timed out. Check the connection or update the daemon.");
      }, 15_000));
    } catch {
      fail("Could not request the file preview. Check the connection and retry.");
    }
  };

  useEffect(() => {
    const client = getClient();
    if (!sessionId || !hasWorkspace) return;
    if (!client || !connected) {
      if (activePath) requestPreview(sessionId, activePath);
      return;
    }
    useSessionStore.setState({ dirListing: null, dirListingPath: null, dirListingError: null, dirResults: new Map() });
    processedDirectories.current.clear();
    setDirectories(new Map());
    setExpandedPaths(new Set([""]));
    setLoadingPaths(new Set());
    setDirectoryErrors(new Map());
    setPreviews(new Map());
    setFilter("");
    loadDirectory(sessionId, "");
    if (activePath) requestPreview(sessionId, activePath);
    return () => {
      for (const timer of previewTimers.current.values()) clearTimeout(timer);
      previewTimers.current.clear();
      for (const timer of directoryTimers.current.values()) clearTimeout(timer);
      directoryTimers.current.clear();
    };
  }, [sessionId, hasWorkspace, connected]);

  useEffect(() => {
    if (!fileState) return;
    const { openPaths: paths, activePath: selected } = normalizeFileWorkbenchState(fileState);
    if (paths.length !== openPaths.length || paths.some((path, index) => path !== openPaths[index])) setOpenPaths(paths);
    if (selected !== activePath) {
      setActivePath(selected);
      if (selected && !previews.has(selected) && sessionId) requestPreview(sessionId, selected);
    }
  }, [fileState]);

  useEffect(() => {
    if (!refreshRequest || !sessionId) return;
    const { path } = refreshRequest;
    setPreviews((current) => { const next = new Map(current); next.delete(path); return next; });
    requestPreview(sessionId, path);
  }, [refreshRequest]);

  useEffect(() => {
    if (useSessionStore.getState().dirResults !== dirResults) return;
    for (const [rawPath, result] of dirResults) {
      if (processedDirectories.current.get(rawPath) === result) continue;
      processedDirectories.current.set(rawPath, result);
      const path = normalizeRelativePath(rawPath);
      const timer = directoryTimers.current.get(path);
      if (timer) clearTimeout(timer);
      directoryTimers.current.delete(path);
      setLoadingPaths((current) => withoutPath(current, path));
      if (result.error) {
        setDirectoryErrors((current) => new Map(current).set(path, result.error!));
      } else if (result.entries) {
        setDirectories((current) => new Map(current).set(path, result.entries!));
        setDirectoryErrors((current) => { const next = new Map(current); next.delete(path); return next; });
      }
    }
  }, [dirResults]);

  useEffect(() => {
    if (filePreview) {
      const timer = previewTimers.current.get(filePreview.path);
      if (timer) clearTimeout(timer);
      previewTimers.current.delete(filePreview.path);
    }
    if (!filePreview || !openPaths.includes(filePreview.path)) return;
    setPreviews((current) => new Map(current).set(filePreview.path, filePreview));
  }, [filePreview]);

  const requestDirectory = (path: string) => {
    const client = getClient();
    const sid = useSessionStore.getState().sessionId;
    if (!client || !sid || !connected) return;
    const normalized = normalizeRelativePath(path);
    if (directories.has(normalized) && !directoryErrors.has(normalized)) return;
    loadDirectory(sid, normalized);
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
    const next = openPaths.includes(path) ? openPaths : [...openPaths, path];
    setActivePath(path);
    setOpenPaths(next);
    onFileStateChange?.({ openPaths: next, activePath: path });
    if (previews.has(path)) return;
    const sid = useSessionStore.getState().sessionId;
    if (sid) requestPreview(sid, path);
  };

  const openExternal = (path: string) => {
    const client = getClient();
    const sid = useSessionStore.getState().sessionId;
    if (client && sid) client.openPath(sid, path);
  };

  const closeFile = (path: string) => {
    const index = openPaths.indexOf(path);
    const next = openPaths.filter((openPath) => openPath !== path);
    const timer = previewTimers.current.get(path);
    if (timer) clearTimeout(timer);
    previewTimers.current.delete(path);
    setOpenPaths(next);
    const nextActive = activePath === path ? next[Math.max(0, index - 1)] ?? next[0] ?? null : activePath;
    setActivePath(nextActive);
    onFileStateChange?.({ openPaths: next, activePath: nextActive });
    if (nextActive && !previews.has(nextActive) && sessionId) requestPreview(sessionId, nextActive);
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
      <div className="flex h-8 shrink-0 items-center justify-between gap-2 border-b border-border px-2">
        <div className="flex min-w-0 items-center gap-2">
          <FolderOpen className="h-3.5 w-3.5 shrink-0 text-primary" />
          <span className="truncate text-[11px] font-semibold">{workspaceName(sessionCwd)}</span>
          {activePath && <span className="truncate text-[10px] text-muted-foreground">/ {activePath}</span>}
        </div>
        <div className="flex shrink-0 items-center gap-1">
        {headerActions}
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
      </div>

      <ResizablePanelGroup orientation="horizontal" className="min-h-0 min-w-0 flex-1 overflow-hidden">
        {explorerVisible && (
          <>
            <ResizablePanel id="file-explorer" defaultSize="28%" minSize="8rem" maxSize="42%" className="min-w-0">
              <div className="flex h-full min-h-0 flex-col">
                <div className="flex h-8 shrink-0 items-center gap-2 border-b border-border px-2">
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
                  {directoryErrors.has("") ? (
                    <DirectoryError path="" error={directoryErrors.get("")!} onRetry={requestDirectory} />
                  ) : directories.has("") ? (
                    <ExplorerTree
                      path=""
                      entries={directories.get("")!}
                      directories={directories}
                      expandedPaths={expandedPaths}
                      loadingPaths={loadingPaths}
                      directoryErrors={directoryErrors}
                      onRetryDirectory={requestDirectory}
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
            <div className="flex h-8 shrink-0 items-center border-b border-border px-1">
              <div className="flex min-w-0 flex-1 items-end overflow-x-auto">
              {openPaths.map((path) => (
                <div key={path} className={cn("group flex h-8 shrink-0 items-center border-b-2", activePath === path ? "border-primary" : "border-transparent")}>
                  <button
                    type="button"
                    onClick={() => openFile(path)}
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
              {activePath && previews.has(activePath) && !previews.get(activePath)?.error && <div className="flex shrink-0 items-center gap-0.5">
                <button type="button" onClick={() => setWrapLines((wrapped) => !wrapped)}
                  className={cn("rounded p-1.5 text-muted-foreground hover:bg-accent hover:text-foreground", wrapLines && "bg-accent text-foreground")}
                  title={wrapLines ? "Stop wrapping lines" : "Wrap long lines"} aria-label={wrapLines ? "Stop wrapping lines" : "Wrap lines"} aria-pressed={wrapLines}>
                  <WrapText className="h-3 w-3" />
                </button>
                <button type="button" onClick={() => openExternal(activePath)} className="rounded p-1.5 text-muted-foreground hover:bg-accent hover:text-foreground" title="Open in default editor" aria-label="Open in default editor">
                  <ExternalLink className="h-3 w-3" />
                </button>
              </div>}
            </div>
            {activePath && previews.get(activePath)?.error ? (
              <div className="flex min-h-0 flex-1 flex-col items-center justify-center gap-2 px-6 text-center text-xs">
                <p>Could not open {activePath}</p>
                <p className="text-muted-foreground">{previews.get(activePath)?.error}</p>
                <button type="button" className="rounded px-2 py-1 hover:bg-accent" onClick={() => {
                  setPreviews((current) => { const next = new Map(current); next.delete(activePath); return next; });
                  if (sessionId) requestPreview(sessionId, activePath);
                }}>Retry</button>
              </div>
            ) : activePath && previews.has(activePath) ? (
              <FileDocument key={activePath} preview={previews.get(activePath)!} wrapLines={wrapLines} />
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
  directoryErrors,
  onRetryDirectory,
  filter,
  activePath,
  onToggleDirectory,
  onOpenFile,
  depth = 0,
}: {
  path: string;
  entries: DirEntry[];
  directories: Map<string, DirEntry[]>;
  expandedPaths: Set<string>;
  loadingPaths: Set<string>;
  directoryErrors: Map<string, string>;
  onRetryDirectory: (path: string) => void;
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
        const children = directories.get(entryPath);
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
            {entry.is_dir && isExpanded && directoryErrors.has(entryPath) && (
              <DirectoryError path={entryPath} error={directoryErrors.get(entryPath)!} onRetry={onRetryDirectory} />
            )}
            {entry.is_dir && isExpanded && children && (
              <ExplorerTree
                path={entryPath}
                entries={children}
                directories={directories}
                expandedPaths={expandedPaths}
                loadingPaths={loadingPaths}
                directoryErrors={directoryErrors}
                onRetryDirectory={onRetryDirectory}
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

function DirectoryError({ path, error, onRetry }: { path: string; error: string; onRetry: (path: string) => void }) {
  return <div role="alert" className="px-3 py-3 text-center text-[11px] text-muted-foreground">
    <p>{error}</p>
    <button type="button" className="mt-1 rounded px-2 py-1 text-foreground hover:bg-accent focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring" aria-label={`Retry ${path || "workspace"}`} onClick={() => onRetry(path)}>Retry</button>
  </div>;
}

function FileDocument({ preview, wrapLines }: { preview: FilePreview; wrapLines: boolean }) {
  return (
    <div className="flex min-h-0 min-w-0 w-full flex-1 flex-col overflow-hidden">
      <div className="flex min-h-0 min-w-0 flex-1 flex-col overflow-auto bg-muted/10 py-2">
        <File
          className="mew-file-preview"
          file={{ name: preview.path, contents: preview.content }}
          options={{ theme: "mew-file", disableFileHeader: true, overflow: wrapLines ? "wrap" : "scroll" }}
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
