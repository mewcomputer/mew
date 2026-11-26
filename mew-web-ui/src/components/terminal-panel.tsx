import { useEffect, useRef, useState, type ReactNode } from "react";
import type { MewClient } from "@mew/web-client";
import type { IDisposable, ITerminalOptions, ITheme } from "ghostty-web";
import { RotateCcw, Terminal as TerminalIcon } from "lucide-react";
import { cn } from "../lib/utils";

export interface GhosttyTerminalInstance {
  cols: number;
  rows: number;
  options?: { theme?: ITheme };
  open(parent: HTMLElement): void;
  write(data: string | Uint8Array): void;
  focus(): void;
  dispose(): void;
  loadAddon(addon: GhosttyFitAddonInstance): void;
  onData(listener: (data: string) => void): IDisposable;
  onResize(listener: (size: { cols: number; rows: number }) => void): IDisposable;
}

export interface GhosttyFitAddonInstance {
  activate(terminal: unknown): void;
  dispose(): void;
  fit(): void;
}

export interface GhosttyModule {
  init(): Promise<void>;
  Terminal: new (options?: ITerminalOptions) => GhosttyTerminalInstance;
  FitAddon?: new () => GhosttyFitAddonInstance;
}

export interface TerminalPanelProps {
  client: MewClient | null;
  connected: boolean;
  active: boolean;
  sessionId: string | null;
  sessionCwd: string | null;
  jobCount: number;
  jobsPanel?: ReactNode;
  loadGhostty?: () => Promise<GhosttyModule>;
}

type TerminalStatus = "idle" | "connecting" | "running" | "exited" | "error";

let ghosttyInitPromise: Promise<void> | null = null;

function ensureGhosttyInit(mod: GhosttyModule): Promise<void> {
  if (!ghosttyInitPromise) {
    ghosttyInitPromise = mod.init().catch((err) => {
      ghosttyInitPromise = null;
      throw err;
    });
  }
  return ghosttyInitPromise;
}

function defaultLoadGhostty(): Promise<GhosttyModule> {
  return import("ghostty-web") as Promise<GhosttyModule>;
}

function cssVar(styles: CSSStyleDeclaration, name: string, fallback: string): string {
  const value = styles.getPropertyValue(name).trim();
  return value || fallback;
}

export function readTerminalTheme(): ITheme {
  if (typeof document === "undefined" || typeof getComputedStyle === "undefined") {
    return {};
  }
  const styles = getComputedStyle(document.documentElement);
  const background = cssVar(styles, "--background", "#0f1117");
  const foreground = cssVar(styles, "--foreground", cssVar(styles, "--text-body", "#e4e4e7"));
  const muted = cssVar(styles, "--muted-foreground", "#71717a");
  const red = cssVar(styles, "--red-fg", "#ef4444");
  const green = cssVar(styles, "--green-fg", "#22c55e");
  const yellow = cssVar(styles, "--orange-fg", "#eab308");
  const blue = cssVar(styles, "--blue-fg", "#3b82f6");
  const magenta = cssVar(styles, "--purple-fg", "#a855f7");
  const cyan = cssVar(styles, "--cyan-fg", "#06b6d4");

  return {
    background,
    foreground,
    cursor: cssVar(styles, "--primary", foreground),
    cursorAccent: background,
    selectionBackground: cssVar(styles, "--selection-background", "#3f3f46"),
    selectionForeground: cssVar(styles, "--selection-foreground", foreground),
    black: background,
    red,
    green,
    yellow,
    blue,
    magenta,
    cyan,
    white: foreground,
    brightBlack: muted,
    brightRed: cssVar(styles, "--red-med", red),
    brightGreen: cssVar(styles, "--green-med", green),
    brightYellow: cssVar(styles, "--orange-med", yellow),
    brightBlue: cssVar(styles, "--blue-med", blue),
    brightMagenta: cssVar(styles, "--purple-med", magenta),
    brightCyan: cssVar(styles, "--cyan-med", cyan),
    brightWhite: foreground,
  };
}

function workspaceLabel(cwd: string | null): string {
  if (!cwd) return "Terminal";
  const parts = cwd.split(/[\\/]/).filter(Boolean);
  return parts[parts.length - 1] || cwd;
}

export function TerminalPanel({
  client,
  connected,
  active,
  sessionId,
  sessionCwd,
  jobCount,
  jobsPanel,
  loadGhostty = defaultLoadGhostty,
}: TerminalPanelProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const termRef = useRef<GhosttyTerminalInstance | null>(null);
  const fitRef = useRef<GhosttyFitAddonInstance | null>(null);
  const terminalIdRef = useRef<string | null>(null);
  const activeRef = useRef(active);
  activeRef.current = active;
  const loadGhosttyRef = useRef(loadGhostty);
  loadGhosttyRef.current = loadGhostty;

  const [restartNonce, setRestartNonce] = useState(0);
  const [startedSessionKey, setStartedSessionKey] = useState<string | null>(null);
  const [status, setStatus] = useState<TerminalStatus>("idle");
  const [statusDetail, setStatusDetail] = useState<string | null>(null);
  const [jobsOpen, setJobsOpen] = useState(false);

  const sessionKey = connected && client && sessionId ? `${sessionId}:${restartNonce}` : null;

  useEffect(() => {
    if (active && sessionKey && startedSessionKey !== sessionKey) {
      setStartedSessionKey(sessionKey);
    }
  }, [active, sessionKey, startedSessionKey]);

  const shouldRun = Boolean(sessionKey && startedSessionKey === sessionKey);

  useEffect(() => {
    if (!shouldRun || !client || !sessionId) {
      setStatus("idle");
      setStatusDetail(null);
      return;
    }

    const container = containerRef.current;
    if (!container) return;

    let cancelled = false;
    const encoder = new TextEncoder();
    const disposables: IDisposable[] = [];
    let resizeObserver: ResizeObserver | null = null;
    let themeObserver: MutationObserver | null = null;
    let term: GhosttyTerminalInstance | null = null;
    let fitAddon: GhosttyFitAddonInstance | null = null;

    setStatus("connecting");
    setStatusDetail(null);

    const onOpened = ({ terminal_id }: { terminal_id: string }) => {
      if (cancelled) return;
      terminalIdRef.current = terminal_id;
      setStatus("running");
      setStatusDetail(null);
      const currentTerm = termRef.current;
      if (currentTerm) {
        fitRef.current?.fit();
        const rows = Math.max(1, currentTerm.rows || 24);
        const cols = Math.max(1, currentTerm.cols || 80);
        client.terminalResize(terminal_id, rows, cols);
        if (activeRef.current) {
          currentTerm.focus();
        }
      }
    };

    const onOutput = ({ terminal_id, bytes }: { terminal_id: string; bytes: number[] }) => {
      if (cancelled || terminal_id !== terminalIdRef.current) return;
      termRef.current?.write(new Uint8Array(bytes));
    };

    const onExited = ({ terminal_id, status: exitStatus }: { terminal_id: string; status: string }) => {
      if (cancelled || terminal_id !== terminalIdRef.current) return;
      terminalIdRef.current = null;
      setStatus("exited");
      setStatusDetail(exitStatus || "exited");
    };

    const onError = ({ terminal_id, message }: { terminal_id: string | null; message: string }) => {
      if (cancelled) return;
      if (terminal_id && terminalIdRef.current && terminal_id !== terminalIdRef.current) {
        return;
      }
      if (!terminal_id || terminal_id === terminalIdRef.current) {
        setStatus("error");
        setStatusDetail(message);
      }
    };

    void (async () => {
      try {
        const mod = await loadGhosttyRef.current();
        if (loadGhosttyRef.current === defaultLoadGhostty) {
          await ensureGhosttyInit(mod);
        } else {
          await mod.init();
        }
        if (cancelled || !containerRef.current) return;

        term = new mod.Terminal({
          cursorBlink: true,
          fontSize: 12,
          fontFamily:
            'ui-monospace, SFMono-Regular, "SF Mono", Menlo, Monaco, Consolas, monospace',
          scrollback: 5000,
          theme: readTerminalTheme(),
        });
        termRef.current = term;
        term.open(containerRef.current);

        if (mod.FitAddon) {
          fitAddon = new mod.FitAddon();
          fitRef.current = fitAddon;
          term.loadAddon(fitAddon);
          if (containerRef.current.clientWidth > 0 && containerRef.current.clientHeight > 0) {
            fitAddon.fit();
          }
        }

        disposables.push(
          term.onData((data) => {
            if (terminalIdRef.current) {
              client.terminalInput(terminalIdRef.current, encoder.encode(data));
            }
          }),
        );

        disposables.push(
          term.onResize(({ cols, rows }) => {
            if (terminalIdRef.current) {
              client.terminalResize(
                terminalIdRef.current,
                Math.max(1, rows),
                Math.max(1, cols),
              );
            }
          }),
        );

        if (typeof ResizeObserver !== "undefined" && fitAddon) {
          const fit = fitAddon;
          resizeObserver = new ResizeObserver(() => {
            const el = containerRef.current;
            if (!activeRef.current || !el || el.clientWidth === 0 || el.clientHeight === 0) {
              return;
            }
            fit.fit();
          });
          resizeObserver.observe(containerRef.current);
        }

        if (typeof MutationObserver !== "undefined") {
          themeObserver = new MutationObserver(() => {
            if (termRef.current?.options) {
              termRef.current.options.theme = readTerminalTheme();
            }
          });
          themeObserver.observe(document.documentElement, {
            attributes: true,
            attributeFilter: ["data-theme", "class", "style"],
          });
        }

        client.on("terminal-opened", onOpened);
        client.on("terminal-output", onOutput);
        client.on("terminal-exited", onExited);
        client.on("terminal-error", onError);

        const initialRows = Math.max(1, term.rows || 24);
        const initialCols = Math.max(1, term.cols || 80);
        client.terminalOpen(initialRows, initialCols);
      } catch (err) {
        if (!cancelled) {
          setStatus("error");
          setStatusDetail(err instanceof Error ? err.message : "Failed to start terminal");
        }
      }
    })();

    return () => {
      cancelled = true;
      client.off("terminal-opened", onOpened);
      client.off("terminal-output", onOutput);
      client.off("terminal-exited", onExited);
      client.off("terminal-error", onError);
      resizeObserver?.disconnect();
      themeObserver?.disconnect();
      for (const d of disposables) d.dispose();
      fitAddon?.dispose();
      fitRef.current = null;
      if (terminalIdRef.current) {
        client.terminalClose(terminalIdRef.current);
        terminalIdRef.current = null;
      }
      term?.dispose();
      termRef.current = null;
    };
  }, [shouldRun, client, sessionId, restartNonce]);

  useEffect(() => {
    if (!active) return;
    const frame = requestAnimationFrame(() => {
      const el = containerRef.current;
      if (el && el.clientWidth > 0 && el.clientHeight > 0) {
        fitRef.current?.fit();
      }
      termRef.current?.focus();
    });
    return () => cancelAnimationFrame(frame);
  }, [active]);

  const dotClass =
    status === "running"
      ? "bg-green-fg"
      : status === "connecting"
        ? "bg-blue-fg animate-pulse"
        : status === "error"
          ? "bg-destructive"
          : "bg-muted-foreground";

  if (!connected || !client || !sessionId) {
    return (
      <div className="flex h-full min-h-0 flex-col">
        <div className="flex flex-1 flex-col items-center justify-center px-6 text-center">
          <div className="flex h-9 w-9 items-center justify-center rounded-full bg-muted text-muted-foreground">
            <TerminalIcon className="h-4 w-4" />
          </div>
          <span className="mt-3 text-xs font-medium text-foreground">Terminal unavailable</span>
          <span className="mt-1 max-w-[16rem] text-[11px] leading-relaxed text-muted-foreground">
            Open or connect to a session to launch an interactive shell.
          </span>
        </div>
        {jobCount > 0 && jobsPanel && (
          <div className="max-h-48 shrink-0 overflow-y-auto border-t border-border/70 p-2.5">
            {jobsPanel}
          </div>
        )}
      </div>
    );
  }

  return (
    <div className="flex h-full min-h-0 flex-col bg-background">
      <div className="flex h-8 shrink-0 items-center gap-2 border-b border-border/60 px-2.5">
        <span
          data-terminal-status={status}
          className={cn("h-1.5 w-1.5 shrink-0 rounded-full", dotClass)}
        />
        <span className="truncate font-mono text-[11px] font-medium text-foreground">
          {workspaceLabel(sessionCwd)}
        </span>
        {statusDetail && (
          <span
            className={cn(
              "truncate text-[10px]",
              status === "error" ? "text-destructive" : "text-muted-foreground",
            )}
            title={statusDetail}
          >
            {statusDetail}
          </span>
        )}
        <div className="ml-auto flex items-center gap-1">
          {jobCount > 0 && jobsPanel && (
            <button
              type="button"
              onClick={() => setJobsOpen((prev) => !prev)}
              aria-expanded={jobsOpen}
              className={cn(
                "rounded px-1.5 py-0.5 text-[10px] font-medium transition-colors",
                jobsOpen
                  ? "bg-accent text-accent-foreground"
                  : "text-muted-foreground hover:bg-accent hover:text-foreground",
              )}
            >
              Jobs ({jobCount})
            </button>
          )}
          <button
            type="button"
            onClick={() => setRestartNonce((n) => n + 1)}
            aria-label="Restart terminal"
            title="Restart terminal"
            className="rounded p-1 text-muted-foreground hover:bg-accent hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          >
            <RotateCcw className="h-3 w-3" />
          </button>
        </div>
      </div>

      <div
        className="min-h-0 flex-1 overflow-hidden p-1.5"
        onClick={() => termRef.current?.focus()}
      >
        <div
          ref={containerRef}
          data-testid="terminal-viewport"
          className="h-full w-full overflow-hidden"
        />
      </div>

      {jobsOpen && jobCount > 0 && jobsPanel && (
        <div className="max-h-48 shrink-0 overflow-y-auto border-t border-border/70 p-2.5">
          {jobsPanel}
        </div>
      )}
    </div>
  );
}
