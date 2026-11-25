import { Folder as FolderIcon, GitBranch } from "lucide-react";
import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { parsePatchFiles } from "@pierre/diffs";
import { FileDiff } from "@pierre/diffs/react";
import { getClient } from "../lib/client-ref";
import { ResizableHandle, ResizablePanel, ResizablePanelGroup } from "./ui/resizable";
import { FileTree, useFileTree } from "@pierre/trees/react";
import { WorkspaceFileWorkbench } from "./file-workbench";
import type { GitEntry, WorkspaceFileDiff } from "@mew/web-client";
import type { FileWorkbenchState } from "../lib/workbench-tabs";

/** The Files workbench entry point. Changes remains a sibling surface. */
export function FileTreePanel({ hasWorkspace, fileState, onFileStateChange, refreshRequest, headerActions }: {
  hasWorkspace: boolean;
  fileState?: FileWorkbenchState;
  onFileStateChange?: (state: FileWorkbenchState) => void;
  refreshRequest?: { path: string };
  headerActions?: ReactNode;
}) {
  return <WorkspaceFileWorkbench hasWorkspace={hasWorkspace} fileState={fileState} onFileStateChange={onFileStateChange} refreshRequest={refreshRequest} headerActions={headerActions} />;
}

/** The Changes tab: live git status entries from the daemon. */
export function ChangesPanel({
  gitStatus,
  sessionId,
  connected,
  statusError,
  statusLoaded,
  hasWorkspace,
  onOpenFile,
}: {
  gitStatus: GitEntry[];
  sessionId?: string | null;
  connected?: boolean;
  statusError?: string | null;
  statusLoaded?: boolean;
  hasWorkspace: boolean;
  onOpenFile: (path: string) => void;
}) {
  if (!hasWorkspace) {
    return <WorkspaceEmptyState kind="changes" />;
  }

  if (statusError) return <div role="alert" className="px-2 py-6 text-center text-xs text-muted-foreground">{statusError}</div>;

  if (statusLoaded === false) return <div className="py-8 text-center text-[11px] text-muted-foreground">Loading changes…</div>;

  if (gitStatus.length === 0) {
    return (
      <div className="py-8 text-center text-[11px] text-muted-foreground">
        No changes
      </div>
    );
  }

  return <ChangesWorkspace gitStatus={gitStatus} sessionId={sessionId} connected={connected} onOpenFile={onOpenFile} />;
}

function ChangesWorkspace({ gitStatus, sessionId, connected, onOpenFile }: {
  gitStatus: GitEntry[]; sessionId?: string | null; connected?: boolean; onOpenFile: (path: string) => void;
}) {
  const [selected, setSelected] = useState<string | null>(null);
  const [refresh, setRefresh] = useState(0);
  const [result, setResult] = useState<{ path: string; files?: WorkspaceFileDiff[]; error?: string } | null>(null);

  useEffect(() => { setSelected(null); setResult(null); }, [sessionId]);
  useEffect(() => {
    if (selected && !gitStatus.some((entry) => entry.path === selected)) { setSelected(null); setResult(null); }
  }, [gitStatus, selected]);
  useEffect(() => {
    const client = getClient();
    if (!client || !sessionId || !selected || !connected) return;
    const requestId = crypto.randomUUID();
    let loading = true;
    let dirty = false;
    const finished = () => {
      loading = false;
      if (dirty) setRefresh((value) => value + 1);
    };
    setResult(null);
    const timer = setTimeout(() => {
      loading = false;
      dirty = false;
      setResult({ path: selected, error: "Loading the diff timed out. Check the connection or update the daemon." });
    }, 15_000);
    const onResult = (data: { session_id: string; request_id: string; files: WorkspaceFileDiff[] }) => {
      if (data.session_id !== sessionId || data.request_id !== requestId) return;
      clearTimeout(timer);
      finished();
      const files = data.files.filter((item) => item.path === selected);
      setResult(files.length ? { path: selected, files } : { path: selected, error: "This file no longer has changes." });
    };
    const onError = (data: { session_id: string; request_id: string; message: string }) => {
      if (data.session_id === sessionId && data.request_id === requestId) {
        clearTimeout(timer);
        finished();
        setResult({ path: selected, error: data.message });
      }
    };
    const onLegacyError = (data: { message: string }) => {
      if (!data.message.includes("read_session_file_diff") || !data.message.includes("unknown variant")) return;
      clearTimeout(timer);
      setResult({ path: selected, error: "Please update the daemon to view selected diffs." });
    };
    const onChanged = (data: { session_id?: string; paths: string[] }) => {
      if (data.session_id && data.session_id !== sessionId) return;
      if (!data.paths.some((path) => path === selected || path.startsWith(".git/"))) return;
      if (loading) dirty = true;
      else setRefresh((value) => value + 1);
    };
    client.on("session-diff", onResult);
    client.on("session-diff-error", onError);
    client.on("fs-changed", onChanged);
    client.on("errorMessage", onLegacyError);
    try { client.readSessionDiff(sessionId, requestId, selected); }
    catch { clearTimeout(timer); setResult({ path: selected, error: "Could not request the diff. Check the connection." }); }
    return () => {
      clearTimeout(timer);
      client.off("session-diff", onResult);
      client.off("session-diff-error", onError);
      client.off("fs-changed", onChanged);
      client.off("errorMessage", onLegacyError);
    };
  }, [sessionId, selected, connected, refresh]);

  return <ResizablePanelGroup orientation="horizontal" className="min-h-0 min-w-0 flex-1">
    <ResizablePanel defaultSize="32%" minSize="9rem" className="min-w-0 overflow-auto">
      <ChangedFilesTree gitStatus={gitStatus} selected={selected} onOpenFile={setSelected} />
    </ResizablePanel>
    <ResizableHandle withHandle aria-label="Resize changes tree" />
    <ResizablePanel defaultSize="68%" minSize="12rem" className="min-w-0">
      <DiffPreview path={selected} result={result} connected={!!connected} onOpenFile={onOpenFile} onRetry={() => setRefresh((value) => value + 1)} />
    </ResizablePanel>
  </ResizablePanelGroup>;
}

function DiffPreview({ path, result, connected, onOpenFile, onRetry }: {
  path: string | null; result: { path: string; files?: WorkspaceFileDiff[]; error?: string } | null; connected: boolean;
  onOpenFile: (path: string) => void; onRetry: () => void;
}) {
  if (!path) return <div className="flex h-full items-center justify-center px-4 text-center text-xs text-muted-foreground">Select a changed file to see its diff</div>;
  const error = result?.path === path ? result.error : undefined;
  const files = result?.path === path ? result.files : undefined;
  return <div className="flex h-full min-h-0 min-w-0 flex-col">
    <div className="flex h-8 shrink-0 items-center gap-2 border-b border-border px-3">
      <span className="min-w-0 flex-1 truncate text-[11px] font-medium" title={path}>{path}</span>
      <button type="button" onClick={() => onOpenFile(path)} className="rounded px-2 py-1 text-[11px] text-muted-foreground hover:bg-accent hover:text-foreground">Open file</button>
    </div>
    {!connected ? <div className="p-4 text-xs text-muted-foreground">Disconnected. Reconnect to load the diff.</div>
      : error ? <div role="alert" className="p-4 text-xs text-muted-foreground"><p>{error}</p><button type="button" onClick={onRetry} className="mt-2 rounded px-2 py-1 text-foreground hover:bg-accent">Retry</button></div>
      : !files ? <div className="p-4 text-xs text-muted-foreground">Loading diff…</div>
      : <div className="min-h-0 flex-1 overflow-auto">{files.map((file, index) => <DiffSection key={`${file.layer ?? "combined"}-${index}`} file={file} />)}</div>}
  </div>;
}

function DiffSection({ file }: { file: WorkspaceFileDiff }) {
  const parsed = useMemo(() => {
    if (file.patch) {
      try { return parsePatchFiles(file.patch, { throwOnError: true })[0]?.files[0] ?? null; }
      catch { return null; }
    }
    if (file.hunks.length === 0) return null;
    const basename = file.path.split("/").at(-1) ?? "";
    const extension = file.path.match(/\.[a-zA-Z0-9]+$/)?.[0] ?? "";
    const name = /^[a-zA-Z0-9._-]+$/.test(basename) ? basename : `preview${extension}`;
    const patch = [`diff --git a/${name} b/${name}`, `--- a/${name}`, `+++ b/${name}`,
      ...file.hunks.flatMap((hunk) => [
        `@@ -${hunk.old_start},${hunk.old_count} +${hunk.new_start},${hunk.new_count} @@`,
        ...hunk.lines.flatMap((line) => [
          `${line.kind === "Addition" ? "+" : line.kind === "Deletion" ? "-" : " "}${line.text}`,
          ...(line.ending === "None" ? ["\\ No newline at end of file"] : []),
        ]),
      ]), ""].join("\n");
    try { return parsePatchFiles(patch, { throwOnError: true })[0]?.files[0] ?? null; }
    catch { return null; }
  }, [file]);
  const modeChanged = !!file.patch && /^old mode [0-7]+$/m.test(file.patch) && /^new mode [0-7]+$/m.test(file.patch);
  const binaryPatch = !!file.patch && /^Binary files .+ differ$/m.test(file.patch);
  const endings = new Set(file.hunks.flatMap((hunk) => hunk.lines.filter((line) => line.kind !== "Context").map((line) => line.ending).filter(Boolean)));
  return <section aria-label={file.layer ?? "Changes"}>
    {file.layer && <div className="border-b border-border px-3 py-1.5 text-[11px] font-medium text-muted-foreground">{file.layer}</div>}
    {endings.size > 1 && <div className="px-3 py-2 text-[11px] text-muted-foreground">Line endings differ in this change.</div>}
    {file.old_path && file.old_path !== file.path && <div className="px-3 py-2 text-[11px] text-muted-foreground">Renamed from {file.old_path}</div>}
    {modeChanged && <div className="px-3 py-2 text-[11px] text-muted-foreground">File mode changed.</div>}
    {file.status === "Binary" || binaryPatch ? <div className="p-4 text-xs text-muted-foreground">Binary file changes cannot be displayed.</div>
      : !parsed && (file.patch || file.hunks.length > 0) ? <pre className="overflow-auto whitespace-pre-wrap p-3 text-xs text-muted-foreground">{file.patch || "Could not display this diff."}</pre>
      : !parsed || parsed.hunks.length === 0 ? <div className="p-4 text-xs text-muted-foreground">No text changes to display.</div>
      : <FileDiff className="mew-file-diff" fileDiff={parsed} options={{ theme: "mew-file", disableFileHeader: true, diffStyle: "unified", diffIndicators: "classic" }} />}
  </section>;
}

function ChangedFilesTree({ gitStatus, selected, onOpenFile }: { gitStatus: GitEntry[]; selected: string | null; onOpenFile: (path: string) => void }) {
  const openRef = useRef(onOpenFile);
  openRef.current = onOpenFile;
  const { model } = useFileTree({
    paths: gitStatus.map((entry) => entry.path),
    gitStatus,
    density: "compact",
    initialExpansion: "open",
    onSelectionChange: (paths) => {
      const path = paths.at(-1);
      if (!path || path.endsWith("/")) return;
      openRef.current(path);
    },
  });
  useEffect(() => {
    const expanded = model.getVisibleRows(0, model.getVisibleCount()).filter((row) => row.kind === "directory" && row.isExpanded).map((row) => row.path);
    model.resetPaths(gitStatus.map((entry) => entry.path), { initialExpandedPaths: expanded });
    model.setGitStatus(gitStatus);
    if (selected && !model.getItem(selected)?.isSelected()) model.getItem(selected)?.select();
  }, [gitStatus, model, selected]);
  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="px-1 pb-2 text-[10px] uppercase tracking-wide text-muted-foreground">Working tree ({gitStatus.length})</div>
      <FileTree model={model} className="mew-changes-tree min-h-0 flex-1" aria-label="Changed files" />
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
