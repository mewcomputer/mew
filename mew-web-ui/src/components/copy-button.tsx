import { useEffect, useRef, useState } from "react";
import { AlertCircle, Check, Copy } from "lucide-react";
import { cn } from "../lib/utils";

export function CopyButton({
  text,
  className,
  label,
}: {
  text: string;
  className?: string;
  label?: string;
}) {
  const [copied, setCopied] = useState(false);
  const [failed, setFailed] = useState(false);
  const reset = useRef<ReturnType<typeof setTimeout> | null>(null);
  useEffect(() => () => { if (reset.current) clearTimeout(reset.current); }, []);

  const handleCopy = async () => {
    setFailed(false);
    try {
      await navigator.clipboard.writeText(text);
      setCopied(true);
      if (reset.current) clearTimeout(reset.current);
      reset.current = setTimeout(() => setCopied(false), 2000);
    } catch {
      setFailed(true);
    }
  };

  return (
    <button
      onClick={handleCopy}
      className={cn(
        "motion-pressable flex h-7 shrink-0 items-center justify-center gap-1 rounded-md px-1.5 text-xs text-muted-foreground opacity-0 hover:bg-accent hover:text-foreground group-hover:opacity-100 group-focus-within:opacity-100 focus-visible:opacity-100",
        className,
      )}
      aria-label={copied ? "Copied" : "Copy"}
      title={failed ? "Could not copy. Try again." : copied ? "Copied" : "Copy"}
    >
      {copied ? <Check className="h-3.5 w-3.5 text-text-success" /> : failed ? <AlertCircle className="h-3.5 w-3.5 text-text-error" /> : <Copy className="h-3.5 w-3.5" />}
      {label && (copied ? "Copied" : label)}
      {failed && <span role="alert" className="sr-only">Could not copy. Try again.</span>}
    </button>
  );
}
