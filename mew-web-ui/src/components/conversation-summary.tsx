import { useState } from "react";
import { File, GitBranch, Layers, ListTree, Terminal } from "lucide-react";
import { useSessionStore } from "../stores/session";
import type { WorkbenchTabKind } from "../lib/workbench-tabs";
import { cn } from "../lib/utils";
import { Button } from "./ui/button";
import { Popover, PopoverContent, PopoverTrigger } from "./ui/popover";

export function ConversationSummary({ onOpenSurface }: {
  onOpenSurface: (kind: WorkbenchTabKind) => void;
}) {
  const [open, setOpen] = useState(false);
  const active = useSessionStore((state) => {
    for (const agent of state.subagents.values()) if (agent.status === "running") return true;
    for (const job of state.jobs.values()) {
      if (job.state !== "done" && job.state !== "failed" && job.state !== "cancelled") return true;
    }
    return false;
  });

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <Button variant="ghost" size="icon" className="relative h-7 w-7 shrink-0" aria-label={open ? "Close summary" : "Open summary"} title="Summary">
          <ListTree className="h-3.5 w-3.5" />
          {active && <span className="absolute right-1 top-1 h-1.5 w-1.5 rounded-full bg-text-accent ring-2 ring-background" />}
        </Button>
      </PopoverTrigger>
      <PopoverContent align="end" sideOffset={8} aria-label="Conversation summary" data-mew-overlay className="max-h-[min(32rem,calc(100vh-5rem))] w-72 max-w-[calc(100vw-2rem)] overflow-y-auto rounded-2xl border-border/60 bg-popover p-3 shadow-lg">
        <ConversationSummaryContent onOpenSurface={(kind) => { setOpen(false); onOpenSurface(kind); }} />
      </PopoverContent>
    </Popover>
  );
}

export function ConversationSummaryContent({ onOpenSurface }: {
  onOpenSurface: (kind: WorkbenchTabKind) => void;
}) {
  const sessionId = useSessionStore((state) => state.sessionId);
  const changeStats = useSessionStore((state) => state.availableSessions.find((session) => session.session_id === sessionId)?.change_stats);
  const cwd = useSessionStore((state) => state.sessionCwd);
  const subagents = useSessionStore((state) => state.subagents);
  const jobs = useSessionStore((state) => state.jobs);
  const sources = useSessionStore((state) => state.flaggedFiles);
  const changes = useSessionStore((state) => state.gitStatus);
  const agents = [...subagents.values()];
  let working = 0, done = 0, failed = 0, cancelled = 0;
  for (const agent of agents) {
    if (agent.status === "running") working++;
    else if (agent.status === "completed") done++;
    else if (agent.status === "failed") failed++;
    else if (agent.status === "cancelled") cancelled++;
  }
  const agentStatus = agents.length ? [working && `${working} working`, done && `${done} done`, failed && `${failed} failed`, cancelled && `${cancelled} cancelled`].filter(Boolean).join(", ") : "No subagents";
  const activeJobs = [...jobs.values()];
  const workspace = cwd?.replace(/\/+$/, "").split("/").pop() || "mew";
  const rowClass = "flex w-full min-w-0 items-center gap-2 rounded-md px-2 py-1.5 text-left text-xs hover:bg-accent focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring";

  return (
    <>
      <div className="mb-1 truncate px-2 py-1 text-[11px] font-medium text-muted-foreground" title={cwd ?? undefined}>{workspace}</div>
      <button type="button" className={rowClass} onClick={() => onOpenSurface("changes")}>
        <GitBranch className="h-3.5 w-3.5 shrink-0" />
        <span>Changes</span>
        {changeStats ? <span className="ml-auto flex gap-1.5 text-[11px] tabular-nums"><span className="text-text-success">+{changeStats.added.toLocaleString()}</span><span className="text-text-error">-{changeStats.removed.toLocaleString()}</span></span>
          : <span className="ml-auto text-[11px] text-muted-foreground">{changes.length} {changes.length === 1 ? "file" : "files"}</span>}
      </button>

      <h3 className="mt-4 px-2 text-[11px] font-normal text-muted-foreground">Subagents</h3>
      <button type="button" className={cn(rowClass, "mt-1")} onClick={() => onOpenSurface("agents")} aria-label={`Subagents: ${agentStatus}`}>
        <span className="flex shrink-0 -space-x-1" aria-hidden="true">
          {agents.length ? agents.slice(0, 4).map((agent) => (
            <Layers key={agent.parentCallId} className={cn("h-4 w-4 rounded-sm bg-popover", agent.status === "running" ? "text-text-accent" : agent.status === "completed" ? "text-text-success" : agent.status === "failed" ? "text-text-error" : "text-muted-foreground")} />
          )) : <Layers className="h-3.5 w-3.5 text-muted-foreground" />}
        </span>
        <span className="truncate text-[11px] text-muted-foreground">{agentStatus}</span>
      </button>

      <section className="mt-3 border-t border-border/60 pt-3" aria-label="Background processes">
        <h3 className="mb-1 px-2 text-[11px] font-normal text-muted-foreground">Background processes</h3>
        {activeJobs.length ? activeJobs.map((job) => (
          <button key={job.jobId} type="button" className={rowClass} onClick={() => onOpenSurface("jobs")} title={job.command}>
            <Terminal className="h-3.5 w-3.5 shrink-0" />
            <span className="min-w-0 truncate">{job.command}</span>
            <span className="ml-auto shrink-0 text-[10px] text-muted-foreground">{job.state}</span>
          </button>
        )) : <button type="button" className={cn(rowClass, "text-muted-foreground")} onClick={() => onOpenSurface("jobs")}>No background processes</button>}
      </section>

      <section className="mt-3" aria-label="Sources">
        <h3 className="mb-1 px-2 text-[11px] font-normal text-muted-foreground">Sources</h3>
        {sources.length ? sources.slice(0, 3).map((source) => (
          <div key={source.path} className="flex min-w-0 items-center gap-2 px-2 py-1.5 text-[11px] text-muted-foreground" title={source.path}>
            <File className="h-3.5 w-3.5 shrink-0" />
            <span className="truncate">{source.path.split("/").pop()}</span>
          </div>
        )) : <p className="px-2 py-1.5 text-[11px] text-muted-foreground">No pinned sources</p>}
        <button type="button" className={cn(rowClass, "text-muted-foreground")} onClick={() => onOpenSurface("file")} aria-label="View all sources">
          <Layers className="h-3.5 w-3.5" /> View all
        </button>
      </section>
    </>
  );
}
