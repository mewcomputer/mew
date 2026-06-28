import { useState, useRef, type KeyboardEvent } from "react";
import { Send, Square } from "lucide-react";
import { cn } from "../lib/utils";
import { useSessionStore } from "../stores/session";

export function InputArea({
  onSend,
  onCancel,
  connected,
}: {
  onSend: (text: string) => void;
  onCancel: () => void;
  connected: boolean;
}) {
  const [text, setText] = useState("");
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  // Track streaming state from the store
  const hasStreaming = useSessionStore_streaming();

  const handleKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      handleSubmit();
    }
  };

  const handleSubmit = () => {
    const trimmed = text.trim();
    if (!trimmed || !connected) return;
    onSend(trimmed);
    setText("");
    // Reset textarea height
    if (textareaRef.current) {
      textareaRef.current.style.height = "auto";
    }
  };

  const autoResize = () => {
    const el = textareaRef.current;
    if (el) {
      el.style.height = "auto";
      el.style.height = Math.min(el.scrollHeight, 200) + "px";
    }
  };

  return (
    <div className="border-t border-border p-4">
      <div className="mx-auto flex max-w-3xl items-end gap-2">
        <textarea
          ref={textareaRef}
          value={text}
          onChange={(e) => {
            setText(e.target.value);
            autoResize();
          }}
          onKeyDown={handleKeyDown}
          placeholder={connected ? "Ask mew anything…  (Cmd/Ctrl + Enter to send, Enter for newline)" : "Connecting…"}
          disabled={!connected}
          rows={1}
          className={cn(
            "flex-1 resize-none rounded-lg border border-border bg-card px-4 py-2.5 text-sm",
            "placeholder:text-muted-foreground focus:outline-hidden focus:ring-1 focus:ring-ring",
            "disabled:opacity-50",
          )}
        />
        {hasStreaming ? (
          <button
            onClick={onCancel}
            className="flex h-10 w-10 shrink-0 items-center justify-center rounded-lg border border-red-500/50 text-red-500 hover:bg-red-500/10"
            title="Cancel (Cmd+.)"
          >
            <Square className="h-4 w-4" />
          </button>
        ) : (
          <button
            onClick={handleSubmit}
            disabled={!text.trim() || !connected}
            className="flex h-10 w-10 shrink-0 items-center justify-center rounded-lg bg-primary text-primary-foreground hover:bg-primary/90 disabled:opacity-50"
            title="Send (Cmd/Ctrl + Enter)"
          >
            <Send className="h-4 w-4" />
          </button>
        )}
      </div>
    </div>
  );
}

// Small hook to check if the agent is currently streaming
function useSessionStore_streaming(): boolean {
  return useSessionStore((s) => s.streamingPartId !== null);
}