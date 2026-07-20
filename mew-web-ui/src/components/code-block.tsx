import { useEffect, useState } from "react";
import { CopyButton } from "./copy-button";
import { useTheme } from "../lib/theme";
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
}: {
  code: string;
  lang: string;
  showHeader?: boolean;
  lineNumbers?: boolean;
  wrapLines?: boolean;
  flush?: boolean;
  fill?: boolean;
}) {
  const { mode } = useTheme();
  const [html, setHtml] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    highlightCode(code, lang, mode).then((h) => {
      if (!cancelled) setHtml(h);
    });
    return () => {
      cancelled = true;
    };
  }, [code, lang, mode]);

  return (
    <div className={cn(
      "group relative w-full min-w-0 max-w-full overflow-hidden",
      fill && "flex min-h-0 flex-1 flex-col",
      showHeader && "my-4 rounded-lg border border-border",
    )}>
      {showHeader && (
        <div className="flex items-center justify-between border-b border-border bg-muted px-3 py-1.5">
          <span className="font-mono text-xs text-muted-foreground">{lang}</span>
          <CopyButton text={code} />
        </div>
      )}
      {html ? (
        <div
          className={cn(
            "w-full min-w-0 max-w-full overflow-x-auto p-4 text-sm",
            lineNumbers && "file-viewer-code",
            wrapLines && "file-viewer-code-wrapped overflow-x-hidden",
            flush && "p-0",
            fill && "file-viewer-code-fill flex min-h-0 flex-1 flex-col",
          )}
          dangerouslySetInnerHTML={{ __html: html }}
        />
      ) : (
        <pre className="w-full min-w-0 max-w-full overflow-x-auto bg-muted p-4 text-sm">
          <code>{code}</code>
        </pre>
      )}
    </div>
  );
}
