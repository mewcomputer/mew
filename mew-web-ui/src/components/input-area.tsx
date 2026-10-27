import {
  forwardRef,
  useRef,
  useState,
  useEffect,
  useImperativeHandle,
  type ClipboardEvent,
  type DragEvent,
  type KeyboardEvent,
} from "react";
import { Square, Plus, X, ArrowUp, Shield } from "lucide-react";
import { cn } from "../lib/utils";
import { useSessionStore } from "../stores/session";
import { useSidebar } from "@/components/ui/sidebar";
import { ModelPill } from "./model-pill";
import { PersonaPill } from "./persona-pill";
import type { Attachment } from "@mew/web-client";

interface InputAreaProps {
  onSend: (text: string, attachments?: Attachment[]) => void | Promise<void>;
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

interface ComposerDraft { text: string; files: File[] }
const composerDrafts = new Map<string, ComposerDraft>();
const EMPTY_DRAFT: ComposerDraft = { text: "", files: [] };

/** Chat composer with attachments, policy, persona and model controls. */
export const InputArea = forwardRef<HTMLTextAreaElement, InputAreaProps>(
  function InputArea({ onSend, onSlash, onCancel, connected }, ref) {
    const { isMobile } = useSidebar();
    const sessionId = useSessionStore((state) => state.sessionId) ?? "new";
    const [, refreshDraft] = useState(0);
    const { text, files } = composerDrafts.get(sessionId) ?? EMPTY_DRAFT;
    const setText = (value: string | ((previous: string) => string)) => {
      const current = composerDrafts.get(sessionId) ?? EMPTY_DRAFT;
      composerDrafts.set(sessionId, { ...current, text: typeof value === "function" ? value(current.text) : value });
      refreshDraft((revision) => revision + 1);
    };
    const setFiles = (value: File[] | ((previous: File[]) => File[])) => {
      const current = composerDrafts.get(sessionId) ?? EMPTY_DRAFT;
      composerDrafts.set(sessionId, { ...current, files: typeof value === "function" ? value(current.files) : value });
      refreshDraft((revision) => revision + 1);
    };
    const [menuOpen, setMenuOpen] = useState<"slash" | "persona" | null>(null);
    const [menuIndex, setMenuIndex] = useState(0);
    const [focused, setFocused] = useState(false);
    const [sendError, setSendError] = useState<string | null>(null);
    const permissionMode = useSessionStore((state) => state.permissionMode);
    const [historyIndex, setHistoryIndex] = useState<number | null>(null);
    const textareaRef = useRef<HTMLTextAreaElement>(null);
    const fileInputRef = useRef<HTMLInputElement>(null);

    useImperativeHandle(ref, () => textareaRef.current!, []);

    useEffect(() => {
      if (connected) textareaRef.current?.focus();
    }, [connected, sessionId]);

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
      if (e.nativeEvent.isComposing || e.nativeEvent.keyCode === 229) return;
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

      if (e.key === "Enter" && !e.shiftKey) {
        e.preventDefault();
        void handleSubmit();
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
      setSendError(null);
      try {
        // Convert File objects to Attachment[] (data URLs for wire protocol).
        let attachments: Attachment[] | undefined;
        if (files.length > 0) {
          attachments = await Promise.all(
            files.map((f) => fileToAttachment(f)),
          );
        }

        if ((useSessionStore.getState().sessionId ?? "new") !== sessionId) return;
        const pending = onSend(trimmed, attachments);
        if (pending !== undefined) await pending;
        pushPromptHistory(trimmed);
        setText("");
        setHistoryIndex(null);
        setFiles([]);
        closeMenu();
        if (textareaRef.current) textareaRef.current.style.height = "auto";
      } catch (e) {
        setSendError(e instanceof Error ? e.message : "Could not send the message.");
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
        className="shrink-0 px-4 pb-3 pt-2 sm:px-6"
      >
        <div className="mew-conversation-width mx-auto">
          <div className="relative">
            <div
              data-testid="composer-surface"
              onDrop={handleDrop}
              onDragOver={handleDragOver}
              onDragLeave={handleDragLeave}
              className={cn(
                "pointer-events-auto rounded-2xl border bg-card px-3 pb-2 pt-2 transition-colors duration-150",
                focused ? "border-foreground/20" : "border-border/50",
                isDragging && "border-primary border-2",
              )}
            >
              <div className="flex items-end gap-2">

                <textarea
                  ref={textareaRef}
                  value={text}
                  onChange={(e) => handleChange(e.target.value)}
                  onKeyDown={handleKeyDown}
                  onPaste={handlePaste}
                  onFocus={() => setFocused(true)}
                  onBlur={() => setFocused(false)}
                  placeholder={connected ? "Ask anything, or describe a task" : "Connecting…"}
                  aria-label="Message prompt"
                  disabled={!connected}
                  rows={1}
                  className="min-h-9 min-w-0 flex-1 resize-none bg-transparent px-0.5 pb-2 pt-1 text-[13px] leading-5 text-foreground placeholder:text-muted-foreground focus:outline-hidden"
                />

                <input
                  ref={fileInputRef}
                  type="file"
                  multiple
                  className="hidden"
                  onChange={(e) => handleAttach(e.target.files)}
                />
              </div>

              {(attachmentError || files.length > 0) && (
                <div className="mt-1.5 flex min-w-0 flex-wrap items-center gap-1.5 border-t border-border/50 pt-1.5">
                  {attachmentError && (
                    <span className="text-[10px] text-destructive">
                      {attachmentError}
                    </span>
                  )}
                  {files.map((f, i) => (
                    <span
                      key={`${f.name}-${i}`}
                      className="flex max-w-full items-center gap-1 rounded-md bg-muted px-1.5 py-0.5 text-[10px] text-muted-foreground"
                    >
                      <span className="truncate">{f.name}</span>
                      <button
                        onClick={() => removeFile(f)}
                        className="shrink-0 rounded-full hover:text-foreground"
                        title="Remove"
                        aria-label={"Remove " + f.name}
                      >
                        <X className="h-3 w-3" />
                      </button>
                    </span>
                  ))}
                </div>
              )}

              {sendError && <p role="alert" className="pb-2 text-xs text-destructive">{sendError}</p>}
              <div className="flex items-center justify-between gap-1">
                <div className="flex min-w-0 items-center gap-1">
                <button
                  onClick={() => fileInputRef.current?.click()}
                  disabled={!connected}
                  className="motion-pressable flex h-6 w-6 shrink-0 items-center justify-center rounded-md text-muted-foreground hover:bg-accent hover:text-accent-foreground disabled:opacity-50"
                  title="Attach file"
                  aria-label="Attach file"
                >
                  <Plus className="h-4 w-4" />
                </button>
                  <span className="flex items-center gap-1 whitespace-nowrap px-1 text-[10px] text-muted-foreground" title={`Permission mode: ${permissionMode}`}>
                    <Shield className="h-3 w-3" />
                    {permissionMode === "dangerous" ? "Full access" : permissionMode === "standard" ? "Default permissions" : permissionMode.replace("_", " ")}
                  </span>
                  <PersonaPill />
                </div>
                <div className="flex min-w-0 items-center gap-1">
                <ModelPill />
                {hasStreaming ? (
                  <button
                    onClick={onCancel}
                    className="motion-pressable flex h-7 w-7 shrink-0 items-center justify-center rounded-full bg-foreground text-background hover:opacity-90"
                    title="Cancel"
                    aria-label="Cancel response"
                  >
                    <Square className="h-3 w-3 fill-current" />
                  </button>
                ) : (
                  <button
                    onClick={handleSubmit}
                    disabled={!text.trim() || !connected || isSending}
                    className={cn(
                      "motion-pressable flex h-7 w-7 shrink-0 items-center justify-center rounded-full bg-foreground text-background hover:opacity-90 disabled:opacity-40",
                    )}
                    title="Send"
                    aria-label="Send prompt"
                  >
                    <ArrowUp className="h-4 w-4" />
                  </button>
                  )}
                  </div>
              </div>
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
    <div data-mew-overlay="" className="motion-enter absolute bottom-full left-0 z-30 mb-1 w-56 rounded-lg border border-border bg-popover shadow-lg">
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
        "motion-pressable flex w-full flex-col gap-0.5 px-2 py-1.5 text-left",
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
