import { useCallback, useEffect, useRef, useState } from "react";
import { ArrowUpRight } from "lucide-react";

export const ARTIFACT_SANDBOX = "allow-scripts allow-downloads allow-forms";
const MIN_INLINE_HEIGHT = 120;
const MAX_INLINE_HEIGHT = 900;

const THEME_VAR_NAMES = [
  "--background",
  "--foreground",
  "--card",
  "--card-foreground",
  "--popover",
  "--popover-foreground",
  "--primary",
  "--primary-foreground",
  "--secondary",
  "--secondary-foreground",
  "--muted",
  "--muted-foreground",
  "--accent",
  "--accent-foreground",
  "--destructive",
  "--destructive-foreground",
  "--border",
  "--divider",
  "--input",
  "--ring",
  "--chart-1",
  "--chart-2",
  "--chart-3",
  "--chart-4",
  "--blue-fg",
  "--blue-bg",
  "--green-fg",
  "--green-bg",
  "--cyan-fg",
  "--cyan-bg",
];

function collectThemePayload() {
  const root = document.documentElement;
  const computed = getComputedStyle(root);
  const vars: Record<string, string> = {};
  for (const name of THEME_VAR_NAMES) {
    const value = computed.getPropertyValue(name).trim();
    if (value) vars[name] = value;
  }
  return {
    type: "mew:theme",
    isDark: root.classList.contains("dark"),
    theme: root.getAttribute("data-theme") ?? "",
    vars,
  };
}

export function isSafeArtifactUrl(url?: string | null): url is string {
  if (!url) return false;
  return url.startsWith("/artifacts/") && !url.startsWith("//") && !/[\r\n]/.test(url);
}

export interface ArtifactVersionEntry {
  url: string;
  title: string;
  version: number;
}

export function collectArtifactVersions(
  messages: ReadonlyArray<{ parts: ReadonlyArray<{ type: string; [key: string]: unknown }> }>,
  target: { sourcePath?: string; title?: string },
): ArtifactVersionEntry[] {
  const entries: ArtifactVersionEntry[] = [];
  let fallbackVersion = 1;
  for (const msg of messages) {
    for (const part of msg.parts) {
      if (part.type !== "artifact") continue;
      const partSource = typeof part.sourcePath === "string" ? part.sourcePath : undefined;
      const partTitle = typeof part.title === "string" ? part.title : "Artifact";
      const partUrl = typeof part.url === "string" ? part.url : "";
      if (!partUrl) continue;
      const matches = target.sourcePath
        ? partSource === target.sourcePath
        : target.title != null && partTitle === target.title;
      if (!matches) continue;
      const version =
        typeof part.version === "number" && Number.isFinite(part.version)
          ? part.version
          : fallbackVersion;
      fallbackVersion = Math.max(fallbackVersion + 1, version + 1);
      if (!entries.some((e) => e.url === partUrl)) {
        entries.push({ url: partUrl, title: partTitle, version });
      }
    }
  }
  entries.sort((a, b) => a.version - b.version);
  return entries;
}

export function ArtifactFrame({
  title,
  url,
  mode,
  height,
  version,
  versions,
  onSelectVersion,
  onOpenInPanel,
}: {
  title: string;
  url?: string | null;
  mode: "inline" | "panel";
  height?: number;
  version?: number;
  versions?: ArtifactVersionEntry[];
  onSelectVersion?: (entry: ArtifactVersionEntry) => void;
  onOpenInPanel?: () => void;
}) {
  const iframeRef = useRef<HTMLIFrameElement>(null);
  const [inlineHeight, setInlineHeight] = useState(() => height ?? 360);
  const [selectedUrl, setSelectedUrl] = useState<string | null | undefined>(url);

  useEffect(() => {
    setSelectedUrl(url);
  }, [url]);

  const activeUrl = onSelectVersion ? url : selectedUrl;
  const activeEntry = versions?.find((v) => v.url === activeUrl);
  const displayTitle = activeEntry?.title ?? title;
  const displayVersion = activeEntry?.version ?? version;

  const syncTheme = useCallback(() => {
    const frameWindow = iframeRef.current?.contentWindow;
    if (!frameWindow) return;
    frameWindow.postMessage(collectThemePayload(), "*");
  }, []);

  useEffect(() => {
    setInlineHeight(height ?? 360);
  }, [url, height]);

  useEffect(() => {
    if (typeof MutationObserver === "undefined") return;
    const observer = new MutationObserver(() => syncTheme());
    observer.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ["class", "data-theme", "style"],
    });
    return () => observer.disconnect();
  }, [syncTheme]);

  useEffect(() => {
    if (mode !== "inline") return;
    const onMessage = (event: MessageEvent) => {
      const frameWindow = iframeRef.current?.contentWindow;
      if (!frameWindow || event.source !== frameWindow) return;
      const data = event.data as { type?: unknown; height?: unknown } | null;
      if (!data || (data.type !== "mew:artifact-resize" && data.type !== "mew:resize")) return;
      if (typeof data.height === "number" && Number.isFinite(data.height)) {
        setInlineHeight(
          Math.max(MIN_INLINE_HEIGHT, Math.min(MAX_INLINE_HEIGHT, Math.round(data.height))),
        );
      }
    };
    window.addEventListener("message", onMessage);
    return () => window.removeEventListener("message", onMessage);
  }, [mode]);

  if (!isSafeArtifactUrl(activeUrl)) {
    return (
      <div className="flex items-center justify-center rounded-md border border-border bg-muted/20 p-4 text-xs text-muted-foreground">
        Invalid artifact URL.
      </div>
    );
  }

  return (
    <div
      className={
        mode === "inline"
          ? "my-2 overflow-hidden rounded-lg border border-border bg-background"
          : "flex h-full min-h-0 flex-col"
      }
    >
      <div
        className={
          mode === "inline"
            ? "flex h-8 items-center justify-between gap-2 border-b border-border bg-muted/30 px-3 text-xs"
            : "flex h-9 shrink-0 items-center justify-between gap-2 border-b border-border px-3 text-xs"
        }
      >
        <div className="flex min-w-0 items-center gap-2">
          <span className="truncate font-medium text-foreground">{displayTitle}</span>
          {versions && versions.length > 1 ? (
            <select
              aria-label="Artifact version"
              value={activeUrl}
              onChange={(e) => {
                const next = versions.find((v) => v.url === e.target.value);
                if (!next) return;
                if (onSelectVersion) onSelectVersion(next);
                else setSelectedUrl(next.url);
              }}
              className="h-5 rounded border border-border bg-background px-1.5 text-[11px] font-medium text-muted-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
            >
              {versions.map((v) => (
                <option key={v.url} value={v.url}>
                  v{v.version}
                </option>
              ))}
            </select>
          ) : displayVersion != null ? (
            <span className="rounded bg-muted px-1.5 py-0.5 text-[11px] font-medium text-muted-foreground">
              v{displayVersion}
            </span>
          ) : null}
        </div>
        {mode === "inline" && onOpenInPanel ? (
          <button
            type="button"
            onClick={onOpenInPanel}
            className="inline-flex shrink-0 items-center gap-1 text-muted-foreground hover:text-foreground focus-visible:outline-none focus-visible:underline"
          >
            <span>Open in panel</span>
            <ArrowUpRight className="h-3 w-3" aria-hidden="true" />
          </button>
        ) : (
          <a
            href={activeUrl}
            target="_blank"
            rel="noopener noreferrer"
            className="shrink-0 text-muted-foreground hover:text-foreground"
          >
            Open in tab ↗
          </a>
        )}
      </div>
      <iframe
        ref={iframeRef}
        sandbox={ARTIFACT_SANDBOX}
        src={activeUrl}
        title={displayTitle}
        onLoad={syncTheme}
        style={mode === "inline" ? { height: `${inlineHeight}px` } : undefined}
        className={
          mode === "inline"
            ? "block w-full border-0 bg-background"
            : "min-h-0 w-full flex-1 border-0 bg-background"
        }
      />
    </div>
  );
}
