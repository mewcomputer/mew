import { useEffect, useState } from "react";
import { FileCode2, FileJson2, FileText, type LucideIcon } from "lucide-react";
import { Markdown, type IslandData } from "@pierre/words/react";
import { createHighlightsPlugin } from "@pierre/words/highlights";
import dark from "@pierre/highlights/themes/pierre-dark";
import light from "@pierre/highlights/themes/pierre-light";
import { useSessionStore } from "../stores/session";
import { fileExists } from "../lib/file-existence";
import "@pierre/words/styles.css";

const plugins = {
  code: createHighlightsPlugin({
    themes: { light, dark },
    defaultColor: "light-dark()",
  }),
};

const controls = { code: { copy: true, download: false } };

function fileStyle(path: string): { icon: LucideIcon; color: string } {
  const extension = path.split(".").pop()?.toLowerCase();
  if (["json", "jsonl", "yml", "yaml", "toml"].includes(extension ?? "")) {
    return { icon: FileJson2, color: "#5ecb71" };
  }
  if (["ts", "tsx", "js", "jsx", "mjs", "cjs"].includes(extension ?? "")) {
    return { icon: FileCode2, color: "#69b1ff" };
  }
  if (["rs", "go", "py", "rb", "java", "c", "cpp"].includes(extension ?? "")) {
    return { icon: FileCode2, color: "#ffa359" };
  }
  return { icon: FileText, color: "#a3a3a3" };
}

function FileChip({ path, label }: { path: string; label: string }) {
  const sessionId = useSessionStore((state) => state.sessionId);
  const { icon: Icon, color } = fileStyle(path);
  return (
    <button
      type="button"
      title={`Open ${path}`}
      aria-label={`Open ${path}`}
      onClick={() => window.dispatchEvent(new CustomEvent("mew:open-file", { detail: { sessionId, path } }))}
      className="mx-0.5 inline-flex max-w-full items-center gap-1 rounded px-1 py-0.5 align-[-2px] text-foreground hover:bg-muted focus-visible:bg-muted"
    >
      <Icon className="h-3.5 w-3.5 shrink-0" style={{ color }} aria-hidden="true" />
      <span className="truncate">{label}</span>
    </button>
  );
}

function FileRef({ path, label }: { path: string; label: string }) {
  const sessionId = useSessionStore((state) => state.sessionId);
  const connected = useSessionStore((state) => state.connectionState === "connected");
  const [exists, setExists] = useState(false);

  useEffect(() => {
    if (!connected || !sessionId) { setExists(false); return; }
    let active = true;
    void fileExists(sessionId, path).then((found) => { if (active) setExists(found); });
    return () => { active = false; };
  }, [connected, sessionId, path]);

  if (!exists) return <code>{label}</code>;
  return <FileChip path={path} label={label} />;
}

const islands = {
  file: {
    inline: true,
    render: (data: IslandData) => {
      const path = typeof data.path === "string" ? data.path : "";
      if (!path) return null;
      const label = typeof data.label === "string" ? data.label : path;
      return <FileRef path={path} label={label} />;
    },
  },
};

/** Render chat markdown. `highlight` is false while the text is still streaming. */
export function MarkdownBody({
  children,
  highlight = false,
}: {
  children: string;
  highlight?: boolean;
}) {
  return (
    <div className="min-w-0 max-w-full overflow-x-hidden break-words [overflow-wrap:anywhere]">
      <Markdown
        mode="streaming"
        isAnimating={!highlight}
        plugins={plugins}
        controls={controls}
        lineNumbers={false}
        islands={islands}
      >
        {children}
      </Markdown>
    </div>
  );
}
