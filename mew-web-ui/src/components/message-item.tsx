import { memo, useEffect, useState } from "react";
import { ChevronRight, Download, Paperclip } from "lucide-react";
import type { AssistantMeta } from "@mew/web-client";
import { useSessionStore, type ChatMessage, type MessagePart } from "../stores/session";
import { getClient } from "../lib/client-ref";
import { ToolCallCard } from "./tool-call-card";
import { MarkdownBody } from "./markdown-body";
import { CopyButton } from "./copy-button";
import { MessageInspector } from "./message-inspector";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "./ui/collapsible";
import { Dialog, DialogContent, DialogTitle } from "./ui/dialog";
import { cn } from "../lib/utils";

export function messageCopyText(parts: MessagePart[]): string {
  return parts
    .filter((part): part is Extract<MessagePart, { type: "text" }> => part.type === "text")
    .map((part) => part.text)
    .join("\n\n");
}

/** Render message parts. Shared by single messages and folded turns. */
export function PartList({ parts, isUser }: { parts: MessagePart[]; isUser: boolean }) {
  const hasStreamingText = parts.some((part) => part.type === "text" && part.streaming);
  const hasStreamingReasoning = parts.some((part) => part.type === "reasoning" && part.streaming);
  const streamingText = useSessionStore((s) => hasStreamingText ? s.streamingText : "");
  const streamingReasoningText = useSessionStore((s) => hasStreamingReasoning ? s.streamingReasoningText : "");

  return (
    <>
      {parts.map((part, i) => {
        // Empty streaming parts still render the live text buffer. Only
        // completed empty parts should be omitted.
        if (part.type === "text" && !part.streaming && !part.text?.trim()) return null;
        return (
          <PartRenderer
            key={i}
            part={part}
            isUser={isUser}
            streamingText={streamingText}
            streamingReasoningText={streamingReasoningText}
          />
        );
      })}
    </>
  );
}

export function MessageActions({ meta, text }: { meta?: AssistantMeta; text: string }) {
  if (meta?.manifest) {
    return <MessageInspector manifest={meta.manifest} actions={text && <CopyButton text={text} className="opacity-60" />} />;
  }
  if (!text) return null;
  return (
    <div role="group" aria-label="Message actions" className="flex min-h-7 items-center">
      <CopyButton text={text} className="opacity-60" />
    </div>
  );
}

export const MessageItem = memo(function MessageItem({
  message,
}: {
  message: ChatMessage;
}) {
  const isUser = message.role === "user";
  const copyText = messageCopyText(message.parts);

  return (
    <div
      className={cn(
        "group flex min-w-0 max-w-full flex-col gap-2 typeset",
        isUser ? "items-end" : "items-start",
      )}
    >
      {isUser && message.steered && (
        <span className="rounded bg-muted/70 px-1.5 py-0.5 text-[10px] font-medium text-muted-foreground">
          Steered
        </span>
      )}
      <PartList parts={message.parts} isUser={isUser} />
      {message.role === "assistant" && <MessageActions meta={message.assistantMeta} text={copyText} />}
    </div>
  );
});

function PartRenderer({
  part,
  isUser,
  streamingText,
  streamingReasoningText,
}: {
  part: MessagePart;
  isUser: boolean;
  streamingText: string;
  streamingReasoningText: string;
}) {
  switch (part.type) {
    case "text":
      return (
        <div
          className={cn(
            "min-w-0 max-w-full py-1",
            isUser ? "max-w-[90%] rounded-2xl bg-muted px-3.5 py-2 text-foreground" : "",
          )}
        >
          {part.streaming ? (
            <MarkdownBody highlight={false}>
              {streamingText || "…"}
            </MarkdownBody>
          ) : (
            <MarkdownBody highlight={true}>{part.text}</MarkdownBody>
          )}
        </div>
      );

    case "reasoning":
      return <Thinking text={part.streaming ? streamingReasoningText || part.text : part.text} streaming={!!part.streaming} />;

    case "tool-call":
      return <ToolCallCard part={part} />;

    case "file":
      return <FilePartView part={part} isUser={isUser} />;

    case "compaction":
      return <CompactionDivider part={part} />;

    case "error":
      return (
        <div className="max-w-[85%] rounded-lg border border-destructive/50 bg-destructive/10 px-4 py-2.5 text-sm text-destructive">
          <span className="font-medium">Error:</span> {part.message}
        </div>
      );

    default:
      return null;
  }
}

function CompactionDivider({
  part,
}: {
  part: Extract<MessagePart, { type: "compaction" }>;
}) {
  const [expanded, setExpanded] = useState(false);
  const hasSummary = part.summary.trim().length > 0;
  const label =
    part.removed != null && part.removed > 0
      ? `Context compacted · ${part.removed} ${part.removed === 1 ? "message" : "messages"}`
      : "Context compacted";

  const divider = (
    <div className="flex w-full items-center gap-2 text-xs text-muted-foreground">
      <div className="h-px flex-1 bg-border/60" />
      <span className="inline-flex shrink-0 items-center gap-1 rounded-full border border-border/70 bg-muted/40 px-2.5 py-0.5 text-[11px] font-medium">
        {hasSummary && (
          <ChevronRight
            className={cn("h-3 w-3 shrink-0 transition-transform", expanded && "rotate-90")}
            aria-hidden="true"
          />
        )}
        <span>{label}</span>
      </span>
      <div className="h-px flex-1 bg-border/60" />
    </div>
  );

  if (!hasSummary) {
    return <div className="w-full py-1">{divider}</div>;
  }

  return (
    <Collapsible open={expanded} onOpenChange={setExpanded} className="w-full py-1 text-xs text-muted-foreground">
      <CollapsibleTrigger
        type="button"
        aria-label={expanded ? "Hide compaction summary" : "Show compaction summary"}
        className="w-full rounded-md py-0.5 hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
      >
        {divider}
      </CollapsibleTrigger>
      <CollapsibleContent className="mt-2 rounded-md border border-border/60 bg-muted/30 px-3 py-2 text-xs text-muted-foreground">
        <MarkdownBody highlight={false}>{part.summary}</MarkdownBody>
      </CollapsibleContent>
    </Collapsible>
  );
}

// Raster data URLs and daemon-hosted raster blobs are safe to inline; svg and
// arbitrary file paths stay as labelled cards instead of being loaded inline.
const INLINE_IMAGE = /^data:image\/(png|jpe?g|gif|webp|avif|bmp);base64,/i;
const SAFE_RASTER_MIME = /^image\/(png|jpe?g|gif|webp|avif|bmp)$/i;
const BLOB_URL_RE = /^mew-blob:\/\/([0-9a-f]{64})$/;

const blobUrlCache = new Map<string, string>();
const pendingBlobFetches = new Map<string, Promise<string | null>>();

function parseBlobId(url: string): string | null {
  return url.match(BLOB_URL_RE)?.[1] ?? null;
}

function base64ToObjectUrl(dataBase64: string, mime: string): string {
  const binary = atob(dataBase64);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) {
    bytes[i] = binary.charCodeAt(i);
  }
  return URL.createObjectURL(new Blob([bytes], { type: mime || "application/octet-stream" }));
}

function fetchBlobObjectUrl(id: string, mime: string): Promise<string | null> {
  const cached = blobUrlCache.get(id);
  if (cached) return Promise.resolve(cached);
  const inFlight = pendingBlobFetches.get(id);
  if (inFlight) return inFlight;

  const client = getClient();
  if (!client) return Promise.resolve(null);

  const task = client
    .getAttachment(id)
    .then(({ data_base64 }) => {
      const objectUrl = base64ToObjectUrl(data_base64, mime);
      blobUrlCache.set(id, objectUrl);
      return objectUrl;
    })
    .catch(() => null)
    .finally(() => {
      pendingBlobFetches.delete(id);
    });
  pendingBlobFetches.set(id, task);
  return task;
}

function useBlobUrl(url: string, mime: string): { resolvedUrl: string | null; loading: boolean } {
  const blobId = parseBlobId(url);
  const cached = blobId ? (blobUrlCache.get(blobId) ?? null) : null;
  const [fetchedUrl, setFetchedUrl] = useState<string | null>(cached);
  const [loading, setLoading] = useState(Boolean(blobId && !cached));

  useEffect(() => {
    if (!blobId) {
      setFetchedUrl(null);
      setLoading(false);
      return;
    }
    const existing = blobUrlCache.get(blobId);
    if (existing) {
      setFetchedUrl(existing);
      setLoading(false);
      return;
    }
    let cancelled = false;
    setLoading(true);
    void fetchBlobObjectUrl(blobId, mime).then((next) => {
      if (cancelled) return;
      setFetchedUrl(next);
      setLoading(false);
    });
    return () => {
      cancelled = true;
    };
  }, [blobId, mime]);

  if (blobId) return { resolvedUrl: fetchedUrl, loading };
  if (INLINE_IMAGE.test(url)) return { resolvedUrl: url, loading: false };
  return { resolvedUrl: null, loading: false };
}

function FilePartView({ part, isUser }: { part: Extract<MessagePart, { type: "file" }>; isUser: boolean }) {
  const [lightboxOpen, setLightboxOpen] = useState(false);
  const blobId = parseBlobId(part.url);
  const isImage = INLINE_IMAGE.test(part.url) || (blobId !== null && SAFE_RASTER_MIME.test(part.mime));
  const { resolvedUrl, loading } = useBlobUrl(part.url, part.mime);
  const label = part.filename ?? (part.mime || "Attachment");

  if (isImage) {
    if (loading && !resolvedUrl) {
      return (
        <div
          aria-label={`Loading ${label}`}
          className="h-32 w-48 animate-pulse rounded-lg border border-border bg-muted/40"
        />
      );
    }
    if (resolvedUrl) {
      return (
        <>
          <div className={cn("min-w-0 max-w-full py-1", isUser ? "max-w-[90%] rounded-2xl bg-muted p-2" : "")}>
            <button
              type="button"
              onClick={() => setLightboxOpen(true)}
              aria-label={`Preview ${label}`}
              className="block cursor-zoom-in rounded-lg focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            >
              <img
                src={resolvedUrl}
                loading="lazy"
                alt={part.filename ?? "Attached image"}
                className="max-h-80 max-w-full rounded-lg border border-border object-contain"
              />
            </button>
          </div>
          <Dialog open={lightboxOpen} onOpenChange={setLightboxOpen}>
            <DialogContent className="max-w-4xl border-border bg-background p-3">
              <DialogTitle className="truncate pr-8 text-xs font-medium text-muted-foreground">
                {label}
              </DialogTitle>
              <img
                src={resolvedUrl}
                alt={part.filename ?? "Attached image"}
                className="max-h-[80vh] w-full rounded-md object-contain"
              />
            </DialogContent>
          </Dialog>
        </>
      );
    }
  }

  return (
    <div className="inline-flex max-w-full items-center gap-2 rounded-lg border border-border bg-muted/60 px-3 py-2 text-xs">
      <Paperclip className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
      <span className="truncate">{label}</span>
      {resolvedUrl && (
        <a
          href={resolvedUrl}
          download={part.filename ?? "attachment"}
          aria-label={`Download ${label}`}
          className="rounded p-0.5 text-muted-foreground hover:bg-accent hover:text-foreground"
        >
          <Download className="h-3.5 w-3.5" />
        </a>
      )}
    </div>
  );
}

function Thinking({ text, streaming }: { text: string; streaming: boolean }) {
  const [expanded, setExpanded] = useState(false);
  const preview = text.split("\n").find((line) => line.trim())?.trim();
  return (
    <Collapsible open={expanded} onOpenChange={setExpanded} className="min-w-0 max-w-full text-sm text-muted-foreground">
      <CollapsibleTrigger
        type="button"
        aria-label={expanded ? "Hide thinking" : "Show thinking"}
        className="-ml-1 flex max-w-full min-w-0 items-center gap-1.5 rounded-md px-1 py-1 text-left text-xs hover:bg-muted/50 hover:text-foreground"
      >
        <ChevronRight className={cn("h-3 w-3 shrink-0 transition-transform", expanded && "rotate-90")} aria-hidden="true" />
        <span className="shrink-0">{streaming ? "Thinking" : "Thought"}</span>
        {!expanded && preview && <span className="truncate">· {preview}</span>}
      </CollapsibleTrigger>
      <CollapsibleContent className="mew-thinking-content">
        <p className="whitespace-pre-wrap">{text || "…"}</p>
      </CollapsibleContent>
    </Collapsible>
  );
}
