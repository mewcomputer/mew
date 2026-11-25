import { useRouter } from "@tanstack/react-router";
import { AutomationSettings } from "@/components/automation-settings";
import { RemoteSettings } from "@/components/remote-settings";
import { SettingsGroup, SettingsRow } from "@/components/settings-primitives";
import { useState, useMemo } from "react";
import { useSessionStore } from "@/stores/session";
import {
  useTheme,
  TITLE_FONT_CHOICES,
  BODY_FONT_CHOICES,
  type ThemeDef,
  type TitleFontChoiceId,
  type BodyFontChoiceId,
} from "@/lib/theme";
import { getClient } from "@/lib/client";
import { cn } from "@/lib/utils";
import { Input } from "@/components/ui/input";
import {
  AlertTriangle,
  Brain,
  Check,
  ChevronRight,
  Cpu,
  MessageSquare,
  Palette,
  Search,
  Shield,
  SlidersHorizontal,
  Smartphone,
  Sparkles,
} from "lucide-react";

const PERMISSION_MODES = [
  {
    id: "standard",
    label: "Standard",
    description: "Prompt for mutating and dangerous tools. Deny rules apply.",
  },
  {
    id: "permissive",
    label: "Permissive",
    description: "Auto-allow mutating tools. Still prompt on dangerous tools.",
  },
  {
    id: "auto",
    label: "Auto",
    description: "Route every tool call through the classifier LLM.",
  },
  {
    id: "auto_plus",
    label: "Auto+",
    description: "Classifier cannot escalate; escalations and errors deny.",
  },
  {
    id: "dangerous",
    label: "Dangerous!",
    description: "No prompts, no rules, no guards. Use with caution.",
  },
];

const APPEARANCE_MODES = [
  { id: "system", label: "System" },
  { id: "light", label: "Light" },
  { id: "dark", label: "Dark" },
] as const;

const SETTINGS_SECTIONS = [
  {
    id: "general",
    label: "General",
    subtitle: "Manage sessions, active context, and automatic behavior.",
    icon: SlidersHorizontal,
  },
  {
    id: "appearance",
    label: "Appearance",
    subtitle: "Customize interface typography, color mode, and theme.",
    icon: Palette,
  },
  {
    id: "permissions",
    label: "Permissions",
    subtitle: "Choose how tool approvals and safety checks behave.",
    icon: Shield,
  },
  {
    id: "remote",
    label: "Remote & Mobile",
    subtitle: "Pair mobile or desktop devices over iroh.",
    icon: Smartphone,
  },
] as const;
type SettingsSectionId = (typeof SETTINGS_SECTIONS)[number]["id"];

export function SettingsPanel({
  inDialog = false,
  hideSidebarTitle = false,
}: {
  inDialog?: boolean;
  hideSidebarTitle?: boolean;
}) {
  const router = useRouter();
  const connected = useSessionStore((s) => s.connectionState === "connected");
  const {
    themeId,
    titleFontId,
    setTitleFontId,
    bodyFontId,
    setBodyFontId,
    themes,
    themeMode,
    setThemeMode,
    lightTheme,
    darkTheme,
    setLightTheme,
    setDarkTheme,
  } = useTheme();
  const [query, setQuery] = useState("");
  const [activeSection, setActiveSection] = useState<SettingsSectionId>("general");

  const activeMeta = SETTINGS_SECTIONS.find((s) => s.id === activeSection) ?? SETTINGS_SECTIONS[0];

  const { lightCategories, darkCategories, hasMatches } = useMemo(() => {
    const q = query.toLowerCase();
    const light = new Map<string, ThemeDef[]>();
    const dark = new Map<string, ThemeDef[]>();
    let count = 0;
    for (const t of themes) {
      if (themeMode === "light" && t.mode !== "light") continue;
      if (themeMode === "dark" && t.mode !== "dark") continue;
      if (
        q !== "" &&
        !t.name.toLowerCase().includes(q) &&
        !t.id.toLowerCase().includes(q) &&
        !t.description.toLowerCase().includes(q)
      ) {
        continue;
      }
      count++;
      const target = t.mode === "light" ? light : dark;
      const list = target.get(t.category) ?? [];
      list.push(t);
      target.set(t.category, list);
    }
    return { lightCategories: light, darkCategories: dark, hasMatches: count > 0 };
  }, [themes, query, themeMode]);

  return (
    <div className="flex min-h-0 flex-1 overflow-hidden">
      <nav
        aria-label="Settings sections"
        className="hidden w-52 shrink-0 flex-col border-r border-border/80 bg-muted/25 p-3 md:flex"
      >
        {!hideSidebarTitle && (
          <div className="px-2.5 pb-2.5 pt-1 text-xs font-medium text-muted-foreground">
            Settings
          </div>
        )}
        <div className="space-y-0.5">
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
                  "flex w-full items-center gap-2.5 rounded-lg px-2.5 py-2 text-left text-[13px] font-medium transition-colors duration-150 ease-out active:scale-[0.98] motion-reduce:transition-none motion-reduce:active:scale-100",
                  active
                    ? "bg-accent text-foreground shadow-[inset_0_0_0_1px_hsl(var(--border)/0.45)]"
                    : "text-muted-foreground hover:bg-accent/50 hover:text-foreground",
                )}
              >
                <Icon className="h-4 w-4 shrink-0" />
                <span className="truncate">{section.label}</span>
              </button>
            );
          })}
        </div>
      </nav>

      <div className="relative flex min-w-0 flex-1 flex-col overflow-hidden">
        <div className="min-h-0 flex-1 overflow-y-auto">
          <header
            className={cn(
              "sticky top-0 z-10 flex h-12 shrink-0 items-center border-b border-border/60 bg-background/80 px-6 backdrop-blur-md supports-[backdrop-filter]:bg-background/70",
              inDialog && "pr-12",
            )}
          >
            <div className="mx-auto flex w-full max-w-xl items-center justify-between gap-3">
              <h2 className="truncate text-[14px] font-semibold tracking-[-0.01em] text-foreground">
                {activeMeta.label}
              </h2>
            </div>
          </header>

          <div className="px-6 pb-8 pt-5">
            <div className="mx-auto max-w-xl space-y-6">
              <div className="flex gap-1.5 overflow-x-auto pb-1 md:hidden">
                {SETTINGS_SECTIONS.map((section) => (
                  <button
                    key={section.id}
                    type="button"
                    onClick={() => setActiveSection(section.id)}
                    className={cn(
                      "shrink-0 rounded-full border px-3 py-1.5 text-xs font-medium transition-colors active:scale-[0.97]",
                      activeSection === section.id
                        ? "border-primary bg-primary/10 text-foreground"
                        : "border-border text-muted-foreground hover:text-foreground",
                    )}
                  >
                    {section.label}
                  </button>
                ))}
              </div>

              {activeSection === "general" && (
                <>
                  <SettingsGroup title="Sessions">
                    <SettingsRow
                      label="Manage sessions"
                      description="Browse, rename, or remove saved conversations"
                      leading={<MessageSquare className="h-4 w-4 text-muted-foreground" />}
                      trailing={<ChevronRight className="h-4 w-4 text-muted-foreground" />}
                      onClick={() => {
                        window.dispatchEvent(new Event("mew:close-settings"));
                        router.navigate({ to: "/settings/sessions" });
                      }}
                    />
                  </SettingsGroup>

                  <SessionInfoSection />

                  <SettingsGroup
                    title="Behavior"
                    description={
                      !connected
                        ? "Connect to the local daemon to change automatic title and summary settings."
                        : undefined
                    }
                  >
                    <AutomationSettings client={getClient()} connected={connected} />
                  </SettingsGroup>
                </>
              )}

              {activeSection === "permissions" && <PermissionModeSection />}

              {activeSection === "remote" && (
                <RemoteSettings client={getClient()} connected={connected} />
              )}

              {activeSection === "appearance" && (
                <>
                  <SettingsGroup title="Color mode">
                    <SettingsRow
                      label="Appearance mode"
                      description="Match your system preference or lock to light or dark"
                      trailing={
                        <div
                          role="radiogroup"
                          aria-label="Appearance mode"
                          className="inline-flex rounded-lg border border-border/80 bg-muted/60 p-0.5"
                        >
                          {APPEARANCE_MODES.map((option) => {
                            const selected = themeMode === option.id;
                            return (
                              <button
                                key={option.id}
                                type="button"
                                role="radio"
                                aria-checked={selected}
                                onClick={() => setThemeMode(option.id)}
                                className={cn(
                                  "rounded-md px-2.5 py-1 text-xs font-medium transition-all duration-150 ease-out active:scale-[0.97] motion-reduce:transition-none motion-reduce:active:scale-100",
                                  selected
                                    ? "bg-accent text-foreground shadow-xs ring-1 ring-border/70"
                                    : "text-muted-foreground hover:text-foreground",
                                )}
                              >
                                {option.label}
                              </button>
                            );
                          })}
                        </div>
                      }
                    />
                  </SettingsGroup>

                  <section className="space-y-2">
                    <h3 className="px-1 text-xs font-medium text-muted-foreground">
                      Title font
                    </h3>
                    <div className="grid gap-2 sm:grid-cols-2">
                      {TITLE_FONT_CHOICES.map((font) => {
                        const selected = font.id === titleFontId;
                        return (
                          <button
                            key={font.id}
                            type="button"
                            title={font.description}
                            onClick={() => setTitleFontId(font.id as TitleFontChoiceId)}
                            className={cn(
                              "flex items-center justify-between gap-3 rounded-xl border px-3.5 py-2.5 text-left transition-colors duration-150 ease-out active:scale-[0.99] motion-reduce:transition-none motion-reduce:active:scale-100",
                              selected
                                ? "border-primary bg-primary/5 ring-1 ring-primary"
                                : "border-border/80 bg-card/40 hover:bg-accent/40",
                            )}
                          >
                            <span className="flex min-w-0 items-center gap-2">
                              <span className="truncate text-[13px] font-medium text-foreground">
                                {font.name}
                              </span>
                              {selected && (
                                <Check className="h-3.5 w-3.5 shrink-0 text-primary" />
                              )}
                            </span>
                            <span
                              className="shrink-0 text-base text-muted-foreground"
                              style={{ fontFamily: font.family }}
                            >
                              Aa
                            </span>
                          </button>
                        );
                      })}
                    </div>
                  </section>

                  <section className="space-y-2">
                    <h3 className="px-1 text-xs font-medium text-muted-foreground">
                      Body text font
                    </h3>
                    <div className="grid gap-2 sm:grid-cols-2">
                      {BODY_FONT_CHOICES.map((font) => {
                        const selected = font.id === bodyFontId;
                        return (
                          <button
                            key={font.id}
                            type="button"
                            title={font.description}
                            onClick={() => setBodyFontId(font.id as BodyFontChoiceId)}
                            className={cn(
                              "flex items-center justify-between gap-3 rounded-xl border px-3.5 py-2.5 text-left transition-colors duration-150 ease-out active:scale-[0.99] motion-reduce:transition-none motion-reduce:active:scale-100",
                              selected
                                ? "border-primary bg-primary/5 ring-1 ring-primary"
                                : "border-border/80 bg-card/40 hover:bg-accent/40",
                            )}
                          >
                            <span className="flex min-w-0 items-center gap-2">
                              <span className="truncate text-[13px] font-medium text-foreground">
                                {font.name}
                              </span>
                              {selected && (
                                <Check className="h-3.5 w-3.5 shrink-0 text-primary" />
                              )}
                            </span>
                            <span
                              className="shrink-0 text-base text-muted-foreground"
                              style={{ fontFamily: font.family }}
                            >
                              Aa
                            </span>
                          </button>
                        );
                      })}
                    </div>
                  </section>

                  <section className="space-y-3">
                    <h3 className="px-1 text-xs font-medium text-muted-foreground">
                      Theme
                    </h3>
                    <div className="relative">
                      <Search className="pointer-events-none absolute left-3 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground" />
                      <Input
                        type="text"
                        value={query}
                        onChange={(e) => setQuery(e.target.value)}
                        placeholder="Search themes…"
                        className="h-9 rounded-lg pl-8"
                      />
                    </div>

                    {!hasMatches && (
                      <div className="rounded-xl border border-border/80 bg-card/40 py-6 text-center text-xs text-muted-foreground">
                        No themes match &ldquo;{query}&rdquo;
                      </div>
                    )}

                    <div className="space-y-6 pt-1">
                      {(
                        [
                          {
                            mode: "light",
                            label: "Light themes",
                            groups: lightCategories,
                            selectedId: lightTheme,
                            onSelect: setLightTheme,
                          },
                          {
                            mode: "dark",
                            label: "Dark themes",
                            groups: darkCategories,
                            selectedId: darkTheme,
                            onSelect: setDarkTheme,
                          },
                        ] as const
                      ).map(({ mode: sectionMode, label, groups, selectedId, onSelect }) => {
                        if (groups.size === 0) return null;
                        return (
                          <div key={sectionMode} className="space-y-3">
                            <div className="border-b border-border/70 pb-1.5 text-xs font-medium text-foreground">
                              {label}
                            </div>
                            {[...groups.entries()].map(([category, groupThemes]) => (
                              <div key={category} className="space-y-2">
                                <div className="px-0.5 text-[11px] font-medium capitalize text-muted-foreground">
                                  {category}
                                </div>
                                <div className="grid grid-cols-2 gap-2 sm:grid-cols-3">
                                  {groupThemes.map((t) => {
                                    const isSelected = t.id === selectedId;
                                    const isActive = t.id === themeId;
                                    return (
                                      <button
                                        key={t.id}
                                        type="button"
                                        title={t.description}
                                        onClick={() => onSelect(t.id)}
                                        className={cn(
                                          "group flex flex-col gap-1.5 rounded-xl border p-2 text-left transition-[background-color,border-color,box-shadow,transform] duration-150 ease-out active:scale-[0.98] motion-reduce:transition-none motion-reduce:active:scale-100",
                                          isActive
                                            ? "border-primary bg-primary/5 ring-1 ring-primary"
                                            : isSelected
                                              ? "border-primary/60 bg-card/60"
                                              : "border-border/80 bg-card/40 hover:border-foreground/20 hover:bg-accent/50",
                                        )}
                                      >
                                        <div className="flex h-6 gap-1 overflow-hidden rounded-md">
                                          {t.preview.map((color, i) => (
                                            <div
                                              key={i}
                                              className="flex-1"
                                              style={{ backgroundColor: color }}
                                            />
                                          ))}
                                        </div>
                                        <div className="flex items-center justify-between gap-1.5 px-0.5">
                                          <span className="truncate text-xs font-medium text-foreground">
                                            {t.name}
                                          </span>
                                          {isSelected && (
                                            <span
                                              className={cn(
                                                "flex h-4 w-4 shrink-0 items-center justify-center rounded-full",
                                                isActive ? "bg-primary" : "bg-muted",
                                              )}
                                            >
                                              <Check
                                                className={cn(
                                                  "h-2.5 w-2.5",
                                                  isActive
                                                    ? "text-primary-foreground"
                                                    : "text-muted-foreground",
                                                )}
                                              />
                                            </span>
                                          )}
                                        </div>
                                      </button>
                                    );
                                  })}
                                </div>
                              </div>
                            ))}
                          </div>
                        );
                      })}
                    </div>
                  </section>
                </>
              )}
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}

function PermissionModeSection() {
  const permissionMode = useSessionStore((s) => s.permissionMode);

  const handleSelect = (mode: string) => {
    getClient().setPermissionMode(mode);
  };

  return (
    <SettingsGroup
      title="Permission mode"
      description="Applies to the active conversation and newly created sessions using default policy."
    >
      <div role="radiogroup" aria-label="Permission mode" className="divide-y divide-border/60">
        {PERMISSION_MODES.map((m) => {
          const active = m.id === permissionMode;
          const isDangerous = m.id === "dangerous";
          return (
            <SettingsRow
              key={m.id}
              role="radio"
              ariaChecked={active}
              onClick={() => handleSelect(m.id)}
              leading={
                isDangerous ? (
                  <AlertTriangle
                    className={cn(
                      "h-4 w-4",
                      active ? "text-text-error" : "text-text-error/85",
                    )}
                  />
                ) : undefined
              }
              label={m.label}
              labelClassName={cn(
                active && isDangerous
                  ? "text-text-error"
                  : active
                    ? "text-primary"
                    : isDangerous
                      ? "text-text-error/90"
                      : undefined,
              )}
              description={m.description}
              className={cn(
                active && (isDangerous ? "bg-surface-error/50" : "bg-primary/5"),
              )}
              trailing={
                <span
                  className={cn(
                    "flex h-4 w-4 items-center justify-center rounded-full border transition-colors",
                    active
                      ? isDangerous
                        ? "border-text-error bg-destructive text-destructive-foreground"
                        : "border-primary bg-primary text-primary-foreground"
                      : "border-border",
                  )}
                >
                  {active && <Check className="h-2.5 w-2.5" />}
                </span>
              }
            />
          );
        })}
      </div>
    </SettingsGroup>
  );
}

function SessionInfoSection() {
  const currentModel = useSessionStore((s) => s.currentModel);
  const currentThinkingVariant = useSessionStore((s) => s.currentThinkingVariant);
  const currentPersona = useSessionStore((s) => s.currentPersona);

  const items: { icon: React.ReactNode; label: string; value: string }[] = [];
  if (currentModel) {
    items.push({
      icon: <Cpu className="h-4 w-4 text-muted-foreground" />,
      label: "Model",
      value: currentModel,
    });
  }
  if (currentThinkingVariant) {
    items.push({
      icon: <Brain className="h-4 w-4 text-muted-foreground" />,
      label: "Thinking",
      value: currentThinkingVariant,
    });
  }
  if (currentPersona) {
    items.push({
      icon: <Sparkles className="h-4 w-4 text-muted-foreground" />,
      label: "Persona",
      value: currentPersona,
    });
  }

  if (items.length === 0) return null;

  return (
    <SettingsGroup title="Current session">
      {items.map((item) => (
        <SettingsRow
          key={item.label}
          leading={item.icon}
          label={item.label}
          trailing={
            <span className="max-w-[16rem] truncate font-mono text-xs text-muted-foreground">
              {item.value}
            </span>
          }
        />
      ))}
    </SettingsGroup>
  );
}
