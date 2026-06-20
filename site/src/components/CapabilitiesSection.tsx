import React, { useEffect, useState, useRef } from "react";
import "./CapabilitiesSection.css";

// --- Types ---

type TermLine =
  | { type: "blank" }
  | { type: "user"; text: string }
  | { type: "assistant"; text: string }
  | { type: "reasoning"; text: string }
  | { type: "tool-top" }
  | { type: "tool-header"; name: string; status: "completed" | "error" }
  | { type: "tool-path"; text: string }
  | { type: "tool-body"; text: string; tone?: "add" | "rem" | "context" }
  | { type: "tool-more"; count: number }
  | { type: "tool-bottom" };

interface Section {
  id: string;
  title: string;
  paragraphs: React.ReactNode[];
  terminal: TermLine[];
  graphic?: "chat" | "picker";
}

// --- Data ---

const sections: Section[] = [
  {
    id: "navigate",
    title: "It knows how to navigate.",
    paragraphs: [
      "Give it a large, unfamiliar repository, and it will use search, grep, and shell commands to find exactly what needs to change. It doesn\u2019t need a pre-built index; it uses the same tools you do.",
      <>
        It uses{" "}
        <code className="bg-muted px-1.5 py-0.5 rounded text-sm font-mono">
          glob
        </code>{" "}
        to understand the shape of your project, and{" "}
        <code className="bg-muted px-1.5 py-0.5 rounded text-sm font-mono">
          grep
        </code>{" "}
        to find the exact lines that matter. It reads only what it needs,
        keeping its context window focused and your token costs low.
      </>,
    ],
    terminal: [
      { type: "user", text: "where is the rate limiter configured?" },
      { type: "reasoning", text: "let me search for rate limiting logic." },
      { type: "tool-top" },
      { type: "tool-header", name: "grep", status: "completed" },
      { type: "tool-path", text: "'rate_limit'" },
      {
        type: "tool-body",
        text: "src/api/middleware.rs:14:use crate::rate_limit::Limiter;",
      },
      { type: "tool-bottom" },
      { type: "blank" },
      { type: "tool-top" },
      { type: "tool-header", name: "read", status: "completed" },
      { type: "tool-path", text: "src/api/middleware.rs" },
      { type: "tool-body", text: "use crate::rate_limit::Limiter;" },
      { type: "tool-body", text: "use crate::redis::Pool;" },
      { type: "tool-body", text: "use crate::config::RateLimitConfig;" },
      { type: "tool-body", text: "..." },
      { type: "tool-bottom" },
      { type: "blank" },
      {
        type: "assistant",
        text: "the rate limiter is initialized in src/api/middleware.rs at line 14.",
      },
    ],
  },
  {
    id: "permission",
    title: "It works like a careful engineer.",
    paragraphs: [
      "It categorizes every action by risk. Reading and searching happens silently. Editing files or running scripts prompts you once. Dangerous operations\u2014like deploying or force-pushing\u2014ask every single time.",
      <>
        You can declaratively tune these rules in your{" "}
        <code className="bg-muted px-1.5 py-0.5 rounded text-sm font-mono">
          config.toml
        </code>
        . Allow it to write freely in{" "}
        <code className="bg-muted px-1.5 py-0.5 rounded text-sm font-mono">
          src/**
        </code>{" "}
        but always prompt before touching{" "}
        <code className="bg-muted px-1.5 py-0.5 rounded text-sm font-mono">
          terraform/
        </code>
        . The defaults are safe, but the control is yours.
      </>,
    ],
    terminal: [
      { type: "user", text: "clean up unused docker images" },
      {
        type: "reasoning",
        text: "docker system prune will remove unused images.",
      },
      { type: "tool-top" },
      { type: "tool-header", name: "bash", status: "error" },
      { type: "tool-path", text: "docker system prune -f" },
      { type: "tool-body", text: "permission denied" },
      { type: "tool-bottom" },
      { type: "blank" },
      {
        type: "assistant",
        text: "understood. i won\u2019t run the prune command.",
      },
    ],
  },
  {
    id: "subagent",
    title: "It delegates when it needs to.",
    paragraphs: [
      "If a task requires a deep dive into a specific subsystem, mew can spin up a subagent to handle it. The subagent does the research or refactor and reports back, keeping the main thread focused and clean.",
      "Context is precious. Instead of filling the main session with 50 file reads, a subagent does the dirty work, distills the findings into a short summary, and returns the result.",
    ],
    terminal: [
      { type: "user", text: "migrate the user table to UUIDs" },
      {
        type: "reasoning",
        text: "this touches the schema, models, and api handlers.",
      },
      { type: "tool-top" },
      { type: "tool-header", name: "task", status: "completed" },
      { type: "tool-path", text: "research uuid migration" },
      { type: "tool-body", text: "writing migration 0042_add_uuids.sql..." },
      { type: "tool-body", text: "updating models/user.rs..." },
      { type: "tool-body", text: "regenerating handlers..." },
      { type: "tool-bottom" },
      { type: "blank" },
      {
        type: "assistant",
        text: "migration complete. back to the main thread.",
      },
    ],
  },
  {
    id: "model",
    title: "It speaks your model.",
    paragraphs: [
      "mew supports OpenAI-shape and Anthropic-shape API adapters out of the box. It comes with defaults for opencode-zen, opencode-go, z-ai, and deepseek, pulling model catalogs dynamically.",
      "You can point it at any compatible endpoint, local or remote. Whether you want the frontier intelligence of Claude 3.5 Sonnet or the speed of a local Llamafile, mew adapts.",
    ],
    terminal: [
      { type: "user", text: "/model anthropic/claude-3-5-sonnet" },
      { type: "reasoning", text: "switching model..." },
      { type: "assistant", text: "model set to anthropic/claude-3-5-sonnet." },
      { type: "blank" },
      { type: "user", text: "refactor this function to be more idiomatic" },
      { type: "tool-top" },
      { type: "tool-header", name: "edit", status: "completed" },
      { type: "tool-path", text: "src/parser.rs" },
      { type: "tool-body", text: "- for item in items {", tone: "rem" },
      {
        type: "tool-body",
        text: "-   result.push(item.parse()?);",
        tone: "rem",
      },
      { type: "tool-body", text: "- }", tone: "rem" },
      {
        type: "tool-body",
        text: "+ result = items.iter().map(|i| i.parse()).collect::<Result<_,_>>()?;",
        tone: "add",
      },
      { type: "tool-bottom" },
      { type: "assistant", text: "done. tests pass." },
    ],
    graphic: "picker",
  },
  {
    id: "mcp",
    title: "A pluggable runtime.",
    paragraphs: [
      "mew speaks standard agent protocols. It can run entirely in the terminal, or act as a JSON-RPC server for editors like Zed and Neovim. Your agent lives where you write code.",
      "MCP (Model Context Protocol) servers are loaded automatically. If it speaks the protocol, mew can use it. Connect it to your database, your issue tracker, or your internal APIs without writing wrapper code.",
    ],
    terminal: [
      { type: "user", text: "query the postgres db for active users" },
      { type: "blank" },
      { type: "tool-top" },
      { type: "tool-header", name: "mcp_postgres_query", status: "completed" },
      { type: "tool-path", text: "SELECT * FROM users WHERE active = true" },
      { type: "tool-body", text: "found 42 active users." },
      { type: "tool-bottom" },
    ],
  },
  {
    id: "config",
    title: "Configurable workflows.",
    paragraphs: [
      <>
        Configuration lives in a simple{" "}
        <code className="bg-muted px-1.5 py-0.5 rounded text-sm font-mono">
          config.toml
        </code>
        . Credentials can be set via environment variables, the system keyring,
        or a credentials file. Everything is overridable.
      </>,
      <>
        Environment variables with the{" "}
        <code className="bg-muted px-1.5 py-0.5 rounded text-sm font-mono">
          MEW_
        </code>{" "}
        prefix override config values. Drop a markdown file into{" "}
        <code className="bg-muted px-1.5 py-0.5 rounded text-sm font-mono">
          .mew/skills/
        </code>{" "}
        to teach it a new trick, which instantly becomes a slash command.
      </>,
    ],
    terminal: [
      { type: "user", text: "write a test for the auth module" },
      {
        type: "reasoning",
        text: "i\u2019ll add tests for verify_password and hash_password.",
      },
      { type: "tool-top" },
      { type: "tool-header", name: "write", status: "completed" },
      { type: "tool-path", text: "src/auth/password.rs" },
      { type: "tool-body", text: "#[test]" },
      { type: "tool-body", text: "fn verify_handles_invalid_hash() {" },
      {
        type: "tool-body",
        text: '    let hash = hash_password("correct horse").unwrap();',
      },
      {
        type: "tool-body",
        text: '    assert!(verify_password("wrong", &hash).is_err());',
      },
      { type: "tool-body", text: "}" },
      { type: "tool-bottom" },
    ],
  },
  {
    id: "skill",
    title: "You can teach it new tricks.",
    paragraphs: [
      <>
        Drop a markdown file in{" "}
        <code className="bg-muted px-1.5 py-0.5 rounded text-sm font-mono">
          .mew/skills/
        </code>{" "}
        and it becomes a slash command. Share them with your team or keep them
        in your dotfiles. Skills are just prompt fragments and tool bundles.
      </>,
      "For deeper integrations, mew exposes a JSON-RPC 2.0 plugin runtime over stdin/stdout with 14 hook points. The TUI companion itself is a plugin, proving the runtime is robust enough for complex, stateful UI.",
    ],
    terminal: [
      { type: "user", text: "/pr-describe" },
      { type: "reasoning", text: "running skill: pr-describe" },
      { type: "reasoning", text: "checking recent commits..." },
      { type: "tool-top" },
      { type: "tool-header", name: "bash", status: "completed" },
      { type: "tool-path", text: "git diff HEAD~1 --stat" },
      { type: "tool-body", text: " src/auth.rs | 8 ++++----" },
      { type: "tool-body", text: " Cargo.toml  | 2 +-" },
      {
        type: "tool-body",
        text: " 2 files changed, 6 insertions(+), 4 deletions(-)",
      },
      { type: "tool-bottom" },
      { type: "blank" },
      {
        type: "assistant",
        text: "Migrate auth from bcrypt to argon2 for stronger password hashing.",
      },
    ],
  },
  {
    id: "jsonl",
    title: "Nothing is hidden.",
    paragraphs: [
      "Every session is saved locally as plain JSONL. You can read exactly what it did, what it thought, and what commands it ran. Nothing is locked away in a database, and nothing leaves your machine.",
      <>
        You can{" "}
        <code className="bg-muted px-1.5 py-0.5 rounded text-sm font-mono">
          tail -f
        </code>{" "}
        a session if you want. It\u2019s just lines on disk. Subagent sessions
        reference their parent, so you can trace the full execution tree.
      </>,
    ],
    terminal: [
      { type: "user", text: "show me what happened in my last session" },
      { type: "tool-top" },
      { type: "tool-header", name: "bash", status: "completed" },
      {
        type: "tool-path",
        text: "tail -f ~/.local/share/mew/sessions/4b2a/\u2026",
      },
      {
        type: "tool-body",
        text: '{"type":"chat_message","role":"user","content":"fix bug"}',
      },
      {
        type: "tool-body",
        text: '{"type":"tool_call","name":"grep","args":"..."}',
      },
      { type: "tool-body", text: '{"type":"tool_result","output":"..."}' },
      {
        type: "tool-body",
        text: '{"type":"chat_message","role":"assistant","content":"..."}',
      },
      { type: "tool-bottom" },
    ],
  },
  {
    id: "argon2",
    title: "Ready when you are.",
    paragraphs: [
      "It\u2019s not magic, it\u2019s just a careful collaborator that runs on your machine, uses your tools, and respects your boundaries.",
    ],
    terminal: [
      { type: "user", text: "convert the auth module to argon2" },
      { type: "reasoning", text: "let me check the current hashing setup." },
      { type: "tool-top" },
      { type: "tool-header", name: "grep", status: "completed" },
      { type: "tool-path", text: "'bcrypt' src/auth/" },
      {
        type: "tool-body",
        text: "src/auth/password.rs:4:use bcrypt::{hash, verify, BcryptError};",
      },
      { type: "tool-bottom" },
      { type: "blank" },
      { type: "tool-top" },
      { type: "tool-header", name: "edit", status: "completed" },
      { type: "tool-path", text: "src/auth/password.rs" },
      {
        type: "tool-body",
        text: "- use bcrypt::{hash, verify, BcryptError};",
        tone: "rem",
      },
      {
        type: "tool-body",
        text: "+ use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};",
        tone: "add",
      },
      { type: "tool-bottom" },
      { type: "blank" },
      { type: "assistant", text: "tests pass." },
    ],
  },
];

// --- Helpers ---

const getStatusIcon = (status: "completed" | "error") =>
  status === "completed" ? "\u2713" : "\u2717";

const getLineText = (line: TermLine): string | null => {
  switch (line.type) {
    case "user":
    case "assistant":
    case "reasoning":
    case "tool-path":
    case "tool-body":
      return line.text;
    case "tool-header":
      return `${getStatusIcon(line.status)} ${line.name}`;
    case "tool-more":
      return `... (${line.count} more lines)`;
    case "tool-top":
    case "tool-bottom":
    case "blank":
      return null;
  }
};

// --- Typewriter ---

interface TypingProgress {
  lineIndex: number;
  charIndex: number;
  complete: boolean;
}

const CHAR_DELAY_MS = 14;
const SPACE_DELAY_MS = 5;
const LINE_GAP_MS = 55;
const START_DELAY_MS = 160;

const useTypewriter = (
  id: string,
  lines: TermLine[],
  enabled: boolean,
): TypingProgress => {
  const [progress, setProgress] = useState<TypingProgress>({
    lineIndex: 0,
    charIndex: 0,
    complete: false,
  });

  useEffect(() => {
    if (!enabled) {
      setProgress({ lineIndex: 0, charIndex: 0, complete: false });
      return;
    }
    let cancelled = false;
    let timeoutId: ReturnType<typeof setTimeout> | null = null;
    setProgress({ lineIndex: 0, charIndex: 0, complete: false });

    let lineIndex = 0;
    let charIndex = 0;

    const tick = () => {
      if (cancelled) return;
      if (lineIndex >= lines.length) {
        setProgress({ lineIndex: lines.length, charIndex: 0, complete: true });
        return;
      }
      const text = getLineText(lines[lineIndex]);
      if (text === null) {
        setProgress({ lineIndex, charIndex: 0, complete: false });
        lineIndex++;
        timeoutId = setTimeout(tick, LINE_GAP_MS);
        return;
      }
      setProgress({ lineIndex, charIndex, complete: false });
      if (charIndex >= text.length) {
        lineIndex++;
        charIndex = 0;
        timeoutId = setTimeout(tick, LINE_GAP_MS);
      } else {
        charIndex++;
        const ch = text[charIndex - 1];
        const delay = ch === " " ? SPACE_DELAY_MS : CHAR_DELAY_MS;
        timeoutId = setTimeout(tick, delay);
      }
    };
    timeoutId = setTimeout(tick, START_DELAY_MS);
    return () => {
      cancelled = true;
      if (timeoutId) clearTimeout(timeoutId);
    };
  }, [id, enabled, lines]);
  return progress;
};

// --- Terminal line renderer ---

const TerminalContent = ({
  lines,
  progress,
}: {
  lines: TermLine[];
  progress: TypingProgress;
}) => {
  return (
    <>
      {lines.map((line, i) => {
        const text = getLineText(line);
        const isInstant = text === null;
        if (!progress.complete && i > progress.lineIndex) return null;
        const isCurrent = !progress.complete && i === progress.lineIndex;
        const displayText =
          isCurrent && !isInstant
            ? (text as string).slice(0, progress.charIndex)
            : (text ?? "");
        const showCursor = isCurrent && !isInstant;

        switch (line.type) {
          case "user":
            return (
              <div key={i}>
                <span className="text-cyan-400">{">\u00A0"}</span>
                <span>{displayText}</span>
                {showCursor && (
                  <span className="mew-cursor" aria-hidden="true" />
                )}
              </div>
            );
          case "assistant":
            return (
              <div key={i}>
                <span>{displayText}</span>
                {showCursor && (
                  <span className="mew-cursor" aria-hidden="true" />
                )}
              </div>
            );
          case "reasoning":
            return (
              <div key={i} className="text-zinc-500 italic">
                <span>{displayText}</span>
                {showCursor && (
                  <span className="mew-cursor" aria-hidden="true" />
                )}
              </div>
            );
          case "tool-top":
            return (
              <div
                key={i}
                className="h-2 bg-gradient-to-b from-tui-chat to-tui-tool"
                aria-hidden="true"
              />
            );
          case "tool-header": {
            const colorClass =
              line.status === "completed"
                ? "text-emerald-400"
                : "text-rose-400";
            return (
              <div key={i} className="bg-tui-tool pl-[2ch] pr-3 font-bold">
                <span className={colorClass}>{displayText}</span>
                {showCursor && (
                  <span className="mew-cursor" aria-hidden="true" />
                )}
              </div>
            );
          }
          case "tool-path":
            return (
              <div key={i} className="bg-tui-tool pl-[6ch] pr-3 text-zinc-500">
                <span>{displayText}</span>
                {showCursor && (
                  <span className="mew-cursor" aria-hidden="true" />
                )}
              </div>
            );
          case "tool-body": {
            const toneClass =
              line.tone === "add"
                ? "text-emerald-400"
                : line.tone === "rem"
                  ? "text-rose-400"
                  : "";
            return (
              <div key={i} className={`bg-tui-tool pl-[6ch] pr-3 ${toneClass}`}>
                <span>{displayText}</span>
                {showCursor && (
                  <span className="mew-cursor" aria-hidden="true" />
                )}
              </div>
            );
          }
          case "tool-more":
            return (
              <div key={i} className="bg-tui-tool pl-[6ch] pr-3 text-zinc-500">
                <span>{displayText}</span>
                {showCursor && (
                  <span className="mew-cursor" aria-hidden="true" />
                )}
              </div>
            );
          case "tool-bottom":
            return (
              <div
                key={i}
                className="h-2 bg-gradient-to-b from-tui-tool to-tui-chat"
                aria-hidden="true"
              />
            );
          case "blank":
            return <div key={i}>&nbsp;</div>;
        }
      })}
    </>
  );
};

// --- Model picker ---

const MODELS = [
  "opencode-go/deepseek-v4-pro",
  "opencode-go/deepseek-v4-flash",
  "opencode-zen/anthropic/claude-3-5-sonnet",
  "z-ai/glm-4.5-air",
  "anthropic/claude-3-5-sonnet",
  "openai/gpt-4o",
  "deepseek/deepseek-chat",
  "anthropic/claude-3-haiku",
];

const ModelPicker = () => {
  const selected = 0;

  return (
    <div className="flex items-center justify-center py-6">
      <div className="w-80 bg-tui-status font-mono text-[13px]">
        {/* filter */}
        <div className="px-2 pt-2 pb-1 flex items-center">
          <span className="text-cyan-400">{">\u00A0"}</span>
        </div>
        {/* divider */}
        <div className="h-px bg-tui-divider mx-2" />
        {/* list + scrollbar */}
        <div className="px-2 pt-1 pb-2 flex">
          <div className="flex-1 min-w-0">
            {MODELS.map((m, i) => (
              <div
                key={m}
                className={`px-2 py-0.5 text-xs leading-snug whitespace-nowrap ${
                  i === selected
                    ? "bg-white text-black font-bold"
                    : "text-zinc-300"
                }`}
              >
                {m}
              </div>
            ))}
          </div>
          {/* scrollbar */}
          <div className="w-3 flex-shrink-0 flex flex-col items-center ml-1">
            <span className="text-zinc-600 text-[8px] leading-none">▲</span>
            <div className="bg-zinc-600 w-px leading-none flex-1" />
            <div className="bg-zinc-400 w-full h-[0.6rem]" />
            <div className="bg-zinc-600 w-px leading-none flex-1" />
            <span className="text-zinc-600 text-[8px] leading-none">▼</span>
          </div>
        </div>
      </div>
    </div>
  );
};

// --- TUI chrome ---

const TuiChrome = () => (
  <>
    <div className="h-px bg-tui-divider" />
    <div className="bg-tui-status flex items-center h-7 px-2 gap-1 text-[11px] font-mono">
      <span className="px-1.5 py-0.5 bg-tui-model-bg text-tui-model-fg">
        opencode-go/deepseek-v4-pro
      </span>
      <span className="px-1.5 py-0.5 bg-tui-cwd-bg text-tui-cwd-fg">
        ~/code/mew
      </span>
      <span className="px-1.5 py-0.5 bg-tui-git-bg text-tui-git-fg">
        git: main
      </span>
      <span className="ml-auto text-zinc-500">
        0 / 128k tok {" \u00B7 "} $0.00
      </span>
    </div>
    <div className="bg-tui-status flex items-center h-9 px-3 font-mono text-[13px]">
      <span className="text-cyan-400">{">\u00A0"}</span>
      <span className="mew-cursor" aria-hidden="true" />
    </div>
  </>
);

// --- Desktop sticky terminal ---

const DesktopTerminal = ({ section }: { section: Section }) => {
  const progress: TypingProgress = {
    lineIndex: section.terminal.length,
    charIndex: 0,
    complete: true,
  };
  const body =
    section.graphic === "picker" ? (
      <ModelPicker />
    ) : (
      <div className="p-8 min-h-[320px] text-[13px] leading-snug">
        <TerminalContent lines={section.terminal} progress={progress} />
      </div>
    );
  return (
    <div className="mew-enter rounded-lg overflow-hidden font-mono text-zinc-100 bg-tui-chat shadow-sm" style={{ viewTransitionName: "terminal-card" }}>
      {body}
      <TuiChrome />
    </div>
  );
};

// --- Section item ---

const SectionItem = ({
  section,
  index,
  onRef,
}: {
  section: Section;
  index: number;
  onRef: (el: HTMLElement | null) => void;
}) => {
  const ref = useRef<HTMLElement | null>(null);
  const [visible, setVisible] = useState(false);
  const [hasStarted, setHasStarted] = useState(false);
  const progress = useTypewriter(section.id, section.terminal, hasStarted);

  useEffect(() => {
    if (visible && !hasStarted) setHasStarted(true);
  }, [visible, hasStarted]);

  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const observer = new IntersectionObserver(
      ([entry]) => {
        if (entry.isIntersecting) setVisible(true);
      },
      { threshold: 0.15 },
    );
    observer.observe(el);
    return () => observer.disconnect();
  }, []);

  const isFirst = index === 0;
  const titleClass = isFirst
    ? "text-2xl font-semibold tracking-tight mb-8 text-foreground"
    : `text-2xl font-semibold tracking-tight mb-8 text-foreground mew-reveal ${
        visible ? "mew-reveal-in" : ""
      }`;

  return (
    <section
      id={section.id}
      ref={(el) => {
        onRef(el);
        ref.current = el;
      }}
    >
      <h2 className={titleClass}>{section.title}</h2>

      <div className="space-y-6">
        {section.paragraphs.map((p, i) => {
          const paragraphClass = isFirst
            ? "text-lg text-foreground/80 leading-relaxed"
            : `text-lg text-foreground/80 leading-relaxed mew-reveal ${
                visible ? "mew-reveal-in" : ""
              }`;
          return (
            <p
              key={i}
              className={paragraphClass}
              style={
                isFirst ? undefined : { transitionDelay: `${(i + 1) * 90}ms` }
              }
            >
              {p}
            </p>
          );
        })}
      </div>

      <div className="lg:hidden mt-8 rounded-lg overflow-hidden font-mono text-zinc-100 bg-tui-chat shadow-sm">
        {section.graphic === "picker" ? (
          <ModelPicker />
        ) : (
          <div className="p-6 text-xs leading-snug min-h-[200px]">
            <TerminalContent lines={section.terminal} progress={progress} />
          </div>
        )}
        <TuiChrome />
      </div>
    </section>
  );
};

// --- Main component ---

export default function CapabilitiesSection() {
  const [activeId, setActiveId] = useState(sections[0].id);
  const [shown, setShown] = useState(false);
  const [overlayTop, setOverlayTop] = useState<number | null>(null);
  const sectionRefs = useRef<(HTMLElement | null)[]>([]);
  const gridRef = useRef<HTMLDivElement | null>(null);
  const overlayRef = useRef<HTMLDivElement | null>(null);
  const activeSection =
    sections.find((s) => s.id === activeId) || sections[0];

  const HAS_VIEW_TRANSITION =
    typeof document !== "undefined" && "startViewTransition" in document;

  useEffect(() => {
    const observer = new IntersectionObserver(
      (entries) => {
        entries.forEach((entry) => {
          if (entry.isIntersecting) {
            if (HAS_VIEW_TRANSITION) {
              (document as any).startViewTransition(() => {
                setActiveId(entry.target.id);
              });
            } else {
              setActiveId(entry.target.id);
            }
          }
        });
      },
      { threshold: 0, rootMargin: "-45% 0px -45% 0px" },
    );
    sectionRefs.current.forEach((sec) => {
      if (sec) observer.observe(sec);
    });
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    const grid = gridRef.current;
    const overlay = overlayRef.current;
    if (!grid || !overlay) return;
    let raf = 0;
    const update = () => {
      raf = 0;
      const g = grid.getBoundingClientRect();
      const overlayH = overlay.offsetHeight;
      const viewportH = window.innerHeight;
      const minTop = g.top;
      const maxTop = g.bottom - overlayH;
      let top = (viewportH - overlayH) / 2;
      if (maxTop < minTop) {
        top = minTop;
      } else {
        top = Math.max(top, minTop);
        top = Math.min(top, maxTop);
      }
      setOverlayTop(top);
      setShown(g.top < viewportH && g.bottom > 0);
    };
    const onScroll = () => {
      if (raf) return;
      raf = requestAnimationFrame(update);
    };
    update();
    window.addEventListener("scroll", onScroll, { passive: true });
    window.addEventListener("resize", onScroll);
    return () => {
      window.removeEventListener("scroll", onScroll);
      window.removeEventListener("resize", onScroll);
      if (raf) cancelAnimationFrame(raf);
    };
  }, [activeId]);

  return (
    <div>
      <div
        ref={gridRef}
        className="grid grid-cols-1 lg:grid-cols-2 gap-16 lg:gap-32 items-start"
      >
        <div className="hidden lg:block" aria-hidden="true" />

        <div className="space-y-48 lg:space-y-64">
          {sections.map((section, index) => (
            <SectionItem
              key={section.id}
              section={section}
              index={index}
              onRef={(el) => {
                sectionRefs.current[index] = el;
              }}
            />
          ))}
        </div>
      </div>

      <div
        ref={overlayRef}
        className={`hidden lg:block fixed inset-x-0 z-10 pointer-events-none transition-opacity duration-300 ${
          shown ? "opacity-100" : "opacity-0"
        }`}
        style={overlayTop !== null ? { top: `${overlayTop}px` } : undefined}
      >
        <div className="mx-auto max-w-7xl px-6">
          <div className="grid grid-cols-2 gap-32">
            {HAS_VIEW_TRANSITION ? (
              <DesktopTerminal section={activeSection} />
            ) : (
              <DesktopTerminal key={activeSection.id} section={activeSection} />
            )}
            <div />
          </div>
        </div>
      </div>
    </div>
  );
}
