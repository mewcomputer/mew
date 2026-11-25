import { useEffect, useState } from "react";
import { CopyButton } from "./copy-button";
import { highlightCode } from "../lib/shiki";
import { cn } from "../lib/utils";

export function CodeBlock({
  code,
  lang,
  showHeader = true,
  lineNumbers = false,
  wrapLines = false,
  flush = false,
  fill = false,
  highlight = true,
}: {
  code: string;
  lang: string;
  showHeader?: boolean;
  lineNumbers?: boolean;
  wrapLines?: boolean;
  flush?: boolean;
  fill?: boolean;
  highlight?: boolean;
}) {
  const [highlighted, setHighlighted] = useState<{ code: string; lang: string; html: string } | null>(null);

  useEffect(() => {
    if (!highlight) {
      setHighlighted(null);
      return;
    }
    let cancelled = false;
    highlightCode(code, lang).then((h) => {
      if (!cancelled) setHighlighted({ code, lang, html: h });
    });
    return () => {
      cancelled = true;
    };
  }, [code, lang, highlight]);

  return (
    <div className={cn(
      "group relative w-full min-w-0 max-w-full overflow-hidden",
      fill && "flex min-h-0 flex-1 flex-col",
      showHeader && "not-typeset my-2 rounded-md border border-border bg-muted/30",
    )}>
      {showHeader && (
        <div className="flex h-7 items-center justify-between px-3">
          <span className="font-mono text-xs text-muted-foreground">{lang}</span>
          <CopyButton text={code} />
        </div>
      )}
      {highlight && highlighted?.code === code && highlighted.lang === lang ? (
        <div
          className={cn(
            "w-full min-w-0 max-w-full overflow-x-auto p-4 text-sm",
            showHeader && "mew-code-content px-3 pb-2 pt-0 text-[13px] leading-relaxed",
            lineNumbers && "file-viewer-code",
            wrapLines && "file-viewer-code-wrapped overflow-x-hidden",
            flush && "p-0",
            fill && "file-viewer-code-fill flex min-h-0 flex-1 flex-col",
          )}
          dangerouslySetInnerHTML={{ __html: highlighted.html }}
        />
      ) : (
        <pre className={cn("w-full min-w-0 max-w-full overflow-x-auto bg-muted p-4 text-sm", showHeader && "m-0 bg-transparent px-3 pb-2 pt-0 text-[13px] leading-relaxed")}>
          <code>{code}</code>
        </pre>
      )}
    </div>
  );
}
