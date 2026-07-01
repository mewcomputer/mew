import { useEffect, useState } from "react";
import { CopyButton } from "./copy-button";
import { useTheme } from "../lib/theme";
import { highlightCode } from "../lib/shiki";

export function CodeBlock({ code, lang }: { code: string; lang: string }) {
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
    <div className="group relative my-4 overflow-hidden rounded-lg border border-border">
      <div className="flex items-center justify-between border-b border-border bg-muted px-3 py-1.5">
        <span className="font-mono text-xs text-muted-foreground">{lang}</span>
        <CopyButton text={code} />
      </div>
      {html ? (
        <div
          className="overflow-x-auto p-4 text-sm"
          dangerouslySetInnerHTML={{ __html: html }}
        />
      ) : (
        <pre className="overflow-x-auto bg-muted p-4 text-sm">
          <code>{code}</code>
        </pre>
      )}
    </div>
  );
}
