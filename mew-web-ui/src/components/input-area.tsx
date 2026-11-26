import {
  forwardRef,
  useRef,
  useId,
  useState,
  useEffect,
  useLayoutEffect,
  useImperativeHandle,
  type ClipboardEvent,
  type CSSProperties,
  type DragEvent,
  type KeyboardEvent,
  type ReactNode,
} from "react";
import { Square, Plus, X, ArrowUp, Shield, Loader2, File, CornerDownRight, Paperclip, Sparkles, Cpu, Bot, UserRound } from "lucide-react";
import { cn } from "../lib/utils";
import { selectSessionRunning, useSessionStore } from "../stores/session";
import { useSidebar } from "@/components/ui/sidebar";
import { ModelPill } from "./model-pill";
import { ContextIndicator } from "./session-status";
import { PersonaPill } from "./persona-pill";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from "./ui/dropdown-menu";
import { getClient } from "../lib/client-ref";
import type { Attachment } from "@mew/web-client";

interface InputAreaProps {
  onSend: (text: string, attachments?: Attachment[]) => void | Promise<void>;
  onEnqueue?: (text: string, attachments?: Attachment[]) => void | Promise<void>;
  onSteer?: (text: string, attachments?: Attachment[]) => void | Promise<void>;
  onSlash?: (command: string) => void;
  onCancel: () => void;
  connected: boolean;
  draftId?: string;
}

interface SlashCommand {
  command: string;
  description: string;
}

interface MentionItem {
  id: string;
  group: string;
  primary: string;
  secondary?: string;
  insert?: string;
  persona?: string;
  action?: "attach";
}

/** The active `@token` before the caret, with the index of its `@`. */
function activeAtToken(value: string, caret: number): { start: number; query: string } | null {
  const match = /(?:^|\s)@([^\s@]*)$/.exec(value.slice(0, caret));
  if (!match) return null;
  const query = match[1] ?? "";
  return { start: caret - query.length - 1, query };
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
  function InputArea({ onSend, onEnqueue, onSteer, onSlash, onCancel, connected, draftId }, ref) {
    const { isMobile } = useSidebar();
    const sessionId = useSessionStore((state) => state.sessionId) ?? "new";
    const draftOwner = draftId ?? sessionId;
    const activeDraft = useRef<{ owner: string } | null>(null);
    const [, refreshDraft] = useState(0);
    const { text, files } = composerDrafts.get(draftOwner) ?? EMPTY_DRAFT;
    const setText = (value: string | ((previous: string) => string)) => {
      const current = composerDrafts.get(draftOwner) ?? EMPTY_DRAFT;
      composerDrafts.set(draftOwner, { ...current, text: typeof value === "function" ? value(current.text) : value });
      refreshDraft((revision) => revision + 1);
    };
    const setFiles = (value: File[] | ((previous: File[]) => File[])) => {
      const current = composerDrafts.get(draftOwner) ?? EMPTY_DRAFT;
      composerDrafts.set(draftOwner, { ...current, files: typeof value === "function" ? value(current.files) : value });
      refreshDraft((revision) => revision + 1);
    };
    const [menuOpen, setMenuOpen] = useState<"slash" | "persona" | null>(null);
    const [mention, setMention] = useState<{ start: number; query: string } | null>(null);
    const [menuIndex, setMenuIndex] = useState(0);
    const [focused, setFocused] = useState(false);
    const [sendError, setSendError] = useState<string | null>(null);
    const permissionMode = useSessionStore((state) => state.permissionMode);
    const [historyIndex, setHistoryIndex] = useState<number | null>(null);
    const textareaRef = useRef<HTMLTextAreaElement>(null);
    const fileInputRef = useRef<HTMLInputElement>(null);
    const mentionListRef = useRef<HTMLDivElement>(null);
    const mentionListId = useId();

    useEffect(() => {
      if (menuOpen === "persona") {
        mentionListRef.current?.querySelector<HTMLElement>('[aria-selected="true"]')?.scrollIntoView?.({ block: "nearest" });
      }
    }, [menuOpen, menuIndex, mention]);

    useLayoutEffect(() => {
      activeDraft.current = { owner: draftOwner };
      return () => { activeDraft.current = null; };
    }, [draftOwner, connected]);

    useLayoutEffect(() => {
      const input = textareaRef.current;
      if (!input) return;
      input.style.height = "auto";
      input.style.height = `${Math.min(input.scrollHeight, isMobile ? 120 : 160)}px`;
    }, [text, draftOwner, isMobile]);

    useEffect(() => {
      setSendError(null);
      setHistoryIndex(null);
      setMenuOpen(null);
    }, [draftOwner]);

    useImperativeHandle(ref, () => textareaRef.current!, []);

    useEffect(() => {
      if (connected) textareaRef.current?.focus();
    }, [connected, draftOwner]);

    const hasStreaming = useSessionStore((s) => s.streamingPartId !== null);
    const sessionRunning = useSessionStore(selectSessionRunning);
    const sessionBusy = hasStreaming || sessionRunning;
    const queuedMessages = useSessionStore((s) => s.queuedMessages);
    const selectPersona = useSessionStore((s) => s.selectPersona);
    const availablePersonas = useSessionStore((s) => s.availablePersonas);
    const availableModels = useSessionStore((s) => s.availableModels);
    const mentionSkills = useSessionStore((s) => s.mentionSkills);
    const mentionSubagents = useSessionStore((s) => s.mentionSubagents);
    const dirResults = useSessionStore((s) => s.dirResults);
    const gitStatus = useSessionStore((s) => s.gitStatus);
    const flaggedFiles = useSessionStore((s) => s.flaggedFiles);
    const promptHistory = useSessionStore((s) => s.promptHistory);
    const pushPromptHistory = useSessionStore((s) => s.pushPromptHistory);
    const [isSending, setIsSending] = useState(false);
    const [isDragging, setIsDragging] = useState(false);
    const [attachmentError, setAttachmentError] = useState<string | null>(null);

    const filteredSlash = SLASH_COMMANDS.filter(
      (c) =>
        text === "/" || c.command.toLowerCase().startsWith(text.toLowerCase()),
    );

    const mentionItems = (() => {
      if (!mention) return [] as MentionItem[];
      const query = mention.query;
      const separator = query.indexOf(":");
      const namespace = separator >= 0 ? query.slice(0, separator) : null;
      const term = (namespace ? query.slice(separator + 1) : query).toLowerCase();
      const matches = (value: string) => value.toLowerCase().includes(term);
      const items: MentionItem[] = [];
      if (!namespace && (matches("attach files") || matches("upload"))) {
        items.push({ id: "attach", group: "Actions", primary: "Attach files", secondary: "Choose files from your device", action: "attach" });
      }

      if (!namespace || namespace === "skill") {
        for (const skill of mentionSkills) {
          if (matches(skill.name) || matches(skill.description)) {
            items.push({ id: `skill:${skill.name}`, group: "Skills", primary: skill.name, secondary: skill.description, insert: `@skill:${skill.name} ` });
          }
        }
      }
      if (!namespace || namespace === "model") {
        for (const model of availableModels) {
          if (matches(model.id) || matches(model.display_name ?? "")) {
            items.push({ id: `model:${model.id}`, group: "Models", primary: model.display_name ?? model.id, secondary: model.id, insert: `@model:${model.id} ` });
          }
        }
      }
      if (!namespace || namespace === "subagent") {
        for (const subagent of mentionSubagents) {
          if (matches(subagent.name) || matches(subagent.description)) {
            items.push({ id: `subagent:${subagent.name}`, group: "Subagents", primary: subagent.name, secondary: subagent.description, insert: `@subagent:${subagent.name} ` });
          }
        }
      }
      if (!namespace) {
        const files = new Set<string>();
        for (const result of dirResults.values()) {
          for (const entry of result.entries ?? []) if (!entry.is_dir) files.add(entry.name);
        }
        for (const entry of gitStatus) files.add(entry.path);
        for (const file of flaggedFiles) files.add(file.path);
        for (const file of files) {
          if (matches(file)) items.push({ id: `file:${file}`, group: "Files", primary: file, insert: `@${file} ` });
        }
        for (const persona of ["default", ...availablePersonas.map((p) => p.name)]) {
          if (matches(persona)) items.push({ id: `persona:${persona}`, group: "Personas", primary: persona, persona });
        }
      }
      const order = ["Actions", "Files", "Skills", "Models", "Subagents", "Personas"];
      return items.sort((a, b) => order.indexOf(a.group) - order.indexOf(b.group));
    })();
    const menuLength = menuOpen === "slash" ? filteredSlash.length : mentionItems.length;

    const handleKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
      if (e.nativeEvent.isComposing || e.nativeEvent.keyCode === 229) return;
      if (menuOpen && menuLength > 0) {
        if (e.key === "ArrowDown") {
          e.preventDefault();
          setMenuIndex((i) => (i + 1) % menuLength);
          return;
        }
        if (e.key === "ArrowUp") {
          e.preventDefault();
          setMenuIndex((i) => (i - 1 + menuLength) % menuLength);
          return;
        }
        if (e.key === "Enter") {
          e.preventDefault();
          if (menuOpen === "slash") selectSlash(filteredSlash[menuIndex]!);
          else selectMention(mentionItems[menuIndex]);
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

    const handleChange = (value: string, caret = value.length) => {
      setText(value);
      setHistoryIndex(null);
      if (value.startsWith("/")) {
        setMenuOpen("slash");
        setMention(null);
        setMenuIndex(0);
      } else {
        const token = activeAtToken(value, Math.min(caret, value.length));
        setMention(token);
        if (token) {
          setMenuOpen("persona");
          setMenuIndex(0);
        } else closeMenu();
      }
    };

    const closeMenu = () => {
      setMenuOpen(null);
      setMention(null);
      setMenuIndex(0);
    };

    const handleSubmit = async (mode: "default" | "steer" = "default") => {
      // Guard against double-submit while async file conversion is in flight.
      if (isSending) return;
      const trimmed = text.trim();
      if (!trimmed || !connected) return;
      if (menuOpen === "slash" && filteredSlash.length > 0) {
        selectSlash(filteredSlash[0]!);
        return;
      }
      if (menuOpen === "persona" && mentionItems.length > 0) {
        selectMention(mentionItems[0]);
        return;
      }

      setIsSending(true);
      setSendError(null);
      const source = activeDraft.current;
      try {
        // Convert File objects to Attachment[] (data URLs for wire protocol).
        let attachments: Attachment[] | undefined;
        if (files.length > 0) {
          attachments = await Promise.all(
            files.map((f) => fileToAttachment(f)),
          );
        }

        if (activeDraft.current !== source) return;
        const dispatch =
          mode === "steer" && onSteer
            ? onSteer
            : sessionBusy && onEnqueue
              ? onEnqueue
              : onSend;
        const pending = dispatch(trimmed, attachments);
        if (pending !== undefined) await pending;
        pushPromptHistory(trimmed);
        setText((current) => current === text ? "" : current);
        if (files.length > 0) setFiles((current) => current.filter((file) => !files.includes(file)));
        if (activeDraft.current?.owner === draftOwner) {
          setHistoryIndex(null);
          closeMenu();
        }
      } catch (e) {
        if (activeDraft.current?.owner === draftOwner) {
          setSendError(e instanceof Error ? e.message : "Could not send the message.");
        }
      } finally {
        setIsSending(false);
      }
    };

    const selectSlash = (cmd: SlashCommand) => {
      if (!connected) return;
      if (onSlash) {
        onSlash(cmd.command);
      }
      setText("");
      closeMenu();
      if (textareaRef.current) textareaRef.current.style.height = "auto";
    };

    const selectMention = (item?: MentionItem) => {
      if (!item) return;
      if (item.action === "attach") {
        if (!mention) return;
        const caret = textareaRef.current?.selectionStart ?? text.length;
        setText(text.slice(0, mention.start) + text.slice(caret));
        closeMenu();
        fileInputRef.current?.click();
        return;
      }
      if (item.persona) { selectPersonaAction(item.persona); return; }
      if (!item.insert || !mention) return;
      const caret = textareaRef.current?.selectionStart ?? text.length;
      const next = text.slice(0, mention.start) + item.insert + text.slice(caret);
      setText(next);
      closeMenu();
      requestAnimationFrame(() => {
        const el = textareaRef.current;
        if (!el) return;
        const position = mention.start + item.insert!.length;
        el.focus();
        el.setSelectionRange(position, position);
      });
    };

    const selectPersonaAction = (name: string) => {
      if (!connected) return;
      selectPersona(name);
      if (mention) {
        const caret = textareaRef.current?.selectionStart ?? text.length;
        setText(text.slice(0, mention.start) + text.slice(caret));
      }
      closeMenu();
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
        className="mew-conversation-gutters shrink-0 pb-3 pt-3"
      >
        <div className="mew-conversation-width mx-auto">
          {sessionId !== "new" && queuedMessages.length > 0 && (
            <div className="mb-2 space-y-1" aria-label="Queued messages">
              {queuedMessages.map((item) => (
                <div
                  key={item.id}
                  className="flex min-w-0 items-center gap-2 rounded-xl border border-border/60 bg-muted/40 px-2.5 py-1.5 text-xs"
                >
                  <CornerDownRight className="h-3.5 w-3.5 shrink-0 text-muted-foreground" aria-hidden="true" />
                  <span className="min-w-0 flex-1 truncate text-foreground">{item.text}</span>
                  <button
                    type="button"
                    onClick={() => getClient()?.updateQueuedMessage(sessionId, item.id, { mode: "steer" })}
                    className="shrink-0 rounded px-1.5 py-0.5 text-[11px] font-medium text-muted-foreground hover:bg-accent hover:text-foreground"
                  >
                    Steer
                  </button>
                  <button
                    type="button"
                    onClick={() => getClient()?.removeQueuedMessage(sessionId, item.id)}
                    aria-label={`Remove queued message ${item.text}`}
                    className="flex h-5 w-5 shrink-0 items-center justify-center rounded text-muted-foreground hover:bg-accent hover:text-foreground"
                  >
                    <X className="h-3 w-3" />
                  </button>
                </div>
              ))}
            </div>
          )}
          <div className="relative">
            <div
              data-testid="composer-surface"
              onDrop={handleDrop}
              onDragOver={handleDragOver}
              onDragLeave={handleDragLeave}
              className={cn(
                "mew-composer pointer-events-auto rounded-2xl border bg-card px-3 pb-2 pt-2 shadow-xs transition-colors duration-150",
                focused ? "border-foreground/25" : "border-border/60 hover:border-foreground/15",
                isDragging && "border-primary ring-2 ring-primary/20",
              )}
            >
              <div className="flex items-end gap-2">

                <textarea
                  ref={textareaRef}
                  value={text}
                  onChange={(e) => handleChange(e.target.value, e.target.selectionStart ?? e.target.value.length)}
                  onKeyDown={handleKeyDown}
                  onPaste={handlePaste}
                  onFocus={() => setFocused(true)}
                  onBlur={() => setFocused(false)}
                  placeholder={connected ? "Ask anything, or describe a task" : "Connecting…"}
                  aria-label="Message prompt"
                  aria-expanded={menuOpen === "persona" && mentionItems.length > 0}
                  aria-controls={menuOpen === "persona" && mentionItems.length > 0 ? mentionListId : undefined}
                  aria-activedescendant={menuOpen === "persona" && mentionItems[menuIndex] ? `${mentionListId}-${menuIndex}` : undefined}
                  disabled={!connected}
                  rows={1}
                  className="min-h-10 min-w-0 flex-1 resize-none bg-transparent px-0.5 pb-2 pt-1 text-[13px] leading-[1.6] text-foreground placeholder:text-muted-foreground focus:outline-hidden disabled:cursor-not-allowed"
                />

                <input
                  ref={fileInputRef}
                  type="file"
                  multiple
                  className="hidden"
                  onChange={(e) => { handleAttach(e.target.files); e.target.value = ""; }}
                />
              </div>

              {(attachmentError || files.length > 0) && (
                <div className="mt-1.5 flex min-w-0 flex-wrap items-center gap-1.5 border-t border-border/50 pt-1.5">
                  {attachmentError && (
                    <span role="alert" className="text-xs text-text-error">
                      {attachmentError}
                    </span>
                  )}
                  {files.map((f, i) => (
                    <span
                      key={`${f.name}-${i}`}
                      className="flex max-w-full items-center gap-1.5 rounded-lg border border-border/70 bg-background/50 py-1 pl-2 pr-1 text-[11px] text-foreground"
                    >
                      <File className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
                      <span className="truncate">{f.name}</span>
                      <button
                        onClick={() => removeFile(f)}
                        className="flex h-5 w-5 shrink-0 items-center justify-center rounded text-muted-foreground hover:bg-accent hover:text-foreground"
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
              <div className="flex min-w-0 items-center justify-between gap-2">
                <div className="flex min-w-0 items-center gap-1">
                <button
                  onClick={() => fileInputRef.current?.click()}
                  disabled={!connected}
                  className="motion-pressable flex h-7 w-7 shrink-0 items-center justify-center rounded-md text-muted-foreground hover:bg-accent hover:text-accent-foreground disabled:opacity-50"
                  title="Attach file"
                  aria-label="Attach file"
                >
                  <Plus className="h-4 w-4" />
                </button>
                  <PermissionPill mode={permissionMode} />
                  <PersonaPill />
                </div>
                <div className="flex min-w-0 flex-1 items-center justify-end gap-1">
                <ContextIndicator />
                <ModelPill />
                <div className="flex shrink-0 items-center">
                  <ActionSlot open={Boolean(sessionBusy && text.trim() && onSteer)} delayMs={35}>
                    <button
                      type="button"
                      onClick={() => void handleSubmit("steer")}
                      disabled={!connected || isSending}
                      tabIndex={sessionBusy && text.trim() && onSteer ? undefined : -1}
                      className="motion-pressable whitespace-nowrap rounded-md border border-border/70 px-2 py-1 text-[11px] font-medium text-foreground hover:bg-accent disabled:opacity-40"
                      title="Interrupt and steer active turn"
                    >
                      Steer
                    </button>
                  </ActionSlot>
                  <ActionSlot open={!sessionBusy || Boolean(text.trim())}>
                    <button
                      onClick={() => void handleSubmit("default")}
                      disabled={!text.trim() || !connected || isSending}
                      aria-busy={isSending}
                      tabIndex={!sessionBusy || text.trim() ? undefined : -1}
                      className={cn(
                        "motion-pressable flex h-7 w-7 shrink-0 items-center justify-center rounded-full bg-foreground text-background hover:opacity-90 disabled:opacity-40",
                      )}
                      title={isSending ? "Sending…" : sessionBusy ? "Queue follow-up" : "Send"}
                      aria-label={sessionBusy ? "Queue follow-up" : "Send prompt"}
                    >
                      {isSending ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <ArrowUp className="h-4 w-4" />}
                    </button>
                  </ActionSlot>
                  <ActionSlot open={Boolean(sessionBusy)}>
                    <button
                      onClick={onCancel}
                      tabIndex={sessionBusy ? undefined : -1}
                      className="motion-pressable flex h-7 w-7 shrink-0 items-center justify-center rounded-full bg-foreground text-background hover:opacity-90"
                      title="Cancel"
                      aria-label="Cancel response"
                    >
                      <Square className="h-3 w-3 fill-current" />
                    </button>
                  </ActionSlot>
                </div>
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

            {menuOpen === "persona" && mentionItems.length > 0 && (
              <div ref={mentionListRef} id={mentionListId} role="listbox" aria-label="Mention suggestions" data-mew-overlay=""
                className="motion-enter absolute bottom-full left-0 z-30 mb-2 max-h-[min(50vh,24rem)] w-full overflow-y-auto rounded-xl border border-border bg-popover p-1.5 shadow-lg">
                {(["Actions", "Files", "Skills", "Models", "Subagents", "Personas"] as const).map((group) => {
                  const groupItems = mentionItems.filter((item) => item.group === group);
                  if (!groupItems.length) return null;
                  const Icon = { Actions: Paperclip, Files: File, Skills: Sparkles, Models: Cpu, Subagents: Bot, Personas: UserRound }[group];
                  return (
                    <div key={group} role="group" aria-label={group}>
                      <div className="px-2 py-1.5 text-[10px] font-semibold uppercase tracking-wide text-muted-foreground">{group}</div>
                      {groupItems.map((item) => {
                        const index = mentionItems.indexOf(item);
                        return <button key={item.id} id={`${mentionListId}-${index}`} type="button" role="option" aria-selected={index === menuIndex}
                          onMouseDown={(e) => e.preventDefault()} onClick={() => selectMention(item)}
                          className={cn("flex min-h-9 w-full items-center gap-2 rounded-lg px-2 py-2 text-left text-xs", index === menuIndex ? "bg-accent" : "hover:bg-accent")}>
                          <Icon aria-hidden="true" className="h-4 w-4 shrink-0 text-muted-foreground" />
                          <span className="min-w-0 truncate font-medium text-foreground">{item.primary}</span>
                          {item.secondary && <span className="min-w-0 truncate text-muted-foreground">{item.secondary}</span>}
                        </button>;
                      })}
                    </div>
                  );
                })}
              </div>
            )}
          </div>

        </div>
      </div>
    );
  },
);

const PERMISSION_OPTIONS = [
  { id: "standard", label: "Standard", description: "Prompt on edits & commands" },
  { id: "permissive", label: "Permissive", description: "Auto-allow edits, prompt on dangerous" },
  { id: "auto", label: "Auto", description: "Classifier reviews tool calls" },
  { id: "auto_plus", label: "Auto+", description: "Classifier reviews, no escalation" },
  { id: "dangerous", label: "Full access", description: "Bypass all permission prompts" },
] as const;

function PermissionPill({ mode }: { mode: string }) {
  const isCustom = mode !== "standard";
  const label = mode === "dangerous"
    ? "Full access"
    : mode === "standard"
      ? "Standard"
      : mode.replace("_", " ");
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <button
          type="button"
          title={`Permission mode: ${mode}`}
          aria-label={`Permission mode: ${mode}`}
          className={cn(
            "motion-pressable flex h-7 shrink-0 items-center gap-1.5 whitespace-nowrap rounded-md px-1.5 text-[11px] hover:bg-accent hover:text-accent-foreground data-[state=open]:bg-accent",
            mode === "dangerous"
              ? "text-destructive"
              : isCustom
                ? "text-foreground"
                : "text-muted-foreground",
          )}
        >
          <Shield className="h-3.5 w-3.5 shrink-0" />
          {isCustom && <span className="mew-composer-permission-label capitalize">{label}</span>}
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent side="top" align="start" sideOffset={10} aria-label="Permissions" className="w-56 max-w-[calc(100vw-2rem)] rounded-xl p-1.5">
        <DropdownMenuLabel className="text-[11px] font-normal text-muted-foreground">Permissions</DropdownMenuLabel>
        <DropdownMenuRadioGroup value={mode} onValueChange={(next) => getClient()?.setPermissionMode(next)}>
          {PERMISSION_OPTIONS.map((option) => (
            <DropdownMenuRadioItem key={option.id} value={option.id} className="min-h-8 rounded-lg text-xs">
              <span className="min-w-0 flex-1">
                <span className={cn("block truncate font-medium", option.id === "dangerous" && "text-destructive")}>{option.label}</span>
                <span className="block truncate text-[10px] text-muted-foreground">{option.description}</span>
              </span>
            </DropdownMenuRadioItem>
          ))}
        </DropdownMenuRadioGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

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

const ACTION_SLOT_EXIT_MS = 180;

function ActionSlot({
  open,
  delayMs = 0,
  children,
}: {
  open: boolean;
  delayMs?: number;
  children: ReactNode;
}) {
  const [mounted, setMounted] = useState(open);
  const [visible, setVisible] = useState(open);

  useEffect(() => {
    const reducedMotion =
      typeof window !== "undefined" &&
      window.matchMedia?.("(prefers-reduced-motion: reduce)").matches;

    if (open) {
      setMounted(true);
      if (reducedMotion) {
        setVisible(true);
        return;
      }
      let raf2 = 0;
      const raf1 = requestAnimationFrame(() => {
        raf2 = requestAnimationFrame(() => setVisible(true));
      });
      return () => {
        cancelAnimationFrame(raf1);
        cancelAnimationFrame(raf2);
      };
    }

    setVisible(false);
    if (reducedMotion) {
      setMounted(false);
      return;
    }
    const timer = setTimeout(() => setMounted(false), ACTION_SLOT_EXIT_MS);
    return () => clearTimeout(timer);
  }, [open]);

  if (!mounted) return null;

  return (
    <div
      className="mew-action-slot"
      data-open={visible ? "true" : "false"}
      aria-hidden={!open || undefined}
      style={open && delayMs > 0 ? ({ "--mew-slot-delay": `${delayMs}ms` } as CSSProperties) : undefined}
    >
      <div>{children}</div>
    </div>
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
