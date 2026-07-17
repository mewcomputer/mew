import {
  forwardRef,
  useRef,
  useState,
  useImperativeHandle,
  type ClipboardEvent,
  type DragEvent,
  type KeyboardEvent,
} from "react";
import { Square, Paperclip, X, CornerDownLeft } from "lucide-react";
import { cn } from "../lib/utils";
import { useSessionStore } from "../stores/session";
import { useSidebar } from "@/components/ui/sidebar";
import { ModelPill } from "./model-pill";
import { PersonaPill } from "./persona-pill";
import type { Attachment } from "@mew/web-client";

interface InputAreaProps {
  onSend: (text: string, attachments?: Attachment[]) => void;
  onSlash?: (command: string) => void;
  onCancel: () => void;
  connected: boolean;
}

interface SlashCommand {
  command: string;
  description: string;
}

const SLASH_COMMANDS: SlashCommand[] = [
  { command: "/clear", description: "Clear the current session" },
  { command: "/compact", description: "Compact message history" },
  { command: "/help", description: "Show available commands" },
];

/** Max file size for data-URL attachments (10 MB). Larger files would
 *  produce very large WebSocket frames that may exceed daemon limits. */
const MAX_FILE_SIZE = 10 * 1024 * 1024;

/** Minimal Claude Code-style composer: a single rounded bar with attach,
 *  textarea, and send/stop. Slash commands and @ personas still surface as
 *  small palettes when the user types `/` or `@`. */
export const InputArea = forwardRef<HTMLTextAreaElement, InputAreaProps>(
  function InputArea({ onSend, onSlash, onCancel, connected }, ref) {
    const { isMobile } = useSidebar();
    const [text, setText] = useState("");
    const [files, setFiles] = useState<File[]>([]);
    const [menuOpen, setMenuOpen] = useState<"slash" | "persona" | null>(null);
    const [menuIndex, setMenuIndex] = useState(0);
    const [focused, setFocused] = useState(false);
    const [historyIndex, setHistoryIndex] = useState<number | null>(null);
    const textareaRef = useRef<HTMLTextAreaElement>(null);
    const fileInputRef = useRef<HTMLInputElement>(null);

    useImperativeHandle(ref, () => textareaRef.current!, []);

    const hasStreaming = useSessionStore((s) => s.streamingPartId !== null);
    const selectPersona = useSessionStore((s) => s.selectPersona);
    const availablePersonas = useSessionStore((s) => s.availablePersonas);
    const promptHistory = useSessionStore((s) => s.promptHistory);
    const pushPromptHistory = useSessionStore((s) => s.pushPromptHistory);
    const [isSending, setIsSending] = useState(false);
    const [isDragging, setIsDragging] = useState(false);
    const [attachmentError, setAttachmentError] = useState<string | null>(null);

    const filteredSlash = SLASH_COMMANDS.filter(
      (c) =>
        text === "/" || c.command.toLowerCase().startsWith(text.toLowerCase()),
    );

    const personaNames = ["default", ...availablePersonas.map((p) => p.name)];
    const filteredPersonas = personaNames.filter(
      (p) =>
        text === "@" || p.toLowerCase().startsWith(text.slice(1).toLowerCase()),
    );

    const activeMenu = menuOpen === "slash" ? filteredSlash : filteredPersonas;

    const handleKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
      if (menuOpen && activeMenu.length > 0) {
        if (e.key === "ArrowDown") {
          e.preventDefault();
          setMenuIndex((i) => (i + 1) % activeMenu.length);
          return;
        }
        if (e.key === "ArrowUp") {
          e.preventDefault();
          setMenuIndex((i) => (i - 1 + activeMenu.length) % activeMenu.length);
          return;
        }
        if (e.key === "Enter") {
          e.preventDefault();
          if (menuOpen === "slash") selectSlash(filteredSlash[menuIndex]!);
          else selectPersonaAction(filteredPersonas[menuIndex] ?? "default");
          return;
        }
        if (e.key === "Escape") {
          e.preventDefault();
          closeMenu();
          return;
        }
      }

      // Prompt history recall (only when the slash/persona menu is closed).
      if (!menuOpen && promptHistory.length > 0) {
        if (e.key === "ArrowUp") {
          const el = textareaRef.current;
          // Only recall when the caret is on the first line.
          const onFirstLine = el ? el.value.lastIndexOf("\n", el.selectionStart - 1) === -1 : true;
          if (onFirstLine) {
            e.preventDefault();
            setHistoryIndex((prev) => {
              const next = prev === null ? promptHistory.length - 1 : Math.max(0, prev - 1);
              setText(promptHistory[next] ?? "");
              return next;
            });
            return;
          }
        }
        if (e.key === "ArrowDown") {
          if (historyIndex !== null) {
            const el = textareaRef.current;
            const onLastLine = el
              ? el.value.indexOf("\n", el.selectionStart) === -1
              : true;
            if (onLastLine) {
              e.preventDefault();
              setHistoryIndex((prev) => {
                if (prev === null) return null;
                const next = prev + 1;
                if (next >= promptHistory.length) {
                  setText("");
                  return null;
                }
                setText(promptHistory[next] ?? "");
                return next;
              });
              return;
            }
          }
        }
      }

      if (isMobile) {
        if (e.key === "Enter" && !e.shiftKey) {
          e.preventDefault();
          handleSubmit();
        }
      } else {
        if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
          e.preventDefault();
          handleSubmit();
        }
      }

      if (e.key === "Escape" && hasStreaming) {
        e.preventDefault();
        onCancel();
      }
    };

    const handleChange = (value: string) => {
      setText(value);
      setHistoryIndex(null);
      if (value.startsWith("/")) {
        setMenuOpen("slash");
        setMenuIndex(0);
      } else if (value.startsWith("@")) {
        setMenuOpen("persona");
        setMenuIndex(0);
      } else {
        closeMenu();
      }
      autoResize();
    };

    const closeMenu = () => {
      setMenuOpen(null);
      setMenuIndex(0);
    };

    const handleSubmit = async () => {
      // Guard against double-submit while async file conversion is in flight.
      if (isSending) return;
      const trimmed = text.trim();
      if (!trimmed || !connected) return;
      if (menuOpen === "slash" && filteredSlash.length > 0) {
        selectSlash(filteredSlash[0]!);
        return;
      }
      if (menuOpen === "persona" && filteredPersonas.length > 0) {
        selectPersonaAction(filteredPersonas[0]!);
        return;
      }

      setIsSending(true);
      try {
        // Convert File objects to Attachment[] (data URLs for wire protocol).
        let attachments: Attachment[] | undefined;
        if (files.length > 0) {
          attachments = await Promise.all(
            files.map((f) => fileToAttachment(f)),
          );
        }

        onSend(trimmed, attachments);
        pushPromptHistory(trimmed);
        setText("");
        setHistoryIndex(null);
        setFiles([]);
        closeMenu();
        if (textareaRef.current) textareaRef.current.style.height = "auto";
      } catch (e) {
        // FileReader error — log and reset, don't crash.
        console.error("[mew] attachment conversion failed:", e);
      } finally {
        setIsSending(false);
      }
    };

    const selectSlash = (cmd: SlashCommand) => {
      if (onSlash) {
        onSlash(cmd.command);
      }
      setText("");
      closeMenu();
      if (textareaRef.current) textareaRef.current.style.height = "auto";
    };

    const selectPersonaAction = (name: string) => {
      selectPersona(name);
      setText("");
      closeMenu();
      if (textareaRef.current) textareaRef.current.style.height = "auto";
    };

    const autoResize = () => {
      const el = textareaRef.current;
      if (el) {
        el.style.height = "auto";
        el.style.height =
          Math.min(el.scrollHeight, isMobile ? 120 : 160) + "px";
      }
    };

    const addFiles = (incoming: File[]) => {
      const oversized = incoming.filter((f) => f.size > MAX_FILE_SIZE);
      const valid = incoming.filter((f) => f.size <= MAX_FILE_SIZE);
      if (oversized.length > 0) {
        const names = oversized.map((f) => f.name).join(", ");
        setAttachmentError(`File too large (max 10 MB): ${names}`);
        // Auto-clear error after 4 seconds
        setTimeout(() => setAttachmentError(null), 4000);
      }
      if (valid.length > 0) {
        setFiles((prev) => [...prev, ...valid]);
      }
    };

    const handleAttach = (next: FileList | null) => {
      if (!next) return;
      addFiles(Array.from(next));
    };

    const removeFile = (file: File) => {
      setFiles((prev) => prev.filter((f) => f !== file));
    };

    const handlePaste = (e: ClipboardEvent<HTMLTextAreaElement>) => {
      const items = e.clipboardData?.items;
      if (!items) return;
      const pastedFiles: File[] = [];
      let hasText = false;
      for (const item of items) {
        if (item.kind === "file") {
          const file = item.getAsFile();
          if (file) pastedFiles.push(file);
        } else if (item.kind === "string") {
          hasText = true;
        }
      }
      if (pastedFiles.length > 0 && !hasText) {
        // Only intercept paste when there are files and no text —
        // if both are present, let the browser handle text normally
        // and just add the files.
        e.preventDefault();
      }
      if (pastedFiles.length > 0) {
        addFiles(pastedFiles);
      }
    };

    const handleDrop = (e: DragEvent<HTMLDivElement>) => {
      e.preventDefault();
      setIsDragging(false);
      const dropped = e.dataTransfer?.files;
      if (dropped && dropped.length > 0) {
        addFiles(Array.from(dropped));
      }
    };

    const handleDragOver = (e: DragEvent<HTMLDivElement>) => {
      e.preventDefault();
      setIsDragging(true);
    };

    const handleDragLeave = (e: DragEvent<HTMLDivElement>) => {
      e.preventDefault();
      // Only clear drag state when leaving the container entirely,
      // not when moving between child elements.
      const related = e.relatedTarget as Node | null;
      if (related && e.currentTarget.contains(related)) return;
      setIsDragging(false);
    };

    return (
      <div
        className="shrink-0 border-t border-border/60 bg-background/95 px-3 pb-2 pt-2 sm:px-4 sm:pb-3"
      >
        <div className="mx-auto max-w-3xl space-y-2">
          <div className="relative">
            <div
              onDrop={handleDrop}
              onDragOver={handleDragOver}
              onDragLeave={handleDragLeave}
              className={cn(
                "flex items-end gap-2 rounded-xl border bg-muted/40 px-3 py-2 transition-[background-color,border-color] duration-150 ease-out",
                focused ? "border-ring bg-muted" : "border-border",
                isDragging && "border-primary border-2",
              )}
            >
              <button
                onClick={() => fileInputRef.current?.click()}
                disabled={!connected}
                className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg text-muted-foreground transition-colors hover:bg-accent hover:text-accent-foreground disabled:opacity-50"
                title="Attach file"
                aria-label="Attach file"
              >
                <Paperclip className="h-4 w-4" />
              </button>

              <textarea
                ref={textareaRef}
                value={text}
                onChange={(e) => handleChange(e.target.value)}
                onKeyDown={handleKeyDown}
                onPaste={handlePaste}
                onFocus={() => setFocused(true)}
                onBlur={() => setFocused(false)}
                placeholder={connected ? "Ask mew anything…" : "Connecting…"}
                aria-label="Message prompt"
                disabled={!connected}
                rows={1}
                className="flex-1 resize-none bg-transparent py-1.5 text-sm leading-5 text-foreground placeholder:text-muted-foreground focus:outline-hidden"
              />

              <input
                ref={fileInputRef}
                type="file"
                multiple
                className="hidden"
                onChange={(e) => handleAttach(e.target.files)}
              />

              {hasStreaming ? (
                <button
                  onClick={onCancel}
                  className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg border border-destructive/50 text-destructive transition-colors hover:bg-destructive/10"
                  title="Cancel"
                  aria-label="Cancel response"
                >
                  <Square className="h-4 w-4" />
                </button>
              ) : (
                <button
                  onClick={handleSubmit}
                  disabled={!text.trim() || !connected || isSending}
                  className={cn(
                    "flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-primary text-primary-foreground transition-colors hover:bg-primary/90 disabled:opacity-50",
                  )}
                  title="Send"
                  aria-label="Send prompt"
                >
                  <CornerDownLeft className="h-4 w-4" />
                </button>
              )}
            </div>

            {menuOpen === "slash" && filteredSlash.length > 0 && (
              <MenuPanel title="Commands">
                {filteredSlash.map((cmd, i) => (
                  <MenuRow
                    key={cmd.command}
                    active={i === menuIndex}
                    onClick={() => selectSlash(cmd)}
                    primary={cmd.command}
                    secondary={cmd.description}
                  />
                ))}
              </MenuPanel>
            )}

            {menuOpen === "persona" && filteredPersonas.length > 0 && (
              <MenuPanel title="Personas">
                {filteredPersonas.map((p, i) => (
                  <MenuRow
                    key={p}
                    active={i === menuIndex}
                    onClick={() => selectPersonaAction(p)}
                    primary={p}
                  />
                ))}
              </MenuPanel>
            )}
          </div>

          <div className="flex items-start justify-between gap-3">
            <div className="flex flex-col gap-1">
              {attachmentError && (
                <span className="text-[10px] text-destructive">
                  {attachmentError}
                </span>
              )}
              {files.length > 0 ? (
                <div className="flex flex-wrap gap-1.5">
                  {files.map((f, i) => (
                    <span
                      key={`${f.name}-${i}`}
                      className="flex items-center gap-1 rounded-md bg-muted px-1.5 py-0.5 text-[10px] text-muted-foreground"
                    >
                      {f.name}
                      <button
                        onClick={() => removeFile(f)}
                        className="rounded-full hover:text-foreground"
                        title="Remove"
                        aria-label={"Remove " + f.name}
                      >
                        <X className="h-3 w-3" />
                      </button>
                    </span>
                  ))}
                </div>
              ) : null}
            </div>
          </div>
          <div className="flex gap-2">
            <PersonaPill />
            <ModelPill />
          </div>
        </div>
      </div>
    );
  },
);

function MenuPanel({
  title,
  children,
}: {
  title: string;
  children: React.ReactNode;
}) {
  return (
    <div className="absolute bottom-full left-0 z-30 mb-1 w-56 rounded-lg border border-border bg-popover shadow-lg">
      <div className="px-2 py-1 text-[10px] font-semibold uppercase tracking-wide text-muted-foreground">
        {title}
      </div>
      {children}
    </div>
  );
}

function MenuRow({
  active,
  onClick,
  primary,
  secondary,
}: {
  active: boolean;
  onClick: () => void;
  primary: string;
  secondary?: string;
}) {
  return (
    <button
      onMouseDown={(e) => {
        e.preventDefault();
        onClick();
      }}
      className={cn(
        "flex w-full flex-col gap-0.5 px-2 py-1.5 text-left transition-colors",
        active ? "bg-accent" : "hover:bg-accent",
      )}
    >
      <span className="text-xs font-medium text-foreground">{primary}</span>
      {secondary && (
        <span className="text-[10px] text-muted-foreground">{secondary}</span>
      )}
    </button>
  );
}

/** Convert a browser File to a wire Attachment using a data URL. */
function fileToAttachment(file: File): Promise<Attachment> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => {
      const dataUrl = reader.result as string;
      resolve({
        path: dataUrl,
        mime: file.type || undefined,
      });
    };
    reader.onerror = reject;
    reader.readAsDataURL(file);
  });
}
