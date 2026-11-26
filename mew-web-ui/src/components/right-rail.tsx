import { lazy, Suspense, useState, useEffect, useRef, type ReactNode } from "react";
import {
  Folder,
  GitBranch,
  GitPullRequest,
  Loader2,
  Pin,
  Terminal,
  Plus,
  X,
  XCircle,
  Globe,
  type LucideIcon,
} from "lucide-react";
import { useSessionStore, type SubagentInfo, type JobInfo } from "../stores/session";
import { cn } from "../lib/utils";
import type { GitEntry } from "@mew/web-client";
import { useIsMobile } from "../hooks/use-mobile";
import {
  createWorkbenchTab,
  DEFAULT_WORKBENCH_TABS,
  getActiveWorkbenchTab,
  workbenchTabsReducer,
  type WorkbenchTab,
  type WorkbenchTabsAction,
  type WorkbenchTabKind,
  type WorkbenchTabsState,
} from "../lib/workbench-tabs";
import { getClient } from "../lib/client-ref";
import { isElectronHost } from "../lib/host";
import { FileTreePanel, ChangesPanel } from "./file-tree";
import { isMarkdownPath } from "./file-workbench";
import { BrowserPanel } from "./browser-panel";
import { TerminalPanel } from "./terminal-panel";
import { ArtifactFrame, collectArtifactVersions, isSafeArtifactUrl } from "./artifact-frame";
import { Sheet, SheetContent, SheetTitle } from "@/components/ui/sheet";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { FrameWorkbenchTitlebar, useOptionalWorkspaceFrame } from "./workspace-frame";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { SubagentIdenticon } from "./subagent-identicon";

const SubagentConversation = lazy(() => import("./subagent-conversation"));

const NEW_TAB_TOOLS: ReadonlyArray<{
  kind: WorkbenchTabKind;
  label: string;
  description: string;
  icon: LucideIcon;
}> = [
  { kind: "changes", label: "Changes", description: "Inspect the active working tree", icon: GitBranch },
  { kind: "terminal", label: "Terminal", description: "Interactive shell in the active workspace", icon: Terminal },
  { kind: "file", label: "Files", description: "Browse the active project files", icon: Folder },
  { kind: "review", label: "Review", description: "Inspect and review working-tree changes", icon: GitPullRequest },
];

interface RightRailProps {
  open: boolean;
  present?: boolean;
  onOpenChange: (open: boolean) => void;
  mode?: "sheet" | "dock";
  workbenchTabs?: WorkbenchTabsState;
  workbenchSessionId?: string;
  sessionTabs?: Record<string, WorkbenchTabsState>;
  onWorkbenchTabsChange?: (tabs: WorkbenchTabsState) => void;
  onWorkbenchTabsAction?: (action: WorkbenchTabsAction, sessionId?: string) => void;
}

export function RightRail({
  open,
  present = open,
  onOpenChange,
  mode = "sheet",
  workbenchTabs = DEFAULT_WORKBENCH_TABS,
  workbenchSessionId,
  sessionTabs,
  onWorkbenchTabsChange,
  onWorkbenchTabsAction,
}: RightRailProps) {
  const isMobile = useIsMobile();
  const frame = useOptionalWorkspaceFrame();
  const inFrameTitlebar = mode === "dock" && !isMobile && Boolean(frame?.workbenchTitlebar);
  const tabList = useRef<HTMLDivElement>(null);
  const [uncontrolledWorkbenchTabs, setUncontrolledWorkbenchTabs] = useState(DEFAULT_WORKBENCH_TABS);
  const [fileRefresh, setFileRefresh] = useState<{ tabId: string; owner?: string; path: string }>();
  const selectedWorkbenchTabs = onWorkbenchTabsChange || onWorkbenchTabsAction
    ? workbenchTabs
    : uncontrolledWorkbenchTabs;
  const selectedWorkbenchTab = getActiveWorkbenchTab(selectedWorkbenchTabs);
  useEffect(() => {
    tabList.current?.querySelector<HTMLElement>('[data-state="active"]')?.scrollIntoView({ block: "nearest", inline: "nearest" });
  }, [selectedWorkbenchTab?.id, open]);
  const setSelectedWorkbenchTabs = (next: WorkbenchTabsState) => {
    if (onWorkbenchTabsChange) onWorkbenchTabsChange(next);
    else setUncontrolledWorkbenchTabs(next);
  };
  const applyWorkbenchAction = (action: WorkbenchTabsAction, owner?: string) => {
    if (onWorkbenchTabsAction) onWorkbenchTabsAction(action, owner);
    else setSelectedWorkbenchTabs(workbenchTabsReducer(selectedWorkbenchTabs, action));
  };
  const subagents = useSessionStore((s) => s.subagents);
  const flaggedFiles = useSessionStore((s) => s.flaggedFiles);
  const jobs = useSessionStore((s) => s.jobs);
  const connected = useSessionStore((s) => s.connectionState === "connected");
  const confirmedSessionId = useSessionStore((s) => s.confirmedSessionId);
  const gitStatus = useSessionStore((s) => s.gitStatus);
  const gitStatusError = useSessionStore((s) => s.gitStatusError);
  const gitStatusLoaded = useSessionStore((s) => s.gitStatusLoaded);
  const sessionId = useSessionStore((s) => s.sessionId);
  const sessionCwd = useSessionStore((s) => s.sessionCwd);
  const availableSessions = useSessionStore((s) => s.availableSessions);
  const sessionScratch = useSessionStore((s) => s.sessionScratch);
  const electron = isElectronHost();
  const currentOwner = workbenchSessionId ?? sessionId ?? undefined;
  const tabValue = (id: string, owner = currentOwner) => electron ? `${owner}/${id}` : id;
  const renderedTabs = [
    ...selectedWorkbenchTabs.tabs.map((tab) => ({ tab, owner: currentOwner })),
    ...(electron ? Object.entries(sessionTabs ?? {}).filter(([owner]) => owner !== currentOwner).flatMap(([owner, tabs]) => tabs.tabs.filter((tab) => tab.kind === "browser").map((tab) => ({ tab, owner }))) : []),
  ];
  // Moving an attached webview through the DOM can destroy its guest process.
  if (electron) renderedTabs.sort((a, b) => tabValue(a.tab.id, a.owner).localeCompare(tabValue(b.tab.id, b.owner)));
  const activeSession = availableSessions.find((s) => s.session_id === sessionId);
  const sessionHasWorkspace = Boolean(
    sessionCwd || activeSession?.cwd || activeSession?.scratch || sessionScratch,
  );

  const addWorkbenchTab = (kind: WorkbenchTabKind) => {
    const tab = createWorkbenchTab(kind, {
      sessionId: sessionId ?? undefined,
      cwd: sessionCwd ?? undefined,
    });
    applyWorkbenchAction({ type: "add", tab });
  };

  const openAddWorkbenchMenu = () => {
    addWorkbenchTab("browser");
  };

  const openChangedFile = (path: string, fromLink = false) => {
    const existing = selectedWorkbenchTabs.tabs.find((tab) => tab.kind === "file");
    const paths = Array.isArray(existing?.payload?.files?.openPaths) ? existing.payload.files.openPaths : [];
    const files = { ...existing?.payload?.files, openPaths: paths.includes(path) ? paths : [...paths, path], activePath: path, ...(!existing && fromLink && isMarkdownPath(path) ? { explorerVisible: false } : {}) };
    if (existing && paths.includes(path)) setFileRefresh({ tabId: existing.id, owner: currentOwner, path });
    if (!existing) {
      const tab = createWorkbenchTab("file", {
        sessionId: sessionId ?? undefined, cwd: sessionCwd ?? undefined, payload: { files },
      });
      applyWorkbenchAction({ type: "add", tab });
      return;
    }
    const update: WorkbenchTabsAction = { type: "update", id: existing.id, patch: { payload: { ...existing.payload, files } } };
    const select: WorkbenchTabsAction = { type: "select", id: existing.id };
    if (onWorkbenchTabsAction) {
      applyWorkbenchAction(update);
      applyWorkbenchAction(select);
    } else {
      setSelectedWorkbenchTabs(workbenchTabsReducer(workbenchTabsReducer(selectedWorkbenchTabs, update), select));
    }
  };

  const openFileRef = useRef(openChangedFile);
  openFileRef.current = openChangedFile;
  const openChangeRef = useRef(onOpenChange);
  openChangeRef.current = onOpenChange;
  useEffect(() => {
    const receive = (event: Event) => {
      const detail = (event as CustomEvent<{ sessionId?: string | null; path?: string }>).detail;
      if (!detail?.path || detail.sessionId !== sessionId) return;
      openChangeRef.current(true);
      openFileRef.current(detail.path, true);
    };
    window.addEventListener("mew:open-file", receive);
    return () => window.removeEventListener("mew:open-file", receive);
  }, [sessionId]);

  const tabsStateRef = useRef(selectedWorkbenchTabs);
  tabsStateRef.current = selectedWorkbenchTabs;
  const applyActionRef = useRef(applyWorkbenchAction);
  applyActionRef.current = applyWorkbenchAction;
  useEffect(() => {
    const receive = (event: Event) => {
      const detail = (event as CustomEvent<{
        sessionId?: string | null;
        title?: string;
        url?: string;
        sourcePath?: string;
        version?: number;
      }>).detail;
      if (!isSafeArtifactUrl(detail?.url)) return;
      if (detail.sessionId != null && detail.sessionId !== sessionId) return;
      openChangeRef.current(true);
      const existing = tabsStateRef.current.tabs.find(
        (tab) =>
          tab.kind === "artifact" &&
          ((detail.sourcePath && tab.payload?.sourcePath === detail.sourcePath) ||
            tab.payload?.url === detail.url),
      );
      if (existing) {
        applyActionRef.current({
          type: "update",
          id: existing.id,
          patch: {
            title: detail.title?.trim() || existing.title,
            payload: {
              ...existing.payload,
              url: detail.url,
              sourcePath: detail.sourcePath ?? existing.payload?.sourcePath,
              version: detail.version ?? existing.payload?.version,
            },
          },
        });
        applyActionRef.current({ type: "select", id: existing.id });
        return;
      }
      applyActionRef.current({
        type: "add",
        tab: createWorkbenchTab("artifact", {
          title: detail.title?.trim() || "Artifact",
          sessionId: sessionId ?? undefined,
          payload: {
            url: detail.url,
            sourcePath: detail.sourcePath,
            version: detail.version,
          },
        }),
      });
    };
    window.addEventListener("mew:open-artifact", receive);
    return () => window.removeEventListener("mew:open-artifact", receive);
  }, [sessionId]);

  // When the Files or Changes tab is opened, enable workspace watching so
  // fs_changed events keep the listings/git status live. Stop watching
  // when those tabs are closed to avoid unnecessary traffic.
  useEffect(() => {
    if (!open || !connected || !sessionId || confirmedSessionId !== sessionId || !sessionHasWorkspace) return;
    if (!selectedWorkbenchTab) return;
    if (selectedWorkbenchTab.kind !== "changes" && selectedWorkbenchTab.kind !== "file" && selectedWorkbenchTab.kind !== "review") return;
    const client = getClient();
    if (!client) return;
    try { client.watchWorkspace(sessionId, true); }
    catch {
      useSessionStore.setState({ gitStatusError: "Could not watch the workspace. Check the connection." });
      return;
    }
    // Also seed the git status when the Changes tab opens.
    let timer: ReturnType<typeof setTimeout> | undefined;
    if (selectedWorkbenchTab.kind === "changes" || selectedWorkbenchTab.kind === "review") {
      useSessionStore.setState({ gitStatusLoaded: false, gitStatusError: null });
      try { client.gitStatus(sessionId); }
      catch { useSessionStore.setState({ gitStatusError: "Could not load changes. Check the connection." }); }
      timer = setTimeout(() => {
        const state = useSessionStore.getState();
        if (state.sessionId === sessionId && !state.gitStatusLoaded && !state.gitStatusError) {
          useSessionStore.setState({ gitStatusError: "Loading changes timed out. Check the connection and reopen this tab." });
        }
      }, 15_000);
    }
    return () => {
      if (timer) clearTimeout(timer);
      try { client.watchWorkspace(sessionId, false); } catch { /* The connection is already closed. */ }
    };
  }, [open, connected, confirmedSessionId, selectedWorkbenchTab?.id, selectedWorkbenchTab?.kind, sessionId, sessionHasWorkspace]);

  const activeCounts = {
    subagents: [...subagents.values()].filter((s) => s.status === "running").length,
    jobs: [...jobs.values()].filter(
      (j) => j.state !== "done" && j.state !== "failed" && j.state !== "cancelled",
    ).length,
  };

  useEffect(() => {
    if (!open) return;
    if (!selectedWorkbenchTab) return;
    const coreKinds: WorkbenchTabKind[] = ["agents", "jobs"];
    if (!coreKinds.includes(selectedWorkbenchTab.kind)) return;

    const preferredKind: WorkbenchTabKind | null = activeCounts.subagents > 0
      ? "agents"
      : activeCounts.jobs > 0
        ? "jobs"
        : null;
    if (!preferredKind) return;
    const preferredTab = selectedWorkbenchTabs.tabs.find((tab) => tab.kind === preferredKind);
    if (preferredTab && preferredTab.id !== selectedWorkbenchTab.id) {
      applyWorkbenchAction({ type: "select", id: preferredTab.id });
    }
  }, [
    open,
    selectedWorkbenchTab?.id,
    selectedWorkbenchTab?.kind,
    selectedWorkbenchTabs.tabs,
    activeCounts.subagents,
    activeCounts.jobs,
  ]);

  useEffect(() => {
    const action = (name: string) => {
      if (name === "new-tab") addWorkbenchTab("browser");
      if (name === "terminal") addWorkbenchTab("terminal");
      if (name === "close-tab" && open && selectedWorkbenchTab?.closable) applyWorkbenchAction({ type: "close", id: selectedWorkbenchTab.id });
    };
    const handleKeyDown = (event: KeyboardEvent) => {
      if (!open || !(event.metaKey || event.ctrlKey)) return;
      const key = event.key.toLowerCase();
      if (key === "w" && selectedWorkbenchTab?.closable) { event.preventDefault(); action("close-tab"); }
      else if (key === "t") { event.preventDefault(); action("new-tab"); }
      else if (/^[1-9]$/.test(key)) {
        const tab = selectedWorkbenchTabs.tabs[Number(key) - 1];
        if (tab) { event.preventDefault(); applyWorkbenchAction({ type: "select", id: tab.id }); }
      }
    };
    const menu = (event: Event) => action((event as CustomEvent<string>).detail);
    window.addEventListener("keydown", handleKeyDown);
    window.addEventListener("mew:menu-action", menu);
    return () => { window.removeEventListener("keydown", handleKeyDown); window.removeEventListener("mew:menu-action", menu); };
  }, [open, selectedWorkbenchTab, selectedWorkbenchTabs, sessionId]);

  const tabStrip = (
    <div
      className={cn(
        "mew-drag-region flex h-9 min-w-0 shrink-0 items-center",
        inFrameTitlebar ? "w-full gap-0.5" : "border-b border-border/70 bg-sidebar/60",
      )}
    >
      <TabsList
        ref={tabList}
        aria-label="Workbench tabs"
        className={cn(
          "mew-tab-strip h-9 min-w-0 flex-none justify-start gap-0.5 overflow-x-auto rounded-none border-0 bg-transparent p-0 px-1",
          inFrameTitlebar ? "max-w-[calc(100%_-_2.25rem)]" : "max-w-[calc(100%_-_4.5rem)]",
        )}
      >
        {selectedWorkbenchTabs.tabs.length === 0 && <span className="mx-1 flex h-7 items-center gap-1.5 rounded-md border border-border bg-muted/70 px-2 text-[11px]"><Globe className="h-3.5 w-3.5" />New tab</span>}
        {selectedWorkbenchTabs.tabs.map((tab) => {
          const meta = workbenchTabMeta(tab.kind);
          const Icon = meta.icon;
          const count = tab.kind === "agents"
            ? activeCounts.subagents
            : tab.kind === "jobs"
              ? activeCounts.jobs
              : tab.kind === "changes"
                ? gitStatus.length
                : 0;
          return (
            <div
              key={tab.id}
              className={cn("group my-1 flex h-7 shrink-0 items-center rounded-md border border-transparent", tab.id === selectedWorkbenchTab?.id && "border-border bg-muted/70")}
            >
              <TabsTrigger
                value={tabValue(tab.id)}
                aria-label={tab.title}
                onClick={() => applyWorkbenchAction({ type: "select", id: tab.id })}
                className="h-7 gap-1.5 rounded-md border-0 px-2 text-[11px] font-normal text-muted-foreground shadow-none hover:text-foreground data-[state=active]:bg-transparent data-[state=active]:text-foreground data-[state=active]:shadow-none"
              >
                <Icon className="h-3.5 w-3.5" aria-hidden="true" />
                <span className="max-w-32 truncate">{tab.title}</span>
                {count > 0 && <span className="flex h-4 min-w-4 items-center justify-center rounded-full bg-primary/10 px-1 text-[9px] text-primary">{count}</span>}
              </TabsTrigger>
              {tab.closable && (
                <button
                  type="button"
                  onClick={() => applyWorkbenchAction({ type: "close", id: tab.id })}
                  className={cn("motion-pressable mr-1 rounded p-1 text-muted-foreground opacity-0 hover:bg-accent hover:text-foreground focus-visible:opacity-100 group-hover:opacity-100", tab.id === selectedWorkbenchTab?.id && "opacity-100")}
                  aria-label={`Close ${tab.title}`}
                  title={`Close ${tab.title}`}
                >
                  <X className="h-3 w-3" />
                </button>
              )}
            </div>
          );
        })}
      </TabsList>
      <div className="relative shrink-0">
        <button
          type="button"
          onClick={openAddWorkbenchMenu}
          className="motion-pressable ml-1 rounded p-1.5 text-muted-foreground hover:bg-accent hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          aria-label="Add workbench tab"
          title="Add workbench tab"
        >
          <Plus className="h-3.5 w-3.5" />
        </button>
      </div>
      <div className="flex-1" />
      {!inFrameTitlebar && (
        <button
          type="button"
          onClick={() => onOpenChange(false)}
          className="motion-pressable mr-1 rounded p-1.5 text-muted-foreground hover:bg-accent hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          aria-label="Close workbench"
          title="Close workbench"
        >
          <X className="h-3.5 w-3.5" />
        </button>
      )}
    </div>
  );

  const workbenchContent = (
    <div className="flex min-h-0 flex-1 flex-col">
      <Tabs
        value={selectedWorkbenchTab ? tabValue(selectedWorkbenchTab.id) : ""}
        onValueChange={(value) => {
          const tab = selectedWorkbenchTabs.tabs.find((tab) => tabValue(tab.id) === value);
          if (tab) applyWorkbenchAction({ type: "select", id: tab.id });
        }}
        className={cn(
          "flex min-h-0 flex-1 flex-col",
          selectedWorkbenchTab?.kind === "browser" && "bg-background",
        )}
      >
        {inFrameTitlebar ? ((open || present) && <FrameWorkbenchTitlebar>{tabStrip}</FrameWorkbenchTitlebar>) : tabStrip}
        <div className="min-h-0 flex-1 overflow-hidden">
          {selectedWorkbenchTabs.tabs.length === 0 && (
            renderWorkbenchSurface(createWorkbenchTab("browser", { id: "launcher" }), true)
          )}
          {renderedTabs.map(({ tab, owner }) => {
            const selected = owner === currentOwner && tab.id === selectedWorkbenchTab?.id;
            return (
            <TabsContent
              key={electron ? `${owner}/${tab.id}` : tab.id}
              value={tabValue(tab.id, owner)}
              forceMount={tab.kind === "artifact" ? undefined : true}
              hidden={!selected}
              className={cn(
                "mt-0 h-full min-h-0 overflow-hidden focus-visible:outline-none",
                !selected && "hidden",
              )}
            >
              {renderWorkbenchSurface(tab, selected && (!electron || owner === sessionId), electron ? owner : undefined)}
            </TabsContent>
            );
          })}
        </div>
      </Tabs>

    </div>
  );

  function renderWorkbenchSurface(tab: WorkbenchTab, active: boolean, owner?: string) {
    switch (tab.kind) {
      case "agents":
        return <SubagentRailPanel key={sessionId} parentId={sessionId} subagents={subagents} active={open && active} />;
      case "jobs":
        return <div className="flex h-full min-h-0 flex-col overflow-y-auto p-3"><JobsRailPanel jobs={jobs} /></div>;
      case "browser":
        return <BrowserPanel
          client={getClient()}
          connected={connected}
          active={open && active}
          present={present && active}
          tab={tab}
          onTabChange={(patch) => {
            if (tab.id === "launcher") applyWorkbenchAction({ type: "add", tab: createWorkbenchTab("browser", { ...patch, sessionId: sessionId ?? undefined }) });
            else applyWorkbenchAction({ type: "update", id: tab.id, patch }, owner);
          }}
          sessionId={owner === "new" ? undefined : owner ?? tab.sessionId ?? sessionId ?? undefined}
          launcher={<div className="mx-auto w-full max-w-[44rem] px-6 py-8">
            <h2 className="mb-4 text-xs font-medium">Tools</h2>
            <div className="grid grid-cols-2 gap-x-3 gap-y-1">
              {NEW_TAB_TOOLS.map((surface) => {
                const Icon = surface.icon;
                return <button key={surface.kind} type="button" title={surface.description} className="flex h-9 min-w-0 items-center gap-2 rounded-md bg-muted/35 px-2.5 text-left text-xs hover:bg-accent focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring" onClick={() => {
                  const next = createWorkbenchTab(surface.kind, { sessionId: sessionId ?? undefined, cwd: sessionCwd ?? undefined });
                  if (tab.id !== "launcher" && !tab.payload?.url) applyWorkbenchAction({ type: "update", id: tab.id, patch: next });
                  else applyWorkbenchAction({ type: "add", tab: next });
                }}>
                  <Icon className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
                  <span className="truncate">{surface.label}<span className="sr-only"> {surface.description}</span></span>
                </button>;
              })}
            </div>
          </div>}
        />;
      case "artifact": {
        const versions = collectArtifactVersions(useSessionStore.getState().messages, {
          sourcePath: tab.payload?.sourcePath,
          title: tab.title,
        });
        return (
          <ArtifactFrame
            title={tab.title}
            url={tab.payload?.url}
            mode="panel"
            version={tab.payload?.version}
            versions={versions}
            onSelectVersion={(entry) =>
              applyWorkbenchAction({
                type: "update",
                id: tab.id,
                patch: {
                  title: entry.title,
                  payload: {
                    ...tab.payload,
                    url: entry.url,
                    version: entry.version,
                  },
                },
              })
            }
          />
        );
      }
      case "file":
        if (workbenchSessionId && workbenchSessionId !== sessionId) return null;
        return (
          <div className="flex h-full min-h-0 flex-col">
            <div className="min-h-0 flex-1">
              <FileTreePanel
                hasWorkspace={sessionHasWorkspace}
                headerActions={flaggedFiles.length > 0 ? <PinnedContext files={flaggedFiles} /> : undefined}
                fileState={tab.payload?.files}
                refreshRequest={fileRefresh?.tabId === tab.id && fileRefresh.owner === currentOwner ? fileRefresh : undefined}
                onFileStateChange={(files) => applyWorkbenchAction({
                  type: "update", id: tab.id, patch: { payload: { ...tab.payload, files } },
                }, currentOwner)}
              />
            </div>
          </div>
        );
      case "changes":
        if (workbenchSessionId && workbenchSessionId !== sessionId) return null;
        return <div className="h-full min-h-0 p-3"><ChangesPanel gitStatus={gitStatus} sessionId={sessionId} connected={connected} statusError={gitStatusError} statusLoaded={gitStatusLoaded} hasWorkspace={sessionHasWorkspace} onOpenFile={openChangedFile} /></div>;
      case "review":
        if (workbenchSessionId && workbenchSessionId !== sessionId) return null;
        return <div className="h-full min-h-0 overflow-y-auto p-3"><ReviewPanel gitStatus={gitStatus} statusError={gitStatusError} statusLoaded={gitStatusLoaded} hasWorkspace={sessionHasWorkspace} /></div>;
      case "terminal":
        return (
          <TerminalPanel
            client={getClient()}
            connected={connected}
            active={open && active}
            sessionId={confirmedSessionId ?? sessionId}
            sessionCwd={sessionCwd}
            jobCount={jobs.size}
            jobsPanel={<JobsRailPanel jobs={jobs} />}
          />
        );
    }
  }

  if (mode === "dock" && !isMobile) {
    return (
      <aside
        aria-label="Workspace workbench"
        data-open={open}
        aria-hidden={!open}
        className="motion-panel-size @container flex h-full min-w-0 overflow-hidden bg-background data-[open=true]:border-l data-[open=true]:border-border"
      >
        <div className="flex min-w-0 flex-1 flex-col">{workbenchContent}</div>
      </aside>
    );
  }

  return (
    <Sheet open={open} onOpenChange={onOpenChange}>
      <SheetContent
        side="right"
        aria-describedby={undefined}
        showCloseButton={false}
        className="w-full gap-0 p-0 sm:inset-y-2 sm:right-2 sm:h-auto sm:max-h-[calc(100vh-1rem)] sm:w-[min(24rem,calc(100vw-2rem))] sm:max-w-md sm:rounded-xl"
      >
        <SheetTitle className="sr-only">Workbench</SheetTitle>
        {workbenchContent}
      </SheetContent>
    </Sheet>
  );
}

function PinnedContext({ files }: { files: { path: string; reason?: string }[] }) {
  return (
    <Popover>
      <PopoverTrigger asChild>
        <button type="button" aria-label={`Pinned context ${files.length}`} className="flex items-center gap-1 rounded p-1.5 text-[10px] text-muted-foreground hover:bg-accent hover:text-foreground">
          <Pin className="h-3 w-3 text-primary" />
          <span>{files.length}</span>
        </button>
      </PopoverTrigger>
      <PopoverContent align="end" className="w-72 p-3" aria-label="Pinned context">
        <div className="mb-2 text-xs font-medium">Pinned context</div>
        <div className="space-y-1">
          {files.map((file) => (
            <div key={file.path} className="group flex min-w-0 items-center gap-1.5">
              <span className="min-w-0 truncate font-mono text-[10px] text-muted-foreground">{file.path}</span>
              {file.reason && <span className="shrink-0 text-[9px] text-muted-foreground/60">{file.reason}</span>}
              <button
                type="button"
                className="ml-auto shrink-0 rounded p-1 text-muted-foreground opacity-0 transition-opacity hover:bg-background hover:text-foreground focus-visible:opacity-100 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring group-hover:opacity-100"
                onClick={() => {
                  const client = getClient();
                  const sessionId = useSessionStore.getState().sessionId;
                  if (sessionId) client?.unflagFile(sessionId, file.path);
                }}
                title={`Remove ${file.path} from pinned context`}
                aria-label={`Remove ${file.path} from pinned context`}
              >
                <X className="h-3 w-3" />
              </button>
            </div>
          ))}
        </div>
      </PopoverContent>
    </Popover>
  );
}

function ReviewPanel({
  gitStatus,
  statusError,
  statusLoaded,
  hasWorkspace,
}: {
  gitStatus: GitEntry[];
  statusError: string | null;
  statusLoaded: boolean;
  hasWorkspace: boolean;
}) {
  if (!hasWorkspace) {
    return (
      <div className="flex h-full min-h-48 flex-col items-center justify-center px-6 text-center">
        <GitPullRequest className="h-5 w-5 text-muted-foreground" />
        <h3 className="mt-3 text-xs font-semibold text-foreground">Choose a workspace first</h3>
        <p className="mt-1 max-w-[18rem] text-[11px] leading-relaxed text-muted-foreground">
          Review will use the active project&apos;s working tree and selected files.
        </p>
      </div>
    );
  }

  if (statusError) return <div role="alert" className="px-2 py-6 text-center text-xs text-muted-foreground">{statusError}</div>;

  if (!statusLoaded) return <div className="py-8 text-center text-[11px] text-muted-foreground">Loading changes…</div>;

  if (gitStatus.length === 0) {
    return (
      <div className="flex h-full min-h-48 flex-col items-center justify-center px-6 text-center">
        <GitPullRequest className="h-5 w-5 text-muted-foreground" />
        <h3 className="mt-3 text-xs font-semibold text-foreground">Nothing to review</h3>
        <p className="mt-1 max-w-[18rem] text-[11px] leading-relaxed text-muted-foreground">
          Changes to the working tree will appear here when they are ready for review.
        </p>
      </div>
    );
  }

  return (
    <div className="space-y-3">
      <div>
        <p className="text-[10px] font-semibold uppercase tracking-[0.12em] text-muted-foreground">Review queue</p>
        <h3 className="mt-1 text-sm font-semibold text-foreground">{gitStatus.length} changed files</h3>
        <p className="mt-1 text-[11px] leading-relaxed text-muted-foreground">
          Select a file from Changes to inspect the working tree before asking mew for a review.
        </p>
      </div>
      <div className="space-y-1">
        {gitStatus.slice(0, 8).map((entry) => (
          <div key={entry.path} className="flex min-w-0 items-center gap-2 rounded-md bg-muted/35 px-2 py-1.5 text-[11px]">
            <span className="shrink-0 font-mono text-muted-foreground">{entry.status.slice(0, 1).toUpperCase()}</span>
            <span className="min-w-0 truncate font-mono text-foreground">{entry.path}</span>
          </div>
        ))}
      </div>
    </div>
  );
}

function workbenchTabMeta(kind: WorkbenchTabKind): {
  label: string;
  icon: LucideIcon;
} {
  switch (kind) {
    case "agents":
      return { label: "Agents", icon: Loader2 };
    case "jobs":
      return { label: "Jobs", icon: Terminal };
    case "browser":
      return { label: "Browser", icon: Globe };
    case "artifact":
      return { label: "Artifact", icon: Globe };
    case "terminal":
      return { label: "Terminal", icon: Terminal };
    case "file":
      return { label: "Files", icon: Folder };
    case "changes":
      return { label: "Changes", icon: GitBranch };
    case "review":
      return { label: "Review", icon: GitPullRequest };
  }
}

function SubagentRailPanel({ subagents, parentId, active }: {
  subagents: Map<string, SubagentInfo>;
  parentId: string | null;
  active: boolean;
}) {
  const [selectedChild, setSelectedChild] = useState<string | null>(null);
  const subs = [...subagents.values()];
  const selected = subs.find((sub) => sub.childSessionId === selectedChild);
  if (parentId && selected) {
    return (
      <Suspense fallback={<p role="status" className="p-3 text-xs text-muted-foreground">Loading agent conversation…</p>}>
        <SubagentConversation key={selected.childSessionId} parentId={parentId} agent={selected} active={active} onBack={() => setSelectedChild(null)} />
      </Suspense>
    );
  }
  if (subs.length === 0) {
    return (
      <EmptyState
        icon={<Loader2 className="h-4 w-4" />}
        title="No active agents"
        description="Delegated work will appear here while it runs."
      />
    );
  }
  return (
    <div className="h-full space-y-1.5 overflow-y-auto p-3">
      {subs.map((sub) => (
        <SubagentRailRow key={sub.parentCallId} sub={sub} onOpen={parentId && sub.childSessionId ? () => setSelectedChild(sub.childSessionId) : undefined} />
      ))}
    </div>
  );
}

function SubagentRailRow({ sub, onOpen }: { sub: SubagentInfo; onOpen?: () => void }) {
  const dotColor = {
    running: "bg-blue-500 animate-pulse",
    completed: "bg-green-500",
    cancelled: "bg-muted-foreground",
    failed: "bg-destructive",
  }[sub.status] ?? "bg-muted-foreground";

  const content = (
    <>
      <div className="flex items-center gap-2">
        <SubagentIdenticon seed={sub.childSessionId || sub.displayName || sub.parentCallId} size={16} />
        <span className={cn("h-1.5 w-1.5 shrink-0 rounded-full", dotColor)} />
        <span className="truncate text-xs font-medium text-foreground">
          {sub.displayName ?? sub.name}
        </span>
        {sub.status !== "running" && (
          <span className="ml-auto text-[10px] capitalize text-muted-foreground">{sub.status}</span>
        )}
      </div>
      <div className="mt-0.5 flex items-center gap-1 text-[10px] text-muted-foreground">
        <span className="font-mono">{sub.name}</span>
        {sub.childSessionId && <span>· {sub.childSessionId.slice(0, 8)}</span>}
      </div>
      {sub.lastProgress && (
        <div className="mt-1 truncate text-[11px] text-muted-foreground">
          ↳ {sub.lastProgress}
        </div>
      )}
      {sub.outcome?.type === "failed" && (
        <div className="mt-1 flex items-start gap-1 text-[11px] text-destructive">
          <XCircle className="mt-0.5 h-3 w-3 shrink-0" />
          {sub.outcome.reason}
        </div>
      )}
    </>
  );
  const className = "rounded-md border border-border bg-card/50 px-2 py-1.5";
  return onOpen ? (
    <button type="button" className={cn(className, "w-full text-left hover:bg-accent focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring")} aria-label={`Open ${sub.displayName ?? sub.name}`} onClick={onOpen}>
      {content}
    </button>
  ) : <div className={className}>{content}</div>;
}

function JobsRailPanel({ jobs }: { jobs: Map<string, JobInfo> }) {
  const list = [...jobs.values()];
  if (list.length === 0) {
    return (
      <EmptyState
        icon={<Terminal className="h-4 w-4" />}
        title="No background jobs"
        description="Long-running commands will be tracked here."
      />
    );
  }
  return (
    <div className="space-y-1.5">
      {list.map((job) => (
        <JobRailRow key={job.jobId} job={job} />
      ))}
    </div>
  );
}

function JobRailRow({ job }: { job: JobInfo }) {
  const isTerminal = job.state === "done" || job.state === "failed" || job.state === "cancelled";
  const dotColor = isTerminal
    ? job.state === "done"
      ? "bg-green-500"
      : job.state === "failed"
        ? "bg-destructive"
        : "bg-muted-foreground"
    : "bg-blue-500 animate-pulse";

  return (
    <div className="rounded-md border border-border bg-card/50 px-2 py-1.5">
      <div className="flex items-center gap-2">
        <span className={cn("h-1.5 w-1.5 shrink-0 rounded-full", dotColor)} />
        <span className="truncate font-mono text-[11px] text-foreground">
          {job.command}
        </span>
        {isTerminal && (
          <span className="ml-auto text-[10px] capitalize text-muted-foreground">{job.state}</span>
        )}
      </div>
      <div className="mt-0.5 truncate text-[10px] text-muted-foreground">
        {job.jobId.slice(0, 8)}
      </div>
    </div>
  );
}

function EmptyState({
  icon,
  title,
  description,
}: {
  icon: ReactNode;
  title: string;
  description: string;
}) {
  return (
    <div className="flex flex-col items-center px-4 py-12 text-center">
      <div className="flex h-9 w-9 items-center justify-center rounded-full bg-muted text-muted-foreground">
        {icon}
      </div>
      <span className="mt-3 text-xs font-medium text-foreground">{title}</span>
      <span className="mt-1 max-w-[14rem] text-[11px] leading-relaxed text-muted-foreground">
        {description}
      </span>
    </div>
  );
}
