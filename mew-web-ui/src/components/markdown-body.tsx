import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { CodeBlock } from "./code-block";

function guessLanguage(className?: string): string {
  const match = /language-(\w+)/.exec(className ?? "");
  return match?.[1] ?? "text";
}

export function MarkdownBody({
  children,
  highlight = false,
}: {
  children: string;
  highlight?: boolean;
}) {
  const codeComponent = (props: {
    className?: string;
    children?: React.ReactNode;
  }) => {
    // react-markdown v9+ no longer passes an `inline` prop. Inline code
    // (single backticks) has no className and no newline. Block code
    // (triple backticks) has a `language-*` className.
    const isBlock = props.className?.includes("language-") ||
      String(props.children).includes("\n");

    if (!isBlock) {
      // Inline code: render as a styled <code> element.
      return (
        <code className="rounded bg-muted px-1.5 py-0.5 text-[0.875em] font-mono">
          {props.children}
        </code>
      );
    }

    const code = String(props.children).replace(/\n$/, "");
    const lang = guessLanguage(props.className);

    if (highlight) {
      return <CodeBlock code={code} lang={lang} />;
    }

    return (
      <pre className="my-4 overflow-x-auto rounded-lg border border-border bg-muted p-4 text-sm">
        <code className={props.className}>{props.children}</code>
      </pre>
    );
  };

  return (
    <div className="prose prose-sm min-w-0 max-w-full overflow-x-hidden break-words [overflow-wrap:anywhere] dark:prose-invert">
      <ReactMarkdown remarkPlugins={[remarkGfm]} components={{ code: codeComponent }}>
        {children}
      </ReactMarkdown>
    </div>
  );
}
