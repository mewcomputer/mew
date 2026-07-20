import { createFileRoute, useRouter } from "@tanstack/react-router";
import { useState, useMemo, useEffect } from "react";
import { useSessionStore } from "@/stores/session";
import { useTheme, THEMES, FONT_CHOICES, type ThemeDef, type FontChoiceId } from "@/lib/theme";
import { getClient } from "@/lib/client";
import { desktopRemoteEnabled, isDesktopHost, setDesktopRemoteEnabled } from "@/lib/host";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { ChevronLeft, ChevronRight, Search, Check, MessageSquare, Shield, Cpu, Brain, Sparkles, Radio, Palette, SlidersHorizontal } from "lucide-react";

export const Route = createFileRoute("/settings")({
  component: SettingsRouteComponent,
});

const PERMISSION_MODES = [
  {
    id: "standard",
    label: "Standard",
    description: "Prompt for mutating/dangerous tools. Deny rules apply.",
  },
  {
    id: "permissive",
    label: "Permissive",
    description: "Auto-allow mutating tools. Still prompt on dangerous.",
  },
  {
    id: "auto",
    label: "Auto",
    description: "Route every tool call through the classifier LLM.",
  },
  {
    id: "auto_plus",
    label: "Auto+",
    description: "Classifier cannot escalate — escalate/failure = deny.",
  },
  {
    id: "dangerous",
    label: "Dangerous!",
    description: "No prompts, no rules, no guards. You have been warned.",
  },
];

const SETTINGS_SECTIONS = [
  { id: "general", label: "General", description: "Sessions and behavior", icon: SlidersHorizontal },
  { id: "appearance", label: "Appearance", description: "Fonts and themes", icon: Palette },
  { id: "permissions", label: "Permissions", description: "Tool approval behavior", icon: Shield },
  { id: "remote", label: "Remote access", description: "Connect to this daemon remotely", icon: Radio },
] as const;
type SettingsSectionId = (typeof SETTINGS_SECTIONS)[number]["id"];

function SettingsRouteComponent() {
  const router = useRouter();
  const sessionId = useSessionStore((s) => s.sessionId);
  const { themeId, setThemeId, fontId, setFontId } = useTheme();
  const [query, setQuery] = useState("");
  const [activeSection, setActiveSection] = useState<SettingsSectionId>("general");

  const categories = useMemo(() => {
    const map = new Map<string, ThemeDef[]>();
    for (const t of THEMES) {
      const list = map.get(t.category) ?? [];
      list.push(t);
      map.set(t.category, list);
    }
    return map;
  }, []);

  const filtered = useMemo(() => {
    const result = new Map<string, ThemeDef[]>();
    const q = query.toLowerCase();
    for (const [cat, list] of categories) {
      const matches =
        q === ""
          ? list
          : list.filter(
              (t) =>
                t.name.toLowerCase().includes(q) ||
                t.id.toLowerCase().includes(q) ||
                t.description.toLowerCase().includes(q),
            );
      if (matches.length > 0) result.set(cat, matches);
    }
    return result;
  }, [categories, query]);

  return (
    <>
      <div className="flex items-center gap-2 px-3 py-1.5">
        <Button
          variant="ghost"
          size="icon"
          className="h-7 w-7"
          onClick={() => {
            if (sessionId) {
              router.navigate({ to: "/session/$sessionId", params: { sessionId } });
            } else {
              router.navigate({ to: "/" });
            }
          }}
          title="Back to chat"
        >
          <ChevronLeft className="h-4 w-4" />
        </Button>
        <span className="text-xs font-medium text-muted-foreground">Settings</span>
      </div>

      <div className="flex min-h-0 flex-1 overflow-hidden">
        <nav aria-label="Settings sections" className="hidden w-56 shrink-0 border-r border-border p-3 md:block">
          <div className="mb-3 px-2 text-[10px] font-semibold uppercase tracking-[0.14em] text-muted-foreground">Settings</div>
          <div className="space-y-1">
            {SETTINGS_SECTIONS.map((section) => {
              const Icon = section.icon;
              const active = activeSection === section.id;
              return (
                <button
                  key={section.id}
                  type="button"
                  onClick={() => setActiveSection(section.id)}
                  aria-current={active ? "page" : undefined}
                  className={cn(
                    "flex w-full items-start gap-2 rounded-md px-2.5 py-2 text-left transition-colors",
                    active ? "bg-accent text-foreground" : "text-muted-foreground hover:bg-accent/50 hover:text-foreground",
                  )}
                >
                  <Icon className="mt-0.5 h-3.5 w-3.5 shrink-0" />
                  <span className="min-w-0">
                    <span className="block text-xs font-medium">{section.label}</span>
                    <span className="block truncate text-[10px] opacity-70">{section.description}</span>
                  </span>
                </button>
              );
            })}
          </div>
        </nav>

        <div className="min-w-0 flex-1 overflow-y-auto p-4">
          <div className="mb-4 flex gap-1 overflow-x-auto md:hidden">
            {SETTINGS_SECTIONS.map((section) => (
              <button
                key={section.id}
                type="button"
                onClick={() => setActiveSection(section.id)}
                className={cn(
                  "shrink-0 rounded-full border px-3 py-1.5 text-[11px]",
                  activeSection === section.id ? "border-primary bg-primary/10 text-foreground" : "border-border text-muted-foreground",
                )}
              >
                {section.label}
              </button>
            ))}
          </div>
          <div className="mx-auto max-w-2xl space-y-6">
            {activeSection === "general" && <>
              <SettingsSection title="Sessions" icon={<MessageSquare className="h-4 w-4 text-muted-foreground" />}>
                <button
                  onClick={() => router.navigate({ to: "/settings/sessions" })}
                  className="flex w-full items-center justify-between rounded-lg border border-border p-3 text-left transition-colors hover:bg-accent/30"
                >
                  <div>
                    <div className="text-xs font-medium text-foreground">Manage sessions</div>
                    <div className="text-[10px] text-muted-foreground">List, rename, and delete sessions</div>
                  </div>
                  <ChevronRight className="h-4 w-4 text-muted-foreground" />
                </button>
              </SettingsSection>
              <SessionInfoSection />
              <SettingsSection title="Behavior">
                <AutoTitleToggle />
                <AutoSummaryToggle />
              </SettingsSection>
            </>}

            {activeSection === "permissions" && <PermissionModeSection />}

            {activeSection === "appearance" && <>
          <SettingsSection title="Font">
            <div className="grid gap-2 sm:grid-cols-2">
              {FONT_CHOICES.map((font) => (
                <button
                  key={font.id}
                  onClick={() => setFontId(font.id as FontChoiceId)}
                  className={cn(
                    "flex items-center justify-between rounded-lg border p-3 text-left transition-colors",
                    font.id === fontId
                      ? "border-primary bg-primary/5 ring-1 ring-primary"
                      : "border-border hover:bg-accent/30",
                  )}
                >
                  <div className={cn("min-w-0", font.id === "junicode" && "font-serif", font.id === "goudy" && "font-serif")}>
                    <div className="text-sm font-medium text-foreground">{font.name}</div>
                    <div className="mt-0.5 text-[10px] text-muted-foreground">{font.description}</div>
                    <div className="mt-2 text-sm text-foreground">The quick brown fox</div>
                  </div>
                  {font.id === fontId && <Check className="h-4 w-4 shrink-0 text-primary" />}
                </button>
              ))}
            </div>
            <p className="text-[10px] text-muted-foreground">Changes apply immediately and are saved on this device.</p>
          </SettingsSection>
          <SettingsSection title="Theme" icon={<Sparkles className="h-4 w-4 text-muted-foreground" />}>
            <div className="relative">
              <Search className="pointer-events-none absolute left-3 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground" />
              <Input
                type="text"
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                placeholder="Search themes…"
                className="h-9 pl-8"
              />
            </div>

            {filtered.size === 0 && (
              <div className="py-8 text-center text-xs text-muted-foreground">
                No themes match &ldquo;{query}&rdquo;
              </div>
            )}

            {[...filtered.entries()].map(([category, themes]) => (
              <div key={category}>
                <div className="mb-2 text-[10px] font-semibold uppercase tracking-wide text-muted-foreground">
                  {category}
                </div>
                <div className="grid grid-cols-2 gap-2 sm:grid-cols-3">
                  {themes.map((t) => (
                    <button
                      key={t.id}
                      onClick={() => setThemeId(t.id)}
                      className={cn(
                        "group relative flex flex-col gap-1.5 rounded-lg border p-2.5 text-left transition-[background-color,border-color,box-shadow] duration-150 ease-out",
                        t.id === themeId
                          ? "border-primary ring-1 ring-primary"
                          : "border-border hover:border-foreground/20 hover:bg-accent",
                      )}
                    >
                      <div className="flex h-8 gap-1 overflow-hidden rounded-md">
                        {t.preview.map((color, i) => (
                          <div key={i} className="flex-1" style={{ backgroundColor: color }} />
                        ))}
                      </div>
                      <div className="min-w-0">
                        <div className="truncate text-xs font-medium text-foreground">{t.name}</div>
                        <div className="truncate text-[10px] text-muted-foreground">
                          {t.description}
                        </div>
                      </div>
                      {t.id === themeId && (
                        <div className="absolute right-2 top-2 flex h-4 w-4 items-center justify-center rounded-full bg-primary">
                          <Check className="h-2.5 w-2.5 text-primary-foreground" />
                        </div>
                      )}
                    </button>
                  ))}
                </div>
              </div>
            ))}
          </SettingsSection>
            </>}

            {activeSection === "remote" && <RemoteAccessSection />}
          </div>
        </div>
      </div>
    </>
  );
}

function SettingsSection({
  title,
  icon,
  children,
}: {
  title: string;
  icon?: React.ReactNode;
  children: React.ReactNode;
}) {
  return (
    <div className="space-y-2">
      <div className="flex items-center gap-1.5 px-0.5">
        {icon}
        <h2 className="text-xs font-semibold text-foreground">{title}</h2>
      </div>
      <div className="space-y-1.5">{children}</div>
    </div>
  );
}

function RemoteAccessSection() {
  const desktop = isDesktopHost();
  const [enabled, setEnabled] = useState(false);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (desktop) {
      desktopRemoteEnabled().then((value) => {
        setEnabled(value);
        localStorage.setItem("mew.remote.enabled", String(value));
      }).catch(() => undefined);
    }
  }, [desktop]);

  const setRemoteEnabled = async (next: boolean) => {
    if (!desktop) {
      window.alert("Remote access is only available in the desktop app.");
      return;
    }
    if (busy) return;
    if (next && !window.confirm("Remote access lets a paired device control this computer's mew daemon, including sessions, files, commands, and permission requests. Continue?")) return;
    setBusy(true);
    try {
      await setDesktopRemoteEnabled(next);
      setEnabled(next);
      localStorage.setItem("mew.remote.enabled", String(next));
      window.setTimeout(() => window.location.reload(), 50);
    } catch (error) {
      window.alert(error instanceof Error ? error.message : "Could not change remote access.");
    } finally {
      setBusy(false);
    }
  };

  return (
    <SettingsSection title="Remote access" icon={<Radio className="h-4 w-4 text-muted-foreground" />}>
      <div className="rounded-lg border border-amber-500/40 bg-amber-500/10 p-3 text-xs text-foreground">
        <div className="font-medium">remote access shares this daemon over the network</div>
        <p className="mt-1 text-[11px] leading-relaxed text-muted-foreground">
          A paired device receives full control of this daemon, including sessions, files,
          prompts, permission requests, and commands. iroh may use a relay when a direct
          connection is unavailable. Only enable this for a device you trust.
        </p>
      </div>

      <div className="rounded-lg border border-border p-3">
        <div className="flex items-center justify-between gap-3">
          <div>
            <div className="text-xs font-medium text-foreground">Allow remote connections</div>
            <div className="text-[10px] text-muted-foreground">
              {!desktop ? "Available when running the macOS app" : enabled ? "Remote access is enabled while this app is open" : "Remote access is off by default"}
            </div>
          </div>
          <button
            type="button"
            role="switch"
            aria-checked={enabled}
            disabled={!desktop || busy}
            onClick={() => void setRemoteEnabled(!enabled)}
            className={cn("relative h-5 w-9 shrink-0 rounded-full transition-colors", enabled ? "bg-primary" : "bg-muted", (!desktop || busy) && "cursor-not-allowed opacity-50")}
          >
            <span className={cn("absolute top-0.5 h-4 w-4 rounded-full bg-background transition-transform", enabled ? "translate-x-4" : "translate-x-0.5")} />
          </button>
        </div>
        {enabled && desktop && (
          <div className="mt-3 space-y-3 border-t border-border pt-3">
            <div className="rounded-md bg-muted/40 px-2.5 py-2 text-[10px] text-muted-foreground">
              desktop remote is active only while this app-owned daemon is running. For a long-lived
              VPS daemon, start <code className="rounded bg-background px-1">mew daemon --remote</code> and
              pair from that machine instead.
            </div>
            <div className="flex items-center justify-between rounded-md bg-muted/40 px-2.5 py-2 text-[10px] text-muted-foreground">
              <span>pairing</span>
              <span>one-time control invite</span>
            </div>
            <div className="rounded-md border border-border px-3 py-2 text-[10px] leading-relaxed text-muted-foreground">
              Pairing currently runs from the daemon host. Run
              <code className="mx-1 rounded bg-muted px-1">mew pair</code>
              in a terminal and only approve the device you intend to trust.
            </div>
          </div>
        )}
      </div>
    </SettingsSection>
  );
}

function PermissionModeSection() {
  const permissionMode = useSessionStore((s) => s.permissionMode);

  const handleSelect = (mode: string) => {
    getClient().setPermissionMode(mode);
  };

  return (
    <SettingsSection title="Permission Mode" icon={<Shield className="h-4 w-4 text-muted-foreground" />}>
      <div className="space-y-1.5">
        {PERMISSION_MODES.map((m) => {
          const active = m.id === permissionMode;
          const isDangerous = m.id === "dangerous";
          return (
            <button
              key={m.id}
              onClick={() => handleSelect(m.id)}
              className={cn(
                "flex w-full items-start justify-between rounded-lg border p-3 text-left transition-colors",
                active
                  ? isDangerous
                    ? "border-destructive ring-1 ring-destructive"
                    : "border-primary ring-1 ring-primary"
                  : "border-border hover:bg-accent/30",
              )}
            >
              <div className="min-w-0 flex-1">
                <div
                  className={cn(
                    "text-xs font-medium",
                    active && isDangerous
                      ? "text-destructive"
                      : active
                        ? "text-primary"
                        : "text-foreground",
                  )}
                >
                  {m.label}
                </div>
                <div className="text-[10px] text-muted-foreground">{m.description}</div>
              </div>
              {active && (
                <Check
                  className={cn(
                    "h-3.5 w-3.5 shrink-0",
                    isDangerous ? "text-destructive" : "text-primary",
                  )}
                />
              )}
            </button>
          );
        })}
      </div>
    </SettingsSection>
  );
}

function SessionInfoSection() {
  const currentModel = useSessionStore((s) => s.currentModel);
  const currentThinkingVariant = useSessionStore((s) => s.currentThinkingVariant);
  const currentPersona = useSessionStore((s) => s.currentPersona);

  const items: { icon: React.ReactNode; label: string; value: string }[] = [];
  if (currentModel) {
    items.push({
      icon: <Cpu className="h-3.5 w-3.5 text-muted-foreground" />,
      label: "Model",
      value: currentModel,
    });
  }
  if (currentThinkingVariant) {
    items.push({
      icon: <Brain className="h-3.5 w-3.5 text-muted-foreground" />,
      label: "Thinking",
      value: currentThinkingVariant,
    });
  }
  if (currentPersona) {
    items.push({
      icon: <Sparkles className="h-3.5 w-3.5 text-muted-foreground" />,
      label: "Persona",
      value: currentPersona,
    });
  }

  if (items.length === 0) return null;

  return (
    <SettingsSection title="Current Session">
      <div className="grid grid-cols-1 gap-1.5 sm:grid-cols-3">
        {items.map((item) => (
          <div
            key={item.label}
            className="flex items-center gap-2 rounded-lg border border-border p-3"
          >
            {item.icon}
            <div className="min-w-0">
              <div className="text-[10px] text-muted-foreground">{item.label}</div>
              <div className="truncate text-xs font-medium text-foreground">{item.value}</div>
            </div>
          </div>
        ))}
      </div>
    </SettingsSection>
  );
}

function AutoTitleToggle() {
  const [enabled, setEnabled] = useState(() => {
    const stored = localStorage.getItem("mew.autoTitle");
    return stored !== "false"; // default: enabled
  });

  const handleToggle = (next: boolean) => {
    setEnabled(next);
    localStorage.setItem("mew.autoTitle", String(next));
    getClient().setAutoTitle(next);
  };

  return (
    <div className="flex items-center justify-between rounded-lg border border-border p-3">
      <div>
        <div className="text-xs font-medium text-foreground">
          Auto-generate session titles
        </div>
        <div className="text-[10px] text-muted-foreground">
          Creates a short AI title after the first message of each session
        </div>
      </div>
      <button
        onClick={() => handleToggle(!enabled)}
        className={cn(
          "relative h-5 w-9 shrink-0 rounded-full transition-colors",
          enabled ? "bg-primary" : "bg-muted",
        )}
        title={enabled ? "Disable" : "Enable"}
      >
        <span
          className={cn(
            "absolute top-0.5 h-4 w-4 rounded-full bg-background transition-transform",
            enabled ? "translate-x-4" : "translate-x-0.5",
          )}
        />
      </button>
    </div>
  );
}

function AutoSummaryToggle() {
  const [enabled, setEnabled] = useState(() => {
    const stored = localStorage.getItem("mew.autoSummary");
    return stored !== "false"; // default: enabled
  });

  const handleToggle = (next: boolean) => {
    setEnabled(next);
    localStorage.setItem("mew.autoSummary", String(next));
    getClient().setAutoSummary(next);
  };

  return (
    <div className="flex items-center justify-between rounded-lg border border-border p-3">
      <div>
        <div className="text-xs font-medium text-foreground">
          Auto-summarize idle sessions
        </div>
        <div className="text-[10px] text-muted-foreground">
          Generates a 1-2 sentence summary after 10 minutes of inactivity
        </div>
      </div>
      <button
        onClick={() => handleToggle(!enabled)}
        className={cn(
          "relative h-5 w-9 shrink-0 rounded-full transition-colors",
          enabled ? "bg-primary" : "bg-muted",
        )}
        title={enabled ? "Disable" : "Enable"}
      >
        <span
          className={cn(
            "absolute top-0.5 h-4 w-4 rounded-full bg-background transition-transform",
            enabled ? "translate-x-4" : "translate-x-0.5",
          )}
        />
      </button>
    </div>
  );
}
