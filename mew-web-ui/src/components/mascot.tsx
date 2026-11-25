import { useEffect, useRef, useState } from "react";
import type { AnimationItem } from "lottie-web";
import { cn } from "../lib/utils";

export const MASCOT_STATUSES = [
  "thinking",
  "working",
  "searching",
  "waiting",
  "web-search",
  "python",
  "reading-files",
  "running-tests",
  "editing-code",
  "shell-command",
  "fetching-url",
  "reviewing-diff",
  "starting",
  "busy",
  "queued",
  "retrying",
] as const;

export const MASCOT_IDLES = [
  "big-stretch",
  "big-yawn",
  "box-pop",
  "double-hop",
  "dust-speck-chase",
  "ear-flick",
  "happy-wiggle",
  "loaf-flop",
  "moonwalk",
  "paw-fidget",
  "paw-groom",
  "sneak-and-pounce",
  "sneeze-and-reset",
  "snooze-fake-out",
  "tiny-tumble",
  "yarn-bat",
] as const;

export const MASCOT_STATUS_ALTERNATES = [
  "thinking-alt",
  "busy-alt",
  "waiting-alt",
  "retrying-alt",
  "starting-alt",
  "working-alt",
  "searching-alt",
  "queued-alt",
] as const;

export type MascotStatus = (typeof MASCOT_STATUSES)[number];
export type MascotStatusAlternate = (typeof MASCOT_STATUS_ALTERNATES)[number];
export type MascotIdle = (typeof MASCOT_IDLES)[number];
export type MascotState = MascotStatus | MascotStatusAlternate | MascotIdle | "neutral";
export type MascotInk = "dark-ink" | "light-ink";

export const MASCOT_ALTERNATES: Partial<Record<MascotStatus, MascotStatusAlternate>> = {
  thinking: "thinking-alt",
  busy: "busy-alt",
  waiting: "waiting-alt",
  retrying: "retrying-alt",
  starting: "starting-alt",
  working: "working-alt",
  searching: "searching-alt",
  queued: "queued-alt",
};

const STATUS_SET = new Set<string>([...MASCOT_STATUSES, ...MASCOT_STATUS_ALTERNATES]);

const TEST_COMMAND_RE = /\b(test|vitest|pytest|jest|playwright)\b/i;
const DIFF_COMMAND_RE = /\bgit\s+(diff|status|log|show)\b/i;

export function mascotStateForActivity({
  streamingReasoning = false,
  runningToolName = null,
  runningCommand = null,
}: {
  streamingReasoning?: boolean;
  runningToolName?: string | null;
  runningCommand?: string | null;
}): MascotStatus {
  if (runningToolName) {
    const tool = runningToolName.toLowerCase();
    if (tool === "read") return "reading-files";
    if (tool.startsWith("edit") || tool.startsWith("write")) return "editing-code";
    if (tool === "grep" || tool === "glob") return "searching";
    if (tool === "web_fetch") return "fetching-url";
    if (tool === "python") return "python";
    if (tool.startsWith("browser")) return "web-search";
    if (tool === "bash" || tool === "shell") {
      if (runningCommand && TEST_COMMAND_RE.test(runningCommand)) return "running-tests";
      if (runningCommand && DIFF_COMMAND_RE.test(runningCommand)) return "reviewing-diff";
      return "shell-command";
    }
    return "busy";
  }

  if (streamingReasoning) return "thinking";
  return "busy";
}

function useDocumentInk(override?: MascotInk): MascotInk {
  const [dark, setDark] = useState(() =>
    typeof document !== "undefined" && document.documentElement.classList.contains("dark"),
  );

  useEffect(() => {
    if (override || typeof document === "undefined") return;
    const root = document.documentElement;
    const update = () => setDark(root.classList.contains("dark"));
    update();
    const observer = new MutationObserver(update);
    observer.observe(root, { attributes: true, attributeFilter: ["class"] });
    return () => observer.disconnect();
  }, [override]);

  if (override) return override;
  return dark ? "light-ink" : "dark-ink";
}

function usePrefersReducedMotion(): boolean {
  const [reduced, setReduced] = useState(() =>
    typeof window !== "undefined" &&
    typeof window.matchMedia === "function" &&
    window.matchMedia("(prefers-reduced-motion: reduce)").matches,
  );

  useEffect(() => {
    if (typeof window === "undefined" || typeof window.matchMedia !== "function") return;
    const mq = window.matchMedia("(prefers-reduced-motion: reduce)");
    const update = () => setReduced(mq.matches);
    update();
    mq.addEventListener?.("change", update);
    return () => mq.removeEventListener?.("change", update);
  }, []);

  return reduced;
}

export function mascotAssetPath(state: MascotState, ink: MascotInk): string {
  if (state === "neutral") return `/mascot/status/${ink}/neutral.png`;
  const pack = STATUS_SET.has(state) ? "status" : "idle";
  return `/mascot/${pack}/${ink}/${state}.json`;
}

/** Pixel cat mascot rendered from a Lottie status/idle clip, with neutral PNG fallback. */
export function Mascot({
  state,
  size = 32,
  loop = true,
  speed = 1.35,
  alternate = true,
  ink: inkOverride,
  className,
}: {
  state: MascotState;
  size?: number;
  loop?: boolean;
  speed?: number;
  alternate?: boolean;
  ink?: MascotInk;
  className?: string;
}) {
  const containerRef = useRef<HTMLDivElement>(null);
  const ink = useDocumentInk(inkOverride);
  const reducedMotion = usePrefersReducedMotion();
  const isJsdom = typeof navigator !== "undefined" && /jsdom/i.test(navigator.userAgent);
  const staticOnly = state === "neutral" || reducedMotion || isJsdom;
  const [useAlt, setUseAlt] = useState(false);

  useEffect(() => {
    setUseAlt(false);
  }, [state]);

  const altState = alternate && loop ? MASCOT_ALTERNATES[state as MascotStatus] : undefined;
  const activeState = altState && useAlt ? altState : state;
  const shouldCycle = Boolean(altState);

  useEffect(() => {
    if (staticOnly) return;
    const container = containerRef.current;
    if (!container) return;

    let cancelled = false;
    let anim: AnimationItem | null = null;
    import("lottie-web")
      .then(({ default: lottie }) => {
        if (cancelled) return;
        anim = lottie.loadAnimation({
          container,
          renderer: "svg",
          loop: shouldCycle ? false : loop,
          autoplay: true,
          path: mascotAssetPath(activeState, ink),
        });
        anim.setSpeed(speed);
        if (shouldCycle) {
          anim.addEventListener("complete", () => {
            if (!cancelled) setUseAlt((value) => !value);
          });
        }
      })
      .catch(() => {
        // Ignore in environments where dynamic import or SVG loader is unavailable.
      });

    return () => {
      cancelled = true;
      anim?.destroy();
    };
  }, [activeState, ink, loop, shouldCycle, speed, staticOnly]);

  const boxStyle = { width: size, height: size };

  if (staticOnly) {
    return (
      <img
        src={mascotAssetPath("neutral", ink)}
        alt=""
        aria-hidden="true"
        width={size}
        height={size}
        style={boxStyle}
        className={cn("inline-block shrink-0 select-none [image-rendering:pixelated]", className)}
      />
    );
  }

  return (
    <div
      ref={containerRef}
      aria-hidden="true"
      style={boxStyle}
      className={cn("inline-block shrink-0 select-none overflow-hidden", className)}
    />
  );
}
